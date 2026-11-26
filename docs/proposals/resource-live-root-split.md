# Resource Live Root Split

Status: built for 0.26.0 (downstream gap G-120), with the engine design
review's changes; the acceptance tests are written and compile, and have not
been run. It continues [resource root write cost](resource-root-write-cost.md)
(0.16.0, hot terminal buckets) and [resource series scale](resource-series-scale.md)
(0.18.0, live buckets), whose "Known limits" both name the rest of the live
state as still decoded whole.

Source references in "Problem" are to the 0.25.0 tree this branch started
from (`crates/extensions/canwu-resource/src`).

## Problem

The live root, domain record `resource:runtime:v1`, holds every part of
`ResourceState` that is not bucketed. Two of those parts grew with the number
of completion authorities, not with the work a boundary does:

- the run budget, `ResourceState::run_budget`, a `RunBudgetRevisionV1` whose
  `partitions` hold one `CompletionCapacityPartitionV1` per authority and
  operation namespace; and
- the lease book's admission epochs, `CompletionLeaseBookV1::epochs`, one
  `AdmissionEpochV1` per partition, made at the authority's first lease.

Every boundary that changes the resource state validates the whole live root
and writes it as one whole-record `Update`; the invariant system decodes and
validates it again, every operation clones the state to take a refusal back,
and each operation that measures its room validates and encodes a copy. In
0.25.0 the budget's digest covered every partition and was computed three
times inside one `validate`.

The run budget was installed with the state before the run
(`install_run_budget`, refused once the lease book holds an acquisition or an
external grant). No operation revised it, nothing removed a partition or an
epoch, and a lease grant had to cite the current budget revision.

The downstream installs 107 partitions with 61 epochs (about 370 B a
partition and 210 B an epoch, most of a 63.5 KB live root). By campaign
month 14 the root's 1,396 whole-record writes (about 133 KB each, the version
before and after) are 186.4 MB of record changes, and `canwu-resource`'s
daily time grew from 3.1 to 7.4 ms with the root's decode, validation, and
encode. An ended authority's partition stays for the campaign, and an
authority that appears later can have none.

## Decision

### Partitions and their epochs are live records

A partition and its admission epoch persist together as one live record in
the live buckets (`resource:live:{n}`, record kind `canwu.resource`
`bucket`), beside accounts, access grants, definitions, units, and custodian
indexes:

- key `ResourceLiveRecordKeyV1::Partition(CompletionPartitionKeyV1)`, where
  `CompletionPartitionKeyV1 { authority: KnowledgeHolderRef,
  operation_namespace: String }` (ordered by authority, then namespace, as
  the epoch map's tuple key is);
- payload `ResourceLivePayloadV1::Partition(ResourceRunBudgetPartitionEntryV1
  { partition: CompletionCapacityPartitionV1, epoch: Option<AdmissionEpochV1> })`.
  `epoch` is `None` until the authority's first lease.
- bucket: `resource_live_bucket` with kind `"partition"` and the text
  `person\0{id}\0{namespace}` or `entity\0{entity}\0{namespace}`, built
  without a fallible encoding.

The live root keeps the budget's header: `revision`,
`total_completion_units`, `shared_pending_slots`, and `semantic_digest`, with
`partitions` empty and `completion_leases.epochs` empty. Its
`ResourceLiveLedgerV1` gains four fields that count and total what is not
attached, as it already does for accounts and their balances:

| Ledger field | Counts | Read by |
| --- | --- | --- |
| `partitions` | unattached partitions | `MAX_RUN_BUDGET_PARTITIONS`, `partition_count`, whole-state checks |
| `admission_epochs` | unattached epochs | whole-state checks (never more than `partitions`) |
| `partition_reserved_pending_slots` | their `reserved_pending_slots` | a lease's configured pending capacity; the 1,024-slot bound |
| `partition_guaranteed_units` | their `guaranteed_units` | the guarantee bound (`<= total_completion_units`) |

`bytes` counts the entries' encoding as for every live entry, so the byte cap
still counts the partitions.

The live bucket count (`resource_live_bucket_count`) adds
`MAX_RUN_BUDGET_PARTITIONS` records: a new public constant equal to
`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL` (1,024), the bound V1 already
implies, since every partition reserves at least one pending slot and the
slots total at most 1,024. For the canonical limits the count doubles, from
16,384 to 32,768 buckets (65,536 records sized for become 66,560, rounded up
to the next power of two after dividing by four). Unused buckets cost nothing,
but every live record of a root lands in a different bucket than in 0.25.0;
that is a new format, not a migration.

**Why one record for both.** A lease's admission reads its partition and
writes its epoch; nothing else reads either. One entry means one bucket read
and one attachment per lease, retirement removes both together, and the
partition and its epoch are always attached together, so admission can never
mint a fresh epoch over a stored one. The partition's bytes are rewritten
with the epoch, but only that authority's (and those of other entries that
share its bucket, byte-identical).

**Byte bounds.** `CompletionCapacityPartitionV1::validate` now bounds the
operation namespace at `MAX_COMPLETION_OPERATION_NAMESPACE_BYTES` (160 B), so
an entry is bounded by its holder reference, a namespace of at most 160 B,
and fifteen integers. By the driver's per-entry sizes (107 × 370 B +
61 × 210 B = 52.4 KB) the downstream's live root would fall from 63.5 KB to
about 11 KB, and its budget part becomes a fixed header whatever the number
of partitions; that is an estimate until the profile measures it.

### In memory: the attached partitions

`ResourceState` keeps its shape. In a partial state, `run_budget.partitions`
holds the attached partitions in ascending key order, and
`completion_leases.epochs` the epochs of the attached partitions. A whole
state (every bucket attached: `resource_state`, `whole_resource_root_in_view`,
activation, restore) holds them all, so host code sees the budget and the
book as before, except that the partitions are in key order:
`install_run_budget` and `RunBudgetRevisionV1::seal` sort them, and
`RunBudgetRevisionV1::validate` requires ascending order (one encoding).

A partial reader no longer sees every partition or epoch in those fields.
`ResourceState` answers for the root:

- `partition_count()`: attached plus unattached partitions;
- `partition(&key)`: `Ok(Some(_))` when attached, `Ok(None)` when the root
  does not hold it (its ledger counts no unattached partition, or the key was
  looked up in its bucket and is not there), and `Err(NotFound)` when it was
  never attached;
- `holds_partition(&key)` and `admission_epoch(&key)`, with the same three
  answers.

### Attachment, and what catches a missed one

The plugin attaches, before it admits a command or settles a packet, the
partitions `ResourceOperationRequestV1::partition_keys` names:

- `Completion(Acquire)` (`RequestCompletionLeaseV1`),
- `Completion(SingleStepLease)` (`SingleStepCompletionLeaseV1::lease`),
- `Completion(GrantExternalParticipant)`,
- and every key a budget revision retires or adds.

It decodes every carrier `packet_operation_keys` decodes: a tracked command,
an adapter packet, a completion operation, a budget revision, and the
deferred wrapper of any of them. These are derived keys, like the outcome
key of a completion operation. No other lease step reads a partition: grant,
prepare, activate, abort, release, expire, and consume read only the budget
header and the lease book. The in-use test of a retiring revision reads
only books the live root holds: a local acquisition stays in the lease book
until its report reservation publishes, and an external participant that
completed stays in the live root's terminal grants until its last report
reservation releases, since only then does it become an archive candidate.

A lookup of a partition that was neither attached nor found absent answers
"no partition", so the operation would be refused as for an authority without
one. The root also notes the key in shared probes (`PartitionProbes`, an
`Arc` that survives the clone a refused operation rolls back to), and the
boundary fails with `invalid_boundary` before it writes, in every build:
command admission checks the probes after admission, the lifecycle system
after it settles its packets, and `ResourceRootWork::persist` before it
writes. The 0.16.0 rules apply as well: every debug build settles each
boundary on the whole root and requires the same directives, and a boundary
that would write over a live record it did not attach is refused.

### What each operation kind writes

| Operation | Records written |
| --- | --- |
| Any operation that changes the state | the live root (now without partitions and epochs), its outcome's hot bucket |
| `Acquire`, `SingleStepLease` | the above, plus its own partition's live bucket (the epoch) |
| External participant grant | the above, no live bucket (it reads the partition, writes no epoch) |
| Other lease steps | the live root and the hot buckets they touch, no partition |
| Budget revision | the live root, its outcome's hot bucket, the bucket of every partition it retires or adds |
| A boundary with nothing for the plugin | nothing (unchanged) |

A rejected lease request takes back its epoch change with the rest of the
operation, so its bucket is not written. A revision marks no report grant
dirty: it changes nothing a holder report shows.

### The rules that read every partition

| Rule | 0.25.0 | 0.26.0 |
| --- | --- | --- |
| A lease's partition | scan of `partitions` | the attached entry, found by key (binary search) |
| Configured pending capacity | `shared_pending_slots` + sum over `partitions` | `shared_pending_slots` + attached sum + `partition_reserved_pending_slots` |
| Reserved slots <= 1,024, guarantees <= total, partitions <= 1,024 | sum over `partitions` | attached sums in `RunBudgetRevisionV1::validate`; attached plus ledger sums in `ResourceState::validate` |
| Duplicate partitions | a set over `partitions` | strictly ascending keys; the bucket key |
| Epochs | none | every attached epoch has its attached partition, in every state; the ledger counts no more epochs than partitions |
| Budget digest | covers every partition | covers the header only |
| Grant cites the budget | `== budget.revision` | `<= budget.revision` |

`CompletionLeaseBookV1::request_acquisition` and
`ExternalCompletionParticipantBookV1::grant` (public) keep their signatures
for a whole budget; crate-level twins take the partition and the configured
pending capacity the state resolves.

### The budget digest

`RunBudgetRevisionV1::semantic_digest` is the digest of the header
(`revision`, `total_completion_units`, `shared_pending_slots`) under the
domain tag `canwu.resource.run-budget.v2`. A digest over every partition
cannot be checked on a partial state. Each partition is still checked on its
own whenever it is attached, stored, or restored, the sums against the bounds
in every validation, and the partitions are covered by the kernel's record
hashes like every other bucket entry. `canwu-production`,
`canwu-economy-reference`, and `canwu-force-supply-reference` keep budgets of
their own (whole, never bucketed); their digests change with the tag.

### Partition lifecycle: the budget revision

A new operation, `ResourceOperationRequestV1::ReviseRunBudget`
(`ResourceOperationKind::ReviseRunBudget`), carries:

```rust
pub struct ResourceReviseRunBudgetRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub expected_budget_revision: ResourceRevision,
    pub total_completion_units: u64,      // the new value; the current one to keep it
    pub shared_pending_slots: u16,        // likewise
    pub retire: BTreeSet<CompletionPartitionKeyV1>,
    pub add: Vec<CompletionCapacityPartitionV1>, // ascending key order
    pub evidence: DomainRecordVersionRef,
}
```

**Who sends it.** Only the host application, through
`enqueue_resource_run_budget_revision(canwu, due_at, request)` on the
internal ingress `resource_run_budget_revision_v1`
(`RESOURCE_RUN_BUDGET_REVISION_INGRESS`), whose permit only that function
hands out, as for account capacity and rescope. A tracked command and an
adapter packet carrying it are refused (`invalid_authority`), and a packet
another plugin's boundary system schedules on the ingress settles nothing.
`enqueue_*` refuses (`invalid_payload`) a request with more than 256 keys or
an `add` out of key order, and evidence that is not its record's current
version now; settlement checks the first two again, and
reads the evidence again and requires one of the plugin's adapter evidence
kinds, and the outcome keeps it as its exact evidence (the record of the
fallen government or the new office, for example).

**What it does.** Retirements first, then additions, all or nothing: each
retired key's entry (partition and epoch) leaves the root; each added
partition enters with no epoch. A key both retired and added is replaced:
new terms and a fresh epoch, whose backward-time guard (the last admission
instant) restarts. The header takes the new totals, `revision` advances by
one, and the digest is recomputed. One `ReviseRunBudget` outcome records it,
with `quantity` the number of keys retired plus added.

**Refusals,** each the operation's recorded rejection, never a failed
boundary, checked in this order:

| Condition | Code |
| --- | --- |
| `expected_budget_revision` is not the current revision | `version_conflict` |
| More than `MAX_RUN_BUDGET_REVISION_CHANGES` (256) keys retired plus added | `limit_exceeded` |
| Nothing listed and the totals unchanged; `add` not in strictly ascending key order | `invalid_definition` |
| An added partition failing `CompletionCapacityPartitionV1::validate` | `limit_exceeded` |
| A retired key the budget does not hold | `not_found` |
| A retired key in use (below) | `invalid_lifecycle` |
| An added key the budget holds and the request does not retire | `idempotency_conflict` |
| After the change: shared plus every partition's reserved slots > 1,024; guaranteed units > `total_completion_units`; partitions > `MAX_RUN_BUDGET_PARTITIONS`; `total_completion_units` 0, or below the units the lease book holds, or below the units the external participant book holds | `limit_exceeded` |
| Evidence not current, or not of an adapter evidence kind | as for account capacity |

**In use** means, in one pass over the books:

- an acquisition of that holder and namespace in the lease book, in any
  state (requested, granted, preparing, active, aborting, or released and
  waiting for its report reservation to publish: a finished lease leaves the
  book only once its reservation has published);
- an external participant grant of it that is `Held`, `Prepared`, or
  `Consumed` (`Released`, `Expired`, `Rejected`, and `Completed` ones do not
  count);
- a completion report reservation whose acquisition or participant (terminal
  ones included) has that holder and namespace.

**The units bound** is per book: each book's `reserved_units` is at most the
total, as each book is bounded on its own; their sum may exceed it.

A revision whose result does not fit the root is skipped with
`RESOURCE_INGRESS_SKIPPED_EVENT`, as every operation is. A resend under the
same key is a duplicate.

Lowering `shared_pending_slots` below the shared-burst acquisitions now
pending is admitted: they stay, and new shared-burst requests are refused
until the count drops.

**A retired authority's later lease** finds no partition and is refused as
an authority that never had one: the recorded rejection `invalid_authority`.
So is an `Acquire` queued before the revision that settles after it, in the
same boundary or a later one.

**What the host must order itself.** A coordinating plugin (production, for
example) admits its own acquisition before it asks for the resource
participant. That acquisition is in the coordinator's state, not the
resource root's, so it does not hold the resource partition: retiring an
authority while another plugin still coordinates its work leaves that
plugin's next participant request refused.

**Grants across a revision.** A grant records the budget revision it was
admitted under (`CompletionCapacityGrantV1::run_budget_revision`); it stays
valid under any later revision (`<=`), and its units still count in its
book's `reserved_units`, which every revision must leave room for.

`install_run_budget` remains the host's way to set the budget of a state
before the run (or of a detached state). It sorts the partitions, refuses a
partial state (a non-zero partition ledger) and a state after lease
admission, and drops the epochs whose partition the new budget lacks.

### Validation scope

Kept: `ResourceState::validate` on the live root and the attached records at
every boundary that writes the root, once per measured operation, and again
in the invariant system; the second validation and the per-operation
whole-state copies stay, as the review asked. These rules cross the live root
(conservation, demand and transfer indexes, report scheduling, lease
closures). With the header-only digest the budget part of `validate` is cheap
(the header, the attached partitions, and the ledger), so the budget is still
validated by each book's `validate` as before. Whole-state checks
(activation, `from_records`, `resource_state`, the debug-build equivalence
check) add: the ledger's four fields equal what the stored entries give,
every entry's key matches its partition, and no epoch lacks its partition.
`decode_live_root` and `ResourceState::into_record` refuse a live root that
holds partitions or epochs inline.

## Readers and dependents

- `ResourceRootReader::attach_live` accepts `Partition` keys, so another
  plugin or a host reads one authority's partition and epoch without the
  rest, through `load`, `load_run`, and `reattach`. `resource_state`,
  `whole_resource_root_in_view`, and archive preparation read every bucket
  and see the whole budget and book.
- `canwu-production` reads `resource_state.run_budget` only to validate the
  external participant book: partial-safe (header and `<=` revision check).
  Its facility-project archive compared the resource participant grant's
  `run_budget_revision` with its own grant's; the two name different budgets
  (the resource root's and production's own), and after a resource revision
  they differ, so that comparison is dropped (review B1).
  `PRODUCTION_SEMANTIC_HASH` changes.
- `canwu-economy-reference` and `canwu-force-supply-reference` keep budgets of
  their own and seal them; their digests change with the tag, so
  `ECONOMY_SEMANTIC_HASH` and `FORCE_SUPPLY_SEMANTIC_HASH` change.
- `canwu-fiscal` and the other readers of the resource root read no partition.

## Persistence

Same version only: nothing written by 0.25.0 is read; no migration.

| Item | Change |
| --- | --- |
| `RESOURCE_STATE_FORMAT_VERSION` | 8 (was 7): `live` gains four fields; the live root holds no partition or epoch; the `ReviseRunBudget` outcome kind |
| `RESOURCE_LIVE_BUCKET_FORMAT_VERSION` | 3 (was 2): the `partition` entry kind |
| Live bucket count | adds `MAX_RUN_BUDGET_PARTITIONS` (1,024); 16,384 → 32,768 for the canonical limits |
| Run budget digest | tag `canwu.resource.run-budget.v2`, header only |
| Ingress | new `resource_run_budget_revision_v1` (internal, host permit) |
| Operation | `ReviseRunBudget` request, kind, and outcome |
| `RESOURCE_SEMANTIC_HASH` | changes (new ingress, operation, entry kind, and grant revision rule) |
| `PRODUCTION_SEMANTIC_HASH`, `ECONOMY_SEMANTIC_HASH`, `FORCE_SUPPLY_SEMANTIC_HASH` | change |
| Kernel formats | unchanged (snapshot 9, commitment 9, checkpoint 6, continuation checkpoint 4) |

A live bucket may now lose an entry (a retired partition); an emptied live
bucket stays as an empty record, as an emptied hot terminal bucket does,
since a deleted domain record leaves a tombstone and its reference could
never be created again. `persist` removes a retired key's entry, for
`Partition` keys only. `from_records` accepts empty buckets.

Continuation checkpoint and exact replay: the split is a function of the
state (`into_records`); every boundary decision reads the live root, the
attached buckets, the current-version index, and the archive, never retained
evidence. A checkpoint carries the live buckets with the rest of the domain
records and restores through `from_records` (whole validation, ledger
equality). The archive is unchanged: a `ReviseRunBudget` outcome is a hot
terminal record archived like any other; partitions and epochs are live and
never archived.

## Alternatives rejected

- **The budget as one record of its own** (`resource:run-budget:v1`), epochs
  in buckets. Meets the write contract more simply, but every lease still
  decodes the whole budget and computes its digest over every partition, so
  a lease costs O(partitions), and a revision rewrites every partition.
- **One domain record per partition.** Retiring one deletes the record, and a
  deleted record's reference can never be created again, so a retired
  authority could never be re-added under the same key. Buckets are reused.
- **Partition and epoch as two entries.** No fewer bytes per lease once the
  two share a bucket, and two attachments and possibly two bucket reads.
- **Retiring by marking** (a tombstone partition in the bucket). Keeps the
  bytes the contract asks to free, and every rule must skip it.
- **Pruning epochs that equal a fresh one** (full tokens, cooldown passed).
  The epoch's monotonic instant and same-instant count make "equal to fresh"
  subtle, and it leaves the partitions in the root.
- **Revision by tracked command or adapter packet.** The budget is the
  host's policy, like account capacity; no in-world actor holds it.

## Acceptance tests (written, not run)

`crates/extensions/canwu-resource/tests/gap_g120_resource_live_root_split.rs`:

- `a_revision_retires_adds_and_changes_totals_all_or_nothing`: install sorts;
  a retire-and-add applied with its evidence and quantity; every refusal of
  the table that a detached root reaches; a re-add with a fresh epoch; a
  retired authority's lease refused `invalid_authority`, the new one served.
- `a_partition_in_use_is_not_retired`: an acquisition in the book; a held,
  then prepared, external participant (a released one does not block); a
  consumed one; a completed one still waiting for its report reservation;
  unchanged totals while both books hold units admitted; the per-book units
  bound (a total at the larger book admitted, one below refused).
- `only_the_host_revises_the_budget_of_a_running_root`: a tracked command is
  refused; stale evidence is refused at enqueue; an `Acquire` queued after
  the revision in the same boundary is refused; the new authority's single
  step activates; no report grant marked dirty; restore and exact replay
  agree.
- `only_the_admitting_boundary_writes_a_live_bucket`: the four steps, a
  release before activation, and a single step; only the admission writes,
  only its partition's bucket; of two partitions in one bucket, the other's
  entry stays byte-identical.
- `a_partial_reader_attaches_partitions_through_load_run_and_reattach`.
- `a_tampered_partition_split_is_refused`: partitions inline, epochs inline,
  a miscounted partition ledger, more epochs than partitions, an epoch
  without its partition, and `into_record` of a root with partitions.

`tests/resource_root_write_set.rs`: the seal, fork, restore, and exact replay
fixture runs with a partitioned budget and admits a lease after the seal.

`canwu-production/tests/g3_contract.rs`:
`gap_g120_project_completed_after_a_resource_budget_revision_archives`.

The `resource-root-profile` benchmark gains `--partitions` and `--epochs`
(a lease admission beside that many partitions), for the orchestrator to run.

## Known limits

- **The rest of the live root is still decoded whole** (open demands,
  transfers, leases in flight, report grants, observation heads), and cloned
  per operation. It no longer grows with the run budget's partitions and
  epochs, but report grants and observation heads are per holder, and
  external participants accumulate (next item).
- **External participant grants that end without completing are never
  pruned** (review O1, out of scope here): released, expired, and rejected
  participants stay in the live root for the run and count against the
  1,024-grant bound, so a long run eventually refuses new external
  participants.
- **A re-added authority starts with a fresh epoch** (full tokens, no
  cooldown, its backward-time guard restarted). The host decides when to
  re-add.
- **The host orders retirement against coordinating plugins** (above).
- **`canwu-production` keeps its own admission epochs** in its own state,
  outside this change.
- **At most 1,024 partitions**, as V1 already implied (in practice 1,024 less
  the shared slots); more authorities share a partition or wait for a
  retirement.
- No measurement yet.
