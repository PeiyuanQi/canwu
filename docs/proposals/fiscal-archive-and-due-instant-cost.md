# Fiscal Archive and Due-Instant Cost

Status: implemented in 0.18.0. It closes two engine-side findings of the
0.16.0 and 0.17.0 runs: the host archive operations of `canwu-fiscal` and
their host-supplied sealed cut (E-26), and the cost of the world due instant
(E-30). It changes `canwu-fiscal`, `canwu-ming-fiscal-reference`, the fiscal
profiles under `benchmarks/`, and adds one read to `canwu-sim` and
`canwu-api`. Same version only: the `fiscal_report` knowledge schema and the
plugin identity change, and nothing written by 0.17.0 is read.

## Contents

1. [Archive helpers read only what a commit needs](#archive-helpers-read-only-what-a-commit-needs)
2. [The sealed cut comes from the run](#the-sealed-cut-comes-from-the-run)
3. [External-operation IDs, resolved proposals, and the differential test](#external-operation-ids-resolved-proposals-and-the-differential-test)
4. [The due instant](#the-due-instant)
5. [Contract changes](#contract-changes)
6. [What can still fail a boundary](#what-can-still-fail-a-boundary)
7. [Costs before and after](#costs-before-and-after)
8. [Tests](#tests)
9. [Deviations and limits left](#deviations-and-limits-left)

## Archive helpers read only what a commit needs

**Problem.** In 0.17.0 a host prepared an archive commit with
`FiscalState::prepare_fiscal_archive` on a state it assembled with
`load_fiscal_state`, and `enqueue_fiscal_archive` and
`finalize_fiscal_archive_retention` each assembled the whole state again
(`FiscalState::from_records` over every fiscal record). At world scale
(50,000 ledgers, 67,991 fiscal records, 58 MB of JSON) the 0.17.0 gate run
spent, per commit of 4,096 books, 743 ms assembling, 88 ms preparing, 619 ms
verifying and queueing, and 363 ms finalizing: about 1.8 s of host time, of
which preparing proper was 5 %. A month of the gate run makes 13 such commits.

**Decision.** Three run-level helpers that read records, not the state:

- `prepare_fiscal_archive(run, book_limit, store)` reads the root and the
  archive head, walks every ledger record only far enough to see whether it
  holds a closed book (a book whose `status` is not the text `open`), decodes
  the ledgers that hold the first `book_limit` closed books in (scope, cycle)
  order and their offices, and prepares from that partial state with the
  0.17.0 selection, pages, index, and directory code, unchanged. Since the
  selection takes books in that order and never more than `book_limit`, the
  partial state selects exactly what the whole state would.
- `enqueue_fiscal_archive(run, prepared, store)` re-derives the batch with
  `prepare_fiscal_archive` and requires equality, as before, then stores,
  verifies, and queues.
- `finalize_fiscal_archive_retention(run, store, receipt)` decodes the
  archive head alone.

The walk is proportional to the number of ledgers with a cheap check each;
decoding is proportional to the books a commit takes. `CanwuRun` does not
promise an order for `domain_records()`, so the walk collects candidates in
an ordered map rather than stopping early. An index of ledgers with closed
books would make preparation proportional to the selection alone; it is left
out, because the walk takes about 70 ms at world scale, which is most of
preparing a commit of 256 books (see [costs](#costs-before-and-after)).

## The sealed cut comes from the run

**Problem.** Archiving a book ends its ledger's payload-required declaration
of the versions it cites, so on a sealing run a book whose citations are
newer than the last sealed cut must be refused until a seal covers them. In
0.17.0 the cut was a parameter (`sealed_cut: Option<&EvidenceCursor>`); a
sealing host that passed `None` skipped the refusal.

**Decision.** The run supplies the cut. One additive engine read:

```rust
// canwu-sim
impl Simulation { pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> }
impl CompactedSimulation { pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> }
// canwu-api
impl Canwu { pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> }
impl CompactedCanwu { pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> }
pub trait CanwuRun { fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor>; }
```

It returns the cut through the evidence the run no longer retains: the end
of its last sealed segment, or the settled cut of the continuation checkpoint
it was restored from (the runtime's archived cursor, which every seal
advances). It is `None` while the run retains all its evidence: it has never
sealed, or it was restored with its whole journal, whose first seal starts
again at the zero cut. A `Canwu` always answers `None`. The fiscal helpers
pass it to the unchanged refusal: a selected book citing a version that a
boundary after the cut established is refused with
`FiscalArchiveError::UnsealedCitations`. The read is host-facing and changes
no boundary decision, so a sealed run and its unsealed fork still settle
alike; only the host's preparation is refused on the sealing run.

What this guarantees, and what it does not. The cut is the run's own, so a
host cannot pass a wrong one, the commit's ingress still needs the plugin's
permit, and enqueueing re-derives the batch on the same run. But a run that
retains all its evidence refuses nothing: every `Canwu` (which can still
write a continuation checkpoint), a `CompactedCanwu` that has never sealed,
and a run restored from a checkpoint and its journal until its next seal.
Such a run's archived citations reach a sealed segment only at its next
seal, so "every archived citation is in a sealed segment" holds only if the
host seals before it writes a continuation checkpoint. The rule is **seal at
each save**. A host that saves only by continuation checkpoint and never
seals archives with no refusal in its first session, loses those versions
from the saved run, and, after the restore (whose cut is the checkpoint's
settled cut), sees the same kind of citation refused until it seals.

A never-sealed compact run answering `Some(zero cut)` instead of `None` was
considered and not taken. It agrees with the engine's own reading (a version
is archived only when its boundary is at or below the archived cursor's
boundary count, `validation.rs`, so nothing is archived at the zero cut),
but it would refuse every book citing a boundary-established version on a
compact run that never seals, which would then never archive such a book.

Engine hunks (all additive, no state or format change):

1. `crates/runtime/canwu-sim/src/runtime/persistence.rs`, `impl Simulation`:
   `sealed_evidence_cursor`, beside `evidence_cursor`, reading
   `state.evidence.archived` (6 lines).
2. The same file, `impl CompactedSimulation`: the delegating method (4 lines).
3. `crates/api/canwu-api/src/lib.rs`, `impl Canwu`: the delegating method.
4. The same file, `impl CompactedCanwu`: the delegating method.
5. The same file, `trait CanwuRun`: the required method, and its line in
   `impl_canwu_run!`.
6. `crates/api/canwu-api/tests/continuation_checkpoint.rs`: the lockstep
   test pins the answers (below).

## External-operation IDs, resolved proposals, and the differential test

**External-operation IDs: kept, with the reason stated.** A receipt's
`(evidence kind, external_operation_id)` pairs are checked against every
receipt of the scope's hot books (open and closed, not only open). They are
not checked across scopes or after a book is archived, and that stays. An
evidence record's claim binds it to one scope and one request
(`claim.scope_binding_id` and `claim.request_id` must match the receipt), and
one exact evidence version settles at most one receipt, so an external
operation can settle in a second scope only through a second evidence record
that claims that scope: a fault of the evidence domain, which owns its
operation keys, as the resource extension's domains own theirs. A world-wide
check would need a hot index of operation keys written by every receipt and
pruned by every archive commit, about a write per receipt at 16,384 receipts
a boundary, and still could not see archived books without reading the
archive in a boundary, which the design rejects.

**Resolved auditor proposals: the cap is enough.** An office keeps the
latest `MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY` (64) resolved proposals of
its policy and drops older ones; they are not archived. The cap bounds the
office record for the life of a run, no boundary reads a resolved proposal
after it is dropped, and who proposed and who decided stays in the
`canwu.fiscal.action_settled.v1` events of the actions that made and resolved
it, which a sealing host keeps in its sealed segments. Archiving them would
add a key kind, a selection over every office at every commit, and a host
read for records no rule reads. As with action IDs past their ring, the ID of
a dropped proposal can be used again; hosts use each ID once.

**No differential test against the Phase 1 model.** The single-record model
of Phase 1 is no longer in the code (Phase 2 replaced it in 0.16.0), so a
differential test would have to rebuild it. The evidence instead:
`fiscal_partitioned_state.rs` checks, after every boundary of random
sequences of batches, receipts, closes, archive commits, transitions, and
context changes, that the records are exactly the partition of a valid state
and that every report the observers hold is what the records derive, and
that restore and exact replay agree. For the record-scoped archive helpers,
every archive test now prepares through them, and
`fiscal_cycle_archive.rs` checks the selection order across ledgers and its
stop at the limit exactly (`a_commit_spans_several_blobs_and_the_index_is_cumulative`);
counting open books as closed fails `fiscal_archive_lifetime.rs`.

## The due instant

**The finding (E-30).** The 0.17.0 gate run measured the world due instant
(all 4,000 observers due at one boundary, one dirty cycle each) at 2,504 ms,
fiscal 1,821 ms and engine 676 ms, with a 472 MB heap peak and 6.52 million
allocator calls for 15.7 MB of payload.

**Profile.** No sampling profiler runs unprivileged on the recording machine,
so the report system was timed with temporary `Instant` spans (removed before
the commit), in a release build of `fiscal-world-profile` with
`--preset world --courts institution --cadence interval` and a new
`--rows interval`, which measures only the idle boundaries, the spread due
day, and the due instant, in a fresh process. Two findings:

- **Working set.** In a fresh process the same boundary took about
  1,260 ms, fiscal about 790 ms and engine about 470 ms, with the same heap
  peak and within 0.01 % of the same allocator calls, so a record cost fiscal
  about 80 µs at the due instant against about 73 µs on a spread day: no
  per-record cost grows with the instant's size. The 0.17.0 figure was
  measured at the end of the gate run, after 24 held months in the same
  process, so most of the gap to 190 µs a record is that process's state
  (its heap after the months it ran), not the boundary's work. Fewer
  allocations help in both.
- **Where the report system's time went** (one fresh-process sample, before):
  reading the books it reports, 525 ms, of which decoding each scope's ledger
  book and binding 342 ms for 50,000 scopes; building part payloads as
  `serde_json::Value` objects, 183 ms for 5,603 parts; writing observer
  records and queue pages, 62 ms; decoding the 4,000 observer records, 27 ms;
  queue pages, 14 ms; part digests, 16 ms; measuring payloads, 11 ms;
  `knowledge_heads` for 9,603 drafts, 4 ms. The suspects of the finding that
  did not matter: per-observer record loads (27 ms) and per-record
  `knowledge_heads` (4 ms).

**Decision.**

1. **Read a book without decoding it.** The report system reads a scope's
   mechanism and, of its book, only the fields its totals are summed from,
   borrowed from the record's payload (`BookFigures`); `FiscalCycleBook::totals`
   and this read share one function (`cycle_totals`), so they cannot
   disagree. A scope row takes its institution from its office's row block
   (an office index lists only that institution's scopes and is never
   rewritten), its index scopes are borrowed rather than copied, the ledger
   reference is one reused buffer, the reads are keyed by cycle and then
   scope, and the ledger version a scope row cites is read for that row
   alone. The observer is copied once, not twice.
2. **Encode parts compactly.** The engine takes a knowledge payload as a
   `serde_json::Value` and charges `serde_json::to_vec(&payload).len()`, so
   the tree cannot be skipped; it can be made small. A part record (version 6
   of the `fiscal_report` schema) lists its rows' institutions once each in
   an `institutions` table and encodes each row as
   `[row, institution, mechanism, scope_binding_id, facts]` (an index into
   the table, and an empty scope for an institution row) and each fact as
   `[unit, payment_form, assessed minimum, assessed maximum, collected
   minimum, collected maximum, outstanding minimum, outstanding maximum]`,
   arrays rather than an object per row, fact, and amount. `FiscalReportPart`
   keeps its fields and decodes the new form; `FiscalReportRow` on its own
   keeps its object form. The digests that decide whether a part is
   republished are over the rows, not the JSON, so nothing is republished
   because of the change.

## Contract changes

- `canwu_fiscal::prepare_fiscal_archive(run, book_limit, store)` is new;
  `FiscalState::prepare_fiscal_archive` is gone from the public API.
- `enqueue_fiscal_archive(run, prepared, store)` loses its `sealed_cut`
  argument; `finalize_fiscal_archive_retention` keeps its signature.
- `CanwuRun::sealed_evidence_cursor`, `Canwu::sealed_evidence_cursor`,
  `CompactedCanwu::sealed_evidence_cursor`, and the two `canwu-sim` methods
  are new. A type outside the workspace that implements `CanwuRun` must add
  the method.
- The `fiscal_report` knowledge schema moves to version 6 (new schema hash):
  a part's payload is the compact form above. Version 5 is no longer
  registered, so a scenario whose knowledge carries a version-5 report record
  is refused.
- The plugin is `0.5.0-experimental` with a new semantic hash, so a snapshot,
  checkpoint, or journal of 0.17.0 does not restore or replay.
- Unchanged: report contents and lines, part digests, the observer record,
  the publication budget and its charge (now smaller per part), every fiscal
  record format, and `FISCAL_RUNTIME_SCHEMA_VERSION`.

## What can still fail a boundary

Nothing new. The archive helpers are host calls: their refusals
(`UnsealedCitations`, `Backpressure`, `NothingToArchive`, `Mismatch`) are
returned to the host and settle nothing. The report system builds every
payload it proposes from its own rows, inside the budget it read, as in
0.17.0. A part whose table lacks an institution its row names fails to
decode in a host read (`assemble_fiscal_reports`), never in a boundary.

## Costs before and after

Release builds of `fiscal-world-profile` on the `world` preset (200
partitions, 50,000 scopes, 4,200 institutions, 4,000 observers, courts at
institution granularity, monthly interval cadences), on the machine of the
0.17.0 runs. That machine was shared with other workers' builds and a second
profile throughout, so elapsed figures move by a third or more from run to
run and are context; allocator calls, heap peaks, and payload bytes do not
depend on load. Files are under `benchmarks/baselines/` and the method in
`benchmarks/README.md`, "World-scale fiscal profile".

**Allocations, heap, and payload** (allocation-counting build; 0.17.0 from
`2026-10-16-fiscal-world-world-institution-interval-allocations.json`, 0.18.0
from `2026-10-16-fiscal-world-world-institution-interval-0.18.0-allocations.json`):

| Boundary | Measure | 0.17.0 | 0.18.0 | Change |
| --- | --- | ---: | ---: | ---: |
| Due instant (4,000 observers due) | allocator calls | 6,516,716 | 4,261,604 | −35 % |
| | heap peak | 472.1 MB | 305.6 MB | −35 % |
| | report payload | 15,671,758 B | 5,545,628 B | −65 % |
| Spread due day (134 observers due) | allocator calls | 213,438 | 155,798 | −27 % |
| | heap peak | 33.3 MB | 28.7 MB | −14 % |
| | report payload | 437,768 B | 156,327 B | −64 % |
| Idle (one tick, nothing fiscal due) | allocator calls | 11,123 | 11,123 | none |
| Archive commit of 256 books, host side | allocator calls | 6,154,725 (assemble 2,032,767, prepare 14,405, verify and enqueue 2,077,789, finalize 2,029,764) | 98,265 (prepare 33,767, verify and enqueue 64,384, finalize 114) | −98 % |
| Archive commit of 4,096 books, host side | allocator calls | 6,901,382 (assemble 2,032,767, prepare 206,091, verify and enqueue 2,674,634, finalize 1,987,890) | 1,192,755 (prepare 389,516, verify and enqueue 802,921, finalize 318) | −83 % |

The due instant still publishes 9,603 bodies to 4,000 holders in 10
directives, with no drain. Every other boundary of the profile (the bursts,
receipts, and the close of every book, which publish no report on this
cadence) is within 0.01 % of its 0.17.0 count.

**Elapsed, the due instant in a fresh process** (`--rows interval`, three
alternating rounds of a build of the 0.17.0 code against 0.18.0, medians of
three samples each in milliseconds, settlement with the fiscal handlers'
share; `2026-10-16-fiscal-world-interval-rows-{0.17.0,0.18.0}-round{1,2,3}-elapsed.json`):

| Round | 0.17.0 due instant (fiscal) | 0.18.0 due instant (fiscal) | 0.17.0 spread day (fiscal) | 0.18.0 spread day (fiscal) |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1,827 (1,108) | 1,546 (869) | 67.4 (35.9) | 62.7 (34.0) |
| 2 | 2,205 (1,492) | 1,259 (714) | 191 (65.6) | 65.4 (28.0) |
| 3 | 4,054 (2,424) | 1,858 (1,082) | 68.6 (34.9) | 46.2 (21.3) |

A 0.17.0 round recorded with less load just before the alternation
(`...-0.17.0-round0-elapsed.json`) took 1,259 ms (fiscal 789, engine 470) at
the due instant and 45.4 ms (fiscal 22.2) on the spread day; idle
boundaries took 0.64 ms in it and about 1 ms in every loaded round of
either build. In the spans of one fresh-process sample each, the report
system took 962 ms in 0.17.0 and 515 ms in 0.18.0: reading books 525 and
290 ms (decoding ledgers 342, which included the ledger version of every
scope, against 109, with 75 for the version a scope row cites), building
part payloads 183 and 33 ms, digests 16 and 13, measuring payloads 11 and
4, writing observer and queue records 62 and 58, decoding observers 27 and
20.

**The gate run** (`--preset world --courts institution --cadence interval
--held-cycles 24`; 0.17.0 from
`2026-10-16-fiscal-world-world-institution-interval-elapsed.json`, 0.18.0
from `2026-10-16-fiscal-world-world-institution-interval-0.18.0-elapsed.json`).
The 0.18.0 run had the machine at 77 to 94 % load, so the rows this change
does not touch (the last three rows) show the load factor, 1.1 to 3.4:

| Row | 0.17.0 | 0.18.0, loaded |
| --- | ---: | ---: |
| Due instant, ms (fiscal, engine) | 2,504 (1,821, 676) | 2,160 (1,219, 941) |
| Spread due day, ms (fiscal, engine) | 43.2 (22.8, 20.5) | 56.2 (25.1, 31.0) |
| Archive commit of 256 books, host side, ms | 1,402 (assemble 663, prepare 5, verify and enqueue 376, finalize 358) | 166 (prepare 77, verify and enqueue 89, finalize 0.06) |
| Archive commit of 4,096 books, host side, ms | 1,813 (743, 88, 619, 363) | 501 (prepare 167, verify and enqueue 334, finalize 0.1) |
| Held knowledge after 24 months | 14,138 bodies, 15.13 MB of payload | 14,138 bodies, 5.67 MB |
| Continuation checkpoint after months 1 and 24 | 172 MB, 293 MB | 162 MB, 284 MB |
| Assess every scope, settlement, ms (untouched) | 4,568 | 8,102 |
| Close every book, settlement, ms (untouched) | 5,052 | 5,693 |
| Continuation restore after 24 months, s (untouched) | 58 | 196 |

A simulated month took 100 to 240 s in the loaded run against 73 to 131 s,
which follows the load and does not compare; the 13 commits of a month save
about 17 s of host archive work. The checkpoint still grows about 3.9 MB a
month through the engine's request index (E-28).

## Tests

- `report::tests::direct_payloads_are_the_serialized_records` (unit) gains a
  part of two institutions and decodes every payload back: it fails if the
  direct encoder and the wire form of `FiscalReportPart` diverge or the
  decode loses a field.
- `fiscal_cycle_archive::a_book_citing_evidence_newer_than_the_last_seal_is_refused_until_it_is_sealed`
  now seals a compact run before the quote exists and prepares through the
  run: the refusal comes from the run's own cut, the second seal lifts it,
  and the sealed run replays exactly. Passing `None` in the helper instead of
  the run's cut fails it.
- Every archive test, the reference crate, and both profiles prepare through
  `prepare_fiscal_archive(run, ...)`; counting open books as closed fails
  `fiscal_archive_lifetime.rs`.
- `fiscal_report_parts::a_scenario_with_a_version_one_report_record_is_refused`
  checks the schema at version 6.
- `canwu-api` `continuation_checkpoint::a_continuation_restored_run_continues_in_lockstep`
  checks `sealed_evidence_cursor` on all four kinds of run every day: `None`
  for a `Canwu` and for a `Canwu` restored from a full save, `None` for the
  compact run before its first seal, `Some(segment.end)` after each seal,
  unchanged by an idle reseal and the same on a fork, and, for each
  continuation restore, `Some(checkpoint.settled_cut)`, equal to
  `continuation_base().cut`, then `Some(segment.end)` after each later seal.
  Answering `Some` for an unsealed run fails it.

## Deviations and limits left

- The filed fix for E-30 was to profile with sampling; spans were used, as
  allowed, because no sampling profiler runs unprivileged on the machine.
- The compact part encoding is a format change the finding did not name; it
  is the only way to shrink the `Value` tree the engine API requires.
- Left as engine-side findings, outside this package: the view's
  `current_domain_record_version`, which the report system calls once per
  scope row (45,065 calls, about 75 ms at the due instant), checks the
  declared read and looks the record up twice and builds a `StateKey` per
  call, as `domain_record` does once per ledger read (50,000 calls); and the
  engine's own share of the due instant and of its heap peak (the record
  writes of 4,000 observers and 2,561 queue pages, and the boundary's
  journal and commitments).
- The fiscal books are still summed per boundary from every scope of a
  court's partition (50,000 ledger reads at the due instant); a cached total
  per office and cycle would make that proportional to the changed books, at
  the price of a derived field written by every action.
