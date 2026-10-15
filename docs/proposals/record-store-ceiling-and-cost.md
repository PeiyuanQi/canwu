# Record Store Ceiling and Cost

Status: implemented in 0.16.0. Three changes to the generic domain-record
store in `canwu-sim`, reached through `canwu-api`: how large a record a paged
checkpoint can hold, what a boundary pays for the records a run holds, and
what one record write costs. They change the state-page codec to
`raw-canonical-v2`, the paged-checkpoint envelope format to 5, and, within
commitment format 5, the domain-record roots and the boundary hash.

The contracts they change were set by the Format 8 record store in
[legal storage, sharding, and compaction](legal-storage-sharding-compaction.md)
and by the paged checkpoint of 0.13.2. The release entry is in
[Versioning and Persistence](../versioning.md).

## A paged checkpoint holds records of any legal size

### Problem

A paged checkpoint stores the domain-record store as four content-addressed
Patricia trees whose nodes are state pages. Each record was one leaf, and a
leaf page was a JSON object in which the record's canonical encoding was a
JSON array of byte values: about 3.6 bytes of page per byte of record. A
state page holds at most 4 MiB (`MAX_STATE_PAGE_BYTES`), so a record longer
than about 1.1 MB could not be paged: preparing the checkpoint failed. A
plugin that keeps its state in one record reaches that size early (a fiscal
state record of 2.15 MB in the world-scale fiscal benchmark), and the
first-party authoritative-state byte caps go up to 256 MiB
(`canwu-resource`'s `ResourceLimitsV1::MAX_AUTHORITATIVE_STATE_BYTES` and
`canwu-production`'s default `max_serialized_bytes`). The page ID of every
node was also recomputed on every write by serializing the whole leaf,
record included, as that number array.

### Decision

**Binary record pages.** Patricia nodes and the chunks of long records are
binary pages; the envelope and decision pages stay canonical JSON. A binary
page starts with the four bytes `00 63 77 70` (a NUL, which no JSON text can
start with, then `cwp`) and a kind byte, and every integer is little-endian:

```text
leaf   = magic 0x01 domain key_hash[32] entry_count:u64 structural_bytes:u64
         entry*                       (entry_count entries, keys strictly ascending)
entry  = key_len:u64 key value_len:u64 value
value  = value_len bytes              when value_len <= STATE_PAGE_VALUE_CHUNK_BYTES
       | chunk_id[32] * ceil(value_len / STATE_PAGE_VALUE_CHUNK_BYTES)   otherwise
branch = magic 0x02 domain bit:u16 left_page[32] right_page[32]
         entry_count:u64 structural_bytes:u64
chunk  = magic 0x03 payload           (1..=STATE_PAGE_VALUE_CHUNK_BYTES bytes)
domain = domain_len:u16 UTF-8 bytes
```

**Chunks.** A record whose canonical encoding is at most
`STATE_PAGE_VALUE_CHUNK_BYTES` (1 MiB) is held inline in its leaf. A longer
one is cut at every 1 MiB into record-value chunk pages; the leaf names them
by page ID in order, so the leaf's page ID commits to every chunk byte and the
split is the only one a value has. Chunks are content addressed like every
page, so an unchanged chunk is not stored again: a record whose tail changes
emits its leaf, the chunks that changed, and the path above.

**The ceiling.** `MAX_PAGED_RECORD_BYTES` is 512 MiB, twice the largest
first-party state cap, so at most 512 chunks. Preparing a paged checkpoint of
a record longer than that fails with `value_out_of_range`, which a host can
tell apart from a corrupt page (`invalid_snapshot`): the run is valid, only
its paged form cannot hold the record. Writing the record, boundaries, the
flat checkpoint, and snapshots do not check it: no boundary can fail on it.

**Commitment unchanged in what it binds.** A node's hash is still the page
ID of its page, so the four domain-record roots commit to the same content as
before, through the new encoding. The leaf no longer carries a separate value
hash: the page ID covers the inline bytes or the chunk IDs.

**Retention.** The retention ledger's closure walks binary pages: a branch
names its two children, a leaf the chunks of its long values, and a chunk
nothing. A host's collector therefore keeps every chunk a retained root
reaches.

**Hostile pages.** Decoding a binary page reads only bytes the page holds and
refuses trailing bytes; a declared length above the ceiling is refused before
any chunk is loaded. A record value is assembled only from chunks that have
already matched their page IDs and their slice of the canonical split. That
does not bound a restore by the bytes the provider holds: a leaf may name one
chunk page many times, as a value with long runs of one byte legitimately
does, so one 1 MiB chunk page and a 16 KiB leaf describe a 512 MiB value,
and a restore allocates it before the JSON decode or the root check can
refuse it. A restore allocates what the state describes, up to 512 MiB per
record and as many records as the page-count limit admits. A host that
restores pages from an untrusted source must authenticate the checkpoint
first, for example by its root page ID. A restore-wide budget on assembled
bytes is left as a follow-up: any fixed budget would refuse some state the
engine can write, so it needs a host-supplied limit, which is new public API.

Alternatives rejected:

- **A compact JSON encoding of the bytes** (base64 or hex inside the JSON
  leaf). It shrinks the ceiling problem by 2.7 or 1.8 times but does not
  remove it, and the page ID would still need the leaf re-encoded on write.
- **Content-defined chunking.** It would keep chunk boundaries stable when a
  JSON value shifts, so a delta of a large, scattered change would be
  smaller. It needs a rolling hash over every write and a recomputation on
  restore to prove the boundaries canonical; the fixed split already reuses
  the chunks before the first change, and every chunk when nothing changed.
- **Chunks named by a second-level index page.** Not needed: 512 chunk IDs
  fit a leaf (16 KiB).

### Invariants

- One record value has one encoding: inline exactly when its length is at
  most 1 MiB, otherwise one 1 MiB chunk per full MiB and a shorter last chunk.
- A paged checkpoint restores to the checkpoint hash, commitment roots,
  authoritative state hash, and records of the live run and of a flat
  checkpoint restore, whatever the record sizes up to the ceiling.
- A leaf's page ID commits to every byte of each value it holds.
- The retention closure of a root contains every page a restore of it reads.

### Refusals

`PersistentDomainRecordStore::from_state_pages`, and through it
`from_paged_checkpoint_and_journal` and its forms, refuse with:

- `state_page_unavailable` when the provider lacks a node or chunk page (the
  documented provider contract for every missing page);
- `invalid_snapshot` for a page whose bytes do not match its ID, a page under
  the wrong ID, a malformed or truncated page or trailing bytes, an unknown
  page kind, a chunk where a node belongs or a node where a chunk belongs, a
  declared value length above `MAX_PAGED_RECORD_BYTES`, a chunk count that
  is not the value's, a chunk whose length is not its slice of the canonical
  split, a leaf whose entries are out of order or hash to another key, any
  Patricia structure defect, and roots the rebuilt indexes do not reproduce.

Before 0.16.0 the record-page defects were `invalid_archive`. The retention
ledger and `verify_state_delta` keep `invalid_archive` for the page defects
they find, as before.

## An idle boundary does not pay for the records a run holds

### Problem

A boundary that wrote nothing cost 0.023 ms with an empty store, 5 to 6 ms
with 16,384 records, and 35 to 113 ms with 65,536, with 404,364 allocations
and 27 MB allocated, whatever records its systems read. Every boundary
captures a transaction checkpoint of current state for rollback. The capture
shared the record store, but it copied
`current_domain_record_versions`, the ordered map from each record to the
exact version reference and establishment time of its current version: six
string allocations per record.

### Decision

The index is a persistent ordered map (`im::OrdMap`) whose keys and values
are shared (`Arc<DomainRecordRef>`, `Arc<CurrentDomainRecordVersion>`). The
capture, the restore, and `fork` share it; a write copies only the path it
changes, and an update reuses the record's existing key.

### Invariants

- The index answers exactly what it answered before, after any sequence of
  writes, rollbacks, restores, and forks; a restore rebuilds it from the
  retained journal as before.
- A boundary that changes no record allocates nothing per stored record.

## One record write costs what the record costs

### Problem

The world-scale fiscal probe (`record-write-profile`, below) measured the
engine's share of a write, at 10,000 writes per boundary into a 65,536-record
store, as about 194 µs plus 134 µs per KiB of structured JSON payload,
with about 620 allocations plus 590 per KiB, and 138 KiB plus 82 KiB
allocated per KiB of record. At 50,000 writes in one boundary it grew to
1.4 ms per write at 4.5 KiB. The cost followed the number of JSON values,
not the bytes: each write deep-copied its payload many times. A phase-7
write was applied to the store three times (the candidate overlay, the
visible overlay, and the phase-9 commit), each time copying the draft, the
previous record, and the new record; the commit copied each change again to
index it, the boundary record copied every change to journal it, and the
boundary hash serialized the previous and the current record of every
change. Each application also re-encoded the leaf and hashed every branch on
the path, and the store's hash maps copied their keys, record references of
three strings each, along every path they changed.

### Decision

**Records are shared, not copied.** The store holds `Arc<DomainRecord>`, and
`DomainRecordChange::previous` and `current` are now `Arc<DomainRecord>`
(they encode exactly as before). A change, the overlays a boundary builds,
the evidence journal, and the store share one copy of each version.

**One application per write.** Phase 7 applies its same-boundary writes once,
while it builds the visible overlay, and keeps the result. When every record
write of the phase is a same-boundary write, the candidate overlay is that
same result. The phase-9 commit takes the prepared result when the store
still has the roots the preparation started from and the requests are the
same; applying a bundle is a pure function of the two, so the outcome cannot
differ. The preparation moves each draft's payload into its record instead of
copying it, so a commit that found the preparation stale cannot reapply the
drafts; it fails the boundary as an engine invariant failure, which cannot
happen. A debug build keeps copies of the payloads, recomputes every prepared
commit, and asserts that it is equal, so the whole test suite checks the
reuse. Phase 10 prepares phase 11 the same way, without moving payloads.

**Shared store keys.** The record map and the ordered key pages share
`Arc<DomainRecordRef>` keys, so a node copy copies no key text.

**Lazy, single hashing.** A Patricia node's page ID is computed when first
read and kept. A boundary that writes many records therefore hashes each
changed node once, when the roots are read at its end, instead of once per
write along every shared path. A branch caches its subtree's sample key
hash, and the first differing bit of two key hashes is found by byte. A
record is encoded once, through a reused buffer, into one exact-size
allocation.

**The boundary hash commits to record versions by digest.** Commitment format
5 now hashes each record change with its fields in declaration order, except
that `previous` and `current` are the lowercase hex encoding digests
`canonical_byte_hash("canwu.domain-record.encoding.v1", canonical JSON)` of
the versions, which the store keeps beside each value. The digest binds every
byte of the version, so the boundary hash commits to what it committed to
before; settlement takes the digests from the store, and load validation and
replay compute them from the journaled records. `canonical_hash` streams its
JSON into the hash through a stack buffer instead of building it in memory;
its digests are unchanged.

Alternatives rejected:

- **A shared payload type in `DomainRecord`.** Every plugin reads and builds
  `DomainRecord::payload` as a `serde_json::Value`; sharing the record
  instead keeps that contract.
- **Validation-only overlays without the Patricia trees.** The phase-9 commit
  would still apply every write again.
- **Keep the version bodies in the boundary hash and inject their stored
  encodings as raw JSON.** It keeps the hash bytes but needs `serde_json`'s
  `raw_value` feature and its private serializer token.

### Invariants

- A boundary's outcome, evidence, receipts, and commitments are what
  applying each stage's requests to the committed store gives, whether or
  not a prepared stage was reused.
- A change's `previous` is the record the store held and its `current` the
  record it now holds, by value; sharing changes no encoded byte.
- The boundary hash settlement computes equals the one load validation and
  replay recompute from the journal.
- Lazily computed node hashes equal the eagerly computed ones: a node's hash
  depends only on its content.

## Persistence, replay, and formats

A 0.16.0 run never reads what a 0.15.0 run wrote (same version only), so no
migration exists. What changes:

| Persisted or committed item | Change |
| --- | --- |
| `STATE_PAGE_CODEC` | `raw-canonical-v2`: binary Patricia nodes and record-value chunk pages. `StatePageBlob`, `PreparedStateDelta`, and `STATE_PAGE_FORMAT_VERSION` (1) are unchanged. |
| `PAGED_CHECKPOINT_FORMAT_VERSION` | 5. The envelope shape is unchanged; it names `raw-canonical-v2` pages. |
| Domain-record commitment roots | Change for every non-empty store: a node's hash is the page ID of its binary page. The empty-tree roots are unchanged. |
| Boundary hash | Record changes enter it by encoding digest (above). Every boundary with a record change hashes differently. |
| Checkpoint hash, boundary chain | Change with the two items above. |
| `COMMITMENT_FORMAT_VERSION` | Stays 5: format 5 is new in 0.16.0 (the knowledge package) and covers these changes too. |
| Snapshots, checkpoints, journals, `DomainRecordChange` | Unchanged bytes. |

Exact replay settles the same boundaries and reaches the same roots and
hashes as the live run; a sealed run, an unsealed fork, a restore from sealed
segments, and a paged restore reach the same checkpoint hash.

## Public API

- Added: `MAX_PAGED_RECORD_BYTES`, `STATE_PAGE_VALUE_CHUNK_BYTES`.
- Changed (breaking): `DomainRecordChange::previous` is
  `Option<Arc<DomainRecord>>` and `current` is `Arc<DomainRecord>`. Field
  reads work through `Deref`; code that builds a change wraps the records,
  and code that edits one uses `Arc::make_mut`. The serde form is unchanged.
- Changed: `STATE_PAGE_CODEC` is `raw-canonical-v2`,
  `PAGED_CHECKPOINT_FORMAT_VERSION` is 5, and record-page defects found by
  `PersistentDomainRecordStore::from_state_pages`, `state_pages`, and
  `missing_state_pages` are `invalid_snapshot` instead of `invalid_archive`;
  a live record above `MAX_PAGED_RECORD_BYTES` makes `state_pages`,
  `missing_state_pages`, and `prepare_paged_checkpoint` fail with
  `value_out_of_range`.

## What can still fail a boundary

Nothing new. A prepared record stage that no longer matches its commit fails
the boundary with `invalid_boundary`; the stage is prepared from the same
directives against the same store in the same boundary, so this is an engine
invariant check that cannot be reached. A record of any size, including one
above `MAX_PAGED_RECORD_BYTES`, is written and committed as before; only
preparing a paged checkpoint of it fails, outside settlement.

## Measurements

Three profiles in `benchmarks/performance-harness` measure the change, each
built twice from the same source: once in an export of the integration base
(`12d4775`) with the binaries' sources copied in, and once in this change's
working tree before the version bump. Elapsed time and allocations come from
separate release builds (Rust 1.99.0, AMD Ryzen 7 7800X3D, Windows x86-64).
Allocation counts are exact and do not depend on machine load. Other builds
ran on the machine throughout, so elapsed figures vary by about a quarter
between runs of the same build (4.6 KiB records at 10,000 writes measured
31.6 µs per write in one run and 39.9 µs in a later one); the reports keep
every sample and are listed in
[`benchmarks/README.md`](../../benchmarks/README.md) under "Record-store
profiles".

**Record writes** (`record-write-profile`, from the world-scale fiscal work):
a store of 65,536 records of one kind; one boundary updates N of them with
structured ledger entries of about 350 bytes each. The engine's share per
write is the slope over N = 1,000 and 10,000 (medians of 5 samples after one
warm-up); allocations are the slope in the allocation build:

| Record | Base | Record store | Ratio | Allocations, base → record store | Allocated per write, base → record store |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1.36 KiB | 337 µs | 29.3 µs | 11× | 1,418 → 91 | 228 KB → 16.6 KB |
| 2.45 KiB | 526 µs | 24.3 µs | 22× | 2,048 → 91 | 342 KB → 17.7 KB |
| 4.64 KiB | 716 µs | 31.9 µs | 22× | 3,328 → 91 | 589 KB → 19.9 KB |
| 8.64 KiB | 926 µs | 39.9 µs | 23× | 5,632 → 91 | 842 KB → 23.9 KB |

Fitted over the record size, a write cost 300 µs plus 78 µs per KiB, 631
allocations plus 593 per KiB, and 141 KB plus 86 KB per KiB; it now costs
24 µs plus 1.9 µs per KiB, 91 allocations whatever the size, and 15.2 KB
plus 1.0 KB per KiB, the record's own encoding. The smallest record gains
least because the remaining per-write floor (about 20 µs: the encoding, the
leaf and path hashes at the end of the boundary, the version index, the event
and emission, and validation) dominates it. A write of the same record from a
small previous version costs the same as from a full one: the previous record
is no longer copied. At 50,000 writes in one boundary a write costs
48 µs at 4.6 KiB and 66 µs at 8.6 KiB in a separate run, against 1.4 ms and
2.3 ms that the world-scale fiscal probe measured on the base.

**The idle floor.** An idle boundary on the same 65,536-record store cost the
base 51 to 67 ms, 404,364 allocations, and 27.7 MB; it now costs 0.08 to
0.10 ms, 184 allocations, and 18.0 KB, exactly the allocations of a boundary
on an empty store (0.024 ms). The remaining difference from the empty store
is memory traffic, not work per record: with the same allocations, an idle
boundary takes about 45 µs with 1,024 records and 70 µs with 65,536. The
boundary after each write, which admits its events, followed the idle floor
(37 to 74 ms) and now costs 0.02 to 0.04 µs per write.

**Record sizes and paged checkpoints** (`paged-record-profile`): forks of a
run holding 65,536 small records; one boundary replaces one record with a
text payload of the given size, after one unmeasured boundary that writes
another record. Medians of 31 samples after 3 warm-ups; paged cases 5
samples:

| Case | Base | Record store | Allocations, base → record store |
| --- | ---: | ---: | ---: |
| Idle boundary | 54.2 ms | 0.039 ms | 404,355 → 175 |
| Write a 1 KiB record | 37.6 ms | 0.114 ms | 405,220 → 326 |
| Write an 8 KiB record | 43.8 ms | 0.125 ms | 405,226 → 326 |
| Write a 64 KiB record | 40.1 ms | 0.173 ms | 405,235 → 326 |
| Write a 1 MiB record | 68.4 ms | 1.41 ms | 405,247 → 329 |
| Prepare a paged checkpoint of one 0.9 MB record | 7.13 ms (4 pages, 2.71 MB) | 0.61 ms (4 pages, 0.91 MB) | 238 → 219 |
| Restore it | 99.3 ms | 13.2 ms | 3,066 → 2,329 |
| Prepare a paged checkpoint of a 1.2 MB and a 32 MB record | refused | 33.4 ms (39 pages, 33.2 MB) | — → 677 |
| Restore it | — | 472 ms | — → 3,060 |

The base's elapsed write times are below its idle time because the idle
floor of tens of milliseconds, which every boundary paid, varied more than
the write itself. The base's largest pageable payload was 1,151,023 bytes
(the probe's bisection); the record store pages the 1.2 MB record and the
32 MB one. A 32 MB restore spends most of its time decoding the record's JSON
and re-encoding it to rebuild and check the indexes.

**Architecture suite** (allocation build, scales 8 to 512): no case allocates
more; every case that hashes canonical JSON allocates less, because
`canonical_hash` no longer builds its input in memory. At scale 512, for
example, `checkpoint_create` allocates 2.00 MB instead of 3.05 MB,
`exact_replay` 1.13 GB instead of 1.50 GB, and `live_archive_seal` 56.9 MB
instead of 84.3 MB.

## Not done

- A paged restore allocates what the pages describe, up to 512 MiB per
  record, even when the provider holds far fewer distinct bytes, because a
  leaf may name one chunk page many times (see "Hostile pages"). Hosts that
  restore pages from an untrusted source authenticate the checkpoint first; a
  restore-wide budget needs a host-supplied limit and is a follow-up.
- Other per-boundary copies that grow with state remain: the boundary takes a
  stable copy of current state for phases 1 to 9, which copies the plugin
  components, the core entity set, persons, letters, and armies (ordered maps
  that are not persistent). A run that keeps much state in plugin components
  pays for them on every boundary. The record store and the version index no
  longer contribute.
- `fork` still copies every journal and the non-persistent maps (about 140 ms
  for a run that created 65,536 records).
- The reverse-reference, successor, and predecessor lookup maps still use
  owned record-reference keys; a write of a record with references copies
  their key text along the paths it changes.
- The paged-checkpoint envelope page still holds the compact checkpoint body,
  including the knowledge ledger and plugin components, in one 4 MiB page;
  a run whose body exceeds it cannot prepare a paged checkpoint. Chunking the
  body with the same chunk pages is the natural next step.
