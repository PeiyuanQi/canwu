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
  and never treats knowledge or a calendar year as capability. A new execution
  or facility project must cite the current version of each record when it is
  applied, because `seal_evidence` can remove an earlier version's body that
  exact replay still reads.
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
That locked version predates the execution, and every later production
transition, including a live `CompleteExecution`, advances the runtime record.
The output credit therefore cites the exact version the output dispatch pinned
on the execution as `output_source`, which proves the completion; the resource
plugin accepts that source as the locked record at or after its locked version
and requires it to equal the pinned source. The credit settles at the dispatch
boundary's time, at or after the execution's start.
Production consumes its package grant when work starts and completes it only
after cancellation or the terminal resource output acknowledgement; there is no
caller-authored completion flag. Callers submit capacity allocations as
`Reserved`; the Phase 7 lifecycle writer alone transitions them to `Consumed`
when the execution starts.

`CompleteExecution` may record a realized output ratio in thousandths of the
nominal output. Phase 7 floor-scales every output settlement quantity by
`realized_output_per_mille` and stores the ratio and its exact
`realization_evidence` record version on the execution; `None` is the nominal
1,000, and evidence is rejected for a nominal ratio. Any other ratio must be
positive (record a total loss by cancelling the work order), must not scale an
output leg to zero, may not exceed the process revision's
`max_realized_per_mille` (default 1,000; a higher bound admits evidenced yields
above nominal), and requires evidence of a kind listed in the process
revision's `realization_evidence_kinds` (default empty, so nominal only). The
holder, lifecycle, ratio, and kind rules run before the evidence is resolved,
so a rejection does not reveal whether other records exist. The evidence must
then be the current exact version of its record, which a sealed run and its
exact replay resolve alike; an earlier version is rejected even while it is
retained.
The resource credit and output acknowledgement settle exactly the scaled
quantities, so production and resource balances stay conserved.

Facility construction and repair use the same authoritative completion path.
A project names an existing planned/authorized or repairing facility generation,
exact consumed resource inputs, exact provider and technology evidence, one
activated certificate, and matching production/resource grants at its certified
eligibility time. Creation validates the provider and technology evidence;
progress rechecks only the live resource evidence, because the cited exact
bodies cannot change. The reducer consumes the production grant on first
progress, derives the commissioned next-generation operational asset and restored
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
archive state is present. Validation audits the production archive (the whole
directory chain, every pending-retention object, and the head's lookup index,
rebuilt from the chain), resolves active resource consumption/outcome evidence
from either hot state or the resource archive's lookup index, resolves exact
technology bodies, and verifies output acknowledgement identity and digest
before replacing live authority. Each compacted resource record costs one
index path and one blob, however long the resource archive chain is. The
provider-free helpers reject archive-bearing candidates rather than silently
accepting an unauthenticated root. The `_with_archives` restores take each
store as `Arc<dyn ... + Send + Sync>` and attach both to the run: the run and
its forks share them, and a fork may move to another thread. Terminal production records retain exact
identity/digest receipts; this crate does not reconstruct missing material
evidence or invent a migration.

Terminal executions and commissioned facility projects move through the
package-owned archive protocol:
`prepare_production_archive(candidate_limit, store)`,
`PreparedProductionArchiveBatchV1::store_and_verify`,
`enqueue_production_archive`, and `finalize_production_archive_retention`.

- One commit carries up to `MAX_ARCHIVE_PREPARE_CANDIDATES` (1,024) records
  as up to `MAX_ARCHIVE_COMMIT_PAGES` (16) blobs of up to
  `MAX_ARCHIVE_PAGE_ENTRIES` (64), each with its own membership and temporal
  page, within the object and byte budgets of one batch.
- Each commit directory names the root of a cumulative lookup index over every
  archived record (`lookup_root`), in the page format of the resource
  archive's index and stored under `PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE`.
  `archived_production_terminal_record` reads the head directory, one page per
  index level, and on a hit one blob, whatever the number of commits.
- The maintenance receipt of an applied commit is an archive candidate, and
  the next commit archives it first, so the root holds at most one and a run
  can make any number of commits. The receipt of a stale commit is not a
  candidate; the acknowledgement of its retention handle retires it, and a
  repeated acknowledgement settles as a no-op.
- `enqueue_production_archive` requires the batch to equal what the current
  root and the store derive. The boundary reads no archive object: it settles
  archive commits before the plugin's other packets, applies a commit whose
  source root still matches after rebuilding its blobs and directory from the
  root's records, and records any other as stale without failing.
- The host application acknowledges each settled commit;
  `prepare_production_archive` refuses as `archive_backpressure` while 64
  retention handles wait for it. The head commit's receipt is always a
  candidate, so a host application commits when `archive_due_index` or
  `project_archive_due_index` holds records; a batch that selects no
  execution and no project archives only that receipt.
- Building an execution's archive record scans the root's hot operation
  outcomes and completion receipts, so preparing, enqueueing, and settling a
  commit costs time in proportion to the records it selects times the hot
  outcomes of the root. A host application that commits small batches often
  keeps both small.

Every hot execution and facility project, terminal ones included, declares in
its payload-required evidence continuation the record versions restore
validation reads for it, until the archive removes it, so a seal never drops
them first. An ended execution or project no longer declares its inputs'
consumer evidence, which restore compares by reference only. A settled
output's pinned production source is not read back: the acknowledgement must
name exactly the source the dispatch pinned, and the exact resource outcome
cites it. Restore rebuilds a hot settled execution's request digests and
acknowledgement digest at `output_settled_at`, as the archive does.

The host application can queue the public packet types, the operation packet
and the output acknowledgement among them, and any completion coordinator
step through `enqueue_production_completion_operation`; another plugin can
schedule any production packet type. A packet that does not decode settles
nothing and emits `canwu.production.ingress_refused.v1`
(`canwu.production.output_ack_refused.v1` for an acknowledgement); its
boundary settles. An operation packet settles only when the admission of a
tracked production command queued it, which checked the issuer's authority,
the run policy, the outcome capacity, and any degraded-facility decision: its
cause must be that command and the command's payload its envelope. Any other
operation packet is refused the same way, and so is a completion step or a
continuation witness that production's checks refuse with an expected domain
rejection.

A production boundary can still fail on an archive commit or retention
acknowledgement whose content is forged but decodes (only another plugin can
send one), on a failed read of the evidence or archive stores, which settles
once the store serves the object, on counter overflow, and on the 256 MiB
byte cap of the root. Any admitted production packet, a refused one
included, also triggers the incident and report systems, so a host
application that queues junk public packets causes incident draws; that
behavior is older than this crate's 0.16.0 changes.

The resource plugin may defer an output batch that finds no room and settle
it later. The acknowledgement carries `settled_at`, the instant the credits
settled; every outcome's request digest must commit to it, and the execution
records it as `output_settled_at`, from which the archive rebuilds those
digests. An acknowledgement that does not settle its execution exactly is
refused with a `canwu.production.output_ack_refused.v1` event and changes
nothing; it never fails the boundary.

`StartExecution` refuses, as a recorded rejection, an output whose operation
key the resource plugin would never credit: a key under the prefix
`canwu-resource` reserves for its completion operations (`invalid_payload`), a
key another running or output-pending execution names, or a key a resource
outcome already holds in the resource root or its archive
(`idempotency_conflict`). A production completion lease refuses a reserved key
too, because its key is its execution's first output key. The resource
plugin settles all of an execution's outputs in one batch, so a process may
declare at most `canwu_resource::MAX_PRODUCTION_OUTPUT_BATCH_CREDITS` (64)
outputs, `StartExecution` refuses more output requests (`invalid_payload`),
and it refuses an output set the resource root can never hold under its
limits (`invalid_payload`). A resource operation that the host application
queues under an output key after the execution started is not caught, nor is
a credit the resource root rejects or a batch that meets its byte cap after
the longest deferral: the resource plugin then skips the batch, and the
execution waits.

The `g3_contract` tests demonstrate:

- the same schema expressing household hand work and a machine/fuel process;
- exact fuel, maintenance, access, and organization blockers;
- rejection of overlapping capacity;
- WIP completion followed by later resource-owned output settlement, and
  an output batch the resource plugin deferred that settles later and whose
  execution archives;
- evidence-bounded realized output that floor-scales output settlement;
- real persisted/forked/replayed degraded-facility decision branches;
- operation-keyed incident draws with snapshot/replay/tamper checks;
- distinct operator and delayed remote-owner observation cuts; and
- archive restart, retention acknowledgement, damage/waste/output evidence,
  and archive-object tamper rejection.

The `production_archive_volume` tests show a commit of several pages whose
every record resolves through the lookup index, a chain of commits that keeps
one receipt and one handle in the root, stale commits that settle as no-ops
against a full handle budget or as a duplicate of the same batch, and a
commit that applies although an operation was admitted before it.
`production-archive-profile` in the performance harness runs the chain past
8,192 commits. The `production_output_key_admission` tests show the refused
output keys and output sets.
