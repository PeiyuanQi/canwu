# Reference Archive Receipts and Operation Room

Status: implemented in 0.18.0. It closes engine finding E-22 in the two
unpublished reference integrations, `canwu-force-supply-reference` (which
owns the package archive) and `canwu-economy-reference` (which reuses it).
The archive objects, the lookup index, and the commit and acknowledgement
packets keep their 0.17.0 shape.

It builds on the 0.16.0 package archive of
[reference archive lookup and replay profile](reference-archive-and-replay-profile.md),
whose known limits it removes, and follows the refusal rule of
[resource refusals without stalls](resource-refusals-without-stalls.md).

## Problem

Two limits stopped a long run of either reference plugin.

- **Receipts never left the root.** Every settled archive commit, applied or
  stale, left a maintenance receipt in the plugin root, and nothing removed
  it. The root refused more than `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS`
  (8,192); 0.16.0 made the enqueue helpers refuse a commit that could reach
  the bound, so after 8,192 commits a root could never archive again. Before
  that, every receipt was decoded, validated, re-encoded, and journaled by
  every boundary that changed the root (about 425 bytes each).
- **A full root failed its boundary.** The economy root bounds its hot
  operation outcomes (`max_operation_outcomes`, 8,192 by default) and the
  force-supply root its own (`max_externality_outcomes`, 2,048). The plugins
  recorded an operation's outcome, applied or rejected, and then validated
  the root, so the operation past the bound failed the boundary that settled
  it, and since a due packet cannot be withdrawn, every later boundary too.
  Once commits were refused, outcomes could not leave, and the run stopped
  there. The economy root failed the same way when an applied operation took
  another bounded collection (profiles, local economies, observation grants,
  route and price observations, delivery attempts) past its limit, or when
  its result passed the byte cap.

Evidence, measured before this change (release build; see
[measurements](#measurements)): a chain of economy commits is refused after
8,192 settled commits with a 3.5 MB root, and a boundary that settles one
economy operation grows from 7 to 12 ms to 120 to 230 ms on the way (two
runs); in ten simulated
years of monthly maintenance through the grain harness the economy root grows
by one receipt a month (40.2 KB to 92.8 KB) and the force-supply root by one
unarchived operation outcome a month (45.8 KB to 89.8 KB).

## Decision

### Acknowledged receipts fold into a digest chain

Each plugin root gains `archive_receipt_chain`, a
`PackageArchiveReceiptChainV1 { issued, folded, digest }`:

- A settled commit, applied or stale, issues one receipt (`issued` counts it)
  and records it in the root under its retention handle, as before.
- The acknowledgement of a handle (`*_archive_retention_ack_v1`, queued by
  `finalize_*_archive_retention`) removes the handle and folds every receipt
  recorded under it, oldest first, into `digest`:
  `digest = H(prefix + ".archive-receipt-chain.v1", (digest, receipt.semantic_digest))`,
  starting from the empty string. The applied commit and the stale copies of
  the same batch share one handle, so one acknowledgement folds them all.
- An acknowledgement whose receipt the root no longer holds was folded by an
  earlier acknowledgement of the same handle; it settles as a no-op. A
  receipt the root holds must equal the acknowledged one exactly, as before.
- `settle_package_archive_acknowledgement` does this for both plugins and
  reads nothing but the root.

The root therefore holds only the receipts and handles of unacknowledged
commits. `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS` falls from 8,192 to 64
and now bounds those; `MAX_PACKAGE_ARCHIVE_RETENTION_HANDLES` stays 64. A host
that acknowledges each commit it queued before it queues many more is never
refused, however many commits the run makes. `package_archive_commit_room`
still refuses, host-side and before anything is queued, a commit the root and
the commits already queued leave no room for.

`validate_package_archive_maintenance` checks, for both roots: the bounds,
each sealed digest, that every hot receipt names a handle the root holds, and
that hot and folded receipts together number the receipts issued. The economy
root takes its receipt sequence from `issued` (it took the largest hot
sequence plus one, which a fold could reuse); the force-supply root keeps its
shared terminal sequence.

`finalize_*_archive_retention` finalizes a handle as committed when any
receipt recorded under it applied, and as rejected stale otherwise
(`package_archive_finalization`), and returns the acknowledgement's ingress
receipt. Once that acknowledgement settled, the root holds neither the handle
nor its receipts and the store's handle is finalized: the call does nothing
and returns `None`, so a host that retries a finalization after a crash or a
restore treats it as done. A call before the commit settled (the store's
handle in any other phase) is refused. `None` is decided from the store's
handle phase and the root, and one store may serve several runs: in a fork,
or a run restored from a point before the commit settled, of a run that has
since acknowledged it, it reports "done" while the commit is still queued
there; once the commit settles in that run, its root holds the receipt again
and a finalization acknowledges it.

**What a host can still read about a folded receipt.** Not the receipt from
the root. It can read: the receipt's place in `digest`, which it recomputes
with `PackageArchiveReceiptChainV1::fold` over the receipts it kept (the
acknowledgement payload, or the root before the acknowledgement); the commit's
retention handle in its archive store, finalized as committed or rejected
stale, which names the source root and the directory; and, for an applied
commit, its directory on the archive's chain, whose blob, pages, and record
count the restore audit authenticates from the head. The restore audit no
longer reads the store-side handle of a folded receipt.

### Operations are refused for room, never by a failed boundary

Room is measured on the result, as in `canwu-resource`:

- The plugin applies the operation to a copy of the root, records its
  outcome, and only then checks room (`outcome_room_shortage`). A rejected
  operation needs room for its outcome only.
- A result that does not fit is **skipped**: the root stays as it was,
  nothing is recorded under the operation ID, the boundary commits, and the
  plugin emits its own skip event, `canwu.force-supply.operation_skipped.v1`
  (`FORCE_OPERATION_SKIPPED_EVENT`) or
  `canwu.economy-reference.operation_skipped.v1`
  (`ECONOMY_OPERATION_SKIPPED_EVENT`), never its `operation_rejected` event,
  with the summary `operation {id} skipped ({code}): {detail}`.
  `force_operation_skip` and `economy_operation_skip` read it back as a
  `PackageOperationSkipV1 { operation_id, shortage }`. Recording a rejection
  instead would need the very slot that is short and would take the ID.
- **The ID stays free; the host rebuilds the command before it sends it
  again.** Once an archive commit has made room, the command built against
  the root's current runtime revision settles under the same ID. The archive
  commit and the acknowledgement that make room advance the economy root's
  revision, so the same economy command re-sent unchanged settles as a
  recorded `domain_record_version_conflict` and takes the ID; the skip
  detail says so. The force-supply root's archive maintenance does not
  advance its revision, but its other operations may.
- `PackageRootShortageV1` names the reason: `archive_backlog` when the root
  holds as many hot operation outcomes as its limits allow, `root_limit` when
  the result passes the share of the byte cap operations may fill
  (`package_operation_state_bytes`): the cap less
  `PACKAGE_MAINTENANCE_STATE_BYTE_RESERVE` (128 KiB). Both roots' limits now
  require a cap of at least `MIN_PACKAGE_STATE_BYTES` (512 KiB, four times
  the reserve). The receipts and handles the bounds allow (about 1.4 KB per
  commit, 64 of each) fit in the reserve, so an archive commit or an
  acknowledgement does not fail for the byte cap while everything else that
  writes the root stays within the operation share. That holds for the
  economy root, whose operations and externality materialization both stop
  at the share. In the force-supply root, the acknowledgements of resource
  and externality outcomes that the plugin settles are still checked against
  the full cap, so they can fill the reserve; the guarantee holds for that
  root only while they stay under the share (see
  [known limits](#known-limits)).
- The force-supply plugin applies an operation without the byte cap
  (`apply_operation_unmeasured`) and measures room on the result with its
  outcome, so a result past the cap is skipped as `root_limit`; it was a
  recorded `invalid_domain_record` rejection, which took the ID for a result
  an archive commit would have made room for. `apply_operation`, for a
  detached caller, still refuses a result past the cap.
- In the economy root an applied operation that takes another bounded
  collection past its limit (`EconomyReferenceStateV1::exceeded_record_cap`,
  which also says whether an archive commit can take records out of it) is a
  recorded rejection, `query_budget_exceeded`: `economy {collection} hot cap
  requires archive progress` for frames, route and price observations,
  delivery attempts, and externality outcomes, and `economy {collection}
  limit is reached` for profiles, local economies, observation grants, and
  authorized resource consumption intents, which no archive commit removes.
  It failed the boundary before. The force-supply root validates inside its
  operations, so those limits were already recorded rejections there.
- The economy's force-requisition externality system creates an externality
  outcome only while the root has a hot slot for it and the result stays
  within the operation share; otherwise what that intent changed is undone,
  the intent waits in the force-supply root, and a later boundary of the
  system materializes it once an archive commit made room.

### Archived outcomes stay readable

Both roots already archived operation outcomes as candidates and looked a
settled ID up in the archive's lookup index at admission and settlement.
`force_operation_outcome(state, provider, id)` and
`economy_operation_outcome(state, provider, id)` now give that read to hosts
and other plugins: the hot outcome, or the archived record through the head's
index (`id`, `input_digest`, `applied`, `rejection_code`, `rejection_reason`,
`settled_at`), or `None` for an ID that never settled, skipped ones included.
The economy's `CloseMonth` reads the force-supply operation it cites through
it, so a host may archive force-supply outcomes before the month closes; it
read the hot outcome only. The grain harness's `archive_reference_history`
now archives the force-supply root whenever it holds terminal receipts or
outcomes, not only terminal receipts.

### Fixed beside

`prepare_economy_archive` takes the force-supply root of the same run
(`force: Option<&ForceSupplyStateV1>`) and leaves out an externality outcome
that root still holds hot: from the moment a saga takes its intent until the
force-supply archive takes the saga's terminal receipt.
`enqueue_economy_archive` prepares the batch again with the run's own
force-supply root and refuses, before anything is queued, a batch that
differs. In 0.17.0 a commit selected every externality outcome, and the
boundary that settled one selecting a hot outcome failed for good
(`economy externality outcome still has a hot force continuation`), so a host
had to archive the force-supply root first and could not commit the economy
root while a requisition's outcome was hot. The economy's other candidates
now archive whatever the force-supply root holds.

### Force-supply settlement without a scan

On top of the resource live split of
[resource series scale](resource-series-scale.md), the force-supply plugin's
settlement of a resource outcome (`resolve_resource_outcome`) no longer
reads the whole resource root. It attaches, under the intent's identities,
the outcome, the consumption (whose closure brings the fulfillment it names,
`consumption.fulfillment`, and its account), the accepted transfer, and the
completion participant, active or finished, and the destination account by
identity; it checks the fulfillment the consumption names instead of
searching every hot fulfillment. The economy's check of a requisition's
target scope attaches the consumed leg's account (with its definition) by
identity as well, so it reads the same once archiving has moved the leg out.
The observation check (`validate_force_observation_source`) reads the force
holder's accounts through the custodian index, as the resource change left
it.

## Invariants

- A plugin root holds at most 64 maintenance receipts and 64 retention
  handles; every hot receipt names a held handle; hot and folded receipts
  number the receipts issued, and receipt sequences never repeat.
- An archive commit, an acknowledgement, and an operation's skip read only
  the root (the skip also its encoded length). A live run, a restored run, a
  sealed run, and an exact replay settle them alike; no boundary decision
  reads retained evidence. Lookups read the archive store through the plugin
  archive provider, as before.
- A skipped operation contributes nothing to the root; a skipped economy
  operation does not advance the runtime revision.

## Persistence, replay, and formats

- **Plugin roots.** `ForceSupplyStateV1` and `EconomyReferenceStateV1` gain the
  required field `archive_receipt_chain`; `FORCE_FORMAT_VERSION` (new) and
  `ECONOMY_FORMAT_VERSION` become 2, and a root of format 1 does not validate.
- **Plugin identity.** `FORCE_SUPPLY_SEMANTIC_HASH` and `ECONOMY_SEMANTIC_HASH`
  change. Registered commands, ingress, systems, events, and schemas keep
  their names.
- **Archive objects, packets, and kernel formats.** Unchanged. Same version
  only: no 0.17.0 root is read.

## Compatibility

These crates are not published. Source and behavior changes:

- New: `PackageArchiveReceiptChainV1`, `validate_package_archive_maintenance`,
  `settle_package_archive_acknowledgement`, `package_archive_finalization`,
  `PackageRootShortageV1`, `PackageOperationSkipV1`,
  `PACKAGE_MAINTENANCE_STATE_BYTE_RESERVE`, `MIN_PACKAGE_STATE_BYTES`,
  `package_operation_state_bytes`, `FORCE_FORMAT_VERSION`,
  `FORCE_OPERATION_SKIPPED_EVENT`, `ECONOMY_OPERATION_SKIPPED_EVENT`,
  `force_operation_skip`, `economy_operation_skip`, `force_operation_outcome`,
  `economy_operation_outcome`, `encoded_len` and `outcome_room_shortage` on
  both states, `EconomyReferenceStateV1::exceeded_record_cap`, and the hidden
  test constructor `GrainHarness::with_limits`. Each plugin's apply system
  declares its new skip event.
- Both limits validators refuse `max_state_bytes` below
  `MIN_PACKAGE_STATE_BYTES` (they refused only 0).
- `finalize_force_archive_retention` and `finalize_economy_archive_retention`
  return `Option<IngressReceipt>`: `None` when the acknowledgement already
  settled.
- `EconomyReferenceStateV1::prepare_economy_archive` takes the force-supply
  root of the run as a third argument (`None` when the run has none).
- `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS` is 64 (was 8,192).
- An acknowledgement folds its receipts; a repeated one is a no-op (it was
  accepted too, but left the receipt).
- An operation without room is skipped with an `operation_skipped` event
  instead of failing the boundary (or, for a force-supply result past the
  byte cap, instead of a recorded rejection); an economy result past a
  bounded collection is a recorded `query_budget_exceeded` rejection.

## What can still fail a boundary

- An archive commit or acknowledgement packet that decodes but is forged
  (a receipt that differs from the hot one, a commit whose phase, roots, or
  selection are wrong): another plugin that schedules these internal packet
  types can produce one; the enqueue and finalize helpers never do.
- A package archive object the store cannot serve, or serves forged, while a
  boundary looks a key up (admission, settlement, `CloseMonth`): the boundary
  fails and settles once the store serves it, never taken for a miss.
- An economy archive commit that another plugin schedules with an externality
  outcome whose force continuation is hot (the enqueue helper never queues
  one).
- Counter overflow (64-bit), and an invariant failure of a root.
- Unchanged by this item: the force-supply root's acknowledgements of
  resource and externality outcomes past the byte cap (plugin-scheduled
  work, which `canwu-resource` would defer; see
  [known limits](#known-limits)).

## Measurements

Release build, one thread, the grain harness's in-memory archive store, on a
machine that several other builds were sharing, so the times are single
samples with noise of a factor of two; the byte counts are exact. "Before" is
the same probe compiled against the base of this change (`main` at
`8fb922e`, 0.17.0). The probe is not kept in the tree.

**A long economy commit chain.** Each round settles one economy operation (a
recorded rejection, whose outcome becomes a candidate), queues 64 copies of
one archive commit (one applies, 63 settle stale; every copy leaves a
receipt), and acknowledges their handle. The times are of one boundary: the
operation's, the one that settles the 64 commits, and the acknowledgement's.

| Settled commits | Before: root | Before: receipts | Before: operation, commits, ack | After: root | After: receipts | After: operation, commits, ack |
| --- | --- | --- | --- | --- | --- | --- |
| 64 | 64.1 KB | 64 | 6.6, 15.5, 6.9 ms | 37.3 KB | 0 | 4.0, 13.5, 5.9 ms |
| 1,088 | 497 KB | 1,088 | 21.5, 26.7, 23.4 ms | 37.3 KB | 0 | 3.7, 8.9, 3.8 ms |
| 4,160 | 1.80 MB | 4,160 | 112, 113, 116 ms | 37.3 KB | 0 | 3.7, 9.4, 5.9 ms |
| 8,192 | 3.52 MB | 8,192 | 231, 374, 293 ms | 37.3 KB | 0 | 3.7, 8.9, 3.9 ms |
| 8,256 | refused (`archive_backpressure`) | | | 37.3 KB | 0 | 5.4, 9.0, 3.9 ms |
| 25,600 | | | | 37.3 KB | 0 | 3.7, 9.6, 3.8 ms |

Before, the root grew by 425 bytes per settled commit and every boundary that
changed it paid for the whole receipt list; the chain stopped at 8,192 commits
(78 s for 128 rounds). After, the root stays at 37.3 KB and the boundaries stay
flat past 8,192 (6 s for 140 rounds, 20 s for 400 rounds and 25,600 commits).
An earlier run of the same "before" chain measured 12 ms and 119 ms for the
operation boundary at 64 and 8,192 commits.

**Ten simulated years of monthly maintenance.** The grain harness advances
120 months, and after each month its host maintenance
(`archive_reference_history`) commits and acknowledges each root's settled
history. Before, that maintenance archived the force-supply root only while it
held terminal receipts, so its operation outcomes stayed hot; it now archives
them too (two more boundaries a month).

| Month | Before: force-supply root (receipts, outcomes) | Before: economy root (receipts) | After: force-supply root (receipts, outcomes) | After: economy root (receipts) |
| --- | --- | --- | --- | --- |
| 1 | 45.8 KB (0, 0) | 40.2 KB (1) | 45.9 KB (0, 0) | 39.9 KB (0) |
| 12 | 57.0 KB (3, 7) | 47.9 KB (12) | 53.8 KB (0, 0) | 43.1 KB (0) |
| 60 | 71.5 KB (3, 55) | 67.8 KB (60) | 53.8 KB (0, 0) | 43.1 KB (0) |
| 120 | 89.8 KB (3, 115) | 92.8 KB (120) | 53.8 KB (0, 0) | 43.1 KB (0) |

Before, the economy root grew by one receipt (about 440 bytes) a month and the
force-supply root by one hot outcome (about 300 bytes) a month; after, both
are flat from the first year on. The mean boundary of a month, almost all of
it resource, routing, and transport work, went from 4.2 ms in month 1 to
12.1 ms in month 120 before and from 3.4 ms to 5.7 ms after (single samples;
the months between scatter by up to a factor of two in both runs), so the
trend is suggestive only. By the tenth year the two roots held about 87 KB
more before than after, decoded and encoded by every boundary that reads or
changes them; the chain above is the case that isolates that cost.

## Tests

`crates/integrations/canwu-economy-reference/tests/reference_archive_receipts.rs`:

- `acknowledging_a_commit_folds_its_receipts_out_of_the_root`: three copies of
  one batch settle (one applied, two stale) under one handle; two
  acknowledgements settle, the first folds all three receipts and the second
  is a no-op; the chain equals the fold of the receipts; the store-side handle
  is committed; a third finalize is refused; the next commit's receipt takes
  sequence 4 and folds in turn; restore and exact replay agree.
- `an_operation_without_room_is_skipped_and_settles_when_sent_again`: with two
  economy outcome slots, one profile, and one force-supply outcome slot, a
  second profile is a recorded `query_budget_exceeded` rejection, and the
  operation past each outcome bound is skipped with an `archive_backlog`
  event in both plugins while the boundary commits; after an archive commit of
  each root the archived outcomes read through `economy_operation_outcome` and
  `force_operation_outcome`, the skipped commands sent again settle, and
  restore and exact replay agree.

- `a_result_past_the_operation_byte_share_is_skipped_and_maintenance_still_settles`:
  with the smallest byte cap (512 KiB), recorded rejections fill the economy
  root until one is skipped as `root_limit`; a one-record commit and two
  stale copies of it then settle in the reserve (the root ends past the
  operation share), the acknowledgement settles, and after a larger commit
  the skipped command, rebuilt, settles.
- `an_economy_commit_leaves_out_an_externality_outcome_the_force_root_holds_hot`:
  after five months of the grain harness, a requisition month among them,
  the force-supply root's terminal receipt still holds an externality
  outcome; a batch prepared without the force-supply root selects it and
  `enqueue_economy_archive` refuses it, a batch prepared with it leaves the
  outcome out and settles, and after `archive_reference_history` the outcome
  is archived and reads back through the lookup index.

The first test also checks that a finalization before the commit settled is
refused and one after its acknowledgement settled returns `None`; the second
reads the skips with `economy_operation_skip` and `force_operation_skip` and
rebuilds the skipped economy command against the current revision before it
sends it again. Each test was checked against a mutation of the code it
covers (no fold, no room check, no collection check, no externality filter,
no reserve, finalization refused after the acknowledgement); each mutation
fails it.

`grain_harness.rs`'s restore test now forges the retention phase of a commit
the root still holds, since an acknowledged one is folded.

## Deviations from the filed fix

- The filed fix asked to archive hot outcomes "as resource does". Both roots
  already archived their outcomes and looked them up through the index; what
  remained was the boundary failure at the bound, now a skip with an event as
  in `canwu-resource` (a skipped packet with a trace, not a recorded
  rejection, for the reason given above), and the host read.
- The receipts fold at acknowledgement into a digest chain, as filed, rather
  than becoming archive candidates as production's do; the archive's
  directory chain already records every applied commit, and no archive
  object changes.

## Known limits

- The force-supply plugin's acknowledgements of resource and externality
  outcomes, which other plugins schedule, are checked against the full byte
  cap: they can fill the maintenance reserve, and one that passes the cap
  still fails its boundary. They should be deferred, as `canwu-resource`
  defers plugin-scheduled work, rather than refused.
- An externality outcome stays in the economy root while its force-supply
  saga is open or its terminal receipt hot, so a saga that never closes keeps
  one outcome slot.
- One commit still carries one page of at most 512 records.
- Superseded lookup pages are still reclaimed only from each batch's list.
