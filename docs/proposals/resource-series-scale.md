# Resource Series Scale

Status: implemented in 0.18.0. It closes downstream gap G-17 and the
resource side of G-18 in `canwu-resource`, and the engine item E-24. A
host application or plugin that settles many small consumptions can settle
them under one completion lease, and a resource root can hold tens of
thousands of accounts without every boundary paying for them.

It builds on [resource root write cost](resource-root-write-cost.md)
(0.16.0), which split the hot terminal records out of the live root and left
"splitting the live state" and "refusing a host expiry for a boundary it
cannot settle at" as open decisions, and on
[resource refusals without stalls](resource-refusals-without-stalls.md)
(0.16.0), whose rule this change keeps: what an operation cannot do is a
recorded refusal or a skip, never a failed boundary.

## Problem and evidence

Measured with `resource-root-profile` and `resource-archive-profile` on the
integration base `8fb922e` (release, Rust 1.99.0, Windows x86-64, AMD Ryzen 7
7800X3D shared with other builds, medians of five samples after one warm-up).
The tables under [Measurements](#measurements) give the numbers before and
after.

1. **Accounts were paid for by every boundary.** The live root held every
   account, access grant, definition, and unit revision. One boundary that
   settled one resource operation decoded, validated, re-encoded, and
   journaled all of them: with 8,192 accounts the live root was 2.37 MB and
   one operation boundary took 37.7 ms and 4.74 MB of journal; with 16,384
   accounts, 4.74 MB, 100 ms, and 9.49 MB. The canonical limits (8,192
   accounts, 1,024 definitions and units, 4,096 access grants, 2,048
   allocation candidates) were set by that cost, not by any need of the
   model.
2. **One consumption took one lease.** A consumption runs under a
   completion lease bound to its one operation key. Settling one demand per
   household took one single-step lease per household: each active lease
   added about 3.6 KB to the live root, each consumption left nine hot
   terminal records (the lease's receipts and outcome beside the
   consumption's own), and one holder takes at most one lease per simulated
   minute (`MIN_REACQUIRE_COOLDOWN_MINUTES`), so a settlement office that
   requested every household's demand could settle one household a minute.
   Spreading the leases across holders hits the global bound of 1,024
   prepared grants (`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL`) and the
   candidate reservations of the active leases.
3. **Finished leases stayed live.** A completed, released, or expired lease's
   acquisition, grants, and certificate left the live root only when the
   archive commit that archived its last receipt applied, so 500 finished
   household leases kept 500 acquisitions in a 1.85 MB live root.
4. **An archive commit rewrote one bucket per record.** Hot terminal buckets
   held one record each at a full budget, and the commit rewrote each bucket
   it touched after decoding it: 441 ms and 20.8 MB of journal for 16,054
   records, 714 ms and 30.3 MB beside 16,384 accounts.
5. **A host expiry could expire grants early.** A host-queued `Expire` named
   the boundary it settles at, which the host cannot know, and
   `enqueue_resource_completion_operation` checked only its instant.
6. **Force-supply settlement scanned.** Settling a force's resource outcome
   needs the consumption's fulfillment, which only a search of every hot
   fulfillment found.

## Decision

### Live buckets

Accounts, access grants, definitions, unit revisions, and a new custodian
index persist in live buckets, `resource:live:{bucket}`
(`ResourceLiveBucketV1`), beside the hot terminal buckets. A record's bucket
is a hash of its identity, `ResourceLiveRecordKeyV1` (`Account`,
`AccessGrant`, `Definition`, `Unit`, `Custodian`), among
`resource_live_bucket_count(limits)` buckets: everything the limits allow
(accounts, the global access grant bound, definitions, units, and one
custodian per account) divided by `RESOURCE_LIVE_RECORDS_PER_BUCKET` (4) and
rounded up to a power of two, fixed for the life of the root. A bucket
exists as a domain record only once it holds a record. Both bucket kinds are
one record kind, `canwu.resource` `bucket` (`ResourceBucketRecord`, which
replaces `ResourceHotTerminalBucketRecord`), so a reader declares the one
state key it already declared, `resource_hot_terminal_state_key()`.

The live root keeps open demands, their reservations and allocation legs,
active transfers, leases, report grants, and observation heads. It counts
and totals what it does not hold in a `ResourceLiveLedgerV1` (counts per
kind, the total balance, accounts per protected-floor policy, encoded
bytes), as it counts the hot terminal records in its
`ResourceHotTerminalLedgerV1`. Every rule that counts accounts or grants,
totals balances (the conservation check), or asks whether a floor policy is
in use reads the attached records and the ledger together.

`ResourceCustodianAccountsV1` lists the accounts one custodian holds. A
custodian never changes and an account is created once, so an index entry
changes only when its custodian gains an account. Account creation and
`install_opening_account` keep the index, `into_records` derives it, and
validation refuses an account missing from its custodian's index. A fixture
that inserts accounts into `ResourceState::accounts` directly must call
`ResourceState::refresh_custodian_index()` before it applies an operation in
memory, as one that places hot candidates directly calls
`refresh_hot_terminal_ledger`.

### A boundary decodes what it touches

The resource plugin attaches, before it settles a packet, every live record
the packet names and every record those name: a leg's account, a demand's
source accounts and access grant, an account's definition and unit. An
allocation pass attaches the source accounts of the requester's due, dirty,
and expiring demands; a pooled demand attaches every account. A report
attaches the accounts its report grant names. A boundary then reads
and writes the live root and the buckets it changed: settling one operation
writes the same records whatever the number of accounts, and creating an
account writes its own bucket and its custodian's index.

The rules of 0.16.0 carry over to live records: a debug build settles every
boundary on the whole root as well and requires the same proposal, and a
release build refuses a boundary that would write over a live record it did
not attach (`invalid_boundary`).

Other plugins read through `ResourceRootReader`:
`ResourceRootReader::attach_live(keys)` attaches accounts, grants,
definitions, and units by identity, and
`ResourceRootReader::attach_custodian_accounts(holder)` attaches the index of
one holder and its accounts. `attach` and `resource_root_in_view` follow a
hot terminal record's closure into the live records it names.
`whole_resource_root_in_view` and `resource_state` still read every bucket.

### Consumption batches (G-17)

`ResourceConsumptionBatchRequestV1` (`ResourceOperationRequestV1::ConsumeBatch`,
operation kind `ResourceOperationKind::ConsumeBatch`) carries 1 to
`MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS` (256) `ResourceConsumptionBatchItemV1`
items, each one exact allocation leg with its demand, demand revision,
expected account revision, and consumption identity, beside one consumer
evidence, one instant, and one completion certificate.

- **One lease.** The certificate binds the batch's operation key and locks
  exactly `ResourceConsumptionBatchRequestV1::locked_targets()`: every item's
  account, leg, and demand, and the consumer evidence. The lease's holder
  must have requested every item's demand. It is a local lease: an external
  participant's certificate is refused.
- **Entry.** A batch enters only through the adapter ingress of its provider.
  The provider authorizes every item with a typed consumption intent
  (`resource_consumption_intents`) whose operation key is the item's
  reserved key, `resource_consumption_batch_item_key(batch key, index)`
  (`resource:consume-batch:{digest}:{index}`,
  `RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX`), and the consumer evidence is
  the provider's source version. A tracked command cannot carry a batch, and
  the force-supply reference settles its consumptions one at a time.
- **One outcome per item.** Items settle in order. Each item makes the checks
  a single consumption makes (leg, demand, and account revisions, a reserved
  leg with its active reservation, a new consumption identity, a balance that
  covers the leg, a fulfillment within the request, a current grant). An item
  that fails them is refused alone: a rejected `Consume` outcome under its
  key, with the refusal's code and reason and the leg's quantity as
  remainder, and the leg stays reserved. An item that passes debits its leg
  and records its consumption, its fulfillment, and an applied outcome under
  its key. Each item is atomic: all of that, or only its refusal.
- **Several legs of one account.** The batch remembers, for each account it
  debited, the revision it found (the one its certificate locks) and the one
  its last debit left. An item that expects the revision the batch found
  draws on the account as the batch left it, so a settlement office settles
  many demands drawn from one granary account in one batch.
- **The batch outcome.** Under the batch key, `quantity` totals the consumed
  quantity and `remainder` is the quantity of the refused items. The lease
  completes once, with one receipt, whatever the items did, and leaves the
  root: a batch whose every item is refused is applied with quantity zero.
- **Refusing the whole batch.** No items, more than 256, more than
  `max_batch_items_per_boundary`, a certificate that does not bind exactly
  the batch's targets, an item key already recorded, or a settlement error
  after an item's checks passed: one rejected batch outcome, no item outcome,
  and the lease left active. A batch key under a reserved prefix is refused
  by the ingress call, and skipped with `reserved_operation_key` if it is
  queued around the call.
- **Room.** Room is measured on the batch's whole result, as for every
  operation, and a batch is measured as unreserved work: it leaves more
  records than its lease reserves, so it must leave every other lease's
  reservation free (its own lease has finished by then). The terminal
  closures of the demands it finishes are not checked one by one as they
  are added, so a batch that does not fit records nothing and is skipped
  (`archive_backlog`, or `reserved_capacity`, permanent when no commit or
  release can make room), its lease left active for its holder to release.
  An item that consumes the one leg of its demand leaves
  `RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS` (6) hot terminal records, and the
  batch `RESOURCE_CONSUMPTION_BATCH_RECORDS` (2) beside them;
  `ResourceState::consumption_batch_can_fit(items)` says whether a batch of
  that many items can ever settle in the root, and a provider checks it
  before it takes the lease.
- **Idempotency.** A resend of the batch is answered by the batch outcome,
  hot or archived. Item keys are reserved: the ingress calls refuse any other
  request that names one, a plugin-scheduled packet that names one is skipped
  with `reserved_operation_key`, and since a batch key is used once, its item
  keys are unique for the run. Item outcomes, consumptions,
  and fulfillments are hot terminal records and archive like any other.
- **Per-boundary bound.** `ResourceLimitsV1::max_batch_items_per_boundary`
  (renamed from `max_mutations_per_boundary`, which earlier releases
  validated but never applied) bounds the batch items one boundary settles;
  a batch refused whole takes none of it. A batch that fits alone but not
  beside the batches the boundary applied is deferred, with the new
  `boundary_budget` event (`ResourceIngressSkipReasonV1::BoundaryBudget`,
  naming the instant), to the next boundary at the same instant: the
  resource plugin queues it to itself with no delay, the kernel settles no
  later boundary before that one, and that boundary settles the batch, since
  a boundary always settles a batch that fits the bound alone. A batch is
  bound to its instant by its `at` and its certificate, so it never waits for
  a later one.

One holder still takes one lease per simulated minute, and one lease locks
one version of the provider's record, so batches that settle together cite
records of their own or belong to different holders. The canonical limits
settle batches of `CANONICAL_BATCH_ITEMS_PER_BOUNDARY` (32) items, what the
canonical budget of 256 candidates holds beside one other lease's
reservation; a root that settles full batches of 256 raises
`max_batch_items_per_boundary` and `max_archive_candidates` together (a
batch of 256 needs 1,539 candidates). A lease of 256 items takes from the
global bound of 1,024 grants what a lease of one item takes.

### Limits (G-18, resource side)

| Limit | Before | After | Why |
| --- | ---: | ---: | --- |
| `max_accounts` | 8,192 canonical, 262,144 hard | unchanged | Live buckets: a boundary that draws on exact accounts or grants pays nothing per account, so a host may raise it to the hard maximum. A pooled demand's allocation pass reads every account (about 30 µs an account: 0.19 s at 8,192 accounts, 2.0 s at 65,536), which the canonical value keeps near what a 0.17.0 boundary cost. |
| `max_definitions`, `max_unit_revisions` | 1,024 | 16,384 canonical and hard (`MAX_DEFINITIONS`) | Live buckets; a boundary reads the ones it names. |
| `MAX_RESOURCE_ACCESS_GRANTS` | 4,096 | 16,384 | Live buckets; grants are still never archived. |
| `max_allocation_candidates`, `max_dirty_demands` | 2,048 canonical and hard | 2,048 canonical, 16,384 hard (`MAX_ALLOCATION_CANDIDATES`) | A pass writes its legs and reservations into the live root: 0.13 to 0.25 s and 4.5 MB of journal for 2,048 due demands, 0.8 s and 18 MB for 8,192, which leave a 12.6 MB live root. |
| `max_batch_items_per_boundary` | `max_mutations_per_boundary`, 4,096, unused | 32 canonical (`CANONICAL_BATCH_ITEMS_PER_BOUNDARY`), 2,048 hard | What the canonical archive budget holds; a batch item costs its boundary about 0.5 ms and 13 KB of journal. |
| `max_reports_per_boundary` | 256 | 256 | Unchanged: report grants live in the live root. |
| Completion grants (`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL`) | 1,024 | 1,024 | Unchanged: leases live in the live root, at about 3.6 KB each, and a batch covers 256 items with one. |

What exceeds a limit stays a recorded refusal.

### Leases leave the root when they finish (E-24)

`retire_finished_leases` runs after every operation and every completion
step: an acquisition that is released or expired, whose report reservation,
if it named one, has published, leaves the root with its grants, target
locks, expiry entries, and certificate. Its receipts stay hot terminal
records until an archive commit takes them, and the outcomes under its keys
answer a resend. A request about a finished lease under a key it has not
used is refused as `not_found`. Archiving a lease's last receipt no longer
touches the lease.

An acquisition identity starts one lease for the run, as before: an
`Acquire` of an identity a single step took, or a single step of one an
`Acquire` took, is refused as `idempotency_conflict`. With the lease gone
from the root, the evidence is the applied outcome under the other key
(`resource:completion:lease:{id}` for an `Acquire`,
`resource:completion:acquire:{id}` for a single step), hot or archived: each
lease start looks it up through the archive index as it looks up its own
key, one more archive lookup per lease start. A refused lease start took
nothing, so its outcome refuses nobody. A detached root (`apply_operation`)
sees only hot outcomes.

It is safe because nothing reads a finished lease: its certificate is
consumed, its locks are released, a report it reserved has published, and
the per-holder cooldown and token bucket live in the holder's partition, not
in the acquisition.

### Cheaper archive commits (E-24)

Hot terminal buckets now hold about two records each
(`RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET`): the bucket count is the
candidate budget divided by two, rounded up to a power of two. The plugin
applies an applied commit itself (`ResourceRootWork::apply_archive_commit`):
it groups the selected keys by bucket, takes the attached ones out of the
state, removes the others from the stored bucket's entries without decoding
their records, and rewrites or deletes each bucket once.
`ResourceState::record_archive_commit` then only records the receipt,
retention handle, and archive head.

Denser buckets make the commit cheaper and every operation dearer, since an
operation rewrites the bucket its outcome lands in with the records already
there; the density table under Measurements shows both. Two keeps an
operation boundary within a few kilobytes of journal of 0.17.0 and takes a
fifth off the commit's journal. A commit still costs its boundary in
proportion to the records it archives, so a host with a large budget commits
in chunks of about 2,048 records or fewer
(`prepare_resource_archive(candidate_limit)`) instead of the whole backlog
at once.

### Force-supply settlement without a scan (E-24)

`ResourceConsumption::fulfillment` names the fulfillment the consumption
recorded, and the reader's closure follows it, so a reader attaches both by
naming the consumption:
`resource_root_in_view(view, [Outcome(key), Consumption(id), Transfer(id)])`
gives `state.fulfillments[&consumption.fulfillment]` without a search. The
force-supply reference's settlement adopts this in E-22, on top of this
change. Its observation check (`validate_force_observation_source`) already
reads only the force holder's accounts, through
`ResourceRootReader::load(view)` and `attach_custodian_accounts`, and still
cites the current live root version.

### Host expiries (E-24)

`enqueue_resource_completion_operation` refuses `Expire` and
`ExpireExternalParticipants` whatever boundary they name: a host cannot know
which boundary admits a packet, since other ingress may open boundaries
first. `enqueue_resource_completion_expiry_tick`, which names no boundary,
is the host's way to expire grants. An expiry that another plugin schedules
through the completion ingress is checked when it settles: one that names
another boundary than the one settling it is skipped with
`failed_settlement_checks`, takes no key, and expires nothing early; the
tick expires the grant on time. An expiry that does not fit the root is
skipped, never deferred (`completion_step_can_wait` excludes it): a later
boundary could not settle it.

## Persistence and formats

Nothing written by 0.17.0 is read by 0.18.0; there is no migration.

| Item | Change |
| --- | --- |
| `RESOURCE_STATE_FORMAT_VERSION` | 4 (was 3). The state gains `live` and `custodian_accounts`; `ResourceConsumption` gains `fulfillment`. |
| Live root `resource:runtime:v1` | Holds no account, access grant, definition, unit revision, or custodian index; its `live` ledger counts them. |
| Record kind | `canwu.resource` `bucket` (was `hot-terminal`), payload JSON: `ResourceHotTerminalBucketV1` under `resource:hot-terminal:{n}`, `ResourceLiveBucketV1` (`RESOURCE_LIVE_BUCKET_FORMAT_VERSION` 1, entries in ascending key order) under `resource:live:{n}`. |
| Hot terminal buckets | Candidate budget / 2 (was the budget), so records land in other buckets than in 0.17.0. |
| Byte cap | Counts the live entries' encoding through the ledger. |
| Ingress | `ConsumeBatch` on the adapter ingress; the skip reason `boundary_budget`; the deferred packet may wrap an adapter packet. |
| Limits | `max_mutations_per_boundary` is `max_batch_items_per_boundary` in the encoded limits. |
| `RESOURCE_SEMANTIC_HASH` | Changes. |
| Kernel formats | Unchanged. |

`ResourceState::into_record` now refuses any root that holds accounts,
access grants, definitions, unit revisions, or a custodian index, as it
already refused hot terminal records: those persist in buckets, and one
record cannot hold them. This breaks every caller that installed a root with
opening accounts through `into_record`; they call `into_records`.

Restoring a root also checks two links it did not check before: every
account lies in its custodian's index, and every consumption names the
fulfillment it recorded (same demand, same operation key, its leg among the
fulfillment's legs), in the root and within each archive blob that holds
both. A whole root that holds a consumption holds its fulfillment, which is
recorded and archived after it.

## Compatibility

0.18.0 is a pre-1.0 minor release; same-version crates only.

- **Added:** `ResourceConsumptionBatchRequestV1` (with `item_operation_key`
  and `locked_targets`), `ResourceConsumptionBatchItemV1`,
  `MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS`,
  `RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX`,
  `RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS`,
  `RESOURCE_CONSUMPTION_BATCH_RECORDS`,
  `resource_consumption_batch_item_key`,
  `ResourceState::consumption_batch_can_fit`; `ResourceLiveRecordKeyV1`,
  `ResourceCustodianAccountsV1`, `ResourceLivePayloadV1`,
  `ResourceLiveEntryV1`, `ResourceLiveBucketV1`, `ResourceLiveLedgerV1`,
  `RESOURCE_LIVE_BUCKET_FORMAT_VERSION`,
  `RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET`,
  `RESOURCE_LIVE_RECORDS_PER_BUCKET`, `resource_live_bucket_reference`,
  `resource_live_bucket_count`, `resource_live_bucket`;
  `ResourceRootReader::{attach_live, attach_custodian_accounts}`;
  `ResourceState::{live, custodian_accounts}` (fields) and
  `ResourceState::{live_records_attached, account_count, access_grant_count,
  definition_count, unit_count, total_account_balance,
  refresh_custodian_index}`; `ResourceConsumption::fulfillment`;
  `ResourceLimitsV1::{MAX_DEFINITIONS, MAX_ALLOCATION_CANDIDATES,
  MAX_BATCH_ITEMS_PER_BOUNDARY, CANONICAL_BATCH_ITEMS_PER_BOUNDARY}`.
- **Changed (breaking):** new variants of exhaustive enums,
  `ResourceOperationRequestV1::ConsumeBatch`,
  `ResourceOperationKind::ConsumeBatch`, and
  `ResourceIngressSkipReasonV1::BoundaryBudget`;
  `ResourceHotTerminalBucketRecord` is `ResourceBucketRecord`, record kind
  `canwu.resource` `bucket`; `ResourceState` and `ResourceConsumption` gain
  fields (struct literals break, and a fixture that inserts accounts
  directly calls `refresh_custodian_index`); `ResourceLimitsV1`'s
  `max_mutations_per_boundary` is `max_batch_items_per_boundary`, now
  applied; `ResourceState::into_record` refuses a root with live records; a
  live root decoded directly (`ResourceRuntimeRecord`) holds no accounts, so
  a reader attaches what it reads; `resource_hot_terminal_bucket_count` is
  half of what it was; the limits above; validation refuses an account
  missing from its custodian's index and a consumption that names another
  fulfillment; `enqueue_resource_completion_operation` refuses `Expire` and
  `ExpireExternalParticipants`.
- **Behavior:** a finished lease leaves the root, so
  `resource_completion_status`, `resource_completion_certificate`, and
  `resource_completion_grant` find it no more; its identity still starts no
  other lease, at one more archive lookup per lease start;
  `max_batch_items_per_boundary` bounds batch items, and a batch past it is
  deferred to the next boundary at the same instant; a plugin expiry that
  names another boundary is skipped.
- **Other crates, in this change:** `canwu-force-supply-reference`'s
  observation check reads the holder's accounts through the custodian index
  (and gains the test above); `canwu-economy-reference`'s grain harness
  installs its root with `into_records`; in `canwu-production`'s tests,
  `g3_contract.rs` installs one root with `into_records` and calls
  `refresh_custodian_index` after inserting accounts directly, and it and
  `tests/support/mod.rs` set `ResourceConsumption::fulfillment` in their
  consumption literals, as does the harness's `production-archive-profile`.
  The harness's `resource-root-profile` gains `--accounts`, `--households`,
  `--pooled`, `--due`, and `--boundary-batch`.

## What can still fail a boundary

The lists of [resource refusals without stalls](resource-refusals-without-stalls.md#what-can-still-fail-a-boundary)
and [resource root write cost](resource-root-write-cost.md#what-can-still-fail-a-boundary)
hold, with live buckets beside hot terminal buckets:

1. **A live bucket that is malformed, not canonical, misplaced, or short of
   what the ledger counts**, read when a packet names one of its records: a
   tampered restore that activation would have refused, or an engine defect.
2. **In a debug build only, a boundary that would settle differently on the
   whole root.**
3. **A boundary that would write over a live record it did not attach**
   (`invalid_boundary`): the release-build backstop for a missing attachment
   rule.

A consumption batch adds nothing to the list: each refusal is an item
outcome, a batch outcome, or a skip. A host expiry is refused at the call,
before it is queued.

## Durable tests

Each was checked to fail with its fix reverted once in the working tree.

`gap_g17_resource_consumption_batch.rs`:

- `a_batch_settles_each_item_under_one_lease_and_refuses_a_stale_item_alone`:
  a provider authorizes a batch over three households and one demand is
  cancelled after the lease locked it. The batch applies two items, refuses
  the cancelled one alone (`version_conflict`) and leaves its account
  untouched, records one batch outcome (quantity 20, remainder 10, the
  refused leg), and completes its lease once, with one receipt, after which
  the lease leaves the root; each applied consumption names its fulfillment;
  a resend settles nothing, a request under an item key is refused, and a
  restored run and an exact replay agree. Last, the provider's boundary
  system reads the root through `resource_root_in_view`, naming the batch
  outcome and the consumptions only, and finds each applied consumption's
  fulfillment: the scan-free path the force-supply settlement will take. It
  fails when a failing item refuses the whole batch, when finished leases
  stay in the root, when the remainder counts items, and, at the audit, when
  a consumption's closure does not follow its fulfillment.
- `a_boundary_settles_at_most_its_budget_of_batch_items`: with a budget of
  three items, a batch of four is a recorded rejection (remainder 40) that
  leaves its lease active and takes nothing of the budget, so the first
  batch of two beside it settles; the second batch of two is deferred with
  `boundary_budget` to the same instant, and when the host advances time the
  next boundary, at that instant, settles it with nothing skipped. It fails
  when the batch is skipped instead of deferred, and when a refused batch
  takes from the budget.
- `a_full_batch_settles_under_the_canonical_limits`: 32 households, the
  canonical per-boundary bound, settle in one batch under the canonical
  limits, `consumption_batch_can_fit` admits 32 and not 33, and the batch
  leaves 6 records an item and 2 of its own.
- `a_batch_whose_records_do_not_fit_is_skipped_and_records_nothing`: with 64
  archive candidates a batch of twelve (which `consumption_batch_can_fit`
  refuses, as it admits ten) is skipped with `reserved_capacity`, the root
  stays as it was, no key is taken, and the lease stays active. It fails when
  a finished demand's closure checks the budget as it is added, the 0.18.0
  draft's recorded `limit_exceeded` rejection of the whole batch.
- `a_batch_consumes_several_legs_of_one_account`: three demands drawn from
  one account settle in one batch; the account's balance drops by 30 and its
  revision by three. It fails when every item must find the revision it
  names (one item applied, two `version_conflict`).
- `a_consumption_naming_another_fulfillment_is_refused_when_the_root_is_restored`:
  a hot consumption pointed at the other household's fulfillment, and at a
  fulfillment the root does not hold, with its digest and the ledger's bytes
  recomputed, is refused by `ResourceState::from_records`. It fails without
  the link check.

Elsewhere:

- `resource_root_write_set.rs`,
  `a_boundary_writes_the_live_root_and_the_buckets_it_changes_whatever_the_backlog`:
  with no backlog and no other accounts, and with 600 hot records and 1,000
  accounts of other custodians, one operation boundary writes the live root
  and the new outcome's bucket only and its journal differs by less than
  16 KB; creating an account writes its bucket and its custodian's index; a
  budget of 1,024 candidates has 512 hot buckets, and an archive commit of
  every hot record writes each stored bucket once. It fails with one record
  per hot bucket, and when every attached live record is rewritten, changed
  or not (in a debug build that already fails every boundary of every test
  through the whole-root equivalence check).
- `resource_expiry_tick_key.rs`,
  `a_completion_expiry_names_its_settling_boundary_or_settles_nothing`: the
  host's `Expire` is refused at the call for every boundary it might name; a
  plugin's expiry that names a later boundary is skipped and expires nothing
  early, and the tick expires the grant on time under its own key. It fails
  when the call accepts a host expiry, and when settlement checks the
  expiry's instant only.
- `gap_g53_resource_single_step_lease.rs`,
  `same_place_transfer_leaves_fifteen_records_and_eleven_lookups`: a holder's
  tracked `Acquire` of a finished single step's identity is a recorded
  `idempotency_conflict`, while the single step's outcome is hot and again
  once it is archived; the counts include each lease start's lookup of the
  other key. It fails without the sibling lookup.
- `canwu-force-supply-reference` `resource_observation_source.rs`,
  `a_resource_provider_observation_is_checked_against_the_holder_account_stock`:
  a resource provider observation whose interval holds the holder's two
  accounts (25) is recorded and one that excludes it is refused, beside a
  larger account of another holder. It fails when the check reads the live
  root alone, as before this change (the stock reads zero).
- Lease retirement is asserted where leases finish: `gap_g53` (both transfer
  leases and the stray acquisition leave the root, their receipts stay),
  `resource_contract` (an aborted and an expired lease leave the root, their
  receipts record how), and the batch fixture; each fails with retirement
  turned off.

Beside them, the debug-build equivalence check runs in every test of the
resource plugin and its dependents, now over live records as well, and the
batch's item keys and a lease start's sibling key are attached with the
packet, so a check that reads them sees what the whole root holds.

## Measurements

`resource-root-profile` and `resource-archive-profile`
(`benchmarks/performance-harness`), release builds, Rust 1.99.0, Windows
x86-64, AMD Ryzen 7 7800X3D, medians of five samples after one warm-up (the
scale cases, three). Before is the base `8fb922e` with the profile's new
options; after is this change. The reports are
[`2026-10-16-resource-root-0.18.0-before-elapsed.json`](../../benchmarks/baselines/2026-10-16-resource-root-0.18.0-before-elapsed.json),
[`2026-10-16-resource-root-0.18.0-after-elapsed.json`](../../benchmarks/baselines/2026-10-16-resource-root-0.18.0-after-elapsed.json),
and
[`2026-10-16-resource-root-0.18.0-after-scale-elapsed.json`](../../benchmarks/baselines/2026-10-16-resource-root-0.18.0-after-scale-elapsed.json)
(each names the base commit with a dirty tree); the density runs were not
kept. The machine was shared with other builds throughout, so times vary by
up to a factor of two between runs (code this change does not touch, such as
archive lookup misses, moved by as much); journal bytes and record counts
are exact. The density table interleaves the builds to compare them in the
same minutes.

The profile's root holds the given hot terminal backlog (rejected outcomes of
about 550 bytes) and accounts of other custodians; one operation boundary
settles one allocation pass that is refused, so it writes one new outcome.

One operation boundary (time, journal bytes):

| Accounts | Hot records | Before | After |
| ---: | ---: | ---: | ---: |
| 0 | 0 | 0.19 ms, 8.0 KB | 0.28 ms, 8.2 KB |
| 0 | 16,000 | 0.37 ms, 10.7 KB | 0.57 ms, 10.9 KB |
| 8,192 | 0 | 37.7 ms, 4.74 MB | 0.60 ms, 8.3 KB |
| 16,384 | 0 | 100 ms, 9.49 MB | 1.27 ms, 8.3 KB |
| 16,384 | 16,000 | 104 ms, 9.49 MB | 1.02 ms, 10.9 KB |
| 65,536 | 0 | | 0.68 ms, 8.3 KB |
| 65,536 | 16,000 | | 0.72 ms, 10.9 KB |

The live root is 2.3 KB at every account count (2.37 MB at 8,192 accounts
and 4.74 MB at 16,384 before). Eight operations in one boundary with 16,384
accounts: 191 ms and 9.50 MB before, 2 to 16 ms and 18.5 KB after.

The archive commit of every hot record (time, journal bytes):

| Accounts | Records | Before | After |
| ---: | ---: | ---: | ---: |
| 0 | 4,054 | 86.1 ms, 5.21 MB | 64.0 ms, 4.32 MB |
| 0 | 16,054 | 441 ms, 20.8 MB | 315 ms, 17.1 MB |
| 16,384 | 16,054 | 713 ms, 30.3 MB | 370 ms, 17.1 MB |
| 65,536 | 16,054 | | 424 ms, 17.1 MB |

Hot bucket density. Packing records into a bucket makes the commit cheaper
and every operation dearer: an operation rewrites the bucket its new outcome
lands in, with the records already there. Two interleaved rounds of the same
profile with 1 (the base), 2, 4, and 8 records per bucket, at full candidate
budgets of 4,096 and 16,384 (time of each round, journal bytes):

| Per bucket | One operation, 16,000 hot | Eight operations, 4,000 hot | Commit of 4,054 | Commit of 16,054 |
| ---: | ---: | ---: | ---: | ---: |
| 1 (0.17.0) | 0.56 / 0.71 ms, 10.7 KB | 0.91 / 1.34 ms, 27.6 KB | 114 / 170 ms, 5.21 MB | 579 / 2,487 ms, 20.8 MB |
| 2 (chosen) | 0.64 / 0.85 ms, 10.9 KB | 0.90 / 1.92 ms, 36.7 KB | 53 / 134 ms, 4.32 MB | 376 / 538 ms, 17.1 MB |
| 4 | 0.65 / 1.11 ms, 13.2 KB | 1.01 / 1.71 ms, 53.1 KB | 42 / 70 ms, 3.48 MB | 289 / 455 ms, 13.8 MB |
| 8 | 0.76 / 0.80 ms, 17.9 KB | 2.00 / 2.21 ms, 97.0 KB | 44 / 69 ms, 2.95 MB | 447 / 353 ms, 11.7 MB |

At eight records a bucket an operation boundary pays some 0.1 to 0.3 ms and
7 KB more, and a boundary of eight operations about 1 ms and 70 KB more,
than at one; the commit it saves runs once per archive cycle. Two keeps the
operations within a few kilobytes of 0.17.0 and still takes a fifth off the
commit's journal; a host bounds the commit itself by committing in chunks of
about 2,048 records or fewer.

Raised limits, one boundary each on a fresh root (time of each sample,
journal bytes):

| Case | After |
| --- | ---: |
| Pooled demand's pass, 8,192 accounts | 159 to 222 ms, 17.6 KB |
| Pooled demand's pass, 16,384 accounts | 485 to 533 ms, 17.6 KB |
| Pooled demand's pass, 65,536 accounts | 1.91 to 2.44 s, 17.6 KB |
| Allocation pass, 2,048 due demands | 126 to 248 ms, 4.54 MB |
| Allocation pass, 8,192 due demands | 0.79 to 1.02 s, 18.2 MB |
| Consumption batch, 32 items, adapter ingress | 14 to 28 ms, 432 KB |
| Consumption batch, 256 items, adapter ingress | 105 to 151 ms, 3.26 MB |

A pooled pass reads every account, at about 30 µs an account, where a 0.17.0
boundary decoded the whole live root at about 5 µs an account; the
canonical `max_accounts` stays where that costs a pass a fifth of a second.
A large allocation pass writes its legs and reservations into the live root
(12.6 MB after 8,192), so the canonical candidate bound stays 2,048. A batch
item costs its boundary about 0.5 ms and 13 KB of journal, mostly its
account's live bucket and the hot buckets its six records land in; 32 items
fit the canonical archive budget.

Leases and batches, in memory on a whole root (`apply_operation`, which
validates the root after each operation), 100 and 500 households with one
allocated demand each:

| | Before, one lease each | After, one lease each | After, batches of 256 |
| --- | ---: | ---: | ---: |
| Leases | 100 / 500 | 100 / 500 | 1 / 2 |
| Hot terminal records per consumption | 9.0 | 9.0 | 6.0 |
| Live root with the leases active | 583 KB / 2.91 MB | 549 KB / 2.74 MB | 235 KB / 978 KB |
| Live root after the consumptions | 372 KB / 1.85 MB | 54 KB / 262 KB | 2.9 KB / 2.9 KB |
| Finished leases left in the root | 100 / 500 | 0 | 0 |
| Settlement time | 5.5 / 54 ms a consumption | 4.6 / 24 ms a consumption | 6.1 / 23 ms a batch |

What is left after one-lease consumptions is the holders' lease partitions,
one per household here.

`resource-archive-profile --preset smoke` writes the same bytes before and
after (the lookup index is unchanged), and its times agree within the
machine's noise.

## Deviations

- **E-24 host expiry.** The brief asked to refuse a host `Expire` with the
  wrong `current_boundary` at enqueue. A host cannot know the right one, so
  every host `Expire` and `ExpireExternalParticipants` is refused there, and
  a plugin-scheduled expiry is checked when it settles.
- **E-24 force-supply settlement.** The scan-free path is the API above; the
  force-supply reference adopts it in E-22. Only its observation check
  changed here.
- **G-18 report and completion-grant bounds** stay as they were (above):
  both live in the live root, which every changing boundary rewrites, and
  batches remove the need for more leases.
- **`max_batch_items_per_boundary`** (renamed from
  `max_mutations_per_boundary`) gains a meaning, its canonical value drops
  from 4,096 to 32, what the canonical archive budget holds, and its hard
  maximum is 2,048.
- **G-18 accounts, allocation candidates, and dirty demands** keep their
  canonical values (8,192, 2,048, 2,048): the measured costs of a pooled
  pass and of a large allocation pass (above) do not allow more by default.
  Hard maxima are 262,144 accounts (unchanged) and 16,384 candidates and
  dirty demands.
- **The fulfillment-link forgeries** are tested in
  `gap_g17_resource_consumption_batch.rs`, which holds consumptions, not in
  the write-set tamper test, whose root holds none.

## Known limits

- **The rest of the live state is still decoded whole.** Open demands with
  their reservations and legs, active transfers, leases, report grants, and
  observation heads stay in the live root, so `max_demands`,
  `max_transfers`, leases, and report grants bound what every changing
  boundary costs (about 790 KB of live root for 500 open household demands
  with their reservations and legs; an allocation pass over 8,192 due
  demands leaves a 12.6 MB live root).
- **A pooled demand reads every account.** Its allocation pass attaches all
  of them, at about 30 µs an account (0.19 s at 8,192 accounts, 0.5 s at
  16,384, 2.0 s at 65,536): more than the whole live root cost a 0.17.0
  boundary at the same count, so the canonical `max_accounts` stays 8,192,
  and a host that raises it draws on exact accounts or grants.
  `resource_state`, archive preparation, and restore validation read every
  bucket too.
- **A holder's custodian index is one entry.** It lists every account the
  holder holds, so a holder with many accounts rewrites a large entry each
  time it gains one (some 250 KB, journaled twice, for 10,000 accounts), and
  reading one holder's stock reads its whole index. Households hold a few
  accounts each; a holder with thousands is the case it does not serve.
- **One lease per holder per simulated minute** still bounds one holder's
  batches to one a minute.
- **A live bucket can hold several records** (about four at a full budget);
  it is rewritten with each.
- **An operation rewrites its outcome's hot bucket with the record already
  there**, about one at a full budget (see the density table).
- **A batch item costs its boundary about 0.5 ms and 13 KB of journal**,
  mostly the rewrite of its account's live bucket and of the hot buckets its
  records land in, so the hard bound on batch items per boundary is 2,048.
