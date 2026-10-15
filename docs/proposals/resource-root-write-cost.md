# Resource Root Write Cost

Status: implemented in 0.16.0. It changes how `canwu-resource` persists its
root and what a boundary of the resource plugin reads and writes. The
operations, their outcomes, their refusals, and the archive lookup index keep
their 0.16.0 semantics. `canwu-production`, `canwu-economy-reference`, and
`canwu-force-supply-reference` read the root through the new readers.

It builds on [resource archive lookup and volume](resource-archive-lookup-and-volume.md)
(0.15.0), whose "Choosing the budget" section measured the cost this change
removes, and on [resource refusals without stalls](resource-refusals-without-stalls.md)
(0.16.0). The engine's own record writes became cheaper in the same release
([record store ceiling and cost](record-store-ceiling-and-cost.md)); the
measurements below separate the two.

## Problem

The resource root was one domain record, `resource:runtime:v1`, holding the
whole `ResourceState`: the live state (accounts, open demands, active
transfers, leases, reports) and every hot terminal record, that is, every
archive candidate (operation outcomes, closed demands with their reservations
and allocation legs, consumptions, fulfillments, losses, settled transfers,
finished external participants, lease receipts, and the receipts of applied
archive commits). Every boundary that settled a resource operation decoded
that record, applied the operation to a copy, validated and re-encoded the
whole root for the byte cap, re-encoded it as the new version, and journaled
both versions. The invariant systems decoded and validated the proposed root
again, and the report system decoded it on every event-bearing boundary, with
or without resource work.

So a boundary cost time and journal bytes in proportion to the hot terminal
backlog. On the integration base (`f511463`) one boundary settling one
operation took 15 ms and 316 KB of journal at 256 hot records and 397 ms and
4.4 MB at 4,000, with 2.4 million allocations; a boundary with nothing for the
resource plugin still took 29 ms at 4,000. That is why `max_archive_candidates`
stopped at 4,096 and a host application had to commit often.

## Decision

### The root persists in parts

The root is still one `ResourceState` in memory. It persists as:

- **the live root**, the domain record `resource:runtime:v1`, whose payload is
  now `ResourceLiveRootV1`: the state without the bodies of its hot terminal
  records and without their candidate index. It encodes as `ResourceState`
  does. Its new field `hot_terminal` (`ResourceHotTerminalLedgerV1`) counts the
  hot terminal records; and
- **the hot terminal buckets**, domain records `resource:hot-terminal:{bucket}`
  of the new kind `canwu.resource` `hot-terminal`
  (`ResourceHotTerminalBucketRecord`, payload `ResourceHotTerminalBucketV1`,
  format 1). Each hot terminal record is one entry
  (`ResourceHotTerminalEntryV1`: its terminal sequence, its key, the record,
  and, for a demand, the reservations of its closure that are still hot) in
  the bucket its identity hashes to (`resource_hot_terminal_bucket`, a
  domain-separated BLAKE3 of the identity). A root has as many buckets as its
  candidate budget rounded up to a power of two
  (`resource_hot_terminal_bucket_count`), so a full budget holds about one
  record per bucket. A bucket is created when it first receives a record and
  is rewritten, never deleted, after that: an archive commit leaves it empty.

The split is a function of the state. `ResourceState::into_records` persists a
whole state as its live root and its non-empty buckets, and
`ResourceState::from_records` restores it, checking that every entry lies in
its bucket and is in its canonical encoding, that the ledger counts exactly
the stored entries, and that the whole state validates. Plugin activation runs
the same checks, so a restored or installed root is whole and valid before any
boundary reads part of it.

### The ledger

Every rule of the root that counts hot terminal records or asks whether any
of them has some property reads the ledger, so the live root alone answers
it:

| Ledger field | Read by |
| --- | --- |
| `records` | the candidate budget, room measurement, the archive source root |
| `outcomes` | the hot outcome budget (`max_operation_outcomes`) |
| `transfers` | `max_transfers` when a transfer or an exchange starts |
| `applied_archive_maintenance_receipts` | the receipt budget |
| `demand_requesters` | an allocation pass's check that its requester owns a demand |
| `demand_floor_policies` | the retirement of content-addressed protected-floor policies |
| `lease_receipt_acquisitions` | the retirement of a finished lease when its last receipt is archived |
| `bytes` | the byte cap |

The state keeps the ledger current whenever a record becomes an archive
candidate (`add_hot_candidate`) or leaves the root (`remove_hot_terminal`,
and a rejection taken back for want of room). A whole state's ledger equals
what its attached records give; `ResourceState::validate` checks that, and,
for a partial state, that the ledger covers the attached records.

`bytes` is the encoded size of every hot terminal entry in a persisted live
root; in memory it counts only the entries that are not attached. The byte
cap `max_state_bytes`, and the operations' share of it, now bound the encoded
live root plus every hot terminal entry.

### A boundary reads and writes what it touches

A boundary system of the resource plugin works on a `ResourceRootWork`: the
live root, decoded, with the hot terminal records it reads attached from their
buckets. Before it settles each packet, the lifecycle system attaches

- the outcome under every operation key the packet settles, including the key
  a completion operation derives from its identities and the two keys an
  expiry tick settles under, which name the instant and the boundary;
- every hot terminal record whose identity the packet's payload names, as a
  string anywhere in it (a namespaced identity names at most one record of
  each kind);
- for an archive commit, every record it selects, only when its expected
  source root is the root's (a stale commit applies nothing and reads
  nothing; its payload is not searched for names), and for a retention
  acknowledgement, the receipt it names; and
- transitively, every record those name: the closure of a demand (its hot
  reservations and their legs), and what a reservation, allocation leg,
  consumption, fulfillment, transfer, or loss belongs to, followed through
  live records as well. A record archived earlier in the same boundary is not
  attached again.

Command admission attaches the same way for the command it admits, and the
report system attaches the records the observation heads it publishes cite.
After the packets, `ResourceRootWork::persist` writes the live root and every
bucket whose entries changed: a new hot terminal record, an attached one that
changed (a closed demand can still be fulfilled by a transfer accepted after
its closure became hot), and an archived one. An attached record that did not
change is not written again.

`persist` also refuses to write a record it did not attach under a key its
bucket already holds. Such a record is new to the settlement but not to the
root: the boundary read past a stored record, and writing would replace an
answer the settlement never saw and leave the ledger counting both. This is
the release-build backstop for a missing attachment rule. Where the debug
assertion below does not run, a missed rule that would have overwritten a
stored outcome silently, and left every later whole-root read failing on the
miscounted ledger, now fails that one boundary with `invalid_boundary`, an
engine invariant: the boundary rolls back and the stored root stays intact.

A boundary that admits no resource packet reads nothing of the root: the
lifecycle system returns before it reads, the invariant systems read only a
live root proposed in the same boundary, and the report system reads two
fields of the live root's encoded value (`report_dirty_grants` and
`report_due_index`) and decodes the root only when a report is dirty or due.

### Settling on part of the root settles as on the whole root

A partial state settles every operation exactly as the whole state would, for
two reasons. Every count and aggregate over hot terminal records reads the
ledger, so it does not depend on what is attached. Every hot terminal record
an operation reads, it reads by identity, and the attachment rule above
attaches each such record before the packet settles: an operation reads the
records its request names and the records those belong to.

Every debug build checks this. The lifecycle system, command admission, and
the report system each run a second time on the whole root (every bucket
attached), with the archive objects the first run read served again, and
assert that both runs propose exactly the same directives, record writes
included. Every test suite of the resource plugin and of the crates that use
it (production, the economy and force-supply references, fiscal) ran under
this check. It caught one rule while this change was written: a completion
operation derives its operation key from its identities, so its outcome is
not named by its payload and is attached by key. Review found a second: an
expiry tick names nothing, and settles its expiries under keys derived from
the boundary, which a host application's completion operation can take
first. The tick now attaches both keys and resolves their holder before it
settles: the same expiry is a resend, another request's is skipped with
`operation_key_taken`, and the tick is scheduled again, at a boundary whose
key is free. (In 0.15.0 an expiry of another candidate limit under the key
failed the boundary with an idempotency conflict.)

### Other plugins read the root by record

`ResourceRuntimeRecord::Payload` is now `ResourceLiveRootV1`, so code that
decoded the root and read a hot terminal record from it no longer compiles.
A boundary system or command handler of another plugin reads the root with
`resource_root_in_view(view, keys)` or `ResourceRootReader`, which attach the
hot terminal records it names (with their closures), or with
`whole_resource_root_in_view`, which attaches every bucket. It declares the
read of `resource_hot_terminal_state_key()` beside the live root's. The host
application's `resource_state` reads the whole root, from either runtime
handle (`Canwu` or `CompactedCanwu`).

- `canwu-production` attaches the consumptions, allocation legs, and outcomes
  its inputs cite, the outcomes under its output keys, and the external
  participant of a completion. Its checks of the resource root validate the
  live root with those records.
- `canwu-economy-reference` attaches the demands and harvest credit a month
  closes on, the transfer a delivery names, and the outcome, leg, and
  consumption a force externality cites.
- `canwu-force-supply-reference` attaches the outcomes of pending intents. Its
  settlement of a resource outcome finds the consumption's fulfillment among
  every hot fulfillment, so it reads the whole root. Its restore validation
  reads a cited outcome from the resource root while it is hot, and from the
  resource archive, through the archive provider attached to the runtime,
  once it is archived, instead of from the body of the cited live root
  version, which no longer holds it; the cited version must still be
  retained, as before. The restore helpers
  (`from_force_supply_snapshot_json`, `from_force_supply_checkpoint_journal`,
  `replay_force_supply_from_journal`) therefore take the archive provider and
  attach it before they validate, as
  `validate_force_supply_runtime_with_archive_store` reads through the one
  attached. `validate_force_supply_runtime` reads no archive: it compares
  hot citations only, and refuses one that is not hot only when the resource
  root has no archive.

### The archive source root

An archive commit applies only to the root it was prepared from. The digest
that names that root (`archive_source_root`) covered the candidate index,
which a partial root does not hold. It now covers the state revision, the
admitted sequence, the number of hot terminal records, and the archive head
(`canwu.resource.archive-source-root.v2`). Every change of the root advances
the revision or the sequence, and an applied commit changes the head and the
count within the boundary that applies it, so a stale commit is still stale.
`prepare_resource_archive` requires a whole root.

### The ceiling

`ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES` rises from 4,096 to 16,384, and
with it the pages one archive commit may carry
(`MAX_RESOURCE_ARCHIVE_COMMIT_PAGES`, 16 to 64), so one commit still settles
every candidate (`MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS`, 16,384). The hard
maximum of `max_operation_outcomes` rises to 16,384
(`ResourceLimitsV1::MAX_OPERATION_OUTCOMES`); its canonical value stays 8,192
and the canonical candidate budget stays 256.

What bounds the budget now is no longer the cost of ordinary boundaries,
which does not depend on it, but the work that handles the backlog:

- the boundary that applies an archive commit rewrites one bucket per archived
  record: 103 ms and 5.2 MB of journal for 4,054 records, 524 ms and 21 MB for
  16,054, on the record store build;
- the host application reads every hot record to prepare a commit, and the
  lookup index grows by the commit's keys (in 0.15.0, 41 ms and 86 ms to
  prepare and store a commit of 4,096 records);
- a restore, and plugin activation, validate every hot record;
- a commit of 64 pages that goes stale records a retention handle of about
  13 KB in the live root, against the 64 KiB maintenance reserve of the byte
  cap: a root near its cap has room for about 5 unacknowledged stale commits
  of that size, against about 16 of the 16 pages 0.15.0 allowed.

A budget above 16,384 would make the commit that clears it a boundary of
several seconds. A host application with a large backlog commits it in
batches, each acknowledged before the next is prepared, and one that wants a
larger backlog commits part of it more often.

## Invariants

- The records `into_records` gives for a state and the records a boundary's
  writes leave hold the same entries: the live root without the hot terminal
  records, and each hot terminal record as one entry of the bucket its
  identity hashes to. They differ only where no entry is and in versions: a
  bucket an archive commit emptied stays, empty, where `into_records` gives
  none, and a boundary updates record versions where `into_records` gives
  fresh records.
- A whole state's ledger counts exactly its hot terminal records; a partial
  state's ledger counts every hot terminal record of the root, attached or
  not.
- A boundary of the resource plugin proposes exactly what it would propose on
  the whole root (asserted in every debug build).
- A boundary writes the live root and the buckets whose entries changed, and
  nothing else of the root. One with nothing for the resource plugin reads
  and writes nothing of it.
- Every rule the 0.16.0 resource packages set holds unchanged: an expected
  refusal never fails a boundary (skips, deferrals,
  `resource_deferred_packet_v1`), a decision reads no retained evidence, the
  archive lookup index answers as before, single-step leases and protected
  floors behave as before, and a live run, a sealed run, an unsealed fork, a
  run restored from sealed segments, and an exact replay agree.

## Alternatives rejected

- **Keep the root whole and cache its decoded form.** A boundary system is a
  function pointer with no state of its own, a process-wide cache would be
  keyed by the root's content and would still copy the root for every
  operation, and the write and the journal would still carry the whole root.
- **One record per hot terminal record.** A deleted domain record leaves a
  tombstone for the life of the run, and its reference can never be created
  again, so every archived record would leave a kernel record behind. Buckets
  are reused: the number of resource records is bounded by the bucket count.
- **Keep the candidate index in the live root.** It would keep the archive
  source root and the candidate dedupe as they were, but the live root would
  still grow by about 70 bytes per hot record and every boundary would decode
  and journal it.
- **Treat a hot terminal record as archived for every rule** (absent unless
  looked up), which would need no ledger. It changes outcomes: an allocation
  pass of a requester whose only demands are hot terminal would be refused,
  a cancellation of a hot closed demand would read as not found, and a
  protected-floor policy named only by a hot closed demand would be retired.
  The ledger keeps every answer as it was.
- **Pre-load the records each operation kind reads, by kind.** Exact but
  fragile: every new field or operation would need its entry. Attaching every
  identity a packet names, with the closures, is generic, and the debug-build
  equivalence check covers it.
- **Split the live state as well** (one record per account, demand, or
  lease). It would bound a boundary by the records it touches even with a
  large live state, but every rule over live records (conservation, room
  reservation, report scheduling) would need a ledger of its own. The live
  state is bounded by its own budgets; see "Known limits".

## Persistence, replay, and formats

Nothing written by 0.15.0 is read by 0.16.0, so there is no migration.

| Item | Change |
| --- | --- |
| `RESOURCE_STATE_FORMAT_VERSION` | 3 (was 2). The state gains `hot_terminal`. |
| Live root `resource:runtime:v1` | Payload type `ResourceLiveRootV1` (same encoding as `ResourceState`). It holds no hot terminal record and an empty candidate index. Its version is still the state revision. |
| New record kind `canwu.resource` `hot-terminal` | `resource:hot-terminal:{bucket}`, payload `ResourceHotTerminalBucketV1`, `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION` 1, entries in ascending key order. Registered as a record schema of the resource plugin. |
| Byte cap | Measured on the live root and the hot terminal entries, not on one encoded root. |
| Archive objects | Blobs, retention handles, and directories name the v2 archive source root, so their ids differ from 0.15.0. Their shapes are unchanged. A directory may list 64 blobs. |
| Ingress | Unchanged. An archive commit packet may select 16,384 keys. |
| `RESOURCE_SEMANTIC_HASH` | Changes: a new record schema, and the systems and the command declare the read (and the systems the write) of the bucket state key. |
| Kernel formats | Snapshot, commitment, checkpoint, journal, and paged-checkpoint formats are unchanged. A run holds up to one more domain record per bucket. |

Exact replay settles the same boundaries: every boundary decision reads the
live root, the buckets the packets name, the current-version index, and the
archive, never retained evidence. The fixture
`the_split_root_agrees_across_a_seal_a_fork_a_restore_and_an_exact_replay`
settles operations over a backlog spread across buckets, applies an archive
commit that archives part of a demand's closure, seals, and continues: the
sealed run, an unsealed fork, a run restored from the sealed segments (at the
seal), and an exact replay agree on checkpoint hash and snapshot, the
archived records left their buckets, and a later cancellation of the closed
demand reads it from its bucket.

## Compatibility

0.16.0 is a pre-1.0 minor release.

- **Added:** `ResourceLiveRootV1`, `ResourceHotTerminalLedgerV1`,
  `ResourceHotTerminalEntryV1`, `ResourceHotTerminalBucketV1`,
  `ResourceHotTerminalBucketRecord`,
  `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION`,
  `resource_hot_terminal_bucket_reference`, `resource_hot_terminal_bucket_count`,
  `resource_hot_terminal_bucket`, `resource_hot_terminal_state_key`,
  `ResourceRootReader`, `resource_root_in_view`, `whole_resource_root_in_view`,
  `ResourceState::{into_records, from_records, hot_terminal_records,
  hot_operation_outcomes, hot_terminal_records_attached,
  refresh_hot_terminal_ledger}`, the field `ResourceState::hot_terminal`, and
  `ResourceLimitsV1::MAX_OPERATION_OUTCOMES`.
- **Changed (breaking):** `ResourceRuntimeRecord::Payload` is
  `ResourceLiveRootV1`; `ResourceState` gains a field (struct literals break);
  `ResourceState::into_record` refuses a root that holds hot terminal records;
  `prepare_resource_archive` refuses a partial root; `archive_source_root` has
  a new definition; the force-supply restore helpers take an archive
  provider; `MAX_ARCHIVE_CANDIDATES` is 16,384,
  `MAX_RESOURCE_ARCHIVE_COMMIT_PAGES` 64 (`MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS`
  16,384), and `max_operation_outcomes` may be 16,384. A fixture that places
  candidates in the maps directly calls `refresh_hot_terminal_ledger`.
- **Behavior:** operations, outcomes, refusals, skips, and events are
  unchanged, except that an expiry tick whose key another completion
  operation holds is skipped with `operation_key_taken` instead of failing
  the boundary; the byte cap counts the entries' encoding; `resource_state`
  reads every bucket. `validate_force_supply_runtime` no longer compares
  archived resource outcomes; the force-supply restore helpers and
  `validate_force_supply_runtime_with_archive_store` do.
- **Downstream:** another plugin that reads the resource root declares the
  bucket state key and reads through the new readers (see above).

## What can still fail a boundary

The list of [resource refusals without stalls](resource-refusals-without-stalls.md#what-can-still-fail-a-boundary)
holds. This change adds only records that a host application or a holder
cannot produce by sending or racing packets:

1. **A hot terminal bucket that is malformed, not canonical, misplaced, or
   short of what the ledger counts**, read when a packet names one of its
   records: a tampered restore that activation would have refused, or an
   engine defect.
2. **An applied archive commit that selects a record not attached as a hot
   candidate**, which the plugin attaches itself: an engine defect.
3. **In a debug build only, a boundary that would settle differently on the
   whole root**: the equivalence assertion, an engine defect.
4. **A boundary that would write over a hot terminal record it did not
   attach** (`invalid_boundary`): the release-build backstop for a missing
   attachment rule, an engine defect.

Command admission refuses a command when a bucket it reads is invalid; that
is a rejection of the command, not a failed boundary.

## Durable tests

`resource_root_write_set.rs`, each checked to fail with its fix reverted in
the working tree:

- `a_boundary_writes_the_live_root_and_the_buckets_it_changes_whatever_the_backlog`:
  with no backlog and with 600 hot records, one operation boundary writes the
  live root and the new outcome's bucket only, its journal bytes differ by
  less than 4 KB, and a boundary with nothing for the resource plugin writes
  nothing of the root. It fails when every bucket is rewritten (and on the
  base, where the root is one record).
- `the_split_root_agrees_across_a_seal_a_fork_a_restore_and_an_exact_replay`
  (above). It fails when archived entries stay in their buckets, and when a
  packet attaches nothing by name.
- `a_tampered_hot_terminal_bucket_is_refused_when_the_root_is_installed`: an
  entry in another bucket, a ledger that miscounts, and an entry with an
  extra field are refused. It fails without the canonical-encoding check.

The seal fixture also seals a fork before the first boundary and after every
later one; it fails when the live root declares an initial-scenario record
version.

`resource_expiry_tick_key.rs`,
`an_expiry_tick_whose_key_a_host_expiry_took_settles_nothing_and_expires_at_the_next_boundary`:
a host queues `Expire` for the next boundary, with the tick's candidate limit
and with another, and a grant then falls due there. The tick at that boundary
is a resend or a skip naming the key, the host's outcome stays, the root stays
readable, and the grant expires at the next boundary. Without the tick's key
attachment the boundary fails on the backstop (debug and release), and with
the backstop removed as well the release build overwrites the host's outcome;
without the holder resolution the boundary fails with an idempotency
conflict.

`canwu-economy-reference` `grain_harness.rs`,
`a_force_runtime_citing_archived_resource_outcomes_restores_with_its_archive_only`:
after four months a force-cited resource outcome is archived;
`from_force_supply_snapshot_json` and `from_force_supply_checkpoint_journal`
restore with the archive and refuse without one. It fails when a restore
helper validates hot citations only. The force-supply unit test
`an_archived_citation_is_compared_with_the_archived_outcome` compares an
archived citation, a forged one (refused), and one of a never-settled key
(refused); it fails without the comparison. A snapshot that carries a forged
citation does not reach either check: its boundary hash chain refuses it
when it loads.

Beside them, the debug-build equivalence check runs in every existing test of
the resource plugin and of its dependents. Tests that installed a root holding
hot terminal records now install `into_records`; the restore tamper test
`forged_restore_candidate_is_rejected` now tampers with a bucket entry.

## Measurements

`resource-root-profile` (`benchmarks/performance-harness`, described in
[`benchmarks/README.md`](../../benchmarks/README.md)): a root holding the
given number of hot terminal records (rejected outcomes of about 550 bytes
each); one boundary settling one allocation pass that is refused (one new
outcome); eight such passes in one boundary; a boundary admitting only another
plugin's packet; and, after the samples, the boundary applying one archive
commit of every hot record. Release builds (Rust 1.99.0, Windows x86-64, AMD
Ryzen 7 7800X3D shared with other builds), medians of five samples after one
warm-up; allocations from separate builds and exact. Four builds: the
integration base `f511463`, this change on it, the base with the record store
package `305193a`, and this change on that. The two builds before the change
ran the profile with two lines changed (one-record install, no ledger
refresh).

One operation boundary:

| Hot records | Base | Base + split | Record store | Record store + split |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 1.53 ms, 38 KB, 18,821 alloc. | 0.49 ms, 8.0 KB, 4,052 | 0.51 ms, 38 KB, 5,566 | 0.20 ms, 8.0 KB, 1,048 |
| 256 | 14.8 ms, 316 KB, 170,479 | 0.62 ms, 8.0 KB, 6,908 | 3.73 ms, 316 KB, 49,664 | 0.19 ms, 8.0 KB, 1,059 |
| 1,024 | 56.7 ms, 1.15 MB, 625,545 | 0.75 ms, 8.1 KB, 11,020 | 15.2 ms, 1.15 MB, 182,007 | 0.21 ms, 8.1 KB, 1,066 |
| 4,000 | 397 ms, 4.43 MB, 2,388,771 | 1.23 ms, 8.1 KB, 20,830 | 64.6 ms, 4.43 MB, 694,822 | 0.24 ms, 8.1 KB, 1,070 |
| 16,000 | | | | 0.42 ms, 10.7 KB, 1,222 |

(time, journal bytes of the boundary, allocator calls)

A boundary with nothing for the resource plugin:

| Hot records | Base | Base + split | Record store | Record store + split |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0.17 ms, 2,212 alloc. | 0.045 ms, 557 | 0.17 ms, 2,163 | 0.037 ms, 294 |
| 4,000 | 29.1 ms, 198,154 | 0.56 ms, 15,968 | 15.3 ms, 198,105 | 0.037 ms, 294 |
| 16,000 | | | | 0.057 ms, 294 |

Eight operations in one boundary, at 4,000 hot records: 523 ms, 4.44 MB, and
3,248,794 allocations on the base; 139 ms, 4.44 MB, and 1,552,621 with the
record store; 0.68 ms, 27.6 KB, and 5,928 with both (18.3 KB and 5,321 with no
backlog: an entry that shares the new outcome's bucket is rewritten with it).

The archive commit of every hot record, at 4,000 hot records (4,054 records
committed): 102 ms, 2.24 MB, 865,434 allocations on the base; 19.6 ms,
2.24 MB, 207,924 with the record store; 103 ms, 5.21 MB, 849,342 with both;
at 16,054 records, 524 ms, 20.8 MB, 3,394,008. The commit now writes one
bucket per archived record, and the kernel journals each bucket's previous
version, which holds the record: about 25 to 33 µs, 210 allocations, and
1.3 KB of journal per archived record, where the whole-root commit cost about
5 µs and 550 bytes. The backlog is paid once, when it is archived, instead of
in every boundary while it is hot: a record that stays hot through two
boundaries already costs less.

Storage: the root holding 4,000 hot records encodes to 2.47 MB in its live
root and buckets, against 2.20 MB as one record; an entry adds its key,
sequence, and tag to the record (about 12 %), and each bucket is a domain
record of its own.

What the change saves, separated: at 4,000 hot records the record store
package takes one operation boundary from 397 ms to 64.6 ms and its
allocations from 2.39 million to 695,000, and leaves the journal and the
idle boundary's allocations as they were; this change takes it on to 0.24 ms,
1,070 allocations, and 8.1 KB, and the idle boundary from 15.3 ms to 0.037 ms
and 294 allocations, the same as with no backlog. On the base alone this
change takes the operation boundary to 1.23 ms and 20,830 allocations; the
rest of what it still paid there per stored record is the engine's per-record
work that the record store package removed.

## Known limits

- **The live state is still decoded whole.** A boundary that settles resource
  work decodes, validates, and re-encodes the live root once per operation:
  accounts, open demands with their allocations, active transfers, leases,
  report grants, and observation heads. Their own budgets bound them, but a
  root with many accounts or report grants pays for them in every such
  boundary.
- **Finished leases stay live until their last receipt is archived.** A
  completion lease's acquisition, grants, and certificate leave the live root
  only when the archive commit that archives its last receipt applies, so
  many finished, unarchived leases still grow the live root.
- **A bucket can hold several records.** A record's bucket is rewritten with
  every record it shares the bucket with; at a full budget that is about one
  other record.
- **Some readers still read the whole root.** The host application's
  `resource_state`, archive preparation, restore validation, and the
  force-supply reference's settlement of a resource outcome.
- **The archive commit costs more than before** (above), in proportion to
  what it archives.

## Open decisions

- **Refusing a host expiry for a boundary it cannot settle at.**
  `enqueue_resource_completion_operation` checks an `Expire`'s instant, not
  its `current_boundary`. A host expiry for a later boundary is harmless to
  the tick now (the tick at that boundary resends or skips), but it expires
  what falls due by that boundary early. Refusing any `current_boundary`
  other than the next boundary needs the host to know which boundary a
  packet settles in, which it cannot for a packet queued for a later instant.

- **Splitting the live state.** Accounts and leases could move to buckets of
  their own with ledgers for conservation and room; not needed for the hot
  terminal backlog this change addresses.
- **Cheaper archive commits.** A commit attaches and re-encodes each record
  it archives only to count it out of the ledger; the kinds whose ledger
  entry needs no body could be counted from the stored entry's size alone.
