# Paged Restore Budget and Seal Cost

Status: implemented in 0.18.0. Three engine findings of `opus-progress.md`
in `canwu-sim`, reached through `canwu-api`:

- **E-21** (Medium): a paged-checkpoint restore allocates what the pages
  describe, not what the provider holds, with no bound a host can set; and
  the compact checkpoint body is limited to one 4 MiB envelope page.
- **E-32** (Low): the knowledge ledger sits inside that envelope page, so a
  world with thousands of report holders cannot write a paged checkpoint
  (E4 of [fiscal observers at scale](fiscal-observers-at-scale.md), deferred
  in 0.17.0).
- **E-29, the remaining part** (Low): a world-scale seal costs about 30 s
  flat, hashing every sealed record twice to build the archive index and
  twice more to re-verify it. ([Request outcome
  retention](request-outcome-retention.md) fixed the part the request index
  caused.)

Same version only: the paged-checkpoint format moves from 6 to 7 and no
earlier format is read. The evidence segment, its archive index, the
continuation checkpoint, and every hash are unchanged. The release entry
will be in [Versioning and Persistence](../versioning.md).

## The restore budget (E-21)

### Problem

A paged checkpoint names each record value by its length and the page IDs
of its 1 MiB chunks. A leaf may name one chunk page many times, so a
provider holding one chunk and one leaf page describes a 512 MiB record
(`MAX_PAGED_RECORD_BYTES`), and a leaf page holds about 250 such entries:
the restore allocated what the pages described, bounded only by the page
count (4,194,304) times the ceiling. Hosts were told to authenticate the
root page before restoring untrusted pages ([record store ceiling and
cost](record-store-ceiling-and-cost.md), "Not done").

### Decision

Every paged restore entry point takes a host-supplied
`PagedRestoreBudget { max_assembled_bytes: u64 }`. Before the restore
allocates any value it assembles from pages it charges the value's
canonical length to the budget; a value that would take the restore past
it fails with `ErrorCode::RestoreBudgetExceeded`
(`restore_budget_exceeded`) before any chunk of it is read, and nothing is
restored. The charged values are:

- every value of the four domain-record indexes, inline or chunked;
- every value of the two knowledge maps (E-32);
- the compact checkpoint body and the hot decision state, now chunked.

Patricia keys and the single-page decision manifest, directory, and bucket
pages are not charged: each is bounded by one page the provider holds, and
a page named twice is decoded once (the node cache).

`PagedRestoreBudget::DEFAULT` (also `Default`) is 2 GiB, a ceiling for a
host's own checkpoints: four times the record ceiling and about ten times
the current state of the world-scale fiscal preset. A host whose own
checkpoints are larger passes more.

**The budget bounds assembled bytes, not heap.** A restore holds the
assembled values, the state decoded from them, and copies of that state
while it validates it and builds the runtime (the records rebuilt into the
store, the snapshot's record list, the store the snapshot path builds), and
decoded JSON can take many times its text: an array of small numbers, the
worst case, about 16 times while held and 24 times at its peak. Hostile
pages inside a 2 GiB budget (a 512 MiB record of `0,0,...` built from one
chunk, several times) can therefore peak at some tens of GiB. So for pages
from a source it does not trust, a host passes a budget near the size it
expects (the `assembled_bytes` of the checkpoints it writes, with headroom)
and authenticates the root page ID against one it recorded; `DEFAULT` is not
that bound.

The descriptor states the exact charge of an honest restore:
`PagedSimulationCheckpoint::assembled_bytes`. A host restoring a checkpoint
it wrote passes `PagedRestoreBudget::new(checkpoint.assembled_bytes)`. The
restore refuses (`invalid_snapshot`) a descriptor whose count differs from
what its pages assembled, so a descriptor and its root agree, and refuses a
descriptor that states more than the budget (`restore_budget_exceeded`)
before it reads any page; a descriptor that understates is stopped by the
charge.

**Lifting the page cap.** The envelope no longer embeds the compact
checkpoint; it names the body's chunk pages, `PagedBytes { bytes, chunks }`,
cut at every 1 MiB as record values are. The hot decision state, which was
one JSON page under the same 4 MiB cap, is chunked the same way. No part of
a paged checkpoint is limited to one page.

### Public API

| Item | Change |
| --- | --- |
| `PagedRestoreBudget` | new: `max_assembled_bytes`, `new`, `DEFAULT`, `Default`; re-exported by `canwu-api` |
| `ErrorCode::RestoreBudgetExceeded` | new (`restore_budget_exceeded`) |
| `PagedSimulationCheckpoint::assembled_bytes` | new field |
| `Canwu::from_paged_checkpoint_and_journal(_with_plugins)`, `CompactedCanwu::` the same two | new last parameter `budget` |
| `Simulation::from_paged_checkpoint`, `from_paged_checkpoint_and_journal(_with_plugins)`, `from_portable_paged_checkpoint`, `CompactedSimulation::from_paged_checkpoint_and_journal(_with_plugins)` | new last parameter `budget` |
| `PersistentDomainRecordStore::from_state_pages` | new last parameter `budget` |

A breaking change for every paged restore call: a caller chooses its
budget.

## The knowledge ledger is paged (E-32)

### Decision

The knowledge ledger leaves the body and becomes two content-addressed maps
paged as the record store pages its indexes (binary Patricia leaf and
branch pages, values above 1 MiB in chunk pages), loaded with the same page
checks, and, as the record store compares its rebuilt indexes, rebuilt and
compared at restore (below):

- **bodies**, domain `canwu.paged-checkpoint.knowledge.bodies.v1`: key the
  canonical JSON of the body ID, value the canonical JSON of the body;
- **holders**, domain `canwu.paged-checkpoint.knowledge.holders.v1`: key
  the canonical JSON of the holder, value `{ "holder", "next_local",
  "entries" }` with entries in local order, the snapshot's own encoding of
  one holder ledger.

One leaf per body and one per holder ledger (the 0.17.0 sketch had one page
per 256 holders): an unchanged body or holder keeps its leaf page, and a
holder whose entries grow at the end keeps its leading chunks. The maps are
built from the live ledger when a paged checkpoint is prepared; nothing is
kept between checkpoints, so a boundary pays nothing for them.

**Restore.** Both maps load with the record store's page checks and the
budget, then are decoded straight into the knowledge ledger's own decoder
(`HolderLedgers`'s `Deserialize`), one element at a time with no
whole-ledger text, so the ledger is checked as a snapshot's is: bodies in
ascending ID order and each held, every entry naming a held body, local IDs
in order below `next_local`, every relation resolving within its holder.
Then the restore checks that the maps' keys are exactly the ledger's body
IDs and holders and are canonical, so each value sits under its own key.
Last, it rebuilds both maps from the restored ledger and requires their
roots to be the envelope's. The ledger's decoder accepts encodings no
checkpoint writes (fields in another order, unknown fields: its wire types
do not deny them), and the page loader checks only each branch's leftmost
key, so without the rebuild one ledger could restore from many roots; with
it, one ledger has one paged form. The rebuild costs one map build, as a
preparation does.
The full snapshot validation of `from_checkpoint_and_journal` follows
unchanged: the knowledge commitment recomputed from the restored ledger
must equal the committed root, and the checkpoint hash the descriptor's.

**Tamper.** Besides the record store's page refusals (a page that does not
match its ID, a malformed or truncated page, a non-canonical chunk split, a
missing page as `state_page_unavailable`): a value under another key, a
non-canonical key, a value or tree shape that is not the canonical encoding
of the ledger it decodes to, a page graph naming one node page in two places
(which would otherwise multiply the walk), branch counts that pass the
addressable range (such a graph claims about 2^71 entries in 351 pages; the
loader of the record store's own indexes now refuses it too instead of
overflowing), and any ledger the decoder refuses, all `invalid_snapshot`.

### Formats

| Format | Before | After | Change |
| --- | ---: | ---: | --- |
| `PAGED_CHECKPOINT_FORMAT_VERSION` (descriptor, envelope, decision manifest and directory pages) | 6 | 7 | descriptor `assembled_bytes`; envelope `{ format_version, body: PagedBytes, domain_records, knowledge: { bodies?, holders? }, decision_manifest_page_id }`; manifest `hot_state: PagedBytes` in place of `hot_page_id` |
| State-page codec, page and delta envelopes | `raw-canonical-v2`, 1 | unchanged | the two knowledge domains are new Patricia domains of the same codec |
| Snapshot, commitment, checkpoint/evidence-journal, continuation checkpoint | | unchanged | |

The retention walk (`StatePageRetentionLedger`) reads the new envelope's
body chunks and knowledge roots and the manifest's hot-state chunks, so a
host's collector keeps every page.

## The seal hashes each sealed record once (E-29)

### Problem, measured

`seal_evidence` built the segment's archive index (journal roots: every
record encoded and hashed; item commitments: every record encoded and
hashed again; a Merkle leaf per entry, hex-encoded and decoded back; a hex
Merkle path for every entry), then `verify_archived_segment` cloned the
segment and rebuilt the whole index to compare it, re-verifying every
receipt's path. It then inserted a receipt for every entry into the
persistent receipt map and dropped all but the current dependencies'. The
0.17.0 world profile measured, for 472,036 entries: index 6.3 s, verify
11.8 s, receipts 1.3 s of a 27.6 s seal.

### Decision

- One pass per journal encodes each record once and feeds its bytes to both
  its item commitment and the journal root (a JSON array is its items'
  encodings between brackets and commas, so both hashes are byte-identical
  to before). The encoding streams through a 64 KiB buffer into both
  hashers, as `canonical_hash` streams one value, so no record's whole
  encoding is held. Leaves are hashed to raw bytes, Merkle levels are built
  once, and a Merkle path is materialized only for a receipt the run keeps.
- Entries are built in reference order (commands, attempts, events,
  ingress, boundaries, draws, each in ID order) and only the record-version
  entries are sorted; a segment out of order falls back to the full sort
  and duplicate check. A boundary's generated packets are looked up in a set
  instead of scanning every boundary for every packet.
- The seal makes receipts only for the current dependencies, the receipts
  the retention keeps, and finds them by binary search in the sorted index.
  It no longer rebuilds the index to compare it with itself (the index is a
  function of the segment); it still verifies each kept receipt's Merkle
  path against the new header, the check a later load of the segment
  repeats. Old receipts are pruned to the current dependencies before the
  new ones are inserted, so the seal's duplicate-receipt check now covers
  only kept references; a receipt duplicated across segments is still
  refused when the segments are restored.
  repeats. `verify_archived_segment`, used for every segment a host
  supplies (restore, two-phase commit, payload loads), keeps its full
  rebuild and comparison, without the segment clone.
- The index moves into the returned segment instead of being cloned.

The segment, its index, every receipt, and every hash are byte-identical to
0.17.0's; this was checked by running the old index builder beside the new
one over the `canwu-sim` and `canwu-api` test suites and the profile below
(a temporary check, not committed), and by the independent review against
2aabeaf. The committed test `evidence_index_matches_its_canonical_definition`
checks the builder against the definition (each journal root
`canonical_hash(domain, journal)`, each item commitment and leaf, entries
ascending with record changes listed out of order, kept receipts equal to
the full builder's); it fails if, for one, the comma between items is
dropped.

### Measurements

`continuation-profile --seal-months 3 --commands 280 --churn 15000`: a
compact run sealing every month, about 900,000 index entries a seal (450,060
record changes), about twice the 472,036 of the world profile's monthly seal;
release build, shared machine (other workers building: the spread of the
before column is load). The step rows come from a temporary timing probe
in the seal on the original base d7469c7, not committed; the whole-seal rows
are the profile's own timing before and after the rebase onto 2aabeaf.

| Seal step (ms) | Before | After |
| --- | ---: | ---: |
| archive index | 7,044 - 14,870 | 2,303 |
| verify | 19,690 - 47,070 | 236 |
| receipts insert and retain | 3,243 - 11,349 | 21 |
| apply tables and drain | 576 - 2,327 | 223 |
| whole `seal_evidence` (incl. the rollback fork), on d7469c7 | 33,972 - 86,682 | 4,271 - 4,632 |
| whole `seal_evidence`, on 2aabeaf (with the O(1) fork of E-19/E-20) | 30,790 - 59,154 | 3,290 - 3,398 |
| dropping the returned segment (host side), on 2aabeaf | 1,458 - 2,324 | 658 - 735 |

Of the remaining index time, encoding and hashing every record once is
about 1.09 s, the Merkle leaves 0.8 s, the record-version sort 0.15 s.
The segment copy (0.45 s) is the rest of the seal; the rollback fork
became O(1) with [runtime state copies](runtime-state-copies.md)
(E-19/E-20).

`continuation-profile --report-holders 4000 --report-bytes 2000 --paged`:
4,030 knowledge bodies, a 9.8 MB ledger.

| Paged checkpoint (month 1 / month 2) | Pages | Bytes | Time (ms) |
| --- | ---: | ---: | ---: |
| whole, into an empty store | 16,347 / 16,407 | 12.3 MB / 12.3 MB | 89 / 90 to prepare |
| restore with its evidence prefix, budget `assembled_bytes` (10.2 MB) | | | 244 / 316 |
| delta after one more day (about a thirtieth of the holders report) | 2,328 / 2,357 | 0.997 MB / 1.003 MB | 44 / 43 to prepare |
| delta after a month (every holder reported again) | 16,347 / 16,375 | 12.3 MB / 12.3 MB | 86 / 83 to prepare |

The largest page (0.31 MB) is the compact body's one chunk (the scenario lists
4,001 persons); the one-day delta is that chunk, about 135 new bodies and
holder ledgers, and the Patricia paths above them.

Before this change the same run could not write a paged checkpoint: on
2aabeaf it fails with `invalid_archive`, "state page bytes exceed the
bounded page limit", the envelope page holding the 9.8 MB ledger. The
numbers above are on 2aabeaf; on d7469c7 they were within 10 %.

## What can still fail a boundary

Nothing here runs in a boundary. A paged restore, a paged-checkpoint
preparation, and a seal are host operations whose refusals are errors of
that call; the run is unchanged by a refused restore (nothing is built) and
by a refused seal (the rollback fork), as before.

## Tests

- `canwu-api` `paged_knowledge_ledger`: a ledger larger than one page
  (1,500 holders, 5.2 MB) prepares, restores to the same checkpoint hash and
  ledger with exactly `assembled_bytes` as the budget, is refused one byte
  below it (`restore_budget_exceeded`) before any page with an honest
  descriptor and by the charge with an understated one, refuses an
  overstated descriptor (`invalid_snapshot`), fails
  closed on a missing holder page, stores a three-holder revision as 79 pages
  of 110 KB against 6,002 pages, and continues exactly after restore, a
  revised holder's next local ID included.
- `canwu-api` `paged_checkpoint_large_records::
  the_restore_budget_refuses_a_described_value_before_reading_it`: a leaf
  naming one chunk eight times is refused one byte under its described
  length without its chunk being read, and at the length is assembled from
  eight reads and refused only as a non-record.
- `canwu-sim` `paged_knowledge_values_must_sit_under_their_own_keys`: a
  body or holder ledger filed under another key is refused although the
  ledger it decodes to is well formed.
- `canwu-sim` `paged_knowledge_restores_only_from_its_canonical_encoding`:
  the same ledger with its bodies or holder ledgers re-encoded (fields
  reordered, one unknown field), which the decoder accepts, is refused by
  the root check.
- `canwu-sim` `a_page_graph_naming_one_subtree_twice_is_refused`: a
  hand-built graph naming each level's subtree from two parents is refused
  by the walk at 20 levels and, at 70 levels (about 2^71 claimed paths in
  351 pages), by the checked branch counts instead of an overflow panic.
- `canwu-sim` `paged_bytes_load_only_their_canonical_split`: a chunked byte
  string is refused over its budget, with a chunk too many, with a last
  chunk of the wrong length, with a page that is not a chunk, and with a
  missing chunk (`state_page_unavailable`).
- `canwu-sim` `evidence_index_matches_its_canonical_definition`: the seal's
  index builder equals the documented definition (journal roots, item
  commitments, leaves, ascending entries, kept receipts).
- Existing paged, seal, and continuation tests pass unchanged except for the
  chunk counts of `records_longer_than_a_page_round_trip...`, which now
  include the body's and hot state's chunks.

## Deviations from the filed fix

- One leaf per holder ledger instead of one page per 256 holders: a
  Patricia map gives the same reuse with the record store's checks, and a
  large holder is chunked like a large record.
- The hot decision state is chunked too, though only the body was named:
  it had the same one-page cap.
- The descriptor gained `assembled_bytes`, so a host can restore its own
  checkpoints with an exact budget.
- The budget is a required parameter of every paged restore entry point
  rather than a separate `_with_budget` variant, so every caller chooses a
  budget; `PagedRestoreBudget::DEFAULT` is a ceiling for a host's own
  checkpoints, not a bound for untrusted pages.

## Known limits

- The knowledge maps are rebuilt from the ledger at every paged checkpoint:
  O(ledger) encoding and hashing (the whole paged checkpoint of the 4,030-body
  run above prepares in about 90 ms), though only changed pages are stored;
  a restore builds them once more to check the roots.
- The budget bounds assembled bytes, not the restore's heap (above). The
  restore also still holds a decoded copy of the records beside the loaded
  store while the snapshot path validates them.
- A paged restore still needs the whole evidence prefix from the zero cut
  and verifies it; the continuation checkpoint remains the world-scale save.
- The seal still copies the sealed tail and encodes every sealed record
  once; the index costs about 2.6 µs an entry.
