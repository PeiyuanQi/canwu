# Production at Series Scale

Status: implemented in 0.18.0, in `canwu-production` and the
`production-archive-profile` binary of `benchmarks/performance-harness`. It
closes the production side of downstream gap G-18 (canonical caps, a batching
contract for monthly settlement mutations) and the engine-side finding E-23
(plugin-scheduled packets that fail a boundary, stale-commit receipts,
incident draws for refused packets, and the cost of settling an archive
commit). `canwu-resource` is unchanged; its side of G-18 is its own package.

It builds on [production archive volume and lookup](production-archive-volume.md)
(0.16.0), whose open decisions and follow-up it resolves except where
[Known limits](#known-limits) says otherwise, and on the lifecycle of the
[production economy mechanism](production-economy-mechanism.md).

## Problem

Production kept its whole state in one domain record. The profile measured
what that costs before this change (release build, a shared machine; the
`scale` preset of `production-archive-profile`):

| Record | Encoded bytes in the root |
| --- | --- |
| a site with one facility | 482 |
| a running execution with its work order, allocation, work in progress, lease, grants, certificate, and receipts | 12,329 |
| a settled execution, the same closure | 14,023 |
| the outcome of one `StartExecution` (it embeds the whole command) | 4,830 |

| Root (2,048 sites) | Bytes | Decode | Validate | Encode | Clone |
| --- | --- | --- | --- | --- | --- |
| no work | 0.99 MB | 6 ms | 1.5 ms | 9 ms | 1.1 ms |
| 1,024 running executions | 13.6 MB | 41 ms | 33 ms | 107 ms | 20 ms |
| 2,048 running executions | 26.3 MB | 82 ms | 177 ms | 339 ms | 175 ms |

Every production system decoded and validated the whole root, the lifecycle
system copied and validated it twice per operation and reloaded it for each
operation's evidence check, and every command admission decoded and
validated it again. A boundary of tracked `CreateWorkOrder` commands on the
0.99 MB root took 288 ms and journaled 2.0 MB for one command, and 2.39 s for
64; on the 13.6 MB root, 1.9 s and 27 MB for one, 6.7 s for 16. Every
operation also had to expect the root's exact runtime revision, so the
commands of one boundary had to predict each other's revisions.

The caps (2,048 sites and processes, 4,096 facilities, work orders,
executions, allocations, and mutations per boundary) were `const` and
validated whole: a county-level world of several hundred counties and tens of
thousands of sites did not fit, and at 25,600 sites the catalog alone would
be about 12 MB rewritten and journaled by every production boundary. The
4,096-mutation cap was enforced nowhere.

Reading the code for this package found more limits than the ledger row
names, most of them lifetime limits:

- The outcome of a `RequestCompletionLease`, an `AbortCompletionLease`, a
  `RetireFacility`, and any rejected operation naming no hot record never left
  the root: the archive took only outcomes naming an archived execution, work
  order, or project. Every execution needs a lease, so a root admitted no new
  operation after about 8,192 executions in its life.
- A work order cancelled before it had an execution, and a completion lease
  released or expired without one, never left the root either.
- An execution archived every outcome naming its work order, including a
  rejected `StartExecution` that named another execution, which its archive
  validation then refused: the archive stalled on that record.
- Incident receipts stayed for life; the 8,193rd failed its boundary.
- Every observation head rolled out of a scope's window of 16 stayed in the
  root for life; once 8,192 had rolled out, report publication failed its
  boundary.
- A command naming a missing root was rejected with `DomainRecordNotFound`,
  which the engine does not record as a command rejection: it failed the
  boundary.
- The outcome cap was checked at admission only, so two commands admitted at
  one free slot settled into one slot and the second failed the boundary.
- The report system published without reading the engine's publication
  budget.

E-23, from the 0.16.0 run: a well-formed archive commit or retention
acknowledgement that another plugin schedules and production's checks refuse
failed its boundary (a completion step or continuation witness did so for
the codes outside the expected-rejection list); every stale settlement of a
batch queued again left one more receipt, bounded only by the 256 MiB root
cap; any admitted production packet, a refused one included, ran incident
draws and report publication; and settling a commit cost the selected
records times the hot outcomes.

## Decision

### Scope roots

A host application splits its sites into scopes (for example one per county)
and installs one production root per scope with the scenario: a whole
`ProductionState` whose new field `scope` names it, persisted under
`production_scope_runtime_reference(scope)`, the record
`canwu.production:runtime:scope:{scope}` of the production runtime kind. The
primary root `canwu.production:runtime` (`scope` absent) is unchanged, so a
small world needs nothing new. A root holds every site, facility, work order,
execution, facility project, completion lease, outcome, report line,
incident receipt, and archive chain of its scope and nothing of another, so
the whole-root logic and its validation are unchanged and roots settle
independently. Sharding by a hash of the site was rejected: a monthly
settlement runs county by county, and hash shards would put every county's
sites in every shard, so every spread-day boundary would touch every shard.

Every packet names its root. `ProductionCommandEnvelope.scope` and the new
batch envelope carry it; completion steps, continuation witnesses, retention
acknowledgements, and wakes carry a `scope` member beside their payload
(`enqueue_production_completion_operation_in_scope`,
`enqueue_production_resource_continuation_in_scope`); archive batches and
commits carry `scope` (`PreparedProductionArchiveBatchV1.scope`,
`VerifiedProductionArchiveCommitV1.scope`, `ProductionArchiveIngressReceiptV1.scope`).
An output acknowledgement is routed by the production source its execution
pinned, and the resource plugin's participant-completion acknowledgement,
which carries no scope, by the participant's coordinator source. The resource
plugin reads a production root by the record a lease or credit names and
declares the production runtime kind, so it reads every root without change.
A packet naming no installed root is refused with
`canwu.production.ingress_refused.v1`; a command naming none is a recorded
command rejection (`InvalidPayload`).

Roots share no identity: a scope root's completion acquisition ids and lease
and output operation keys begin with `{scope}/`, and the primary root's hold
no `/` (`production_root_names_key`). A scope id holds no `/`, so no two roots
can name one resource-side key, and an output key of one root can never
strand an execution of another. A lease or a start that breaks the rule is a
recorded rejection (`invalid_payload`), and every root's validation checks
its hot leases and executions. A root's local completion grant may lock only
its own record, and a participant acknowledgement settles only in the root
the resource participant names as its coordinator source.

The lifecycle system groups the admitted production packets by root, loads
each root once, settles its packets (archive commits first, as before), and
writes only the roots that changed. It then writes the boundary activity
record `canwu.production:boundary-activity` (`ProductionBoundaryActivityV1`):
the roots it wrote, each with the first packet that changed its production
lifecycle (`ProductionRootActivityV1::trigger`): an applied operation or batch
item, a settled output acknowledgement, or an accepted completion step. A
recorded rejection, a continuation witness, and archive maintenance (a commit,
applied or stale, and a retention acknowledgement) change a root without
being a trigger, and an acknowledgement for a handle the root does not hold
changes nothing. The later systems read that small record, and nothing else,
on every boundary: validation and the incident commit audit load the roots
written; incident evaluation runs for a root with a trigger and cites the
trigger as its draw evidence; output dispatch and report publication run for
a root with a trigger and for a root whose wake was admitted (a wake naming
no installed root is refused with a trace). After the boundary's domain
commit only a version of the record that this boundary wrote is trusted, so a
record planted in a scenario or a snapshot, whatever boundary it names, drives
nothing. A boundary with no production work reads one small record. The
incident operation key gains the root's record id, so draws of two roots
never share an address.

### Limits are per root

`ProductionState.limits` (`ProductionLimitsV1`, `Default` is `canonical()`)
is fixed by the scenario and validated against `ProductionLimitsV1::maximum()`.
The canonical values rise to 16,384 sites, facilities, work orders,
executions, and capacity allocations, 32,768 operation outcomes, and 16,384
observation heads, dirty entries, and archive-due entries; the maxima are
65,536 for the first five and 131,072 for outcomes. `max_mutations_per_boundary`,
which nothing enforced, is replaced by `max_batch_items` (items one batch
carries, canonical 1,024, maximum 4,096) and `max_operations_per_boundary`
(operations and batch items a root records outcomes for in one boundary,
canonical 4,096, maximum 65,536; `max_batch_items` may not exceed it, so
every admitted batch can settle in a boundary of its own);
`max_observation_rollover` (canonical 8,192)
bounds the rolled-out heads; `max_incident_receipts` is gone (see below), and
`max_incidents_per_boundary` and `max_reports_per_boundary` bound each root.
The two report-count bounds, `max_observation_records` and
`max_observation_records_per_holder`, count across every root, as the
knowledge namespace does. Every other bound is a root's own, so the budgets of a world
add up across its roots: a boundary that touches many roots may settle,
publish, and draw for each of them.

Every bound is a recorded refusal. Settlement checks the outcome room of
each operation before it applies it; an operation without room, or past the
root's operations for the boundary, is refused with a trace and records
nothing, so the command can be sent again. An operation that passes those
checks but takes the root past a count or the 256 MiB byte cap is a recorded
rejection; if even the rejection does not fit, the operation is refused
instead. Command admission returns its findings about the command's content
as `InvalidPayload` (a malformed facility project, for one), a code the
engine records as a command rejection, and refuses a payload that is not the
canonical encoding of what it decodes to (an explicit `null` or unknown
member), which would otherwise be admitted and then not match its packet. Each operation is applied to one copy of
the root and validated once with its outcome in it, where 0.17.0 copied it
twice, validated it twice, and decoded the root again for the evidence check;
the evidence check now reads the root as the operations before it left it.

### The batch command

`apply_production_batch_v1` (`PRODUCTION_BATCH_COMMAND`,
`production_batch_command_descriptor`) carries a `ProductionBatchEnvelopeV1
{ holder, scope, expected_runtime_revision: Option<u64>, items:
Vec<ProductionBatchItemV1 { operation_id, operation }> }`.

- Admission checks the issuer's authority over `holder` once, the root, at
  most `max_batch_items` items with distinct operation IDs, no
  `ResolveDegradedFacility` (it needs its own decision), and outcome room for
  every item not settled yet; an item whose operation ID has another
  operation's outcome refuses the command (`IdempotencyConflict`), and a
  facility-project step that would use its reserved outcome while the table
  is full must be sent alone. A batch whose items are all settled admits
  nothing. Each refusal is a recorded command rejection.
- Settlement checks the cause as for an operation packet. A batch whose
  `expected_runtime_revision` is set and is not the root's, behind or ahead,
  or whose items would take the root past `max_operations_per_boundary`, is
  refused whole with a trace and records nothing, so it can be sent again.
  Archive commits settle first in a boundary, so a commit in the same
  boundary makes a batch with an expected revision stale; a batch without one
  has each item checked only by its own rules against the root the items
  before it left, so a monthly settlement need not predict the revisions of
  packets other plugins land in the same boundary. The items are applied in
  order through the single-operation path. Each item's outcome records the
  envelope as it applied (the revision it applied against), so outcomes keep
  their shape and digest rule; a single command that reuses an item's
  operation ID therefore meets `IdempotencyConflict`. One
  `canwu.production.batch_settled.v1` event counts the applied, rejected,
  already settled, and refused items and names the first 16 refused IDs.

What can fail the boundary of a batch is what can fail the boundary of a
single operation (below). An item costs one copy and one validation of its
root; roots are sized by scope for that reason.

### Nothing a root settles stays for life

- **Lease outcomes** leave with the execution or facility project that bound
  the lease: an archived execution's `operation_outcomes` hold its own
  operations' and its lease's, matched as the archive validator checks them,
  so an unmatched rejection no longer stalls the archive.
- **Residues.** A new archive record, `ProductionResidueArchiveRecordV1`
  (blob field `residue_records`, `ProductionArchivedRecordV1::Residue`),
  takes what no execution or project archives once every record it names is
  gone: a work order that ended without an execution
  (`ProductionTerminalArchiveKeyV1::WorkOrder`), a completion lease released
  or expired without binding a hot execution or project, with its grants,
  certificate, participant grants, and receipts (`CompletionLease`), and any
  other outcome (`OperationOutcome`). Residues are candidates after receipts,
  executions, and projects (`ProductionState::residue_archive_candidates`);
  directories, heads, and receipts count them (`archived_residue_count`,
  `archived_residues`). Decision receipts leave with the record whose
  outcome resolved their ticket (`decision_receipts`).
- **Incident receipts fold.** At its first incident pass in a boundary a root
  folds every receipt of an earlier boundary into `incident_history`
  (`ProductionIncidentHistoryV1 { folded_receipts, digest }`, a hash chain in
  receipt-key order), so it keeps at most `max_incidents_per_boundary`.
  Restore checks the hot receipts against the draw journal, as before.
- **Rolled-out observation heads** are pruned when a newer head of the same
  holder and site is eligible under the holder's delay: that cut only moves
  forward, so an older head can never be read again. The report system
  prunes the pairs due now before it visits the dirty pairs, and each pair it
  visits before it checks room, so a pair drains when its heads come due
  whether or not it gets another head, and a pair blocked only by another
  pair's rolled-out head settles in the boundary that prunes it. A
  materialization that would roll a head into a full rollover
  (`max_observation_rollover`) waits as dirty work. Reports are kept for life
  and outlive their heads: restore checks a report against its head while the
  root keeps it, and otherwise by its content, grant, subject, and digest,
  and by its origin naming an installed root; its publication checked it
  against the head, and exact replay rechecks it.
- **Completion lease residues** wait until every participant grant the root
  recorded for the lease has ended, so a participant's reserved units are
  free before its record leaves. An expired lease therefore leaves only once
  the host passes the resource participant's expiry back
  (`AcknowledgeParticipantReleased` carrying the expired grant).

### Refusals and duplicates (E-23)

A completion step, continuation witness, archive commit, or retention
acknowledgement that production's checks refuse settles nothing and is
refused with a trace, whoever queued or scheduled it; the commit and the
acknowledgement are applied to a copy of the root. Only a failed store read
(`InvalidArchive`) and engine invariants (undeclared reads or writes,
identifier exhaustion, a panicked or invalid boundary, an unavailable state
page) fail the boundary. These packets are classified by that deny-list
because any finding about their content must not stall the run; operations
and batch items keep the allow-list of expected domain rejections
(`invalid_authority`, `invalid_domain_record`, and the rest), because their
rejection is recorded as an outcome and an unexpected code there is a bug
that should stay visible.

A stale commit under a retention handle that already has a stale receipt
increments that receipt's `duplicate_settlements` instead of adding one, so
queuing one batch again and again leaves one receipt per handle.

Refused packets draw nothing and publish nothing: the activity record names a
trigger only for a packet that changed the root. A wake still runs output
dispatch and report publication for its root (a root installed or restored
with a pending output resumes on its wake); a wake with nothing due changes
nothing.

Preparing, enqueueing, and settling a commit group the root's outcomes and
lease receipts by the records that claim them once (`ArchiveClaims`), so a
commit costs its selected records and their claims plus one pass over the
hot outcomes and receipts, where it cost the selected records times the hot
outcomes. Removal takes exactly the outcomes and receipts the archived record
holds, and leases no hot record binds any more are released in one pass.

### Reports read the publication budget

The report system reads `knowledge_publication_budget()` and publishes only
what fits. The wake it schedules is the first due time after now, or one
minute later when work it could not take waits: a report past the
publication budget, or keys past `max_reports_per_boundary` while the pass
made progress. A report past a knowledge-count bound waits for the root's
next activity instead, since a wake would find the bound still reached.
Production reserves no share of the budget itself; a host that wants its
reports kept from other publishers registers it through
`KnowledgeReservedPlugin`, and the report system reads that share through
the same `knowledge_publication_budget()`.

## Contract summary

- New types and functions: `ProductionScopeId`, `production_scope_runtime_reference`,
  `production_root_reference`, `production_root_scope`,
  `ProductionState::runtime_reference`, `production_root_state`,
  `ProductionBatchEnvelopeV1`, `ProductionBatchItemV1`,
  `PRODUCTION_BATCH_COMMAND`, `production_batch_command_descriptor`,
  `ProductionBoundaryActivityRecord`, `ProductionBoundaryActivityV1`,
  `ProductionRootActivityV1`, `production_boundary_activity_reference`,
  `PRODUCTION_BOUNDARY_ACTIVITY_ID`, `ProductionIncidentHistoryV1`,
  `ProductionResidueArchiveRecordV1`, `ProductionResidueBodyV1`,
  `ProductionCompletionLeaseArchiveV1`, `ProductionLimitsV1::maximum` and
  `validate`, `ProductionState::residue_archive_candidates`,
  `production_root_names_key`, and the
  `_in_scope` / `_in_root` variants of the completion and continuation
  enqueue helpers, `production_report`, `production_observation_witness`,
  and `degraded_facility_decision_ticket`.
- Admission: a degraded-facility choice without its persisted decision is
  a recorded command rejection (`InvalidPayload`), not a failed boundary.
- Changed shapes: `ProductionState` gains `scope`, `limits`, and
  `incident_history`; `ProductionLimitsV1` replaces
  `max_mutations_per_boundary` with `max_batch_items` and
  `max_operations_per_boundary`, adds `max_observation_rollover`, and drops
  `max_incident_receipts`; `MAX_HOT_RECEIPTS` is gone;
  `acknowledge_production_archive_retention` returns whether it changed the
  root; `ProductionCommandEnvelope` gains `scope`; the
  archive key gains three variants; the blob gains `residue_records` and is
  `PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION` 3; an archived execution gains
  `decision_receipts`; directories and the head gain `archived_residue_count`;
  maintenance receipts gain `archived_residues` and `duplicate_settlements`;
  prepared batches and commits gain `scope` and `selected_residues`; the
  archive source root (domain `canwu.production.archive-source-root.v2`)
  commits to the scope; `ProductionArchiveIngressReceiptV1` gains `scope`;
  the incident operation key names the root record. New packet
  `production_batch_v1`, new events `canwu.production.batch_settled.v1`, new
  record kind `canwu.production` `boundary-activity`, and a new
  `PRODUCTION_SEMANTIC_HASH`
  (`7b39f2f5d5dab13ab1ee6e7c9b051c34eb05870249cdce287d89c879e2248ed0`), the
  BLAKE3 hash in lowercase hex of this UTF-8 string, on one line:

      canwu-production 0.18.0 semantics: scope roots, batch command, residue archive, boundary activity triggers, folded incident receipts

- Behavior: a wake no longer runs incident evaluation; a recorded rejection,
  a continuation witness, and archive maintenance no longer drive incidents,
  dispatch, or reports; the per-boundary report and incident bounds are each
  root's; a scope root's leases and output keys carry its prefix and the
  primary root's hold no `/`.
- Same version only: nothing written by 0.17.0 is read; the root,
  directories, and blobs of 0.17.0 do not decode or are refused by format
  version.

## What can still fail a production boundary

- A failed read of the evidence or archive stores (`InvalidArchive`), which
  settles once the store serves the object. Settlement reads the stores in
  two places: `StartExecution` looks its output keys up in the resource
  archive (`refuse_output_keys_held_by_resource`: the head directory, one
  lookup page per level, and a blob on a hit), and the archive
  reachability participant audits each root's archive. No other production
  packet reads a store while it settles.
- Engine invariants: an undeclared read or write, identifier exhaustion, a
  panicked plugin, an invalid boundary, an unavailable state page.
- A write of a later production system that would take a root past its byte
  cap or overflow a 64-bit counter: an incident receipt, an observation head,
  or a pinned output source on a root already at its 256 MiB cap.
- A root that no longer validates when a later system loads it, which only a
  corrupted engine state can produce.

## Costs, before and after

Release build, one thread, the same machine shared with other builds, the
`scale` preset of `production-archive-profile` against the base commit
(`8fb922e`, before) and this change (after). Times are single samples or
medians of five; the machine was busy, so differences under about 20 % are
noise.

| Boundary | Before | After |
| --- | --- | --- |
| 1 `CreateWorkOrder`, 0.99 MB root: admission / apply / journal | 211 ms / 288 ms / 1.98 MB | 12 ms / 57 ms / 1.98 MB |
| 64 commands, 0.99 MB root: admission / apply | 956 ms / 2.39 s | 526 ms / 188 ms |
| 16 commands, 13.6 MB root: admission / apply | 4.38 s / 6.72 s | 3.66 s / 2.71 s |
| 1 command, 13.6 MB root: apply / journal | 1.91 s / 27.2 MB | 2.00 s / 27.2 MB |
| one batch of 64 items, 0.99 MB root: admission / apply / journal | — | 15 ms / 218 ms / 2.05 MB |
| one batch of 1,024 items, 0.99 MB root | — | 18 ms / 5.90 s / 3.08 MB |
| one batch of 64 items, 13.6 MB root | — | 300 ms / 11.1 s / 27.3 MB |

A world of scope roots, one batch of `CreateWorkOrder` items into one scope
(after only: before, a root held at most 2,048 sites):

| World | All roots | Install | Idle boundary | 64 items: admission / apply / journal | 1,024 items |
| --- | --- | --- | --- | --- | --- |
| 64 scopes of 64 sites (4,096 sites) | 2.1 MB | 0.53 s | 0.13 ms | 1.6 ms / 52 ms / 140 KB | — |
| 400 scopes of 64 sites (25,600 sites) | 13.4 MB | 3.2 s | 0.14 ms | 1.6 ms / 24 ms / 140 KB | 18 ms / 3.87 s / 1.19 MB |
| 400 scopes of 160 sites (64,000 sites) | 31.9 MB | 8.0 s | 0.14 ms | 2.4 ms / 52 ms / 233 KB | — |

A boundary's cost and journal follow the roots it touches, not the world:
the 64,000-site world settles a scope's batch as fast as the 4,096-site one.
An item costs one copy and one validation of its root, about 3.5 ms at 1 MB
and 170 ms at 13.6 MB, which is why the large single root gains little from
batching and why roots are sized by scope. The 1,024-item batch into one
64-site scope grows that root past 1 MB as it settles.

Preparing an archive commit of N settled executions with four claimed
outcomes each, beside U unrelated outcomes (a probe run once; the root's
completion receipts, nine per execution, are claimed the same way):

| N executions, U unrelated | Hot outcomes | Before | After |
| --- | --- | --- | --- |
| 256, 0 | 1,024 | 32 to 52 ms | 24 ms |
| 1,023, 0 | 4,092 | 231 to 257 ms | 108 ms |
| 256, 8,000 | 9,024 | 46 to 51 ms | 32 ms |
| 1,023, 8,000 | 12,092 | 279 to 384 ms | 108 ms |

Before, two runs; after, the residue candidates are built lazily, so the
unrelated outcomes (which are residues) cost one pass. Settling the commit
rebuilds the same records and costs the same.

## Tests

- `tests/production_scope_roots.rs`:
  - `scope_roots_settle_batches_alone_and_archive_their_residues`: three
    roots; a batch into one settles item by item, a configured work-order
    bound is a recorded rejection, the other roots are not written, a re-sent
    batch changes nothing, a batch expecting a revision behind or ahead of
    the root's is refused whole and can be sent again, an oversized batch and
    an unknown root are command rejections, the cancelled order and the
    rejected creations leave as residues and resolve through the lookup, a
    lease without its scope prefix is a recorded rejection, and restore and
    exact replay agree;
  - `only_settled_packets_drive_incidents_and_reports`: junk public packets
    draw nothing and publish nothing; applied operations draw, publish, and
    fold their incident receipts;
  - `a_planted_boundary_activity_record_drives_nothing`: an activity record
    installed with the scenario, naming the next boundary and a trigger,
    draws and publishes nothing;
  - `forged_packets_another_plugin_schedules_are_recorded_refusals`: a
    commit, a completion step, and a continuation witness are refused with a
    trace, and an acknowledgement for no handle changes nothing;
  - `malformed_and_non_canonical_commands_are_command_rejections`: a
    malformed facility project in a command or a batch, a payload with an
    explicit `null`, and a degraded-facility choice without its persisted
    decision are recorded command rejections;
  - `an_operation_without_outcome_room_at_settlement_is_refused`: two
    commands admitted at one free slot; the second is refused with a trace;
  - `a_batch_past_the_operations_a_root_settles_in_one_boundary_is_refused_whole`:
    a root whose `max_batch_items` exceeds `max_operations_per_boundary` does
    not validate, a batch past the operation bound is refused whole and
    settles when sent again, and a single command past it is refused with a
    trace;
  - `a_report_past_the_publication_budget_waits_and_publishes_on_its_wake`:
    another plugin spends the boundary's budget; the report waits and its
    wake a minute later publishes it;
  - `reports_whose_heads_left_the_root_still_restore`: twenty reports of one
    pair, sixteen heads kept;
  - `a_pair_waiting_on_a_full_rollover_drains_when_its_heads_come_due`;
  - `a_pair_blocked_by_another_pairs_rolled_out_head_settles_when_it_is_pruned`:
    two holders share a full rollover; the waiting pair settles at the other
    pair's due time, with no later wake to wait for.
- `tests/production_archive_volume.rs`:
  `the_same_archive_batch_queued_again_and_again_leaves_one_stale_receipt`
  (validation refuses a second stale receipt under a handle and duplicates
  counted on an applied commit's receipt) and
  `an_execution_archives_its_lease_outcome_and_an_unmatched_rejection_leaves_later`
  (with the 0.17.0 claim rule the rejection stalls the archive).
- `tests/g3_contract.rs`: `phase_ten_only_stages_and_phase_eleven_atomically_commits_incidents`
  and `holder_knowledge_cap_rejects_one_more_report_with_bounded_work` drive
  their incidents with applied operations instead of wakes, which no longer
  draw; `canonical_cross_plugin_completion_coordinator_uses_real_ingress_and_exact_grants`
  refuses a local grant that locks another production record; and
  `canonical_commissioning_closes_resource_and_local_grants_before_project_archive`
  keeps a released lease from the residues while its resource grant is held.

Each new or changed assertion was checked to fail with its fix reverted.
The coordinator-source check on a participant acknowledgement has no test
of its own: the resource plugin grants a participant only for a coordinator
source that holds the acquisition, and the prefix rule keeps an acquisition
ID to one root, so no run reaches it.

## Deviations from the filed fix

- The fix raised the caps to 16,384 and also sharded, by host-assigned scope
  rather than by a hash of the site (see [Scope roots](#scope-roots)); raising
  the caps alone leaves every boundary costing the whole root.
- The 4,096-mutation-per-boundary cap, which nothing enforced, became the
  batch item bound and an enforced bound on the operations and items a root
  settles in one boundary.
- Incident receipts fold into a digest chain rather than being archived.
- The batch's expected revision is optional; without it each item stands on
  its own checks.
- The lifetime limits found while reading the code (lease and unclaimed
  outcomes, ended work orders and leases, the rolled-out heads) are removed
  in this package, as the brief's "remove any lifetime cap" asks.

## Known limits

- An item of a batch, like an operation, costs one copy and one validation of
  its root; a root of several megabytes settles items slowly.
- A key a resource operation queued by the host application takes after an
  execution started still strands the execution, as before; the scope prefix
  only keeps production roots from colliding with each other.
- Roots are installed with the scenario; a run cannot add a scope.
- Production reports keep `KnowledgeRetention::Keep`: a holder still keeps at
  most `max_observation_records_per_holder` reports for life, and later
  reports wait. Superseding a site's previous report needs a knowledge
  subject per site, and production sites are not entities.
- An activated lease that never starts an execution never ends, so it stays
  in its root. A re-sent operation whose outcome was archived is applied
  again, and residues widen that: once a cancelled order's residue is
  archived, a re-sent `CreateWorkOrder` recreates the order.
- The incident history is a digest chain: a forged folded receipt or chain in
  a snapshot is caught only by exact replay, as a forged hot receipt's body
  is caught by the draw journal at restore.
- A forged activity record in a scenario or snapshot is ignored after the
  domain commit; in invariant validation it can only make the system validate
  a root again.
- A host application that archives must keep each root's lookup pages for the
  heads a restore, replay, or fork passes, as before; each root has its own
  archive chain.
