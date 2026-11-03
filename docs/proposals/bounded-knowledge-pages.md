# Bounded Knowledge Pages

Status: implemented on branch `claude/youthful-johnson-b38ta4`, for the
release after 0.22.0 (not released; the workspace version stays 0.22.0). It
closes downstream gap G-94 (`canwu-knowledge`, read through `canwu-sim`'s
`SimulationView::knowledge_records`): an in-boundary knowledge page costs
the page and the entries it reads, not the holder's whole history. It
changes `canwu-knowledge` only (and rustdoc in `canwu-sim`). No public API,
state, record, snapshot, checkpoint, hash, or format changes: the read cut's
root keeps its material, so every stored read cut and every hash over one is
unchanged.

## Problem

Four boundary systems of the downstream page knowledge records inside a
boundary (its ledger pages with `learned_after`, an event sweep, character
assessments, an espionage perspective rebuild), and their cost grows with
each holder's history: `Daily` p99 went from 2.82 to 3.74 ms between campaign
months 14 and 22. Verified in the 0.22.0 source:

- `SimulationView::knowledge_records` (`canwu-sim` `runtime/view.rs`) calls
  `KnowledgeSnapshot::query_current`, or `query_with_overlay` while the
  boundary has staged records (`canwu-knowledge` `lib.rs`).
- Both derive the read cut with `holder_projection_root`, which builds the
  view of every entry the holder holds (`full_history_views`), encodes them
  as one JSON document, and hashes it: O(history) per page.
- `query_records` then checks every entry's relations (`InvalidLedger`),
  filters every entry, and sorts the candidates before it applies the
  cursor and the limit: O(history) again, whatever the `limit`,
  `learned_after`, or cursor.
- `knowledge_heads` and the 0.22.0 host reads (`knowledge_heads`,
  `knowledge_current_records` on the run handles) are indexed, but they
  answer current heads, not a page bound to a cursor.

## Decision

Keep the page contract and its answers exactly; change how they are
computed:

1. **The read cut from a running hash.** Each holder ledger keeps the
   state of the hash of its projection root after every entry's view, so
   deriving the cut finalizes a copy of that state and reads no entry. The
   root's material does not change (below).
2. **Relation integrity at admission.** Each holder ledger keeps the set of
   relation targets it does not hold. Admission through
   `insert_publication` refuses an unresolved relation, so the set stays
   empty; the authoring path `HolderLedgers::insert` may fill it. A page
   tests that set (and the absence of global ID zero) instead of reading
   every entry, and refuses with `InvalidLedger` exactly when 0.22.0 did.
3. **A page index.** Each holder ledger files every entry, and separately
   every current entry, under `(schema, learned_at, local)`. A page seeks
   past `learned_after` and its cursor in the range of each schema it names
   (every schema the holder has held when it names none), merges those
   ranges forward by `(learned_at, local)`, and stops at one match more than
   its limit or past `learned_at_or_before`.

A cursor from another cut is still refused by `validate_cursor`
(`ReadCutUnavailable`), and every refusal comes in 0.22.0's order.

## Root material (unchanged)

`holder_projection_root` is blake3 of the domain
`canwu.knowledge.holder-projection.v1`, a zero byte, and the compact JSON
`{"holder":<holder>,"full_history_views":[<view>,...]}`: one
`KnowledgeRecordView` per entry the holder holds, in global ID order, with
relations as the holder's local IDs. That is 0.22.0's material, byte for
byte; the overlay root (`canwu.knowledge.holder-overlay.v1` over the staged
entries' views) is unchanged too.

blake3 hashes a stream, so the hash state after the prefix (everything
before the closing `]}`) can be kept and extended by one view: an entry
admitted with a higher global ID than every entry its holder holds appends
its view when it is admitted. Any other change changes the bytes from some
entry on: an entry that leaves (retention, `remove_entry`, `remove_body`)
and the entries that named it, which drop the name; an entry admitted below
the holder's highest global ID (an existing body given to another holder);
an entry whose dangling relation comes to resolve (authoring path only).
Such a change marks the holder's hash stale from the lowest global ID it
touched. `refresh_commitment`, which the runtime calls at every commit and
which decoding calls, re-hashes the holder's views from the last kept state
before that ID; the ledger keeps a copy of the state every 256 views. Until
the refresh, a cut is still exact: it hashes the same views from that copy
without keeping the result.

## Invariant

- For every ledger, holder, query, cursor, and overlay, a page, its `next`
  cursor, its read cut, and every refusal equal 0.22.0's, whether the
  running hash is fresh or stale, and after a decode.
- The running hash, the copies, the page index, and the unresolved-target
  set are functions of the holder's entries alone: derived on every
  insertion and removal, rebuilt on decode, never encoded, never part of
  the commitment or of `PartialEq`. Two ledgers with the same entries page
  alike whatever histories built them (a schema's index in the page keys
  depends on the order schemas arrived; it groups keys and never orders a
  page).
- The page index holds exactly the held entries, and its current part
  exactly the entries no entry of the holder supersedes, maintained at the
  same points as the current-record index.

## Ownership and affected surfaces

| Surface | Change |
| --- | --- |
| `canwu-knowledge` | `projection.rs` (new, private): the running hash. `ledger.rs`: `HolderLedger` gains the page index, the unresolved-target set, and the running hash, maintained in `insert_entry` / `remove_entry` / `index` / `unindex`, refreshed in `refresh`; `with_staged` marks the merged copy's hash stale (it is never read) so staged entries do not extend it. `lib.rs`: `query_records` seeks; `holder_projection_root` reads the running hash. |
| `canwu-sim` | rustdoc of `SimulationView::knowledge_records` only; it calls the same functions. |
| `canwu-api`, extensions, `canwu-debug`, examples | none. `admin_query_knowledge` and `CanwuViewer::query_knowledge` page through the same `query_current` and get the same speed-up. |

## API surface

No public item is added, removed, or changed. Everything new is
`pub(crate)`. The rustdoc of `refresh_commitment`, `HolderLedger`,
`query_current`, `query_with_overlay`, and `SimulationView::knowledge_records`
states the new costs.

## Cost and complexity

With `n` the holder's entries and `k` the schemas the page reads:

- **A page**: `O(k log n)` to seek, plus, per entry it reads, one index
  step, one entry lookup, and an `O(k)` scan of the schema ranges to pick
  the lowest key. It reads the entries it returns, the ones its subject
  filter skips, and one more to learn there is a next page. A page that
  names no schema merges every schema the holder has ever held. Superseded
  entries are never read by a `CurrentHeads` page.
- **The read cut**: a copy of a 1.9 KB hash state and its finalization,
  with a fresh running hash. Stale (between a change and the commit's
  refresh), it hashes the views from the copy before the change.
- **An admission**: three more ordered-map insertions (page index, current
  index, and the schema table on a new schema) and, above the holder's
  highest global ID, one view encoding and hash. Below it, the commit's
  refresh re-hashes from the copy before it.
- **A retirement or removal**: the commit's refresh re-hashes the views of
  the entries after the copy before the earliest change: `O(entries after
  it)`, once per commit per holder. Retiring the head of a line whose
  previous head is recent costs little; retiring an old entry under a long
  `Keep` history costs that history once, at that commit (0.22.0 paid it on
  every page).
- **A decode** (snapshot, continuation, paged restore): one view encoding
  and hash per entry, in local order; an entry whose global ID is below an
  earlier one's, or a relation to a later entry, marks the hash stale and
  the decode's refresh re-hashes from there (at most twice per entry).
  Load validation's replay of the journal appends as commits do and, after
  its first retirement, re-hashes once at the end.
- **Memory**: per holder a hash state (1.9 KB) and three ordered maps (the
  schema table, the page index, the current page index); per entry two
  index keys; a hash-state copy per 256 views.

Measured on this branch and on `a405810` (0.22.0) with the same program
(not committed), in a release build in a 4-core cloud container: median of
200 calls (5 for decode, 1 for a build or a retirement), three runs each,
ranges over the three runs. One holder of 10,000 entries of one schema,
entry `i` learned at minute `i`, every tenth superseding the entry ten
before it:

| Measure | 0.22.0 | This branch |
| --- | --- | --- |
| Views hashed to derive the cut (test counter) | 10,000 | 0 |
| Entries read by the page (test counter) | 20,000 (check, then filter) | ≤ 11 (10 returned, at most 1 more) |
| `learned_after` page past all but 10, limit 50, `CurrentHeads` | 28.8–30.7 ms | 7.0–10.7 µs |
| same, `FullHistory` | 28.1–29.2 ms | 6.9–7.1 µs |
| first page of 50, no schema named | 25.3–35.3 ms | 25.5–41.1 µs |
| page of 50 from a cursor | 26.6–38.4 ms | 26.3–27.3 µs |
| `learned_after` page with 3 staged entries (overlay) | 27.5–31.9 ms | 29.7–30.5 µs |
| admit the 10,000 entries one by one, then refresh | 56–63 ms | 86–104 ms |
| one more admission and refresh | 17.0–29.0 µs | 21.1–21.6 µs |
| retire entry 9,995 and refresh | 91–158 µs | 100–119 µs |
| retire entry 5 and refresh | 70–104 µs | 14.0–14.2 ms |
| decode the 3.7 MB ledger | 74–104 ms (median 78) | 86–91 ms (median 89) |
| live heap per entry | 1,636 B | 1,800 B (+10%) |
| live heap per holder, 1,000 holders of 10 entries in 2 schemas | 52.9 KB | 61.9 KB (+9.1 KB) |

The committed cost test (`a_page_costs_the_page_not_the_history`) states the
visit counts on a 10,000-entry holder alone and beside 10,000 entries of a
schema the query does not name (0.22.0: 20,000 views and 40,000 entries
there), from a cursor, and with an overlay. Timings are this container's
and indicative only; debug builds were not measured.

## Persistence and replay

Nothing persisted changes: no snapshot field, checkpoint, journal, record,
schema, plugin descriptor, semantic hash, or format number. The read cut's
root is byte-identical to 0.22.0's, so the read cuts `canwu-correspondence`
stores in operation addresses and `canwu-law` in actor contexts and outbox
items, and every state, record, evidence, and checkpoint hash over them, are
unchanged. Checked three ways: the equivalence tests compare every root
with 0.22.0's computation; the run's commitment code is untouched; and the
checkpoint hash at the end of `canwu-correspondence`'s
`gap_g25_correspondence_delegated_carrier` and `canwu-law`'s
`live_actor_context_is_derived_and_persisted_by_the_kernel` (both store read
cuts) was printed on `a405810` and on this branch and is the same (not
committed; see the hand-back). The derived structures are rebuilt on every
decode, so a sealed run, its fork, a continuation restore, a snapshot reload,
and an exact replay page alike; the 0.22.0 host-read and continuation tests
pass unchanged.

## What can still fail

As in 0.22.0: `InvalidLimit`, `InvalidCursor`, `ReadCutUnavailable`,
`InvalidLedger` (now from the unresolved-target set: only an authoring-path
ledger with a dangling relation or global ID zero, or an overlay that names
an entry the holder does not hold), and `Encoding` (a part of the root that
cannot be encoded, which JSON values never fail). No boundary fails for a
reason it did not fail for before.

## Alternatives not taken

- **Key the cut by the ledger's revision** (a holder revision counter, or
  the holder's commitment root). O(1) and no hash state, but it changes the
  root's material, so every read cut stored by correspondence and law, and
  every state and evidence hash over one, changes; the commitment root also
  covers origin evidence and global IDs, which a holder-relative cut does
  not. Not needed: the material can be kept.
- **A chunked view root** (leaf hash per view, a root per chunk, like the
  commitment). A removal would cost a chunk, not the entries after it, but
  the material changes, with the same hash break. It is the path to take if
  the owner prefers bounded retirement cost to hash continuity.
- **Compute the running hash lazily on read** (a cell inside the persistent
  ledger, filled by the first page). No cost for holders nobody reads and
  none at decode, but interior mutability in a structure shared by forks,
  checkpoints, and rollback copies, and the first page after each change
  pays. The commitment's pattern (stale marks, refresh at commit, an exact
  stale read) was kept.
- **One page index per holder without schema ranges.** Less memory, but a
  page naming one schema reads every other schema's entries in its range.
- **A map of page indexes per schema.** The first version did this; each
  empty `im` map allocates a node, so a small holder of two schemas paid
  16 KB more. Interned schema indexes in one key cost 9 KB.
- **A subject index for pages.** Pages filter subjects as they read; a
  subject index per schema would add memory for every holder for a filter
  the downstream pages do not lead with.

## Known limits

- A page that filters by subject reads the entries of its schemas and time
  range that lack the subject; only schema, `learned_after`, the cursor,
  `learned_at_or_before`, and `CurrentHeads` are seeks.
- A retirement or removal costs the commit a re-hash of the holder's views
  after the hash copy before it, once per commit per holder: retiring an old
  entry under a long history costs that history once. An existing body given
  to a holder below its highest global ID costs the same from that entry.
- An admission above the holder's highest global ID encodes and hashes the
  entry's view, about 3 µs more per entry (10,000 admissions: 56–63 ms
  before, 86–104 ms now); decoding re-hashes every entry's view (median
  +14% on the measured 3.7 MB ledger), and load validation's journal replay
  does it again for the ledger it rebuilds.
- Memory grows by about 10% per entry and 9 KB per holder.
- A page that names no schema seeks once per schema the holder has ever
  held, even one it no longer holds.

## Tests

- `canwu-knowledge` `equivalence::pages_and_read_cuts_equal_the_previous_path`:
  600 steps of runtime-shaped publications to three holders (shared bodies,
  existing bodies given to new holders below their highest ID, supersessions
  and contradictions, schemas in two kinds and two versions, learned times
  with ties and out of ID order), removals, retirements, body removals, and
  refreshes at random points. After every step each holder's root equals
  0.22.0's; every third step a random query (schemas with duplicates and an
  unheld one, subjects, both time bounds, both views, limits including 0 and
  1,001) is paged to its end on `query_current` and on `query_with_overlay`
  with a random overlay, every page and refusal equal to 0.22.0's, then a
  cursor is replayed under another query, forged, moved, and checked against
  a fixed cut, and a cursor from the previous step's cut is refused alike.
  Then again on a decoded copy. The 0.22.0 path is kept verbatim in the test
  module as the oracle.
- `equivalence::authoring_ledgers_are_refused_and_rooted_alike`: the
  authoring path with dangling relations, relations to IDs still to come,
  self-reference, global ID zero, and replaced bodies: roots and
  `InvalidLedger` refusals equal 0.22.0's.
- `equivalence::a_page_costs_the_page_not_the_history`: the cost bounds of
  acceptance (b), above, with a test-only visit counter (`cfg(test)`).
- The tests use a hash copy every 4 views instead of 256 so that generated
  ledgers cross many copies.
- Checked to fail (not committed), each caught by these tests: no stale mark
  on removal, or for the entries naming a removed one, or for an entry whose
  dangling relation resolves; appending below the highest ID; keeping or
  replaying from the copy at the stale ID; a dangling target kept after its
  namer leaves, or a self-reference left dangling; a superseded entry not
  restored to the current index, or not removed from it; no schema dedup; a
  `learned_after` seek that keeps equal times; the cursor and `learned_after`
  bounds combined with `min`; a schema range read past its schema. Moving
  where copies are taken is not caught, as it should not be: any copy at any
  ID is valid.
- `canwu-api` `gap_g92_api_host_knowledge_reads` (extended): besides its
  0.22.0 checks (`knowledge_heads`, `knowledge_current_records`, and an
  in-boundary `CurrentHeads` page agree on full, sealed,
  continuation-restored, and reloaded runs), every run now pages two
  holders' full histories (one holds a `Keep` and a `RetireWhenSuperseded`
  line, so retirements happen) with `admin_query_knowledge` before every
  boundary: the first page and its read cut equal the full run's, and the
  full run's cursor continues on each restored run to the same page. So a
  continuation checkpoint still saves, restores, and pages.
- Unchanged and passing: `canwu-api` `continuation_checkpoint`.

## Upgrading notes

Nothing to do. Pages answer as before, faster; a host or plugin that kept
its own cache of knowledge pages can drop it.

## Documentation delta

In this change: `docs/architecture.md` (the holder query paragraph),
`docs/end-state.md` (the query surface paragraph), `docs/versioning.md`
(the next release's notes and the 0.22.0 known limit), `opus-progress.md`
(G-94, E-57), the rustdoc. Not in this change, because the website is the
owner's to edit and deploy: the mirror of those paragraphs in
`website/src/content/docs/architecture/knowledge-information.mdx` and
`website/src/content/docs/en/architecture/knowledge-information.mdx` (a page
costs its page, not the history; the read cut's running hash; the
retirement and decode costs), which the release pass updates in both
languages with the release notes below; until then the English page still
implies that a cursor page costs the holder's history.

## Terminology candidates

None: the page index, the unresolved-target set, and the running hash are
internal. "read cut" and "holder-relative knowledge" keep their rows.

## Release notes text (for `docs/versioning.md`)

- **Knowledge pages cost the page** (downstream gap G-94;
  [proposal](proposals/bounded-knowledge-pages.md)). A knowledge page
  (`SimulationView::knowledge_records`, `admin_query_knowledge`,
  `CanwuViewer::query_knowledge`) no longer reads the holder's whole history.
  Each holder ledger keeps a page index by schema and `(learned_at, local)`,
  a set of unresolved relation targets that admission keeps empty, and the
  running hash of its read cut's projection root; a page seeks past
  `learned_after` and its cursor and reads what it returns plus what its
  subject filter skips, and deriving the cut reads no entry. Every page,
  cursor, read cut, and refusal is what 0.22.0 returned, and the root's
  material is unchanged, so no state, record, evidence, or checkpoint hash
  changes. Costs moved: a commit that retires or removes an entry re-hashes
  the holder's views after the hash copy before it (every 256 views); an
  existing body given to a holder below its highest global ID does the same;
  each admission encodes and hashes its view (about 3 µs) and decoding
  re-hashes every view; memory grows by about 10% per entry and 9 KB per
  holder. On a 10,000-entry holder a `learned_after` page went from about
  29 ms to 7 µs.

Formats: none change. Upgrading from 0.22.0: nothing to do.

## Review record

One independent review by a sub-agent that had not seen the author's
reasoning, given the code and the contract: verdict "ready", nothing
blocking. It checked every mutation path of the running hash, the byte
equality of the borrowed view encoding with `KnowledgeRecordView`, the
unresolved-target set against 0.22.0's scan, the page indexes through
self-supersession and re-currenting, the bounds, error order, the absence
of reverse `im` iteration, `PartialEq`, `Debug`, the wire, `Send`/`Sync`,
and ran its own probes against the oracle (authoring decodes with forward
references, twelve mixed seeds without refresh, an empty overlay, the real
256-view interval on a 1,900-entry holder with removals on checkpoint
boundaries). Non-blocking findings and what was done:

1. Retiring an old entry costs the commit `O(entries after it)`, and
   holders nobody pages pay it too. Kept: it is what keeping the root's
   bytes costs, and it is stated in the costs, the known limits, and the
   release notes. Left for the owner (hand-back): measure how old the
   downstream's retired entries are before the release, or choose the
   chunked view root, which changes the hashes.
2. Two silent fallbacks would hide a broken invariant (the append of an
   entry it cannot find returned without marking the hash stale; the page
   walk skipped an index key it could not resolve). Fixed: the append marks
   the hash stale, and the walk asserts in debug builds.
3. Proposal precision (the merge's `O(k)` per entry, the decode's re-hashes,
   the `with_staged` wording) and a long line in `docs/end-state.md`.
   Fixed.
4. The website mirror of `knowledge-information.mdx` is not updated. Not
   changed here (the task leaves the website to the owner); the deferral is
   stated in the documentation delta.
5. The cost test's retirement bound named the release interval while tests
   use 4. Fixed: the bound uses the interval in force.
6. Minor costs (a buffer per admission, the schema lookup per admission, 9
   KB per holder even when empty). Not changed; the memory is a stated
   known limit.

## Hand-back

- **Branch and commit.** Branch `claude/youthful-johnson-b38ta4` (the
  branch the cloud session was given), one commit on top of `a405810`
  (0.22.0): the commit that adds this file. A commit cannot name its own
  hash; `git log -1 --format=%H -- docs/proposals/bounded-knowledge-pages.md`
  gives it, and the hand-back message states it. No pull request, merge,
  tag, release, publication, or website deploy; the version stays 0.22.0.
- **Public API changes.** None. Everything new in `canwu-knowledge` is
  `pub(crate)`; rustdoc only in `canwu-sim`.
- **Hashes.** Unchanged. The read cut's root keeps its material byte for
  byte (the equivalence tests compare every root with 0.22.0's
  computation); no persisted or committed structure changes; the
  checkpoint hash after `gap_g25_correspondence_delegated_carrier`
  (`9f5d0360…e79f8`, read-cut root `9bfbe6b5…bb041`) and after
  `live_actor_context_is_derived_and_persisted_by_the_kernel`
  (`8268da06…494ec`) is the same on `a405810` and on this branch.
- **Tests.** `cargo test --workspace`: 867 passed, 0 failed, 3 ignored
  (0.22.0's 864 and three new). `cargo test --workspace --features
  canwu-api/continuation-gate`: 867 passed, 0 failed, 3 ignored, on the final
  tree. After the review's fixes, `canwu-knowledge`, `canwu-sim`, and
  `canwu-api` with their doc tests: 228 passed. `cargo fmt --check`, Clippy
  with warnings denied, `cargo doc --workspace --no-deps` with warnings
  denied, and `cargo check -p canwu-debug`: clean. The two workspace runs
  used `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_INCREMENTAL=0` to fit the
  session's disk allowance. Not run: the public examples and the
  performance harness (no API, behaviour, or harness workload change).
- **Measured.** On a 10,000-entry holder: deriving the cut hashes 0 views
  (0.22.0: 10,000) and a `learned_after` page past all but ten reads at most
  11 entries (0.22.0: 20,000); it takes 7.0–10.7 µs instead of 28.8–30.7 ms
  in a release build. Costs moved: retiring entry 5 of 10,000 costs its
  commit 14 ms (0.22.0: 0.1 ms); 10,000 admissions 86–104 ms (56–63);
  decode median 89 ms (78); memory +10% per entry, +9.1 KB per holder. The
  table above has the rest.
- **Review.** One independent review, "ready", nothing blocking; its six
  findings and what was done are in the review record above.
- **Left open.**
  1. The owner's decision on the retirement cost: a commit that retires an
     old entry under a long history re-hashes that history once, for
     holders nobody pages too. Before the release, measure how old the
     downstream's retired entries are on its campaign; if that cost
     matters more than hash continuity, the chunked view root (Alternatives)
     bounds it and changes the read-cut root, so every stored read cut and
     hash over one.
  2. The website mirror of the knowledge paragraphs (both languages) and
     the release entry of `docs/versioning.md` (the release notes text
     above), at the release.
  3. The downstream's adoption at its next pin move, after the release;
     its `Daily` p99 past month 22 is the real check.
  4. Debug-build timings were not measured; subject-filtered pages still
     read the non-matching entries of their range.
