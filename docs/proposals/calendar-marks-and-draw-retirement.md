# Calendar Marks and Keyed-Draw Retirement

Status: implemented in 0.20.0. Two changes to boundary scheduling and random
draws in `canwu-sim`, reached through `canwu-api`: host calendar marks on
calendar boundaries (downstream gap G-69) and a retirement horizon for
operation-keyed draws (downstream gap G-71). `canwu-core` is unchanged. Same
version only: no compatibility work with 0.19.0 bytes or hashes.

## Calendar marks (G-69)

### Problem

A calendar boundary carries only fixed cadences:
`schedule_calendar_boundary(at, cadences: Vec<SystemCadence>)`, with
`SubDaily`, `Daily`, `Monthly`, `Seasonal`, `Annual`, and `EraScheduled`. A
host that runs two calendars whose month or term starts sometimes differ (two
polities with their own reckonings, say) cannot say which calendar a
`Monthly`, `Seasonal`, or `Annual` boundary belongs to. Every system
registered on that cadence wakes at both calendars' starts and has to work
out from the time whether the boundary is its own, the same filter repeated
in every plugin. Verified in the 0.19.0 source: `IngressPayload::Calendar`
holds `cadences` only, settlement merges the cadences of every calendar
ingress due at the instant into `BoundaryRequest::cadences`, and
`boundary_system_due` wakes a scheduled system whenever its cadence is among
them.

### Invariant

- The core keeps no calendar. The host names its calendars and computes when
  each one's periods start; the engine only carries the host's marks and
  wakes systems by them.
- Which systems are due in a boundary is a pure function of the boundary's
  cadences, its calendar marks, and whether it admitted events or ingress,
  so live, sealed, restored, and replayed runs wake the same systems.
- A system without a calendar key behaves exactly as in 0.19.0.

### Design

- **`CalendarMark { cadence, calendar }`**: the boundary is a `cadence`
  boundary of the host calendar `calendar`. The key is a canonical label
  (non-empty, no surrounding whitespace or control characters) of at most `MAX_CALENDAR_KEY_BYTES`
  (64) bytes; the cadence is scheduled, never `EventDriven`. Marks order by
  cadence, then key.
- **Scheduling.** `schedule_calendar_marks(due_at, marks)` on `Simulation`,
  `CompactedSimulation`, `Canwu` (through `gated!`), and `CompactedCanwu`
  queues one calendar ingress record whose cadences are the marks' cadences.
  `schedule_calendar_boundary` is unchanged and queues unmarked cadences.
  Calendar ingress due at one instant is still merged into one boundary,
  marks included. `IngressPayload::Calendar` gains `calendar_marks`
  (canonical: sorted, unique, each mark's cadence among `cadences`).
- **Registration.** `BoundarySystemContract::calendar: Option<String>`. With
  a key, the system is due only in a boundary that carries the mark
  `(cadence, calendar)`. Without one it is due in every boundary of its
  cadence, marked by any calendar or by none (0.19.0 behaviour). A keyed
  system is therefore not woken by an unmarked cadence boundary; a host that
  wants a common period start for two calendars marks both.
- **Reads.** `SimulationView::calendar_marks()` returns the marks of the
  boundary being settled, without a read declaration: they are the host's
  calendar input to this boundary, like its time, and name no other source's
  work. A view outside a boundary system returns none. `BoundaryRecord` and
  `BoundarySummary` gain `calendar_marks`, so the boundary ledger
  (`boundaries_between`) lists the marks of settled, settling, and queued
  boundaries.
- **Knowledge reservations** count a keyed knowledge publisher as due only on
  its marks, by the same rule.
- `BoundaryRequest` is unchanged: a boundary the host settles directly takes
  its marks from the calendar ingress due at its instant, which is what exact
  replay re-queues.

Rejected: putting the calendar inside `SystemCadence` (every match on the
cadence in every extension would change, and a cadence would stop being a
plain category); a calendar table in the run configuration (the engine would
own calendar names and their lifecycle, which belong to the host).

### Persistence and replay

- The calendar ingress record, the boundary record, and the boundary summary
  carry `calendar_marks`, omitted while empty; the boundary record hash
  material (`canwu.boundary-record.v1`) covers it, omitted while empty, so an
  unmarked run encodes and hashes as before. The plugin descriptor carries
  `calendar`, omitted while `None`, so the descriptor of an unkeyed system is
  unchanged.
- Load validation refuses calendar ingress or a boundary whose cadences or
  marks are not canonical, a boundary whose marks are not exactly those of
  the calendar ingress it admitted, and a marked boundary in a journal without
  ingress; a continuation restore checks the pinned
  calendar ingress and the boundary ledger's summaries the same way; a
  descriptor with a key on an `EventDriven` system or a malformed key is
  refused. Every snapshot, journal, and continuation hash covers the marks.
- Exact replay re-queues each calendar record with its cadences and marks
  through the same canonical path, so it reproduces the record, the
  boundary's marks, and the systems woken.

### Refusals

- `schedule_calendar_marks`: `InvalidBoundary` for an empty list, an
  `EventDriven` mark, or a non-canonical or over-long key. Host errors before
  anything is queued.
- Registration: `InvalidPluginRegistration` for a key on an `EventDriven`
  system or a malformed key.
- Load: `InvalidSnapshot` for the shapes above.

### What can still fail a boundary

Nothing new: marks are fixed when the ingress is queued, and the wake rule
cannot fail.

### Known limits

- A mark says that a period starts, not which one; a system that needs the
  month's number computes it from the time and its own calendar.
- The number of marks a calendar ingress carries is not capped (the host is
  trusted and settles a bounded set of calendars).
- `BoundaryRequest` has no marks: a host that settles a boundary directly
  queues the marks as calendar ingress at the same instant.

## Keyed-draw retirement (G-71)

### Problem

An operation-keyed draw stays in the keyed-draw ledger for the rest of the
run: the ledger (since 0.15.0, the record of every drawn address since
0.16.0) gains an entry at each fresh keyed draw and nothing removes one.
Verified in the 0.19.0 source: `KeyedDrawLedger` has `insert_new` and no
removal, and a continuation checkpoint carries every entry. Settling a
boundary does no work per retained entry (lookups and inserts are logarithmic,
0.15.0's measurement), but each entry holds about 522 bytes of memory for the
run, is written into every continuation checkpoint, and is validated (its
value recomputed) at every continuation restore. A stream that draws daily
for a record that is discarded a few days later keeps every day's draw
although nothing will retry its address or cite it again. The downstream
measures about 0.35 µs per retained draw per boundary across its whole
boundary and budgets its retained draws against that figure.

### Invariant

- A retired draw is never answered again; until it retires, a draw is
  answered exactly as in 0.19.0.
- Which draws the ledger holds is a pure function of the committed draw
  journal, the persisted plugin descriptors, and simulated time, so a live
  run, a sealed run, a run restored from a snapshot, a checkpoint journal, a
  paged checkpoint, sealed segments, or a continuation checkpoint, and exact
  replay answer every keyed request alike and write the same continuation
  checkpoint at the same cut.
- Draw records are evidence and are not touched: a record that cites a
  retired draw's ID still resolves it (retained, sealed, or by the journal
  counters).

### Design

- **Declaration.** `BoundarySystemContract::keyed_draw_retirement:
  Vec<KeyedDrawRetirement>`, each `KeyedDrawRetirement { stream, after }`
  naming a stream the system declares in `random_streams`, at most once,
  with a positive `after`. It is part of the plugin descriptor, so it is
  fixed for the run and restored and replayed with it. A stream without one
  keeps its draws for the run.
- **Retirement time.** A fresh keyed draw of such a stream made at boundary
  time `t` retires at `t + after` (saturating). The ledger keeps a second
  persistent map from `(retirement time, draw ID)` to the entry, so a
  boundary finds what retires in time order.
- **When.** At the start of every boundary, before any system runs, the
  boundary's working copy of the ledger drops every draw that retires at or
  before the boundary time (`retire_through`). The drop does work only for
  what it drops and leaves the tree untouched when nothing retires, so a
  boundary with nothing to retire and a fork still share the kept tree. A
  failed boundary restores the ledger it started with; a commit keeps the
  pruned copy.
- **A request with a retired key** is a fresh draw: it needs fresh operation
  evidence (the seal-stable rule of 0.16.0), records a new draw record under
  a new ID, and returns the same value whenever its bound and purpose are the
  retired draw's, because the value is a function of the root seed, stream,
  address, bound, and purpose alone. A request with other evidence, bound, or
  purpose is no longer refused as `RandomOperationConflict`. The new draw
  retires in turn at its own time plus `after`.
- **Choosing `after`.** At least the longest time anything retries the
  address or cites the draw's ID (for example, the retention of the record
  that cites it). The engine cannot see which records cite a draw, so the
  row's alternative, a horizon tied to a record kind's retention, would need
  the plugin to declare the same duration anyway.

Rejected: tombstones for retired addresses (the ledger would keep an entry
per address again); a digest filter of retired addresses (a false positive
would refuse a legitimate first draw); a retirement chosen per draw call
(not part of the run's identity, and a retry could pass a different one).

### Persistence and replay

- Nothing new is persisted. `KeyedDrawEntry` keeps its shape: a restore
  derives an entry's retirement time from its draw time and the descriptor.
- **Journal restores** (snapshot, checkpoint journal, paged checkpoint,
  sealed segments) build the ledger once, draw by draw, dropping what retires
  at or before each draw's time before adding it, as the boundary that made it
  did, and finally what retired by the run's `now`. A journal may therefore
  draw one address again after it retired; drawing it again before is still
  refused.
- **Continuation checkpoint.** `keyed_draws` lists the draws not retired at
  the checkpoint's `now`, the same whether the ledger was kept by a live run
  or built by a restore (a live ledger can hold a draw whose retirement time
  passed after the last boundary when time advanced without one; it is never
  answered and is not exported). A restore takes each entry's draw time from
  the checkpoint's time index (`evidence_times.random_draws`) and refuses, as
  `InvalidSnapshot`, an entry that retired at or before `now`.
- **Exact replay** settles the same boundaries at the same times and drops
  the same draws.

### Refusals

- Registration: `InvalidPluginRegistration` for a retirement naming an
  undeclared stream, the same stream twice, or a duration that is not
  positive; the same descriptor shapes are `InvalidSnapshot` at load.
- Load: `InvalidSnapshot` for a continuation checkpoint that lists a retired
  draw; `RandomOperationConflict` (reported as an invalid snapshot) for a
  journal that draws an address again before it retired.

### What can still fail a boundary

The drop itself cannot fail. A fresh draw after retirement can be refused as
any fresh draw can: `InvalidRandomOperationEvidence` when it cites evidence a
fresh draw may not cite (a record version that is no longer current, for
example the one the retired draw cited), and `IdentifierExhausted` when no
draw ID is left. A system that propagates the error fails its boundary; one
that handles it records its own outcome, and every run decides alike.

### Known limits

- A conflict across a retirement is not detected: after `after`, reusing an
  address with other evidence, bound, or purpose draws instead of failing.
- Retirement is by simulated time only, not by whether a record still cites
  the draw.
- Each retiring draw adds one small entry to the retirement map while it is
  live; memory falls only for streams that declare a retirement.

## Public API delta

- `canwu-sim` and `canwu-api` (re-exported): `CalendarMark` (with `new`),
  `MAX_CALENDAR_KEY_BYTES`, `KeyedDrawRetirement` (with `new`);
  `schedule_calendar_marks` on `Simulation`, `CompactedSimulation`, `Canwu`,
  and `CompactedCanwu`; `SimulationView::calendar_marks`.
- Changed types: `BoundarySystemContract` gains `calendar` and
  `keyed_draw_retirement` (set by `BoundarySystemContract::new` to `None`
  and empty); `BoundaryRecord` and `BoundarySummary` gain `calendar_marks`;
  `IngressPayload::Calendar` gains `calendar_marks`. Code that builds these
  with a struct literal or matches `IngressPayload::Calendar { cadences }`
  adds the field or `..`.

## Formats

Snapshot 9, commitment 6, checkpoint/evidence-journal 6, paged checkpoint 7,
continuation checkpoint 3, and plugin descriptor 1 keep their numbers, as
optional fields did in earlier releases (`boundary_schedule_horizon` in
0.14.0, the transition and person fields of the boundary record): every new
field is omitted while empty, so a run that uses neither feature writes the
same shapes and hashes as before. What changes:

| Item | What changed |
| --- | --- |
| `IngressPayload::Calendar` (snapshot, journals, replay journal, pinned continuation ingress) | `calendar_marks: Vec<CalendarMark>`, omitted while empty; canonical form checked at load |
| `BoundaryRecord` and its hash material `canwu.boundary-record.v1` | `calendar_marks`, omitted while empty; must equal the admitted calendar ingress's marks |
| `BoundarySummary` (boundary ledger, continuation `settled_boundaries`) | `calendar_marks`, omitted while empty |
| Plugin descriptor `BoundarySystemContract` | `calendar` (omitted while `None`) and `keyed_draw_retirement` (omitted while empty, sorted by stream); both part of the descriptor and so of the run's identity |
| `ContinuationCheckpoint::keyed_draws` | shape unchanged; lists only the draws not retired at `now`, and a restore refuses a retired one |

## Durable tests

- `a_keyed_system_wakes_only_on_its_own_calendar_marks`
  (`canwu-api/tests/gap_g69_sim_calendar_marks.rs`): two calendars, keyed
  `Monthly` and `Seasonal` systems and an unkeyed `Monthly` system; marked,
  merged, and unmarked boundaries over four days; the keyed systems emit,
  write a record, and publish knowledge. Checks who wakes, the marks on the
  view, the queued boundary ledger, the boundary record, a snapshot restore
  and a continuation restore with marked ingress still queued (same receipts
  and hash), a snapshot restore after the last boundary, the refusal of a
  snapshot whose boundary lost a mark or gained one, exact replay, and the
  host refusals.
- `a_keyed_draw_retires_alike_in_every_run`
  (`canwu-api/tests/gap_g71_sim_keyed_draw_retirement.rs`): a stream retiring
  after two days beside a stream without retirement, in an unsealed run, a
  run sealed after every boundary, and a run restored from a continuation
  checkpoint while the draw is live. Checks the exact retry and the conflict
  before retirement, the fresh draw (new ID, same value) after it, the kept
  stream's retry, equal hashes and continuation checkpoints in every run, the
  day-3 checkpoint's entries, the refusal of a checkpoint that lists the
  retired draw, a snapshot restore of a journal that draws one address twice,
  exact replay, and a checkpoint taken after time passed the re-draw's
  retirement without a boundary, which lists only the kept draw, restores,
  and settles as the live run.

Checked inline (not committed) to fail: no drop at boundary start (the
G-71 test fails at the day-2 draw), a journal rebuild that does not drop
before each draw (the snapshot load fails), a keyed system woken by any
boundary of its cadence (the G-69 test fails on day 1), and replay re-queuing
calendar ingress without its marks (replay mismatch). The existing keyed-draw
index tests, which pin structural sharing and one ledger build per restore,
pass unchanged.

## Documentation delta

For the release documentation pass (canonical page and both website
languages each):

- `docs/architecture.md`: updated in this change (the boundary ledger's
  summary, the calendar ingress paragraph, the keyed-draw ledger paragraph).
  Mirror the same sentences in
  `website/src/content/docs/{,en/}architecture/settlement.mdx` (calendar
  marks, boundary ledger) and `.../architecture/randomness.mdx` (keyed-draw
  retirement).
- `website/src/content/docs/{,en/}developer/persistence.mdx`, "What a
  continued run can do" or the keyed-draw paragraph: "A continuation
  checkpoint carries the keyed draws not retired at its time."
- `docs/terminology.md` and both website term pages: the `keyed-draw ledger`
  row (操作定址抽样明细) replaces "it keeps one entry per address for the life
  of the run" with "it keeps one entry per address until the draw retires,
  for the life of the run unless its stream declares a keyed-draw
  retirement"; the `boundary ledger` row (边界账) adds "calendar marks" to
  what each boundary lists; the candidates below.
- `docs/versioning.md`, the release entry: the paragraph below.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| calendar mark | `CalendarMark`, `schedule_calendar_marks`, `SimulationView::calendar_marks` | 历法标记 | A host calendar's mark on a calendar boundary: the boundary is a period start (of a cadence) of that calendar. A system with a calendar key wakes only on its own calendar's marks of its cadence. |
| calendar key | `BoundarySystemContract::calendar`, `MAX_CALENDAR_KEY_BYTES` | 历法键 | The host's name for one of its calendars, on a mark and on a boundary system's registration. |
| keyed-draw retirement | `KeyedDrawRetirement`, `BoundarySystemContract::keyed_draw_retirement` | 定址抽样退役期 | How long after its boundary an operation-keyed draw of a stream stays in the keyed-draw ledger; part of the plugin descriptor. Not 已退役请求结果 (a command request) or 已清退条目 (a knowledge entry). |
| retired keyed draw | — | 已退役定址抽样 | A keyed draw past its retirement: never answered again; its address draws afresh and its draw record stays as evidence. |

## Release notes text (for `docs/versioning.md`)

- **Calendar marks.** A host that runs more than one calendar marks a
  calendar boundary with the calendars whose period starts there:
  `schedule_calendar_marks(due_at, marks)` with `CalendarMark { cadence,
  calendar }`. A boundary system registered with a calendar key
  (`BoundarySystemContract::calendar`) is due only in a boundary that
  carries its own mark of its cadence; a system without a key is due in every
  boundary of its cadence, as before. `SimulationView::calendar_marks`
  reads the marks of the boundary being settled, and `BoundaryRecord` and
  `BoundarySummary` carry them. The core still keeps no calendar.
- **Keyed-draw retirement.** A boundary system may declare, for a stream it
  owns, how long its operation-keyed draws stay in the keyed-draw ledger
  (`BoundarySystemContract::keyed_draw_retirement`,
  `KeyedDrawRetirement { stream, after }`). The first boundary at or after a
  draw's boundary time plus `after` drops it before any system runs; a later
  request with the same address draws afresh under a new draw ID (same value
  for the same bound and purpose) and is no longer checked against the
  retired draw. Draw records stay as evidence. Every restore and exact
  replay retire alike, and a continuation checkpoint carries only the draws
  not retired at its time.

**Upgrading from 0.19.0.** Nothing changes for a host or plugin that uses
neither feature: an unkeyed system wakes as before, and a stream without a
retirement keeps every keyed draw. Code that builds `BoundaryRecord` or
`BoundarySummary` with a struct literal, or matches
`IngressPayload::Calendar { cadences }`, adds `calendar_marks` (or `..`);
`BoundarySystemContract::new` fills the two new contract fields. To wake a
system only on one calendar's periods, set `contract.calendar =
Some("<calendar>".into())` and schedule that calendar's period starts with
`schedule_calendar_marks`; mark every calendar a keyed system should see,
because an unmarked `schedule_calendar_boundary` wakes only unkeyed systems.
To bound the ledger, add `KeyedDrawRetirement::new(stream, after)` with
`after` no shorter than the time anything retries the address or cites the
draw, and expect a retry after that to draw afresh. Declaring either
changes the plugin descriptor, so a run is restored and replayed only with
the same declaration.
