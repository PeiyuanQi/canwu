# Knowledge Ledger Cost and Authentication

Status: implemented in 0.16.0. Two changes to the generic knowledge ledger in
`canwu-knowledge` and `canwu-sim`, reached through `canwu-api`: what a
boundary pays for the knowledge a run holds, and which hash covers the
knowledge changes a boundary records. The second changes the commitment
format from 4 to 5.

The contracts they change were set in 0.15.0 by
[same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md)
(the relation rule and `SimulationView::knowledge_heads`). The release entry
is in [Versioning and Persistence](../versioning.md).

## What a boundary pays for held knowledge

### Problem

A run holds its generic knowledge records in a ledger keyed by holder and then
by global record ID. Up to 0.15.0 the ledger was a pair of ordered maps inside
`KnowledgeSnapshot`, and the runtime copied the whole snapshot whenever it
needed a stable copy of current state:

- every boundary copied it twice: once into the transaction checkpoint that
  restores a failed boundary, and once into the stable state that systems of
  phases 1 to 9 read;
- every applied command and every scheduled batch copied it into its own
  checkpoint;
- a boundary that published copied the ledger a third time, inserted its
  records, and then hashed the whole knowledge snapshot again to refresh the
  knowledge commitment root;
- `knowledge_records` with a same-boundary overlay copied every holder's
  ledger to add the overlay;
- `knowledge_heads` walked the holder's whole ledger twice per call, and
  `knowledge_record_count_in_namespace` walked every holder's ledger.

Superseded records stay in the ledger, so every publication made every later
boundary slower, including boundaries that publish nothing. The fiscal
profile of 0.15.0 measured an idle boundary at 0.83 ms with 98 KiB of held
report payload and 4.7 ms with 567 KiB, against 0.04 ms without reports.

### Decision

The ledger becomes a persistent structure, the runtime shares it instead of
copying it, and every engine path that read the whole ledger reads a derived
index or an incremental commitment instead.

**Persistent holder ledgers.** `KnowledgeSnapshot::records` is now a
`HolderLedgers`: a persistent map from holder to `HolderLedger`, each a
persistent map from global record ID to the record, built on the `im` crate the
workspace already uses for domain records and decisions. A clone shares all
structure and costs the same whatever the ledgers hold; inserting a record
copies only the paths it changes. Each record is filed under its own `holder`
and `id`, so a key always matches its record and a record ID is unique across
holders. The ledgers also keep an index from global ID to record, which walks
the records in the order the runtime allocates them
(`HolderLedgers::records`, `records_after`), and a count per schema
namespace (`record_count_in_namespace`). Their encoding is byte for byte the
list of `{ "holder", "records" }` entries that 0.15.0 wrote.

**An index of current records.** Each `HolderLedger` keeps, beside its records,
the set of IDs its records supersede (with how many records name each) and its
current records by schema kind and by kind and subject. A record is current
when no record of the holder names it in `supersedes`. The index is a function
of the records alone: every insertion and removal updates it, decoding
rebuilds it, it is never encoded, and two ledgers with the same records are
equal whatever order they were built in. `HolderLedger::current_records(kind,
subjects)` walks it newest first: the current records of `kind` that carry
the listed subject the fewest of them carry, filtered by the other subjects.
It never visits a superseded record.

**Forward iteration only.** `im` 15.1's ordered maps and sets drop elements
when iterated backwards once a tree is three levels deep: from about 2,177
entries a backward walk skips runs of about 33 entries (it returns 4,901 of
5,000). Forward iteration, lookups, forward ranges, `get_min`, and `get_max`
are correct; `get_prev` and `lookup_prev` are not. Each set of current
records is therefore keyed newest first (by `Reverse` of the record ID), so
the newest-first walk is a forward iteration, and no iterator of
`HolderLedgers` or `HolderLedger` implements `DoubleEndedIterator`. The
newest record is still a direct read (`last_key_value`, `last_record_id`).
`PersistentDecisionLog` in `canwu-decision` had the same exposure: its
iterators no longer implement `DoubleEndedIterator` either, and `last()`
reads the newest entry directly. No caller in the workspace walked a decision
log or any other `im` ordered collection backwards; the audit is in the
release report.

**Shared knowledge state.** The runtime keeps its knowledge state behind a
shared pointer. Boundary, command, and scheduled-batch checkpoints, the stable
state of phases 1 to 9, and `fork` share it. A change goes through
copy-on-write: it copies the compatibility actor projection
(`KnowledgeSnapshot::actors`, the army observations of the legacy world) and
shares the holder ledgers, of which it copies only the changed paths. A
boundary that publishes inserts its records into a shared clone of the
committed ledgers; the checkpoint keeps the previous ones, so a boundary that
fails afterwards restores them, their index, and the namespace counts at once.

**Reads.** `knowledge_heads` reads the index for the committed records and the
boundary's own staged records for the rest; the relation rule and its answer
are unchanged. `knowledge_record_count_in_namespace` adds the staged records
to the kept count. `knowledge_records` and the host queries apply a
same-boundary overlay to a shared clone instead of a copy of every holder.

**An incremental knowledge commitment.** Commitment format 4 hashed the whole
knowledge snapshot as one value, so the root of a publishing boundary cost the
whole ledger. Format 5 commits to two sub-roots:

    knowledge = H("canwu.commitment.knowledge.v2",
                  { actors:  H("canwu.commitment.knowledge.actors.v1", actors),
                    records: H("canwu.commitment.knowledge.records.v1",
                               [every record in global ID order]) })

The runtime gives a record an ID above every ID it holds and never changes or
removes a committed record, so the records array only grows at its end. The
commitment cache keeps a running hash of it and feeds it the records published
since it last ran; the actor sub-root is kept until the actor projection
changes. The cache is part of every transaction checkpoint, so a failed
boundary restores it. A restore builds a new cache, which hashes the ledger
once. The accumulator relies on the invariant that committed records never
change: it starts again from empty when the record counts show that the
ledger lost records or gained some below the last one it fed, but it cannot
notice a committed record changed in place, so every path that replaces the
knowledge state restores or drops the cache with it (the transaction
checkpoints, restore, fork, and run creation do). Load validation never
reads the accumulator: it recomputes the root from the snapshot as one
canonical array, and the durable test compares the two after every
boundary, rollback, restore, seal, and replay.

**Superseded records stay.** A superseded record is still in the ledger, in
`knowledge_records` with `FullHistory`, in the holder-local IDs, in every
snapshot, and in the holder-projection root that binds a query cursor. With
the changes above a boundary no longer pays for it, so retiring it would buy
memory and snapshot size at the price of changing what holder views return,
what a cursor binds, and which targets the relation rule can resolve. Those
are public contracts; changing them is left to a design of knowledge
retention and archive.

Alternatives rejected:

- **Retire or page superseded records** now. See above: it changes three
  public contracts and needs an archive design, and the boundary cost is gone
  without it.
- **A runtime-only index beside ordered maps.** The index answers
  `knowledge_heads`, but every capture would still copy the maps.
- **Share the whole snapshot and nothing else.** Idle boundaries would be free,
  but a publication would copy every record, because the checkpoint holds the
  previous snapshot.
- **Keep the format 4 knowledge root.** A publishing boundary would still hash
  every record it holds.
- **Keep the ordered-map field and materialize it lazily for
  `Simulation::knowledge`.** No public type would change, but the runtime would
  keep two representations, and every host read after a publication, including
  the restricted viewer's, would rebuild the whole map.

### Invariants

- The index of current records answers exactly what a scan of the holder's
  records answers, after any sequence of insertions, replacements, and
  removals, and whatever the build order; a decoded ledger has the same index.
- `knowledge_heads` returns what it returned in 0.15.0 for every ledger a load
  accepts: a relation never crosses holders or kinds there, so "superseded by
  a record of the holder" and "superseded by a record of the holder and kind"
  agree.
- A live run, a fork, a run restored from its snapshot, a run that sealed its
  evidence, and an exact replay hold equal ledgers, return the same current
  records, and reach the same checkpoint hash.
- The knowledge root the runtime keeps equals the root a load recomputes from
  the snapshot.
- A boundary that publishes nothing leaves the knowledge state the same
  allocation and does not rehash it.
- A boundary that fails after it committed its records leaves the ledger, its
  index, the namespace counts, and the commitment cache as they were.

### What each operation costs now

With `L` records held, `r` records published by a boundary, and `A` the size
of the compatibility actor projection:

| Operation | 0.15.0 | 0.16.0 |
| --- | --- | --- |
| Boundary, command, or scheduled batch that does not change knowledge | copies `L` (two copies for a boundary) | nothing in `L` |
| Boundary that publishes `r` records | copies `L` three times, hashes `L` | `O(r log L)` inserts, hashes `r` records, copies `A` |
| `knowledge_heads` | two walks of the holder's ledger | the current records of the kind carrying the narrowest listed subject, newest first, until one more than the bound match; plus the boundary's staged records |
| `knowledge_record_count_in_namespace` | walks every ledger | one lookup plus the staged records |
| `knowledge_records` with an overlay | copies every holder's ledger, then walks the holder's | walks the holder's ledger |
| `fork` | copies `L` | nothing in `L` |
| snapshot, restore, checkpoint | `O(L)` | `O(L)`; a decode builds the persistent maps and the index in `O(L log L)`, once |

These still cost the holder's whole ledger on every call, as in 0.15.0:
`knowledge_records` and the host and viewer queries (`admin_query_knowledge`,
`CanwuViewer::query_knowledge`), which derive holder-local IDs and the
holder-projection root that binds a cursor from the full history, and
`CanwuViewer::audit_knowledge_record`, which counts to the holder-local
position. First-party plugins read `knowledge_records` in boundary systems
(`canwu-correspondence`, `canwu-law`, `canwu-production`), so a boundary in
which one of them reads still pays for the holder ledger it reads; that is a
read the plugin chooses, not engine work.

Memory grows with every publication, as before, plus the index: one entry
per current record per kind and per subject it carries, one entry per record
in the global-ID index, and one count per superseded record in its holder's
supersession map. That map grows with history, as the records it counts do;
it is what lets the removal or replacement of a record make the records it
superseded current again, which a host's edit of a snapshot can do and the
runtime never does.

### Persistence, replay, and formats

A 0.16.0 run never reads what a 0.15.0 run wrote: Canwu crates depend on the
exact same version of each other, saves are refused across engine versions,
and no migration is provided. The encoded knowledge ledger is nevertheless
unchanged, byte for byte, in snapshots, scenarios, checkpoints, and journals,
because nothing required a new shape. Decoding refuses a holder entry without
records, which 0.15.0 decoded and a load then refused because boundary
evidence could not rebuild it (and a scenario refused because it held
records). The index and the accumulator are never persisted. A restore
builds the index once, when it decodes the ledgers. Load validation rebuilds
the committed records from boundary evidence into a plain map by record ID,
resolves relations against it, and compares it with the decoded ledgers, so
it builds no second index.

The knowledge commitment root has the new definition above. It is part of
commitment format 5, together with the change in the next section, so every
run's knowledge root, checkpoint hash, and `v1:` boundary state commitments
differ from 0.15.0. Exact replay settles the same boundaries and reaches the
same roots as the live run.

### Measurements

Two profiles in `benchmarks/performance-harness` measure the change, each
built twice from the same source: once in an export of the `v0.15.0` tag with
the binary's source copied in, and once in this change's working tree before
the version bump. Elapsed time and allocation counts come from separate
release builds (Rust 1.99.0, AMD Ryzen 7 7800X3D, Windows x86-64).
Allocation counts are exact and do not depend on machine load. Several other
builds ran on the machine throughout, so elapsed figures show the shape and
the order of magnitude, not a budget; the reports, which keep every sample,
are listed in [`benchmarks/README.md`](../../benchmarks/README.md) under
`2026-10-14` and `2026-10-15`.

**Knowledge-ledger profile.** The new `knowledge-ledger-profile` binary fills
a run the way a reporting plugin does: four holders with 16 lines each, every
fill boundary publishing 32 records on each line that supersede the line's
current records through `knowledge_heads`, with 256 bytes of padding per
payload, so most held records are superseded history. On forks of that run it
measures a boundary in which no system is due, one in which a system
publishes one record superseding its line, one in which a system calls
`knowledge_heads` 64 times and publishes nothing, and a restore from the
snapshot JSON. Medians of 31 samples after 3 warm-ups (3 for the restore),
the two versions run back to back:

| Held records | 0 | 2,048 | 16,384 | 65,536 |
| --- | ---: | ---: | ---: | ---: |
| Idle boundary, 0.15.0 | 0.021 ms | 3.85 ms | 37.5 ms | 182 ms |
| Idle boundary, 0.16.0 | 0.021 ms | 0.023 ms | 0.024 ms | 0.026 ms |
| Publishing boundary, 0.15.0 | 0.035 ms | 6.47 ms | 101 ms | 457 ms |
| Publishing boundary, 0.16.0 | 0.051 ms | 0.100 ms | 0.058 ms | 0.060 ms |
| 64 `knowledge_heads` calls, 0.15.0 | 0.031 ms | 3.73 ms | 154 ms | 832 ms |
| 64 `knowledge_heads` calls, 0.16.0 | 0.032 ms | 0.131 ms | 0.152 ms | 0.190 ms |
| Snapshot restore, 0.15.0 | 0.37 ms | 90 ms | 1.17 s | 5.28 s |
| Snapshot restore, 0.16.0 | 0.37 ms | 97 ms | 1.12 s | 4.91 s |

Allocations of the same operations (medians of 3 samples), and the bytes the
built run holds:

| Held records | 0 | 2,048 | 16,384 | 65,536 |
| --- | ---: | ---: | ---: | ---: |
| Idle boundary, 0.15.0 | 214 | 41,848 | 362,016 | 1,459,736 |
| Idle boundary, 0.16.0 | 215 | 215 | 215 | 215 |
| Publishing boundary, 0.15.0 | 409 | 83,697 | 724,416 | 2,921,139 |
| Publishing boundary, 0.16.0 | 386 | 398 | 402 | 404 |
| 64 `knowledge_heads` calls, 0.15.0 | 540 | 42,430 | 384,486 | 1,553,822 |
| 64 `knowledge_heads` calls, 0.16.0 | 541 | 925 | 925 | 925 |
| Snapshot restore, 0.15.0 | 3,531 | 971,536 | 9,911,528 | 40,562,950 |
| Snapshot restore, 0.16.0 | 3,549 | 836,290 | 8,729,734 | 35,792,922 |
| Run heap bytes, 0.15.0 | 52,130 | 6,317,790 | 57,631,979 | 233,574,803 |
| Run heap bytes, 0.16.0 | 65,762 | 6,226,031 | 55,805,236 | 225,810,444 |

The snapshot JSON is byte for byte the same size in both versions (11,981 to
253,211,400 bytes). A boundary that publishes nothing now allocates the same
215 times whatever the run holds, and a publishing boundary adds a handful of
allocations as the ledger grows, for the paths its insertions copy. The
`knowledge_heads` reads return each line's 32 current records; 0.15.0 walked
the holder's whole ledger twice for each. Elapsed time of the boundaries grows
slightly with the ledger, by cache effects and the logarithmic depth of the
maps. With records held, a restore makes 12 to 14 percent fewer allocations
and takes about as long (8 percent longer at 2,048 records, 4 to 7 percent
less above): decoding builds the persistent maps and the index, and load
validation no longer builds a second ledger. An earlier state of the change,
which built the index again in validation, restored 9 to 51 percent slower.
The run holds about 3 percent fewer bytes, because a record is stored once
and shared by the holder and global-ID maps.

Two costs rise in small ledgers. With nothing held, a publishing boundary
takes 0.051 ms instead of 0.035 ms although it allocates less (386 against
409): the publication copies the shared knowledge state on write and builds
the persistent map nodes, index entries, and records accumulator for its
records, a fixed cost of about 0.016 ms that larger ledgers repay many times
over. At 2,048 records the recorded median, 0.100 ms, is above both
neighbours (0.058 and 0.060 ms); it is noise of that run. A restore with
nothing held costs the same in both versions.

**Fiscal profile.** The fiscal profile of 0.15.0 is unchanged except for a
new `--mode allocations`. Its three cases are four observers of two
institutions each (7 report records, 24,529 bytes per observer), one observer
of all 48 institutions (122 records, 580,780 bytes), and no observer. The
idle boundary is the one on a state a boundary has written. Allocations,
medians of 5 samples:

| Operation | four observers, 0.15.0 | 0.16.0 | one wide observer, 0.15.0 | 0.16.0 | no observer, 0.15.0 | 0.16.0 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Idle boundary | 20,285 | 484 | 116,889 | 484 | 483 | 484 |
| Admitting 40 commands | 2,130,438 | 1,714,677 | 4,159,562 | 1,715,117 | 1,713,716 | 1,713,797 |
| Settling 40 actions | 6,482,380 | 6,442,571 | 6,696,742 | 6,462,804 | 6,433,389 | 6,433,390 |

Elapsed time, the median of three rounds' medians of 31 samples, the
lowest and highest round beside it; the rounds alternated between the two
versions:

| Operation | four observers, 0.15.0 | 0.16.0 | one wide observer, 0.15.0 | 0.16.0 |
| --- | ---: | ---: | ---: | ---: |
| Idle boundary | 0.815 ms (0.806 to 1.40) | 0.037 ms (0.037 to 0.061) | 9.37 ms (5.88 to 18.4) | 0.056 ms (0.037 to 0.066) |
| Admitting 40 commands | 241 ms (192 to 597) | 185 ms (183 to 641) | 347 ms (330 to 507) | 272 ms (195 to 274) |

With reports held, the idle boundary is back at the cost of a run without
fiscal records (0.037 to 0.055 ms in these rounds). Admission no longer pays
for the reports: each admitted command's checkpoint copied the held knowledge
in 0.15.0, which made a wide report cost about 2.4 million extra allocations
for 40 commands. Settlement is dominated by the two writes of the fiscal
state and its elapsed rounds overlap; its allocations fall by 40,000 to
234,000. The 0.16.0 fiscal elapsed rounds were recorded from an earlier state
of the change, which kept its current-record sets in ascending order and
built the index a second time in load validation; neither is on a measured
fiscal path, and the fiscal allocation runs, recorded again from the final
state, give the same counts. The knowledge-ledger reports were recorded from
the final source, which their `source.content_hash` names.

### Compatibility

`KnowledgeSnapshot::records` and `GenericKnowledgeLedger::records` change type
from nested `BTreeMap`s to `HolderLedgers`, and `KnowledgeSnapshot::for_holder`
and `GenericKnowledgeLedger::for_holder` return `Option<&HolderLedger>`. This
is source-incompatible. The read methods keep their names and shapes
(`get`, `contains_key`, `iter`, `keys`, `values`, `len`, `is_empty`,
indexing by `&holder` and `&id`, iteration over `&ledgers` and `&ledger`
yielding the same pairs, and `first_key_value` and `last_key_value` on a
holder's ledger), so most read code compiles unchanged. Unlike a `BTreeMap`'s,
their iterators are forward only: they do not implement
`DoubleEndedIterator`, so `.rev()`, `next_back`, and `rfind` do not compile
on them; read the newest record with `last_key_value` or `last_record_id`,
or collect and reverse. Code that names the map types, uses `BTreeMap::len`
as a function, builds the field from a `BTreeMap`, walks backwards, or edits
a record in place changes:

- build ledgers with `HolderLedgers::new`, `insert(record)`, `extend`, or
  `collect` from records;
- edit a record by cloning it, changing the copy, and inserting it again;
  `insert` replaces the record with the same ID, wherever it is filed, and
  `remove(id)` removes one.

`HolderLedgers`, `HolderLedger`, and their iterators `HolderLedgersIter`,
`HolderRecordsIter`, `RecordsById`, and `CurrentRecords` are new and
re-exported by `canwu-sim` and `canwu-api`. `HolderLedger::current_records`,
`is_current`, and `is_superseded` read the index; `HolderLedgers::record`,
`records`, `records_after`, `record_count`, `last_record_id`, and `ptr_eq`
are new reads. `KnowledgeSnapshot::query_with_overlay` now also refuses an
overlay record that is filed under another holder or reuses an ID held by any
holder (`InvalidLedger`); the runtime never builds such an overlay.
`GenericKnowledgeLedger` now encodes its records as the same entry list a
snapshot uses (its nested-map encoding could not be written as JSON, whose
keys must be strings), and `insert_records` with an empty batch no longer
lists the holder, so `for_holder` returns `None` for it.
`Simulation::knowledge`, `Canwu::knowledge`, and every `SimulationView`
signature are unchanged.

In `canwu-decision`, `PersistentDecisionLog::iter` and `ordinals` return
`impl ExactSizeIterator` instead of `impl DoubleEndedIterator +
ExactSizeIterator`, and `PersistentDecisionLogIter` no longer implements
`DoubleEndedIterator`. Both are source-incompatible for code that walked a
decision log backwards; no code in the workspace did.

### What can still fail a boundary

Nothing new. A publication still fails its boundary for the reasons it did in
0.15.0, each an invalid proposal or an engine invariant failure: an
undeclared or ungranted schema or visibility, a draft that fails its schema,
a holder that is missing, retired, ineligible, or dead, a relation target that
is not visible at the publisher's cut or crosses kinds, more than the batch,
system, or boundary record limits, a duplicate producer correlation, an
exhausted record ID, and a phase other than 4 or 13. `knowledge_heads` still
refuses a subject filter longer than 64, which fails the boundary only if the
calling system propagates the error. Committing records now also refuses a
record whose holder differs from its batch, which settlement never stages.

## Which hash covers a boundary's knowledge changes

### Problem

A boundary record keeps, in `knowledge_changes`, one entry for each knowledge
batch it published: the producing plugin and system, the phase, the holder,
the producer correlation, the records, their visibility, and the summary. The
boundary record hash (`compute_boundary_hash`) covered every other field of
the record, but not this one. Most of its fields are bound elsewhere: the
records rebuild the committed ledger, which the knowledge root covers; the
plugin, system, holder, record count, and summary must match the emitted
`knowledge_published` event, which the event root covers. Two fields were
bound by nothing:

- `visibility` decides when a holder learned of a subject, which
  `CanwuViewer::evaluation_traces` uses to choose the traces a holder may see,
  and it decides which records the relation rule finds visible at load;
- `producer_correlation` is what `knowledge_changes_by_correlation` and its
  prefix form match.

An edit to either that the load checks accept, such as making a same-boundary
publication that no later record relates to a next-boundary one, loaded without
any rehash and changed what a holder's viewer returned.

### Decision

The boundary record hash covers the knowledge changes, every field of each,
when there are any:

    knowledge_changes: Option<&[BoundaryKnowledgeChange]>   // omitted while empty

The field sits after `record_changes` in the hash material, and the material
of a boundary without knowledge changes is what it was. Loading recomputes
each boundary hash and the chain, so an edit to any field of a knowledge
change is refused (`InvalidSnapshot`, "boundary hash chain is inconsistent")
unless the editor rehashes the boundary chain and every commitment after it.

The change is versioned as commitment format 5, together with the knowledge
root of the previous section: `COMMITMENT_FORMAT_VERSION` is 5, every current
snapshot, replay journal, and checkpoint records 5, and a snapshot, journal,
or checkpoint that records 4 is refused. The boundary record hash keeps its
domain, `canwu.boundary-record.v1`, as it did when 0.12.0 and 0.13.0 added
optional fields to it; the commitment format version is what tells the two
contracts apart.

### Other evidence a reader or validator uses

Every field of `BoundaryRecord` is now in its hash. The evidence journals,
events, accepted commands, command attempts, ingress, and random draws, are
committed as whole serialized records by their roots, and the knowledge,
domain-record, decision, plugin-component, world, scheduler, and random-stream
state by theirs. No serialized evidence field is skipped by a hash; the only
skipped fields in the runtime and model crates are derived indexes of
`DecisionState`, which are rebuilt on decode, and a type marker. The compact
checkpoint's archived-segment headers, archived receipts, evidence
dependencies, and keyed reservations are bound by their own roots in the
compact checkpoint hash; this change does not touch them.

### Invariants

- An edit to any field of a boundary's knowledge changes is refused at load
  unless the boundary chain is rehashed.
- The hash material of a boundary without knowledge changes is the format 4
  material.
- Settlement, validation, and every reader see the same knowledge changes they
  saw before; nothing but the hash and the format version changes.

### Persistence, replay, and formats

Persisted shapes are unchanged: the boundary record already carried its
knowledge changes. These bytes change:

- `commitment_format_version` is 5 in snapshots, replay journals, checkpoint
  journals, and compact and paged checkpoints;
- `commitment_roots.knowledge` of every run (previous section), and with it
  every `v1:` boundary state commitment and every checkpoint hash;
- `hash` and `previous_hash` of every boundary, because the hash material
  includes the state commitment, and for a boundary with knowledge changes
  also because the material now includes them;
- `commitment_roots.boundary_chain`, which commits to the last boundary hash.

Snapshot format 8, checkpoint/evidence-journal format 4, the paged-checkpoint
envelope format 4, plugin descriptors, semantic hashes, and the hash
algorithms are unchanged. As with any version change, 0.15.0 saves are
refused by the engine-version check; they would also fail the commitment
format check.

### Compatibility

`COMMITMENT_FORMAT_VERSION` changes from 4 to 5. A host that compares it, or
stores roots or boundary hashes produced by 0.15.0, sees new values. No type
or signature changes.

### What can still fail a boundary

Nothing new: settlement computes the hash over evidence it has already
validated, and hashing serializes values that always serialize.
