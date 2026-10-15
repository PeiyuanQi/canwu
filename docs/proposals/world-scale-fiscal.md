# World-Scale Fiscal State

Status: design for 0.16.0, reviewed ("implement with changes, with phasing")
and amended. Phase 0, the engine probe, is measured
([Phase 0 results](#phase-0-results)), and Phase 1 is implemented on the single
state record ([Phase 1 as implemented](#phase-1-as-implemented)); Phase 2 is
not. The owner
decided on 2026-10-15 that Phases 0, 1, and 2 ship in 0.16.0 and that the
release waits for them, so that 0.16.0 can run the county-level world; Phase 3
stays later ([Phases](#phases)). It answers downstream gaps G-15
(fiscal lifetime caps and no archive) and G-16 (fiscal observers) in
`canwu-fiscal`, the fiscal simulation extension, after the owner raised G-15
to High on 2026-10-14 with the target restated below.

The independent review confirmed in the engine's code the four engine limits
this design rests on (the paged-checkpoint record ceiling, the reverse-reference
closure, the knowledge publication limits, and the sealing rule), found no
boundary read that depends on what a seal removes and no leak between
observers, and asked for the changes recorded here: the office watermark, the
two holes in at-most-once settlement, the cost model of a record write, the
missing cross-record invariants, and the phasing of
[Phases](#phases).

It builds on two contracts: the 0.15.0 design
[fiscal throughput and authority](fiscal-throughput-and-authority.md), and the
0.16.0 package that another worker is implementing at the same time,
`docs/proposals/fiscal-and-society-limits.md` on branch
`gap/fiscal-society-limits`. That package makes receipts and historical-context
changes past the byte budget a recorded refusal, drains deferred report parts
on their own, makes the fiscal action packet internal, and adds a
scenario-declared auditor appointment policy with nominations and removal
petitions. This design assumes all of that and says where each piece moves.
The archive reuses the machinery of
[resource archive lookup and volume](resource-archive-lookup-and-volume.md),
and the lookup-index functions that the 0.16.0 production archive package
(`docs/proposals/production-archive-volume.md` on branch
`gap/production-archive`) makes public in `canwu-resource`.

## Contents

1. [The target](#the-target)
2. [Why one record cannot reach it](#why-one-record-cannot-reach-it)
3. [The design on one page](#the-design-on-one-page)
4. [Phases](#phases)
   - [Phase 1 as implemented](#phase-1-as-implemented)
5. [State layout](#state-layout)
6. [Revisions, staleness, and idempotency](#revisions-staleness-and-idempotency)
7. [Throughput](#throughput)
8. [Accounting cycles: open, close, archive](#accounting-cycles-open-close-archive)
9. [The fiscal archive (G-15)](#the-fiscal-archive-g-15)
10. [Observers and reports (G-16)](#observers-and-reports-g-16)
11. [Receipts and historical context at scale](#receipts-and-historical-context-at-scale)
12. [Every fiscal cap](#every-fiscal-cap)
13. [Cost model](#cost-model)
14. [Authority](#authority)
15. [Persistence, replay, seal independence, and formats](#persistence-replay-seal-independence-and-formats)
16. [Public API](#public-api)
17. [Measurement plan](#measurement-plan)
18. [Acceptance tests](#acceptance-tests)
19. [Beyond fiscal: caps that block the target](#beyond-fiscal-caps-that-block-the-target)
20. [What can still fail a boundary](#what-can-still-fail-a-boundary)
21. [Risks, open decisions, and questions for review](#risks-open-decisions-and-questions-for-review)
22. [Documentation delta](#documentation-delta)

## The target

The owner's statement: a live simulation of every country in the world at
county level, at least. Read as a workload that a harness can generate and
measure:

| Dimension | Value used | Source |
| --- | ---: | --- |
| Polities, each with its own fiscal institutions | 200 | about 200 sovereign states (the United Nations has 193 members and 2 observer states) |
| Fiscal scopes (second-level divisions) | 50,000 | owner's order of magnitude; for scale, China has about 2,800 county-level divisions and the United States about 3,100 counties (owner's brief) |
| Largest polity, scopes | 6,000 | the largest second-level sets are a few thousand units (Brazil's municipalities, about 5,570, are among the largest); the generator uses a heavy tail with this maximum |
| Design ceiling, scopes | 65,536 | headroom of about 30 percent over the target, to be measured, not assumed |
| Fiscal institutions (treasuries, granaries, audit offices, regional offices) | 4,200 | generator: 3 central institutions per polity and one regional office per first-level division, 18 per polity on average; a variant with one institution per scope (50,000) |
| Mechanisms per scope | 1, and 3 in a variant | one scope binding is one mechanism, so the variant has 150,000 scope bindings |
| Observers (each polity's court and its regional officials) | 4,000 | generator: one court per polity, seeing all its institutions, and one official per regional office, seeing that office |
| Accounting cycle | annual and monthly | owner |
| Assessments | one per scope per cycle | owner |
| Boundaries | daily, 36,525 over 100 years | owner |
| Lifetime volume, 100 years | 5 million scope books (annual), 60 million (monthly) | 50,000 scopes times 100 or 1,200 cycles |
| Lifetime caps reached | none | owner |

Arrival profiles the harness generates, because both happen in a world
simulation: a burst, where every scope of every polity opens its cycle on the
same day (50,000 assessments in one boundary), and a spread, where polities
open their cycles on different days of the cycle's first month.

**How cost is judged.** The owner's rule is player lag: a boundary's cost is
acceptable when a human finds the lag acceptable at each game speed, compared
blind with other grand strategy games. Milliseconds are diagnostics. The
implementation reports per-boundary time and allocations on a release build at
the target scale for every boundary kind below; it does not claim a speed
grade. For regression detection only, it uses these diagnostic lines: an idle
boundary adds no measurable fiscal time (no fiscal record is decoded); a
routine day of the annual-cycle world adds tens of milliseconds; a
cycle-opening burst of 50,000 assessments is a once-per-cycle hitch of a few
seconds at most.

## Why one record cannot reach it

`canwu-fiscal` 0.15.0 keeps its whole dynamic state in one domain record,
`canwu.fiscal:state/primary`. Every boundary that changes it decodes, copies,
validates, re-encodes, and journals the whole record, twice (settlement and
the aggregate system). The recorded profile
(`benchmarks/baselines/2026-10-13-fiscal-boundary-0.15.0-round*-elapsed.json`)
has a state of 48 institutions and 1,920 settled assessments, 2,155,406 bytes:
a boundary that settles 40 actions takes about 600 ms, admission costs about
4.4 ms per command because each decodes the state, and each applied action
copies and validates the whole state (about 3 ms).

The world target cannot be reached by raising the caps:

- One open annual cycle of 50,000 scopes, with one assessment, one execution
  request, one receipt, and one aggregate each, is about 55 MB of state at the
  profile's 1.1 KB per assessment chain. Every fiscal boundary would decode,
  encode, and journal that, twice.
- The global procedure revision makes every action stale that was decided
  before any other fiscal change anywhere in the world. With 200 polities
  acting daily, almost every action would be stale.
- The engine has limits that a large single record crosses, found while
  preparing this design:
  - **Paged checkpoints.** A Format 8 state page is at most 4 MiB
    (`MAX_STATE_PAGE_BYTES`). A domain record is stored in one Patricia leaf
    page whose canonical bytes carry the record as `value_bytes: Vec<u8>`,
    which `serde_json` writes as an array of decimal numbers, about 4 bytes
    per byte. Phase 0 confirmed it: a record whose payload is larger than
    1,151,023 bytes cannot be written into a paged checkpoint
    ([Phase 0 results](#phase-0-results)). The 0.15.0 fiscal profile state
    (2.15 MB) is already past it.
  - **Reverse-reference closure.** A record mutation re-validates every record
    that references it, transitively, up to 16,384
    (`MAX_AFFECTED_RECORD_VALIDATION_CLOSURE`). A shard design in which many
    shards reference one mutable record fails boundaries once the shards
    outnumber that.
  - **Knowledge publication.** One boundary system may make at most 64
    publications (one holder each), 1,000 records each, and a boundary holds at
    most 10,000 knowledge records over all plugins (`KnowledgeLimitsV1`).
  - **Sealing.** `seal_evidence` refuses while any canonical ingress is queued,
    including ingress due in the future, so a plugin cannot keep a timer as a
    future-dated packet without blocking seals.

## The design on one page

This is the end state; [Phases](#phases) says which part arrives when.

- **State.** The single state record is replaced by small records, each well
  under 256 KiB: one **root** (global context, archive head, report schedule),
  one **office** per institution (authority, audit, appointment, observer
  index, and cycle bookkeeping), one **scope ledger** per scope binding
  (adoptions and the books of its hot accounting cycles), one **jurisdiction**
  record per catalog region in use (adoption counts and transition
  candidates), one **observer** record per observer binding (report layout and
  published digests), plus create-only index pages, binding-directory buckets,
  and report-queue slots. No fiscal record references another fiscal record,
  except the root's reference to the create-only catalog. A boundary decodes
  and writes only the records its packets touch.
- **Invariants.** The records are the partition of one logical `FiscalState`,
  whose `validate` defines validity. Each write validates the records it
  writes and checks what it can of its neighbours; the cross-record
  invariants are maintained by construction, checked in full when a run is
  created or restored, and by `audit_fiscal_records` after every boundary in
  tests and profiles.
- **Hot and archived.** Hot: everything an open or closed-but-unarchived
  accounting cycle needs, bindings, adoptions, candidates, and recent
  outcomes. Archived: closed cycle books of a scope (with their scope-level
  per-cycle totals), an institution's per-cycle totals once all of its books
  for that cycle are archived, and resolved auditor proposals. Kept in
  evidence only: action outcomes older than a record's ring of recent
  outcomes, each carried by its settled event.
- **Revisions.** Each record has its own revision. An action names the
  revision of the record it writes, so it is stale only when that scope,
  office, jurisdiction, or observer changed since its issuer looked.
- **Idempotency without lifetime memory.** An action applies at most once: its
  expected revision can match only in one boundary, a batch may not name an
  action ID twice, and settlement skips a copy whose ID already has an outcome
  in that boundary. A re-sent action is refused at admission from hot state
  alone: its outcome is still in the target's ring of 8
  (`IdempotencyConflict`), or its expected revision is at or below the target's
  outcome floor (`FISCAL_REVISION_EXPIRED`).
- **Cycles.** Accounting cycles close per scope by an authorized action and
  then archive. A new book must be above two watermarks, its scope's newest
  archived cycle and its office's newest archived institution totals, so
  assessment partitions stay unique and institution totals are archived once
  for the life of a run, without reading the archive.
- **Archive.** Prepare, store and verify, enqueue, finalize retention, as in
  `canwu-resource`, with the cumulative lookup index in the resource page
  format through the public functions of `canwu-resource`, wrapped behind a
  fiscal-owned store trait and error. Commits settle first, carry up to 4,096
  books in up to 64 blobs, terminate when stale, and are acknowledged
  idempotently. **No boundary reads the archive store**, so a missing archive
  object can never fail a boundary, and a restore does not have to audit the
  archive.
- **Throughput.** One command carries a batch of up to 4,096 ordinary actions
  under one binding; each keeps its own action ID, expected revision, outcome,
  and event. A boundary applies up to 65,536 actions (each now costs its own
  scope ledger, not the state) and at most 64 on one record. Receipts come in
  batches of up to 4,096. Every excess is a recorded rejection or refusal.
- **Observers.** `actor` becomes optional so a government can hold a report;
  thousands of observers each have their own record; report lines are named
  on the observer's own record, so re-pointing or removing a binding moves no
  other binding's lines; changed parts are found by digest and published
  within the engine's 64 holders per boundary, with same-instant drain
  boundaries beyond that. Report cadence, per-institution reports, and
  domain-entity holders are Phase 3.
- **Caps.** Every lifetime cap is removed; what remains bounds one record, one
  command, or one boundary, and each excess is refused, never a failed
  boundary.

## Phases

The end state above is reached in four phases. The owner decided on
2026-10-15 that **Phases 0, 1, and 2 ship in 0.16.0** and that the release
waits for them, so that 0.16.0 can run the county-level world; **Phase 3 stays
later**. Phase 0 measured record writes, not knowledge publication, so it does
not show Phase 3 to be cheap. Implementation starts after this design's report,
on top of the integration branch once the 0.16.0 fiscal and society package is
integrated into it.

**Phase 0: engine probes (no fiscal code; done).** What a record write costs
the engine, as a function of the record's size and shape, in a store of
65,536 records; what an idle boundary costs with that store; and whether a
record of 1.2 MB fits a paged checkpoint (it does not). The binary is
`record-write-profile` in `benchmarks/performance-harness`; the results are
in [Phase 0 results](#phase-0-results).

**Phase 1: G-15 and G-16 semantics (0.16.0, first milestone).** Built on top of
the fiscal and society package. Phase 1 changes what the fiscal state means and
keeps where it lives: the state stays one record, `canwu.fiscal:state`, under
its 32 MiB budget and the package's measured settlement, but its contents take
the logical shape the partitioned records will hold, so Phase 2 moves data
between records instead of redesigning it:

- per-scope ledgers inside the state, each with one book per hot accounting
  cycle and the scope-level per-cycle totals in the book, and per-institution
  cycle bookkeeping (hot-book counts, partition keys, archived totals,
  watermark);
- `CloseAccountingCycle`, and the scope and office watermarks of
  [Accounting cycles](#accounting-cycles-open-close-archive);
- the fiscal archive with the lookup index behind a fiscal-owned store trait
  and error ([The fiscal archive](#the-fiscal-archive-g-15)), archiving closed
  books, institution per-cycle totals, and resolved auditor proposals;
- idempotency from a ring of recent outcomes per target and an outcome floor,
  with the **global procedure revision kept** as the staleness basis (the
  floor rule holds for any monotonic revision);
- every receipt and historical-context refusal as a recorded event;
- the optional observer actor with person and government holders, and
  `RebindObserver` and `RemoveObserver`, with each binding's report lines named
  from a **slot allocated once per binding and never reused**, still on the
  state record;
- per-cycle totals kept in the archive.

What Phase 1 proves, each with a durable test or a recorded run: no 0.15.0
lifetime cap is reached (assessments, outcomes, requests, receipts, proposals);
the archive protocol (multi-page commits, commits settle first, stale commits
end as no-ops, duplicate queueing, idempotent acknowledgement, the cumulative
index); no boundary reads the archive store (a store that fails every read
leaves every boundary's result unchanged); the reference integration
`canwu-ming-fiscal-reference` runs 100 simulated years with an archive cadence
while its state stays inside 32 MiB; and report line places survive rebinding
and removing other bindings.

Phase 1 alone does not reach the world target: the state is still decoded,
copied, and journaled whole by every boundary that changes it, the global
revision still makes concurrent actions stale, and a state above the
paged-checkpoint ceiling ([Phase 0 results](#phase-0-results)) cannot be
written into a paged checkpoint. It is a milestone of its own because it is
complete, tested, and reviewable before the storage changes under it; whether
it lands as its own commit is the orchestrator's choice.

**Phase 2: partition and throughput (0.16.0, second milestone).** What lets
0.16.0 run the county-level world:

- the records of [State layout](#state-layout) (root, offices, scope ledgers,
  jurisdictions, observers, directory buckets, index pages), with per-record
  validation, the cross-record invariants, and `audit_fiscal_records`;
- per-record revisions, replacing the global procedure revision as the
  staleness basis;
- the batch command, the raised and per-record limits, and receipt batches;
- for observers, what the partition needs and no more: each binding in its own
  observer record, its report lines named on that record, the static part
  layout of scope rows and cycle slots with a digest per line, the office
  observer index, and publication on change within the engine's per-boundary
  limits, with what does not fit owed to the next boundary at the same instant
  through report-queue slots and the drain packet.

What Phase 2 proves, besides moving every Phase 1 test to the records: the
partition test of [Acceptance tests](#acceptance-tests) (records always equal
`into_records` of the assembled state, after every boundary); per-record
staleness and at-most-once settlement across batches; and the `world` and
`world-monthly` presets of the measurement plan, at the target scale, with
their per-boundary times recorded under `benchmarks/baselines/`.

Phase 0 sets what a day of that world costs ([Phase 0 results](#phase-0-results)):
a ledger write costs the engine about 500 to 700 µs (about 200 µs fixed plus
about 140 µs per KiB of structured JSON), and every boundary pays an idle
floor of 35 to 110 ms for 65,536 stored records. Phase 2 therefore keeps
ledgers lean in JSON values, not only in bytes (which encoding of a book
object costs least is measured with the first ledger code, by the same
probe), and it ships as the owner decided; but at these engine costs a
monthly day of the county-level world takes seconds and a cycle-opening
burst about 25 s. Reaching the player-lag diagnostics needs the two engine
items listed under [Beyond fiscal](#beyond-fiscal-caps-that-block-the-target)
(the write path and the idle floor), which are the owner's to schedule; Phase
2 measures and reports the remaining gap.

**Engine numbers Phase 2 is sized against.** The record-store package
(`docs/proposals/record-store-ceiling-and-cost.md`, in review, integrated before
Phase 2) measured, on 65,536 stored records: an idle boundary of 0.08 to
0.10 ms (from 51 to 67 ms), independent of the number of stored records; one
2.45 KiB structured write 24.3 µs (from 526 µs), and 48 to 66 µs a write at
50,000 writes in one boundary; and a paged-checkpoint record ceiling of
512 MiB in 1 MiB chunks (from 1.15 MB). Phase 2 sizes its record budgets,
per-boundary limits, and the cost of a day of the county-level world against
these numbers, not Phase 0's, and states the ones it used.

**Phase 3: observers at scale (later).** Report cadence (`Interval`) with
future due times, institution granularity, thousands of observers publishing
in one instant, rotation of many parked holders, and domain-entity holders.
Its gates are the engine's knowledge publication limit (64 holders per system
per boundary) and a knowledge retention design (held reports grow for the life
of a run). Until then a 0.16.0 world can keep many observers, but each
publishes whole scope-level rows on every change of its institutions' books;
a world with thousands of observers whose institutions change daily drains
for many boundaries at each instant. Institution granularity is the first item
to pull forward if a 0.16.0 world needs court-level reports.

### Phase 1 as implemented

Phase 1 is implemented on the 0.16.0 integration branch (the fiscal and society
package and the knowledge package included), with the semantics above on the
one state record `canwu.fiscal:state`. What it holds and how it differs from
the design follows; the rest of this proposal still describes Phase 2.

**State.** `FiscalState` keeps one `FiscalScopeLedger` per scope binding
(`ledgers`, keyed by scope ID: the binding, its adoptions, its hot
`FiscalCycleBook`s keyed by accounting cycle, its watermark, and its outcome
ring) and one `FiscalOffice` per institution with hot books, an archived cycle, or an outcome
(`offices`, keyed by `fiscal_office_key`: its hot-book count, archived book
count, and archived totals per hot cycle, its watermark, and its ring).
`FISCAL_RUNTIME_SCHEMA_VERSION` is 4. The flat maps of assessments,
remissions, requests, receipts, audits, adoptions, scope bindings, and action
outcomes are gone; read them through `FiscalState::assessments()` and the
other iterators, `ledger`, `scope_binding(s)`, and `books`. Book objects carry
`source_action_id`.

**Actions.** `FiscalActionRequest` keeps `expected_procedure_revision`: the
global revision stays the staleness basis. `GrantRemission`,
`AuthorizeExecution`, and `RecordAudit` gain `scope_binding_id`; the new
variants are `CloseAccountingCycle`, `RebindObserver`, and `RemoveObserver`.
Each action has a `FiscalActionTarget` (ledger, office, observer); action IDs
are per target. Admission refuses an ID in its target's ring
(`IdempotencyConflict`), a revision above the state's
(`FISCAL_REVISION_UNKNOWN`), and one at or below the target's floor
(`FISCAL_REVISION_EXPIRED`), and settlement skips a copy whose outcome is in
the ring or already settled in the boundary. A host that issued actions
against a revision it predicted, not one it read, now has them refused at
admission rather than rejected as stale at settlement. `ApplyTransition`,
the rebinds, and the appointment actions target the office of the institution
they change. `FiscalSettledAction` reads an outcome back from its
`canwu.fiscal.action_settled.v1` event; `FiscalActionOutcome` and
`MAX_FISCAL_ACTION_OUTCOMES` are removed.

**Archive.** As [The fiscal archive](#the-fiscal-archive-g-15) describes, with
these choices. A commit binds the state's `archive_source_root`, a digest of
the procedure revision and the archive head, so any settled action, receipt,
or context change between preparation and settlement makes it stale; commits
settle first in their boundary, so a commit prepared and queued in one gap is
current. Settlement rebuilds the blobs, pages, and directory from the books
the commit names and compares them, and never advances the procedure
revision. A malformed or non-matching commit is refused with
`canwu.fiscal.ingress_rejected.v1` instead of failing the boundary. A copy of
a commit whose handle the head already holds (the same batch queued again)
settles as a stale duplicate and records nothing; the head keeps at most 64
unacknowledged stale handles beside its 64 applied ones, and a stale commit
past them is refused with the event and changes nothing. A boundary whose
only change is archive maintenance marks the last report plan carried out,
so it publishes no report. The sealed cut is the host's to pass:
preparation cannot read a run's last cut, so a sealing host that passes
`None` skips the refusal of unsealed citations. The
state's byte budget for settlement leaves `FISCAL_ARCHIVE_HEAD_RESERVE_BYTES`
(2 MiB) for the archive head and is measured again after maintenance, so
maintenance is never refused for the budget.

**Observers.** As [Observers and reports](#observers-and-reports-g-16)
describes for the single record: an optional actor, a person or government
holder, `controlled_by`, and `line_slot`, a slot below `MAX_FISCAL_OBSERVERS`
(64) allocated with the binding, unique, and kept by a removed binding. A
line is `slot * 1,024 + index` in four hexadecimal places (65,536 lines), the
0.15.0 line scheme with the binding's slot in place of its position. The
`fiscal_report` knowledge schema is version 3. A withdrawal publishes one
summary marked `withdrawn` with no parts, and assembly ignores parts at or
past a summary's part count, so the withdrawn holder's old parts never
reassemble. Withdrawals count against the boundary's publications
(`KnowledgeLimitsV1`), ahead of reports.

**Receipts and context.** Every refusal is an event with the reasons of
[Receipts and historical context at scale](#receipts-and-historical-context-at-scale)
(one packet per receipt; batches are Phase 2). A receipt the state has no
room for, because it holds 8,192 hot receipts, is refused with
`state_budget_exhausted`, as one past the byte budget is.

**Deviations from the design, for review.**

- Per-cycle totals are not stored in the book: `FiscalCycleBook::totals`
  derives them, the strategic aggregates (which carry the cycle) show them
  while the book is hot, and the archive keeps them with the book and, per
  institution, in the totals record.
- Resolved auditor proposals are not archived. The fiscal and society
  package already bounds them to the newest
  `MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY` (64) per policy, and each
  proposal's actions keep their settled events, so nothing lifetime remains.
- The per-(institution, mechanism, cycle) partition-key bound
  (`FISCAL_PARTITION_KEYS_EXHAUSTED`) is deferred to Phase 2, with the
  institution rows it bounds.
- Phase 1 keeps hot caps on the one record: 4,096 assessments, remissions,
  and audit findings, 8,192 adoptions, execution requests, and receipts, and
  64 books per ledger. None is a lifetime cap.
- `fiscal_reports_of_holder` reads a holder's reports as a trusted host, for
  holders a viewer cannot be (a government).
- Evidence must not predate its request by simulated time, as in 0.15.0. A
  request ID used again in a later book of a scope, at the same simulated
  time as the evidence of an archived request of that ID, could take that
  evidence again; hosts use each request ID once per scope for the life of a
  run. Comparing boundary order instead would need each request to record its
  boundary.

**Proofs.** The tests of [Acceptance tests](#acceptance-tests), Phase 1, with
two changes in how they run:

- The lifetime test, `fiscal_archive_lifetime.rs`, runs its full size only
  in a release build (`cargo test --release -p canwu-fiscal --test
  fiscal_archive_lifetime -- --ignored`). Its pipeline of 274 one-cycle rounds
  fills every boundary's 64 actions: 17,561 settled actions (4,125
  assessments, 8,220 execution requests, 4,117 nominations, 825 closes, 274
  dismissals), 8,220 receipts, 825 archived books, and 275 cycles of
  institution totals, with nothing rejected, refused, or dropped and the state
  at most about 168 KB. It takes 46 s in a release build. In a debug build
  every boundary rewrites the whole state record at about 0.3 s a boundary
  (the engine's write path, as Phase 0 measured it, at about 1.8 ms per KiB
  in a debug build), which is about six minutes for its 550 boundaries, so the
  debug suite runs the same pipeline for 8 rounds and holds that the state
  stops growing once the pipeline is full (7 s). Phase 2, and the
  record-store package's engine changes, make the full size affordable in the
  debug suite again.
- The reference run is a test of `canwu-ming-fiscal-reference`,
  `archive_century.rs` (24 s in a debug build): one sample cycle a year,
  closed every year, archived every ten. Restore and replay of archiving runs
  are held by the fiscal crate's tests.

| Year | Fiscal state (bytes) | Snapshot (bytes) | Archive store (bytes) |
| ---: | ---: | ---: | ---: |
| 10 | 4,141 | 6,011,192 | 40,686 |
| 25 | 15,962 | 13,673,147 | 84,837 |
| 50 | 4,153 | 29,082,317 | 233,492 |
| 100 | 4,157 | 57,931,997 | 508,186 |

The state's largest size over the century was 27,771 bytes (just before a
decade's archive), against the 32 MiB budget. The snapshot grows by about
580 KB a year because an unsealed run keeps every boundary's evidence, most of
it the boundaries' record changes and the knowledge published to the
observer; sealing bounds that, and it is not a fiscal growth.

**Per boundary.** `fiscal-profile` in allocation mode (allocation counts do
not depend on the machine's load), in its default shape (48 institutions with
40 settled cycles each, four observers), against the integration branch just
before Phase 1 (three samples each, all equal):

| Measure | Before | Phase 1 |
| --- | ---: | ---: |
| Fiscal state (bytes) | 2,155,406 | 1,947,029 |
| Admitting boundary of 48 actions (allocations) | 1,714,681 | 1,635,402 |
| Settling boundary of 48 actions | 6,476,053 | 6,282,337 |
| Decode / clone / encode the state | 42,436 / 42,435 / 191,824 | 40,440 / 40,439 / 180,896 |
| Validate the state | 6,412 | 9,418 |
| Settlement's measure | 34,124 | 29,646 |
| Idle boundary, settled state / no fiscal records | 488 / 480 | 489 / 481 |

The state is smaller (rings instead of 3,840 kept outcomes) and every
operation on it a little cheaper, except validation, which now also checks
the ledgers, offices, rings, and observer slots.

## State layout

### Records

All kinds are owned by `canwu-fiscal`. Record identities are canonical
identifiers (at most 160 bytes of `[A-Za-z0-9-_.:]`).

| Kind | Identity | Holds | Written when |
| --- | --- | --- | --- |
| `canwu.fiscal:catalog` | `primary` | the compiled catalog, unchanged and create-only | never after creation |
| `canwu.fiscal:root` | `primary` | schema version, catalog version, historical context and its revision, execution-evidence kinds, archive head, maintenance receipts and retention handles, report schedule (queue slots by due time, earliest due time, parked count) | context change, archive commit or acknowledgement, report queue change |
| `canwu.fiscal:office` | office key of the institution | institution, revision, outcome floor and ring, authority bindings, audit bindings, appointment policy and proposals, static facts (mechanisms, scope count, index pages), cycle watermark, observer reverse index (observer ID to cadence), and for each hot cycle its partition keys in use, its count of hot books, and the totals of its books already archived | an action on the office's bindings or proposals; the first book of a cycle, or the first use of a partition key, in one of its scopes (once per boundary); an archive commit that archives books of its scopes |
| `canwu.fiscal:ledger` | scope binding ID | the scope binding (static, with its rank among the office's scopes), revision, outcome floor and ring, adoptions, the books of hot accounting cycles, cycle watermark | an action, receipt, close, or archive commit on the scope |
| `canwu.fiscal:jurisdiction` | catalog region ID | revision, outcome floor and ring, scope count per mechanism, adoption counts per rule (live and operational), transition candidates, number of index pages | an adoption change or transition in the region, a context change that moves its candidates |
| `canwu.fiscal:observer` | observer binding ID | the binding, revision, outcome floor and ring, row offsets, cycle slots, published digest per report line, pending lines, last publication time, removal mark | the observer is planned for publication, re-pointed, or removed |
| `canwu.fiscal:directory` | 3-hex-digit bucket | binding or policy ID to (kind, office key) for the IDs that hash into the bucket | a binding or policy is created or removed |
| `canwu.fiscal:index` | owner and page | static ordered scope IDs of one office or one jurisdiction, 4,096 per page | never after creation |
| `canwu.fiscal:report-queue` | slot number | one due time and the observer IDs queued for it | an observer is queued or published |

The **office key** is `office.` followed by the first 32 hexadecimal digits of
a domain-separated BLAKE3 digest of the institution's canonical `EntityRef`.
Every office record holds its institution, and validation requires the key to
match it, so a collision is an invalid record, never a merged office.

The **binding directory** exists because a request names only a binding ID.
Admission hashes the ID to a bucket and reads one entry of that bucket's
payload by key, without decoding the bucket, to find the office.

The **index pages** are the static lists a few operations walk: an office's
scopes (to roll up an institution report and to find books ready for the
archive) and a jurisdiction's scopes (to apply a transition). Scope bindings
never change in a run, so these pages are written once, with the scenario.

### Why these boundaries

- **A scope ledger per scope binding.** Every per-assessment invariant
  (remission and collection within the assessed quantity, receipts within the
  authorized quantity, one assessment per accounting partition, aggregates as
  a function of the book) lives inside one scope, so it is checked inside one
  record. Most changes touch one scope. A ledger with one hot annual book is
  about 2.5 KiB (the static scope binding about 0.25 KiB, one or two adoptions
  about 0.3 KiB, the ring of 8 outcomes about 0.4 KiB, the book about
  1.5 KiB), and about 4 KiB with a closed book besides an open one, so a
  change rewrites and journals kilobytes, not the state.
  Larger shards (an office, a polity) make a receipt in a large polity rewrite
  that polity's whole cycle; smaller ones (one record per scope and cycle)
  were considered and set aside: the first assessment of each cycle would
  write two records instead of one, and slot reuse would add bookkeeping, for
  a saving that the first Phase 2 measurements can show is needed before it
  is paid for ([open question 1](#risks-open-decisions-and-questions-for-review)).
- **No reference between fiscal records.** The engine re-validates every record
  that references a changed record. If 6,000 ledgers referenced their office,
  each office write would re-validate 6,000 records, and an office of more
  than 16,384 scopes would fail its boundary. Ledgers therefore name their
  office in the payload only; cross-record consistency is the plugin's (below).
  Observer records reference their actor person (or institution holder) as the
  state record did, which the engine checks at creation, restore, and write.
- **No running institution totals.** An institution's totals for a cycle are
  the sum of its scopes' books. Keeping a running sum in the office would write
  the office on every receipt of any of its scopes, which in a monthly world is
  every office every day. The office keeps only what changes rarely: which
  partition keys (mechanism, unit, payment form) a hot cycle uses, how many of
  its scopes hold a hot book of it, and the totals of the books already
  archived. A report at institution granularity sums the hot books from the
  office's index pages, one partial read per ledger, and adds the archived part.
- **Partial reads.** The engine holds a record payload as a parsed JSON value.
  A handler that needs one field (a scope's office, a ledger's revision and
  floor, one directory entry, one catalog rule) looks it up in the value
  without decoding the record. Only the records an action changes are decoded.
  The catalog in particular is never decoded by a boundary or a command: it is
  create-only and validated at activation, and handlers read the entries they
  need by key.

### Hot, archived, and kept in evidence

| Data | Hot | Archived | Evidence only |
| --- | --- | --- | --- |
| Scope binding, adoptions | always | — | — |
| A scope's open cycle book | until closed | — | — |
| A scope's closed cycle book (assessments, remissions, requests, receipts, audit findings, scope-level totals, the close) | until an archive commit takes it | for good, key `(scope, cycle)` | — |
| An institution's per-cycle totals | the archived part in the office, the rest summed on demand, while any of its books for the cycle is hot | for good, key `(institution, cycle)`, written by the commit that archives its last book of the cycle | — |
| Authority, audit bindings, appointment policy, pending proposals | always | — | — |
| Resolved auditor proposals | until an archive commit takes them | for good, key `(institution, proposal)` | — |
| Transition candidates | always (per jurisdiction) | — | — |
| Action outcomes | the newest 8 per target record (ID, disposition, settled revision) | inside the book when the action created or changed a book object (its source action ID) | every outcome, in its `canwu.fiscal.action_settled.v1` event |
| Observer layout and published digests | always | — | — |
| Published reports | — | — | knowledge ledger of each holder (engine) |

"Per-cycle totals kept" (the G-15 filed fix) is read as: the totals of every
cycle are kept for good, at scope and at institution level, as archived
records found by key through the lookup index, and hot while any part of the
cycle is hot. Keeping every cycle's totals hot for 100 years would be 5 to 60
million scope totals, or 0.5 to 5 million institution totals, carried by every
boundary that touches them.

### The logical state and its validation

`FiscalState` stays the authoring and audit model, now as the set of record
payloads:

```rust
pub struct FiscalState {
    pub root: FiscalRoot,
    pub offices: BTreeMap<EntityRef, FiscalOffice>,
    pub ledgers: BTreeMap<String, FiscalScopeLedger>,
    pub jurisdictions: BTreeMap<String, FiscalJurisdiction>,
    pub observers: BTreeMap<String, FiscalObserver>,
}
```

`FiscalState::validate(&catalog)` checks everything the 0.15.0 validation
checked, re-stated per record, plus the cross-record invariants below.
`FiscalState::into_records(&catalog)` validates and partitions it (deriving the
directory, the index pages, ranks, row offsets, counts, and candidates), and
`FiscalState::from_records` assembles it. A scenario author builds a
`FiscalState` and calls `into_records`; derived fields an author supplies are
recomputed or refused, never trusted.

**Per write.** Every record a boundary writes is validated on its own before it
is proposed: a ledger against its static scope binding and the catalog
entries it names, an office against its bindings, and so on. This is
`O(size of the record)`.

**Cross-record invariants**, maintained by construction on every write and
checked in full by activation (a new scenario or a restored snapshot):

1. Every ledger names an office that exists, its rank is its place in that
   office's index pages, and the office's index pages list exactly its scopes,
   in rank order; the office's scope count equals that number.
2. Every jurisdiction record's index pages list exactly the scopes whose binding
   names the region, its scope counts per mechanism are those of these scopes,
   its adoption counts equal the counts over their ledgers, and its candidates
   are what the catalog, the context, and those counts derive.
3. Every directory entry names an office that holds that binding or policy,
   and every binding and policy has one entry.
4. Every observer's `visible_institutions` and `controlled_by` name offices
   that exist; every office's observer index lists exactly the observers whose
   visible set includes the institution, each with the cadence its binding
   declares.
5. Every observer queued in a report-queue slot is a binding that exists and is
   not removed, appears in at most one slot, and the slot's due time agrees with
   its cadence; the root's queue index lists exactly the slots in use, and the
   root's earliest due time is the least due time among them.
6. The parked slot lists exactly the observers whose holder could not receive
   the last publication that was due, and none of them is also queued.
7. No new book is created at or below either watermark of its scope: the
   scope's own and its office's (see
   [Accounting cycles](#accounting-cycles-open-close-archive)); a scope's
   watermark is the greatest cycle of its archived books.
8. Every cycle in an office's hot bookkeeping has a hot-book count above zero,
   equal to the number of its scopes holding a book of that cycle, and its
   partition keys are exactly those those books use.

**Cheap checks on every write.** What one record can check about its
neighbours without reading them is checked when it is written: a ledger that
its office key and rank are well-formed and that a book it gained in this
write is above its own watermark and the office watermark settlement read
(older hot books may lie below either, since books archive in any order); an
office that its counts are positive and its index page count
matches its scope count; an observer that its visible set and `controlled_by`
are within their caps and well-formed; the root that its earliest due time is
the least of its queue index. Settlement also checks each invariant it
maintains on the records it touches (for example, that an office's hot-book
count it decrements stays consistent with the books it removed).

**A full audit on demand.** `audit_fiscal_records(&Canwu)` (and the same over a
set of records) checks every invariant above from the records, in
`O(fiscal records)`. The harness runs it after every boundary of every profile,
and debug builds of the crate run it after every fiscal boundary in tests; a
release build runs it only at activation.

Activation is `O(fiscal records)`: about 150 MB of JSON in the annual world,
on the order of a second, measured as restore time. A unit test drives random
action, receipt, close, and archive sequences and checks, after every
boundary, that the incrementally maintained records equal the ones
`into_records` derives from the assembled state, which is what "by
construction" has to mean in evidence.

The 0.15.0 rule that "a fiscal record is validated where it enters a run" is
kept: activation validates everything; handlers decode committed records
without validating them again; every write is validated.

### What changes for derived fields

The transition-candidate system (phase 10) and the aggregate system (phase 12)
are removed. Settlement (phase 7) writes each record it touches once, with its
derived fields current: a ledger's books carry their totals, an office its
cycle bookkeeping, a jurisdiction record its counts and candidates. The 0.15.0
behavior in which the first event-bearing boundary computes an authored
state's derived fields is removed, because activation now refuses an authored
state whose derived fields differ from what `into_records` derives. The report
system (phase 13) both plans and publishes (the engine lets a phase-13 system
write records and publish knowledge), so no plan has to cross phases inside
the state.

## Revisions, staleness, and idempotency

### Revision per record

Each ledger, office, jurisdiction, and observer record has a `revision`. An
outcome (applied or rejected) and a receipt on a record advance that record's
revision by one. Maintenance does not: an archive commit that removes closed
books from a ledger, and the cycle bookkeeping settlement writes into an
office, leave the revision alone, as 0.15.0 kept derived refreshes out of the
procedure revision, so they never make a decision stale. The root's
`context_revision` advances with a historical-context change.

Every action has exactly one **target record**, whose revision it names as its
`expected_revision`. It may also write derived records: the office's cycle
bookkeeping, directory entries, and report-queue slots, which do not advance a
revision. The ledgers an `ApplyTransition` changes are different: their
adoptions are what an issuer of an action on that scope decided against, so
each such ledger's revision advances, and its changed adoptions join the
boundary's overwrite set, so a later action of the same boundary that would
overwrite one of them is stale, exactly as if it had been changed by an action
targeting that ledger.

| Action | Target record |
| --- | --- |
| `OpenAssessment`, `GrantRemission`, `AuthorizeExecution`, `RecordAudit`, `ChangeAdoption`, `CloseAccountingCycle` | the scope ledger |
| `ApplyTransition` | the jurisdiction record (it also writes each ledger in the region whose adoption it changes) |
| `RebindAuthority`, `RebindAudit`, `NominateAuditor`, `PetitionAuditorRemoval`, `DecideAuditAppointment` | the office of the institution whose binding or proposal changes |
| `RebindObserver`, `RemoveObserver` | the observer record |

An action is stale, and settles as a `Rejected` outcome, unless its
`expected_revision` equals the target record's revision at the start of the
boundary that settles it. The 0.15.0 within-boundary rules carry over per
record: actions admitted against the same revision settle together in
admission order, each validated against the record the previous one left; a
later action is stale when it would overwrite an adoption or binding that an
earlier action of the same boundary changed; a receipt that settles in the
same boundary does not make an action stale.

**A historical-context change no longer makes an action stale.** In 0.15.0
every change advanced one global revision, so a context change, like any
other change, made the actions admitted before it stale. A context change
affects an action only through what settlement re-checks anyway (a rule's
legal window, transition eligibility), and keeping it in the staleness check
would need a second expected revision on every action. This is a deliberate
narrowing, listed for review.

### Idempotency without lifetime memory

0.15.0 kept every outcome for the life of the state and refused a known action
ID at admission. That is a lifetime cap (16,384). The replacement keeps, per
target record, a ring of the newest `FISCAL_RECENT_OUTCOMES` (8) outcomes, each
holding only the action ID, the disposition, and its **settled revision** `s`:
the target's revision when the outcome is pushed, before the outcome advances
it. The reason is not kept in the ring; it is in the settled event. Each target
also keeps its **outcome floor**: the settled revision of the newest outcome it
has evicted, or zero.

**Admission** refuses, without consuming anything:

- a request whose `expected_revision` is above the target's current revision
  (it was never a revision of the target), with `FISCAL_REVISION_UNKNOWN`;
- a request whose `expected_revision` is at or below the floor, with
  `FISCAL_REVISION_EXPIRED`;
- a request whose action ID has an outcome in the target's ring, with
  `IdempotencyConflict`, as in 0.15.0;
- a batch that names one action ID twice for one target, as `InvalidPayload`.

**Settlement** skips, without an outcome or an event, an entry whose action ID
already has an outcome for its target in the ring or among the outcomes the
boundary has settled for that target so far (a set kept for the boundary, not
only the ring, so an eviction inside the boundary cannot let a copy through),
as 0.15.0's settlement skipped an action whose ID had an outcome.

**At most once.** An action applies only when its `expected_revision` equals
the target's revision at the start of the settling boundary. Copies of one
request settled in **different** boundaries cannot both apply: the first that
applies advances the revision, so the later one is stale (or skipped while the
first's outcome is in the ring). Copies settled in the **same** boundary all
match the boundary's start revision; a copy inside one batch is refused at
admission, and copies in different commands are skipped at settlement by the
boundary's set. Two commands with one action ID can both pass admission when
both are admitted before either settles, since admission reads the committed
state; the settlement skip is what stops the second.

**Why a re-send is refused once its outcome is evicted.** Every recorded
outcome has `e <= s`: admission refused `e` above the current revision, and
revisions only grow. Every evicted outcome has `s <= floor`. So a re-sent
request whose outcome was evicted has `e <= s <= floor` and is refused, while a
fresh request carries the current revision `c`, which is above the settled
revision of every outcome already recorded (each advanced the revision past
its `s`), hence `c > floor`.

**Every outcome is still recorded for good**, in its settled event, one event
per settled entry, whose summary names the target record, the action ID, the
disposition, and the reason. An outcome that created or changed a book object
is also in that object (`source_action_id`), and so in the archive.

**Action IDs are per target.** The ring, the floor, and the skip are per target
record, so two targets may use the same action ID; a host that wants global
uniqueness of its IDs provides it.

**What a flood does.** Every outcome, applied or rejected, takes a ring slot and
raises the target's revision. A flood of stale or unauthorized actions on one
target therefore evicts recent outcomes before their issuers read them (their
events still hold them) and makes every decision taken before it stale. It
cannot make an action apply twice or expire a fresh request.

What is lost: an action ID reused for a *different* decision after the earlier
outcome left the ring is not detected as a reuse. It is not a re-send (its
expected revision is current), so it settles on its merits. Hosts must not
reuse action IDs, as before; the plugin detects a reuse only while the
earlier outcome is hot. The alternative, an archive lookup per action, costs
about 30 to 40 microseconds per new ID at tens of millions of archived keys
and makes boundaries read the archive store; it was rejected (see
[The fiscal archive](#the-fiscal-archive-g-15)).

In Phase 1 the revision is the global procedure revision. The rings and floors
are still per target (scope ledger, office, observer), and the argument above
holds unchanged, because it needs only a revision that every outcome advances
and that never decreases.

### Object identity

Objects inside a scope (assessments, remissions, execution requests, receipts,
audit findings, adoptions) have IDs that are unique among the scope's hot
objects. A reference to one names its scope: `GrantRemission`,
`AuthorizeExecution`, and `RecordAudit` gain `scope_binding_id`; the receipt
packet and the execution-evidence envelope gain it too. Uniqueness for the
life of a run, where it matters, comes from structure:

- An **accounting partition** (scope, cycle, unit, payment form) is assessed at
  most once for the life of the run: hot books are checked directly, and a new
  book for a cycle at or below either of its scope's watermarks is refused
  ([below](#accounting-cycles-open-close-archive)).
- An **evidence version** settles at most one receipt: its claim names the scope
  and request it settles, settlement requires those to match, and a request's
  receipts are all hot while its book can still take receipts (open). A closed
  or archived book takes no receipt. If a request ID is used again in a later
  book, evidence established before the new request is refused, as 0.15.0
  already refuses evidence that predates its request.
- An **external operation** `(evidence kind, external_operation_id)` settles at
  most one receipt among the receipts of one scope's hot books. 0.15.0 checked
  this across the whole state. Across scopes it is now the evidence domain's
  guarantee (the resource extension already guarantees unique operation keys).
  Listed for review.
- An object ID may be used again in a later book once the earlier book is
  archived. Archive keys include the cycle, so archived objects never collide.

## Throughput

A cycle of 50,000 assessments cannot pass through 64 actions a boundary, and
50,000 commands would each pay command admission, a queued item, and an event.
The decision is a combination.

**A batch command.** `apply_fiscal_action_v1` takes a `FiscalActionBatch`:

```rust
pub struct FiscalActionBatch {
    pub authority_binding_id: String,
    pub entries: Vec<FiscalActionEntry>, // 1 to MAX_FISCAL_BATCH_ENTRIES (4,096)
}

pub struct FiscalActionEntry {
    pub action_id: String,
    pub expected_revision: u64, // of the entry's target record
    pub action: FiscalAction,
}
```

The issuer and role are resolved once, at admission, for the binding; settlement
re-checks the role once per batch against the office at the start of the
boundary and again after any entry of the same boundary changed that office.
Each entry is an ordinary action: its own target, staleness check, validation,
and `Applied` or `Rejected` outcome. A batch is not a second outcome model;
one entry is one decision, exactly as in 0.15.0, and a one-entry batch is the
old single action. 0.15.0 rejected a batch-carrying *action*, one action
assessing many scopes with one outcome; this is not that.

A batch may mix entries on different scopes of the same institution, and any
action kinds the role admits (an audit binding admits only `RecordAudit`
entries). Entries settle in order; an entry after a rebind that removed the
issuer's role is rejected with `FISCAL_ISSUER_NOT_BOUND`.

**Raised and per-record limits.** An applied action now costs about its target
ledger (kilobytes), so:

- `MAX_FISCAL_ACTIONS_PER_BOUNDARY` rises from 64 to 65,536 applied actions,
  enough for the burst of 50,000 with margin;
- `MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY` (64) bounds the repeated
  validation of one record in one boundary;
- `MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY` (65,536) bounds the ledgers a
  boundary writes, counting each ledger once however many entries and receipts
  touch it, and every ledger an `ApplyTransition` changes.

An entry past any of these is rejected as its own outcome with
`FISCAL_BOUNDARY_ACTION_LIMIT` or `FISCAL_RECORD_ACTION_LIMIT` and reissued by
the client; a stale or unauthorized entry uses none of them, as in 0.15.0.

**Admission** of a batch reads: the directory bucket entry, the office (one
decode), and for each entry its target record's static owner, revision, floor,
and ring by partial reads. No entry decodes a ledger at admission. The command
is refused as a whole only for what concerns the whole (size, binding,
issuer); an entry that admission can already refuse (unknown scope, a scope of
another institution, an expired revision, a known action ID, bounds) refuses
the command, because a command has one admission outcome and an entry naming
an unknown scope has no record to hold a rejected outcome. The refusal's
message begins with the index of the first inadmissible entry and its stable
reason (`entry 17: FISCAL_REVISION_EXPIRED`), so a host can drop or fix that
entry and resend. A host checks its entries with the same reads before it
sends them (`fiscal_target_revision` and `fiscal_recent_outcome`, see
[Public API](#public-api)).

**Events.** Settlement emits one `canwu.fiscal.action_settled.v1` event per
settled entry, as 0.15.0 emitted one per action, its summary naming the target
record, the action ID, the disposition, and the reason. It is the durable
record of each outcome once the outcome leaves its target's ring. One event per
batch and target was considered: fewer events, but a summary listing up to 64
outcomes, and a host that watches outcomes would parse lists instead of
events. Its cost at a burst of 50,000 entries is measured with Phase 2.

**Settlement** groups the boundary's entries by target, decodes each target
once, applies its entries in admission order on a working copy (each entry on
a copy of the record as the previous entry left it; a rejected entry's copy is
dropped), and writes each changed record once. Archive commits and retention
acknowledgements settle before every other fiscal packet, as in the resource
and production extensions.

**Receipts** come as `FiscalExecutionReceiptBatch` packets of up to
`MAX_FISCAL_RECEIPTS_PER_PACKET` (4,096) through `enqueue_execution_receipts`;
see [Receipts](#receipts-and-historical-context-at-scale).

**Rejected alternatives.**

- *Only raise the per-boundary limit.* 50,000 commands each pay command
  evidence, an ingress item, admission reads of the office, and an event; a
  batch pays the command-level parts once.
- *A batch action with one outcome.* Rejected in 0.15.0 and again here: one
  stale scope would reject the whole batch, or a partial outcome model would be
  needed.
- *Defer excess actions to the next boundary inside the plugin.* The plugin
  would need to re-queue admitted actions with a delay it cannot choose, and
  the deferred actions' staleness basis would be unclear. A recorded rejection
  the client reissues is the 0.15.0 contract.

## Accounting cycles: open, close, archive

A scope ledger holds one **book** per hot accounting cycle. Every object in a
book names the action that created it (`source_action_id`), so an archived
book carries the applied outcomes that shaped it.

```rust
pub struct FiscalCycleBook {
    pub accounting_cycle_id: String,
    pub status: FiscalBookStatus, // Open, or Closed { closed_by, closed_at }
    pub revision: u64,            // advances on every change of the book
    pub assessments: BTreeMap<String, FiscalAssessment>,     // at most 4
    pub remissions: BTreeMap<String, FiscalRemission>,       // at most 256
    pub execution_requests: BTreeMap<String, FiscalExecutionRequest>, // at most 256
    pub execution_receipts: BTreeMap<String, FiscalExecutionReceipt>, // at most 1,024
    pub audits: BTreeMap<String, FiscalAuditFinding>,        // at most 256
    pub totals: BTreeMap<FiscalPartitionKey, FiscalStrategicAggregate>, // per (unit, payment form)
}
```

- **Opening.** The first `OpenAssessment` of a cycle in a scope creates its
  book, provided the cycle is above both watermarks (below). A scope holds at
  most `MAX_FISCAL_HOT_BOOKS_PER_LEDGER` (64) books, open or closed.
- **Closing.** `FiscalAction::CloseAccountingCycle { scope_binding_id,
  accounting_cycle_id }`, issued under an authority binding of the scope's
  institution, closes one book. A batch closes many. A closed book accepts no
  assessment, remission, execution authorization, or receipt; a receipt for it
  is refused with the stable reason `cycle_closed` (an event, as for any
  refused receipt). Audit findings are still accepted on a closed book until it
  is archived, so auditors have the window between close and archive; each
  finding advances the book's revision.
- **Archiving.** Only closed books are archive candidates. An archive commit
  removes the selected books from their ledgers and raises each ledger's
  watermark to the cycle of the book it archives when that is greater.
- **Order.** Accounting-cycle IDs are canonical identifiers compared in byte
  order. Hosts write cycle IDs that sort in time order, for example zero-padded
  years and months. Books may be closed and archived in any order.

**Two watermarks.**

- A **scope watermark**: the greatest cycle of the scope's archived books.
- An **office watermark**: the greatest cycle whose `InstitutionTotals` record
  the archive holds for that institution.

A new book is created only above **both** watermarks of its scope; one at or
below either is refused with `FISCAL_CYCLE_ARCHIVED`. A hot book, open or
closed, is never affected by a watermark, and hot books below the office
watermark may exist: an office can archive the totals of cycle `2027` while a
scope still holds a closed `2026` book.

**Institution totals are archived exactly once.** The office's bookkeeping holds,
for each cycle with a hot book in any of its scopes, the number of such books.
The archive commit that takes a cycle's count to zero, whatever the order in
which cycles are archived, also archives that cycle's `InstitutionTotals` and
raises the office watermark to the cycle if it is greater. After that no new
book of the cycle can be made (it is at or below the office watermark), so the
count never rises again and the totals are never archived twice. A cycle whose
count is zero is not kept in the bookkeeping.

The watermarks are what make an accounting partition unique for the life of a
run without the archive: a partition is either in a hot book, where
settlement sees it, or in a cycle at or below a watermark, where no new book
can be made.

Rejected alternatives: *an explicit cycle ordinal on every action*, robust
against badly ordered labels but one more field everywhere a cycle is named
and a rule to keep label and ordinal consistent; *a register of every cycle
ID ever used per scope*, exact for any labels but growing by every cycle for
the life of the run; *archive lookups at assessment time*, rejected with the
next section.

## The fiscal archive (G-15)

### What is archived

Terminal records, each under a key:

```rust
pub enum FiscalArchiveKeyV1 {
    Book { scope_binding_id: String, accounting_cycle_id: String },
    InstitutionTotals { institution: EntityRef, accounting_cycle_id: String },
    Proposal { institution: EntityRef, proposal_id: String },
    ArchiveMaintenanceReceipt(u64),
}
```

A `Book` record is the closed book with its scope binding's static fields. An
`InstitutionTotals` record is the institution's per-(mechanism, unit, payment
form) totals for one cycle. Each commit adds the totals of the books it
archives to the office's archived totals for their cycle and lowers the
office's hot-book count; the commit that takes the count of a cycle to zero
also archives the institution's totals of that cycle (the archived totals plus
its own books') and raises the office watermark to that cycle if it is
greater ([Two watermarks](#accounting-cycles-open-close-archive)). The totals
are therefore exact sums of every book of the cycle, archived once, and none
of them is read back from the store. A `Proposal` is a resolved auditor nomination or removal
petition (the 0.16.0 appointment package keeps them for life in the state,
capped at 4,096; here they leave the office). The maintenance receipt follows
the production pattern.

### Machinery, reused

The commit protocol, object kinds, and lessons are those of
`canwu-resource` 0.15.0 and `canwu-production` 0.16.0:

1. `prepare_fiscal_archive(canwu, candidate_limit, store)`: the host
   application selects, in a deterministic order, up to `candidate_limit`
   closed books (scope ID, then cycle), the institution totals and office
   watermark moves that become due, and up to the same limit of resolved
   proposals, plus the receipt of the previous applied commit first. It reads
   the archive head directory and the lookup pages on the paths of the new
   keys, and returns a `PreparedFiscalArchiveBatchV1`: blobs of at most 64
   records, one membership and one temporal page each, at most 64 blobs (4,096
   records) per commit, the lookup pages it adds, the superseded lookup pages,
   the directory, and the retention handle. Preparing walks the ledgers in ID
   order with partial reads (about 50,000 in the world, host-side, between
   boundaries) and decodes only the selected ones; an index of closed books
   would make it proportional to the selection and is left out until the walk
   is measured.
2. `store_and_verify(store)`: stores every object, reads the directory back,
   authenticates the closure, and checks that every record resolves through the
   stored index; returns a `VerifiedFiscalArchiveCommitV1`.
3. `enqueue_fiscal_archive(canwu, verified, due_at)`: requires the batch to equal
   what `prepare_fiscal_archive` derives from the current state and the store,
   as `enqueue_resource_archive` and `enqueue_production_archive` do, then
   queues a `fiscal_archive_commit_v1` packet with its retention binding.
   Preparing and enqueueing belong in one gap between two boundaries: a batch
   prepared before a boundary that changes a selected book, an affected
   office's bookkeeping, or the archive head is refused at enqueue, or settles
   stale if it was already queued.
4. `finalize_fiscal_archive_retention(canwu, store, handle)`: after the commit
   settled, persists the handle's final phase and queues a
   `fiscal_archive_retention_v1` acknowledgement.

**The lookup index** is the 16-way trie over key digests of the resource
archive (`ResourceArchiveLookupPageV1`, leaves of 8 entries) under the
namespace `canwu.fiscal.archive.lookup-page`, built with the functions the
production package makes public in `canwu-resource`:
`prepare_resource_archive_lookup_entries`, `find_resource_archive_lookup_entry`,
`verify_resource_archive_lookup_entries`, `collect_resource_archive_lookup_pages`,
and `audit_resource_archive_lookup_index` with `ResourceArchiveAuditEntry`.
`canwu-fiscal` passes them a view of its store that serves only the fiscal
lookup-page namespace, as production does, and digests its keys with
`fiscal_archive_lookup_key_digest` under its own domain. No index code is
copied. `canwu-fiscal` gains a dependency on `canwu-resource` for these
functions only; it uses none of the resource extension's domain types, and
none of them appears in the fiscal API: the host implements the fiscal-owned
`FiscalArchiveStore` trait, fiscal functions return `FiscalArchiveError`, and
the adapter to the resource functions and the conversion of their errors are
private to the crate.

**Required evidence and sealing.** While a book is hot, the versions it cites
(commutation quotes, receipt evidence) are declared as payload-required
evidence, as the state record declared them. Archiving a book ends that
declaration, so the live run stops keeping those versions provable; they stay
provable only through evidence segments the host sealed. On a run that seals
its evidence (a compacted run), `prepare_fiscal_archive` therefore refuses a
book that cites a version newer than the run's last sealed evidence cut, with
`FiscalArchiveError::UnsealedCitations` naming the book: the host seals first,
then archives, so every archived citation is already in a sealed segment when
its book leaves the run. A run that has never sealed keeps every version in
its retained evidence, and its first seal puts them into a segment.

**Settling a commit reads no store.** The batch binds its source: the archive
head it extends, the revision of every selected book (a closed book changes
only by an audit finding), and the revision and cycle bookkeeping of every
office whose counts or totals it changes. The boundary applies the commit when
all of them are still as prepared; otherwise the commit is stale: a terminal
no-op whose receipt is retired by its acknowledgement. Neither can fail for
capacity. Since commits settle first and preparation reads the state the last
boundary left, a commit enqueued right after it is prepared goes stale only
when another commit settles before it. Applying removes the books from their
ledgers (writing each ledger once), updates the offices' cycle bookkeeping and
watermarks, removes archived proposals, moves the head, and records the
receipt in the root. Lessons kept from the other archives:

- commits and acknowledgements settle before every other fiscal packet, each
  group in admitted order;
- the receipt of an applied commit is archived first by the next commit, so
  the root holds at most one; a stale commit's receipt names its handle and is
  retired by the acknowledgement;
- an acknowledgement for a handle the root does not hold is a no-op; the same
  batch queued twice settles once applied and once stale under one handle;
- the root holds at most `MAX_FISCAL_ARCHIVE_PENDING_HANDLES` (64) handles of
  applied commits; preparation refuses while 64 handles wait, which is
  backpressure on the host, never a boundary failure;
- a run may make any number of commits; lookups never walk the chain;
- the reachability participant marks the chain, the queued commits' objects,
  and the lookup pages reachable from them; deleting superseded lookup pages
  follows the resource retention rule.

**Host guidance on superseded lookup pages.** Every commit rewrites the index
pages on the paths of its new keys, so the pages written grow faster than the
pages the head reaches: about 0.8 KB written per archived record for commits
of 4,096 records, against about 200 bytes per record reachable from the head
(resource measurements). Over 100 years of the annual world that is about
4 GB written for 1 GB reachable. A host that keeps every restore point keeps
all of it; a host that only needs to restore from its latest durable
checkpoint may delete a commit's `superseded_lookup_pages` once a checkpoint
taken after that commit is durable, giving up the earlier restore points, as
the resource retention rule says.

### No boundary reads the archive

Every boundary decision is made from hot records: partition uniqueness from
watermarks, idempotency from rings and floors, evidence uniqueness from the
request's own receipts. Consequences:

- A missing, slow, or corrupt store can never fail a boundary, and exact replay
  of settlement does not need the store.
- A restore does not have to audit the archive for settlement to be correct.
  The checked restore authenticates the head directory only (one object); the
  linear audit, `audit_fiscal_archive(store, head)`, is a separate host call
  for offline verification. The resource audit takes about 0.9 s per 100,000
  archived records, so a 60-million-book archive would take minutes; nothing
  makes a restore wait for it.
- Lookups are host-side reads: `archived_fiscal_record(store, head, key)` reads
  the head directory, one lookup page per level, and one blob; a miss reads
  only pages. At 5 to 60 million keys the index is 6 to 7 levels, about
  30 to 40 microseconds per miss by the resource measurements.

Rejected: *archive lookups in boundaries for ID uniqueness*. At monthly volume
a day settles thousands of new keys; at 30 to 40 microseconds per miss that is
a few hundred milliseconds a day, and every such boundary would depend on the
store.

### Growth

Estimates for the measurement plan to confirm (resource measurements: about
0.8 KB of lookup pages written per record for commits of 4,096, about 200 bytes
of index per record reachable from the head):

| Cycle | Books per year | Archive bytes per year (books, pages, index written) | Index reachable from the head after 100 years |
| --- | ---: | ---: | ---: |
| Annual | 50,000 | about 150 MB | about 1 GB |
| Monthly | 600,000 | about 1.8 GB | about 12 GB |

The archive lives in the host application's store, not in the save; a save
holds the hot records and the archive head.

## Observers and reports (G-16)

This section describes the end state. By phase: Phase 1 has the optional
actor with person and government holders, `RebindObserver`, `RemoveObserver`,
and line slots allocated once per binding on the state record. Phase 2 adds
observer records with lines named on them, the office observer index, the
static part layout of scope rows and cycle slots with a digest per line, and
publication on change with report-queue slots for what one boundary cannot
publish. Phase 3 adds `Interval` cadence, institution granularity, the parked
rotation, and domain-entity holders.

### Bindings

```rust
pub struct FiscalObserverBinding {
    pub id: String,
    pub actor: Option<PersonId>,
    pub knowledge_holder: KnowledgeHolderRef,
    pub visible_institutions: BTreeSet<EntityRef>, // at most 4,096
    pub confidence_per_mille: u16,
    pub granularity: FiscalReportGranularity,      // Scope or Institution
    pub cadence: FiscalReportCadence,              // OnChange or Interval { every, offset }
    pub controlled_by: Option<EntityRef>,          // institution that may re-point or remove it
}
```

- **Holders without a person.** With `actor: Some(p)`, the holder must be
  `Person(p)` (the 0.15.0 rule). With `actor: None`, the holder is an
  institution the engine lets hold knowledge. Phases 1 and 2 allow a government
  (`Entity(Government)`), which the observer record references, so the engine
  refuses a state naming one that does not exist, and which can never become
  unable to hold knowledge. **Domain-entity holders** (`Entity(Domain)`, an
  active entity record whose schema allows holding knowledge) are left to
  Phase 3: such a holder cannot be referenced (a reference would make every
  retirement of that record re-validate fiscal records, and fail the other
  plugin's boundary with `allow_retired: false`), so the report system would
  have to read it before publishing, its kind declared with
  `FiscalPlugin::with_report_holder_kinds`, and another plugin's phase-13 system
  could still retire it after that read.
- **Unreachable holders.** The engine refuses to publish to a dead person. The
  report system checks the same before it publishes and parks such an observer
  in the parked slot, with the lines it owes. In Phases 1 and 2 every parked
  observer is re-checked in each boundary that runs the report system, one
  availability read each, so a revived holder is published to by the boundary
  that revives him, as in the 0.16.0 package. Phase 3, for worlds with many
  dead observers left parked, re-checks them in rotation instead,
  `FISCAL_PARKED_CHECKS_PER_BOUNDARY` (16) in each boundary that runs the
  report system, so they cost a fixed amount per boundary; a revived holder is
  then published to when
  the rotation reaches him: within `ceil(parked / 16)` such boundaries, where
  the 0.16.0 package published in the reviving boundary itself. Checking only
  the persons whose availability changed in the boundary would need a view
  read of the boundary's availability changes, which the engine does not
  offer today (`SimulationView` reads one person's availability at a time).

### Re-pointing and removing, keeping line places

Two new actions, issued under an authority binding of the binding's
`controlled_by` institution by its principal or the institution as a body (the
roles that may rebind an authority binding), with the observer record as
target:

- `RebindObserver { observer_binding_id, actor, knowledge_holder }` re-points
  the binding to another actor or holder. The new holder gets the whole report
  at the next publication (it holds none of its lines yet). The old holder gets
  one final summary marked `withdrawn` if it can still receive knowledge, and
  keeps what it was told; `fiscal_reports` omits a withdrawn report.
- `RemoveObserver { observer_binding_id }` withdraws the report the same way
  and marks the record removed. The record stays, so its identity is never
  reused by accident; a removed binding is never published to again.

A binding with no `controlled_by` changes only with the scenario, as in
0.15.0. Neither action changes visible institutions, confidence, granularity,
or cadence: what an observer is entitled to see is set by the scenario. A
runtime install of new observer bindings is an open decision.

**Lines.** A report line is named by subjects on the observer's own record:
the role `fiscal_observer` and five roles `report_line_<place>_<digit>` for the
five hexadecimal digits of the line number. `knowledge_heads(holder, kind,
subjects)` is per holder, and two observers' records are different targets, so
line numbers only have to be unique within one observer. Adding, removing, or
re-pointing one binding never moves another binding's lines; that is what
"keeping report line places" needs, and the 0.15.0 scheme (a line number from
the binding's place among all bindings, all on the one state record) could not
give it.

### What an observer is told

- **Rows.** A `Scope` observer has one row per scope of its visible
  institutions; an `Institution` observer has one row per (visible institution,
  mechanism). Rows are ordered statically: visible institutions in canonical
  order, then scopes by their rank in the office. The observer record keeps
  each visible institution's first row.
- **Facts.** A row has at most `MAX_FISCAL_FACTS_PER_ROW` (4) facts per cycle:
  a scope row the totals of its book's partitions (at most 4 assessments per
  book), an institution row the summed totals per (unit, payment form). The
  office records the partition keys each hot cycle uses, so settlement refuses
  an assessment that would give one (institution, mechanism, cycle) a fifth key,
  with the stable reason `FISCAL_PARTITION_KEYS_EXHAUSTED`. A fact for an
  institution row has no scope.
- **Parts.** A part is 16 consecutive rows of one cycle, so at most 64 facts.
  Units become canonical identifiers like the other texts of a fact, so a fact
  never needs JSON escaping and a full part stays under the 65,536-byte
  knowledge payload; a unit test pins the worst case.
- **Cycles.** Each hot cycle the observer sees takes a cycle slot (at most 255
  at once, reused after the cycle leaves). Line 0 is the summary; part `p` of
  slot `s` is line `1 + s * 4,096 + p`. Five hexadecimal places hold 1,048,576
  lines per observer.
- **What leaves.** When a cycle's books are archived, its facts leave the report:
  the report system publishes its parts empty (withdrawn) and frees the slot.

**An observer of one polity** sees exactly the facts of its visible
institutions, at its granularity and confidence, for their hot cycles. A court
observer at institution granularity sees one row per institution and mechanism
of its polity (a few dozen facts per cycle), not its 6,000 scopes; a regional
official at scope granularity sees its office's scopes. Nothing in an
observer's report depends on another observer's: publication limits move an
observer to a later boundary at the same instant, never to a later simulated
time, so the `as_of` of every report is the time it was due. The residual
coupling is the engine's: knowledge record IDs are global, so a holder can see
how many records were allocated between two of its own.

### When reports are published

- **Cadence.** `OnChange` publishes in the boundary that changed a visible
  fact (0.15.0 behavior; the default). `Interval { every, offset }` publishes at
  the aligned times `offset + k * every` (simulated time) at which something
  visible changed since the last publication. Observers that share a cadence and
  offset are due together; a scenario staggers them with offsets.
- **Queue.** Settlement knows the ledgers it changed. It reads their offices'
  observer index (a read, not a write) and adds each affected observer to the
  report-queue slot of its due time: now for `OnChange`, the next aligned time
  for `Interval`. A queue entry names the observer and the report lines its
  changes touched (the cycle slot and the row page of each changed row), and
  merges with the observer's entry already in the slot. Slots are keyed by
  (due time, page): a slot holds up to 4,096 entries, and a due time with more
  queued observers takes further pages. The root indexes the slots in use by
  key and keeps the earliest due time.
- **No timers.** A future-dated packet would block `seal_evidence`, so the plugin
  schedules none. The report system runs in every event-bearing boundary
  (it is event-driven, as in 0.15.0); it first reads the root's earliest due
  time and the boundary's own queue writes, and returns without decoding
  anything when nothing is due. A report due at time `t` is published by the
  first boundary at or after `t` that runs the fiscal systems; a host whose
  boundaries can stay without any event for long stretches gets it then. A
  configurable system cadence for the report system is an open decision.
- **Publishing.** For each due observer, in order of (due time, observer ID), the
  report system computes only the parts its queue entry names, from the ledgers
  (or institution sums), compares each part's digest with the one the observer
  record holds, publishes the summary and the parts whose digest changed, and
  writes the observer record (new digests, time). A part whose rows did not
  change is not computed. At most the engine's publications per system per
  boundary (64 today, read from `KnowledgeLimitsV1::CURRENT`) and
  `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024) records are published; the
  rest stay queued and the report system schedules the 0.16.0 drain packet
  (`fiscal_report_drain_v1`, due at once), so the next boundary at the same
  instant continues. An observer whose own report exceeds one publication
  (1,000 records) continues the same way; its summary names its pending lines.
- **Drains and sealing.** A drain packet is ingress due at once, so it holds
  back `seal_evidence` until a boundary admits it. Under `step_canonical` and
  `advance_canonical` the boundary that admits it is settled by the same call
  or the next step at the same instant, so a host that seals between steps can
  find a drain queued, and seals after the next step. A world that publishes
  thousands of reports in one instant settles that many extra boundaries at it
  before the instant is sealable.
- **What 0.15.0 kept in the state** (aggregates of the previous derivation, to
  diff against) is replaced by the per-line digests in the observer record:
  32-byte BLAKE3 digests of each part's canonical facts. This is what lets an
  `Interval` observer skip all the changes between its publications and
  publish only what differs at its due time.

### Volume at the target

| Observer | Rows | Parts per cycle | Records per publication |
| --- | ---: | ---: | ---: |
| Regional official, scope granularity, office of 14 scopes | 14 | 1 | 2 |
| Court, institution granularity, 21 institutions, 1 to 3 mechanisms | 21 to 63 | 2 to 4 | 3 to 5 |
| Largest polity's court at scope granularity (6,000 scopes) | 6,000 | 375 | up to 376 |

With 4,000 observers on a monthly interval staggered over 30 days, about 134
observers are due a day: three boundaries a day under the engine's 64
publications per system (two of them same-instant drains), about 600 records.
Every observer on the same day would take 63 boundaries at that instant. The
engine limit, not a fiscal one, is what binds; it is listed under
[Beyond fiscal](#beyond-fiscal-caps-that-block-the-target).

### Reading reports

`fiscal_reports(&viewer)` reads the holder's current fiscal report records,
assembles one `FiscalReport` per observer binding (including bindings whose
holder is an institution the viewer reads for), and omits withdrawn ones.
`FiscalReport::cycles` lists the cycles it covers; `pending_parts` and
`is_stale` keep their 0.15.0 meaning per line. The 0.15.0 invariant is kept: a
complete report, assembled, equals what `compute_projection` derives from the
fiscal records it was published from, at its `as_of`.

## Receipts and historical context at scale

**Receipts.** `FiscalExecutionReceiptPacket` gains `scope_binding_id`; the
evidence claim (`FiscalExecutionEvidence`) names the same scope. A
`fiscal_execution_receipts_v1` packet carries up to 4,096 receipts, settled in
order, each against its ledger. 0.15.0 (and the 0.16.0 package for everything
but the budget) failed the boundary for a receipt whose evidence does not
validate; at world scale one bad receipt in a batch of 4,096 would stop the
run. Every receipt refusal therefore becomes the 0.16.0 refusal event
(`canwu.fiscal.ingress_rejected.v1`, summary naming the packet and receipt,
affected institution when registered) with a stable reason, and the receipts
after it settle:

| Reason | When |
| --- | --- |
| `state_budget_exhausted` (0.16.0) | the ledger would exceed its byte budget |
| `evidence_not_current` | an evidence version is not the current version of its record |
| `evidence_kind_unapproved` | a kind the root does not approve |
| `request_unknown` | no hot request with that ID in that scope |
| `cycle_closed` | the request's book is closed |
| `claim_mismatch` | a claim field differs from the request, or dispositions differ |
| `quantity_exceeded` | the receipts would fulfil more than the request authorizes |
| `operation_reused`, `evidence_reused` | already settled by a receipt of that scope's hot books |
| `receipt_id_conflict` | a hot receipt has the ID with other content |
| `boundary_limit` | the boundary's ledger-write budget is spent |
| `malformed_payload` (0.16.0) | the packet does not decode |

A refused receipt changes nothing and consumes nothing, as in the 0.16.0
package: the request keeps its outstanding quantity and the evidence stays
free. The event is boundary evidence; a host that must account for every
execution watches for it.

**Historical context.** A `fiscal_historical_context_v1` change writes the root
and recomputes the candidates of the jurisdictions it can affect: those of the
transitions whose observed or eligibility window contains exactly one of the
old and new years (all transitions when the mode changes). The jurisdiction
records whose candidates change are written; at most every region in use. A
context outside the pack, a context payload that does not decode, or one that
would put a jurisdiction record past its byte budget is refused with the event
(reasons `context_outside_pack`, `malformed_payload`, `state_budget_exhausted`)
instead of failing the boundary, and changes nothing.

**Transitions.** `ApplyTransition` reads its jurisdiction record and index pages,
and writes the ledgers in the region whose adoptions it suspends or installs
(at most the region's scopes, counted against the ledger-write budget).

## Every fiscal cap

Values for 0.16.0. "Lifetime" means a cap on what a run accumulates; none
remains.

| Cap | 0.15.0 (and 0.16.0 package) | Becomes | Why |
| --- | ---: | ---: | --- |
| `MAX_FISCAL_STATE_JSON_BYTES` | 32 MiB, one record | removed | replaced by per-record budgets |
| Root record bytes | — | 64 KiB | global, small, written often |
| Office, ledger, jurisdiction, observer, directory, index, queue record bytes | — | 256 KiB each | well under the inferred ~1.1 MB paged-checkpoint ceiling; a ledger reaches it only with years of unarchived books, and then refuses (recorded), never fails |
| `MAX_FISCAL_REPORT_PLAN_BYTES` | 152,416 (package) | removed | no plan in a shared record |
| `MAX_FISCAL_RUNTIME_BINDINGS` | 4,096 (authority, audit, scope each) | scope bindings 1,048,576; per office 256 authority and 256 audit bindings | one ledger per scope; bindings live in their office |
| `MAX_FISCAL_OBSERVERS` | 64 | 65,536 | one record each; nothing shared grows with observers. The G-16 filed fix asked for a configurable value "under the size guard"; with no shared record there is no size guard to stay under, so a fixed generous cap replaces the configuration |
| Visible institutions per observer | 4,096 | 4,096 | bounds the observer record |
| Observers per institution (office index) | — | 4,096 | bounds the office record |
| `MAX_FISCAL_ASSESSMENTS` | 4,096 lifetime | removed; 4 per book | partition uniqueness by watermark |
| Remissions, audits | 4,096 lifetime | 256 per book each | per book |
| `MAX_FISCAL_EXECUTION_REQUESTS` | 8,192 lifetime | 256 per book | per book |
| `MAX_FISCAL_EXECUTION_RECEIPTS` | 8,192 lifetime | 1,024 per book | partial receipts |
| Adoptions | 8,192 (shared with requests) | 64 per ledger | rules adopted in one scope |
| Hot books per ledger | — | 64 | forces archive before the ledger budget; a 65th book is refused with `FISCAL_HOT_BOOKS_EXHAUSTED` |
| Partition keys per (institution, mechanism, cycle) | — | 4 | bounds an institution row; a fifth is refused with `FISCAL_PARTITION_KEYS_EXHAUSTED` |
| `MAX_FISCAL_ACTION_OUTCOMES` | 16,384 lifetime | removed; a ring of 8 per target (action ID, disposition, settled revision) and a floor | idempotency from revisions; the reason lives in the event |
| Report-queue slot entries | — | 4,096 per (due time, page) slot | further pages for a busy due time |
| Parked observers checked per boundary | — | 16 | a fixed cost per boundary for dead holders |
| `MAX_FISCAL_ACTIONS_PER_BOUNDARY` | 64 | 65,536 | an action costs its ledger |
| Actions per record per boundary | — | 64 | bounds repeated validation |
| Ledger writes per boundary | — | 65,536 | bounds a boundary |
| Batch entries per command | 1 | 4,096 | one institution's cycle in a few commands |
| `MAX_FISCAL_ACTION_JSON_BYTES` | 32 KiB per request | 32 KiB per entry, 4 MiB per batch | |
| `MAX_FISCAL_ACTION_TEXT_BYTES`, `MAX_FISCAL_IDENTIFIER_BYTES` | 2,048, 160 | unchanged | |
| Units | bounded text | canonical identifiers | bounds a fact without escapes |
| `MAX_FISCAL_EVIDENCE_PER_RECORD`, `MAX_FISCAL_EVIDENCE_KINDS` | 32, 32 | unchanged | |
| Receipts per packet | 1 | 4,096 | batches |
| `FISCAL_REPORT_FACTS_PER_PART` | 16 | 64 (16 rows of at most 4 facts) | |
| `MAX_FISCAL_REPORT_PARTS` | 256 per observer | 255 cycle slots of 4,096 parts per observer | |
| Report line places | 4 hex digits over all observers | 5 hex digits per observer | lines on the observer record |
| `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` | 1,024 | 1,024 | the fiscal share of the engine's 10,000 |
| Observers published per boundary | 64 (engine) | the engine's limit, read from `KnowledgeLimitsV1` | |
| Pending auditor proposals | 256 (package, global) | 256 per office | per office |
| Kept auditor proposals | 4,096 lifetime (package) | removed; resolved proposals archive | |
| `MAX_FISCAL_APPOINTMENT_PARTIES` | 32 (package) | unchanged | |
| `MAX_FISCAL_CATALOG_REGIONS` | 1,024 | 65,536 | a world catalog's regions may be first-level divisions (a few thousand) |
| `MAX_FISCAL_CATALOG_COVERAGE_CELLS` | 65,536 | 1,048,576 | periods × regions × mechanisms |
| Catalog definitions, periods, references per definition | 16,384, 512, 1,024 | unchanged | |
| `MAX_FISCAL_CATALOG_JSON_BYTES` | 64 MiB | unchanged, see risk | boundaries read entries by key; the catalog's size costs activation, snapshots, and, if the inferred page ceiling holds, paged checkpoints above about 1.1 MB |
| Archive: records per blob, blobs per commit | — | 64, 64 (4,096 per commit) | a hit reads one blob of about 150 KB |
| Archive: pending applied handles | — | 64 | backpressure at preparation |
| Counters (revisions, archive counts, receipt sequence) | u64 | u64 | |

## Cost model

What a boundary of the partitioned design costs the engine is

    idle(store) + sum over written records of write(record) + events

- `idle(store)` is what every boundary pays for the domain records the run
  holds, whether or not any system touches them;
- `write(record) = a + b * KiB` is what the engine does for one record a
  boundary writes: payload and reference validation, the affected-record
  closure, the Patricia insert (the record's encoding, its leaf page, which
  carries the record as a JSON array of byte values at about 4 bytes per
  record byte, and every branch page on its path, about 17 at 65,536 records,
  each encoded and hashed), the journal entry with the previous and the
  current record, and the boundary hash over both;
- `events` is one settled event per action, and the boundary after a write
  admits the events the write emitted.

The plugin's own work (decoding a ledger, applying an action, validating and
encoding the ledger) comes on top and is measured with Phase 2's first ledger
code. Phase 0 measured the engine's terms.

### Phase 0 results

`record-write-profile` (`benchmarks/performance-harness`), release build, on
0.15.0 sources (`df945ef`), one Windows x86-64 machine that other workers'
builds were loading at about half of its 16 cores, so single cases moved by up
to a third between neighbouring runs; each figure is a median of three
samples. Baselines:
`benchmarks/baselines/2026-10-15-record-write-entries-elapsed.json`,
`2026-10-15-record-write-text-elapsed.json`, and
`2026-10-15-record-write-entries-allocations.json` (their source hash names
the tree just before a lint-only change to the probe).

A probe plugin keeps 65,536 records of one kind and updates N of them in one
boundary. "Entries" payloads are objects of generated ledger entries of about
350 bytes each (identifiers, numbers, one note), the shape of a fiscal book;
"text" payloads are one string of the same size. The engine's share is the
boundary's time less the probe handler's own time.

**Per write, engine only, microseconds:**

| Record (KiB) | Entries, N = 1,000 | Entries, N = 10,000 | Entries, N = 50,000 | Text, N = 1,000 | Text, N = 10,000 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1.3 (1.2 text) | 329 | 287 | 392 | 326 | 274 |
| 2.4 (2.2 text) | 695 | 623 | 504 | 353 | 300 |
| 4.5 (4.2 text) | 903 | 791 | 1,441 | 389 | 311 |
| 8.4 (8.2 text) | 1,494 | 1,317 | 2,324 | 468 | 504 |

**Fitted** over the record's KiB, at N = 10,000:

- entries: `write ≈ 194 µs + 134 µs per KiB` (at N = 1,000: 217 µs + 153 µs
  per KiB);
- text: `write ≈ 208 µs + 34 µs per KiB`.

Three things follow. A write has a large fixed part, about 200 µs, whatever
the record holds. Beyond it, the cost follows the record's JSON values more
than its bytes: a structured record costs about four times as much per KiB as
a flat one. And at 50,000 writes in one boundary the cost per write of larger
records rises further (1.4 ms at 4.5 KiB, 2.3 ms at 8.4 KiB), as the boundary
holds the previous and current version of every record it changed. Part of
the fixed term also grows with the store: a single check run of the same
1.3 KiB write into a store of 1,024 records cost about 155 µs, against 290 to
330 µs at 65,536 records; it is indicative only, one sample on a loaded
machine.

**The terms the review named**, rebuilt in the harness with the engine's
encodings and timed outside it (`reproduced_terms`), per write of a 1.3 KiB
entries record: branch pages on a 17-page path 15.5 µs, the leaf page 12.3 µs
(5.6 KB of page for 1.36 KB of record), the record change in the boundary hash
(previous and current) 5.7 µs, a record copy 3.4 µs, the record encoding
1.7 µs, and its value hash 1.6 µs: about 40 µs of the 290 to 330 µs measured.
For text records of 1.2 to 8.2 KiB they come to 35 to 120 µs of 270 to 500 µs.
These encodings are a sixth to a quarter of a write; most of the engine's time
is elsewhere on the write path and grows with a record's JSON values, which
points at copies and walks of the payload tree rather than at hashing. **The
previous record** (`update` against `update_from_small`) did not separate from
the noise: the difference changed sign between sizes.

**The idle boundary.** A boundary in which the probe system runs and writes
nothing costs 0.023 ms with an empty store and 35 to 113 ms with 65,536
records (35 to 57 ms for text records, 49 to 113 ms for entries records). A
one-off variant of the probe, not kept in the tree, showed that this does not
depend on which record kinds the running system declares (33 to 39 ms with
the kind declared, with another kind, and with nothing declared) and that it
grows with the store: 5 to 6 ms at 16,384 records. Every boundary of a run
pays it, about 0.5 to 1.7 µs per stored record. The boundary after a write,
which admits the write's events, costs the same as an idle one: the events
add no measurable time per write (the follow-up fits at under 1 µs per write).

**Allocations** (allocation build, one sample per case; allocation counts do
not depend on the machine's load). Per write, engine only, fitted over N:
about 620 allocations plus 590 per KiB of record, and 138 KiB plus 82 KiB per
KiB of record allocated. A 2.4 KiB entries record costs about 2,040
allocations and 330 KiB allocated per write, about 135 times its own size,
which agrees with the time going to copies of the payload tree. The previous
record, which the timings could not separate, separates here: it accounts for
about 200 allocations and 18 KiB allocated per KiB of previous record (at
50,000 writes, 160 allocations at 1.3 KiB and 1,775 at 8.4 KiB, `update`
against `update_from_small`). An idle boundary with 65,536 records makes
404,364 allocations of 27 MB in all, about 6 allocations and 420 bytes per
stored record, against 226 allocations and 33 KB with an empty store: the idle
floor is per-record work the engine repeats in every boundary.

**The paged-checkpoint ceiling is confirmed.** A run holding one record with a
1,247,847-byte payload fails `prepare_paged_checkpoint` with "state page bytes
exceed the bounded page limit". Bisection puts the largest payload that fits
at 1,151,023 bytes (one byte of record takes 3.7 to 4.3 bytes of leaf page).

### What this means for the county-level world

With a ledger of about 2.5 KiB of structured JSON (one hot annual book; about
4 KiB with two monthly), the measured `write` is about 500 to 700 µs, and the
store holds about 60,000 fiscal records besides every other plugin's.

| Workload | Engine work | Engine time |
| --- | --- | ---: |
| Any boundary (idle floor) | about 60,000 fiscal records in the store | 35 to 110 ms |
| Annual world, a spread day | about 550 ledger writes (open, receipts, close: about 4 per scope per cycle) | about 0.3 to 0.4 s with the floor |
| Annual world, a spread month | about 16,500 ledger writes and 30 floors | about 10 to 15 s |
| Annual or monthly world, a burst day | 50,000 ledger writes in one boundary (measured: 25 s at 2.4 KiB) | about 25 s |
| Monthly world, a spread day | about 6,600 ledger writes | about 3.5 to 4.5 s |
| Monthly world, a spread month | about 200,000 ledger writes and 30 floors | about 110 to 140 s |
| Archive commit of 4,096 books | 4,096 ledger writes | about 2 to 3 s |

**The engine's write path must change for the county-level world.** By the
diagnostic lines of [The target](#the-target), a routine day should add tens
of milliseconds and a burst a few seconds; the measured engine alone gives a
monthly day 3.5 to 4.5 s, an annual day 0.3 to 0.4 s, a burst 25 s, and every
boundary an idle floor of 35 to 110 ms. Fiscal-side levers reduce the work but
not by the factor needed: coalescing receipts (one per scope per cycle) halves
the writes, lean ledgers trim the per-KiB part, and per-(scope, cycle) book
records make a write smaller, but the fixed 200 µs per write and the idle
floor are the engine's. For a monthly day to cost tens of milliseconds, a
record write has to cost on the order of 10 µs and the idle floor a few
milliseconds: roughly a twentyfold to fortyfold reduction of the write and a
tenfold reduction of the floor. These are engine items, listed under
[Beyond fiscal](#beyond-fiscal-caps-that-block-the-target); Phase 2 builds the
fiscal side so that its cost is exactly these engine terms, measures the
`world` and `world-monthly` presets, and reports the gap.

Journal volume follows the same writes: a burst of 50,000 ledger writes
journals about 2 × 50,000 × 2.5 KiB = 250 MB of record changes in one
boundary, which a host seals regularly. Hot fiscal state in the save: about
125 to 175 MB for the annual world (ledgers about 125 MB, offices, observers,
directory, and index pages the rest), within the 1 GB save size the owner
allows. No fiscal record approaches the paged-checkpoint ceiling (256 KiB
budget); a world catalog can, and must then be split.

## Authority

Unchanged in substance, re-homed per record:

- Admission resolves the issuer's role under the named binding exactly as 0.15.0
  and the 0.16.0 package do (principal, acting with a current basis, institution
  as a body for rebinds and appointment decisions, auditor, listed actor), from
  the office the directory names. Bases are read through
  `current_domain_record_version`, so a sealed run decides as its replay does.
- Each entry's target must belong to the binding's institution: a scope its
  office owns, the office itself, a jurisdiction whose transition the
  institution may apply (it owns every target scope, as in 0.15.0), or an
  observer the institution controls. An audit binding admits `RecordAudit` on
  the audited institution's scopes only.
- Settlement re-checks the role against the office at the start of the boundary
  and after every earlier entry of the boundary that changed that office.
- The action packet stays internal (0.16.0 package): it settles only when its
  ingress names, as its cause, the command it carries; any other is refused
  with the event.
- Cross-office actions (an appointer institution deciding on another
  institution's auditors) target the audited office and read the appointer's.
  The appointment rules of the 0.16.0 package are unchanged; their state moves
  from `FiscalState` maps into the audited office.

## Persistence, replay, seal independence, and formats

**What changes.** `FISCAL_RUNTIME_SCHEMA_VERSION` becomes 4 (the 0.16.0 package
takes it to 3). The record kind `canwu.fiscal:state` is gone; the kinds of
[State layout](#state-layout) are new, each with its payload schema
registered. The `fiscal_report` knowledge schema moves to version 3 (subjects
on the observer record, five line places, cycle and granularity in the
summary, the `withdrawn` mark, optional scope in a fact); version 2 is no longer
registered. The command payload is the batch; the ingress types are
`fiscal_action_v1` (internal, carrying the admitted batch), `fiscal_execution_receipts_v1`,
`fiscal_historical_context_v1`, `fiscal_archive_commit_v1`,
`fiscal_archive_retention_v1`, and the package's `fiscal_report_drain_v1`.
Archive objects are new kinds under `canwu.fiscal.archive.*`: blob (format
version 1), membership page, temporal page, index directory with `lookup_root`,
and lookup page in the resource page format. The plugin semantic hash and
descriptor change. Snapshot format 8, the commitment format (4, or 5 with the
0.16.0 knowledge package), checkpoint and evidence-journal formats, and the
hash algorithms are unchanged by this design.

**Same version only.** Nothing written by 0.15.0 is read: no migration, no
default for old bytes, no equality with 0.15.0 hashes. A scenario built for
0.15.0 is rebuilt with `FiscalState::into_records`.

**Seal independence.** Every boundary decision reads hot fiscal records, the
catalog, the boundary's own admitted ingress (the action packet's cause is in
its ingress record), the current-version provenance index, replay-stable
record versions (receipt evidence), person availability and domain holder
records, and `knowledge_heads`. None reads retained evidence or the archive
store. The plugin schedules only zero-delay packets (the drain), which
canonical stepping admits in the same call, so the plugin never holds back
`seal_evidence` for long. The acceptance tests prove, for runs that close,
archive, and publish across a seal: sealed run = unsealed fork = restore from
sealed segments = exact replay.

**Required evidence.** Ledgers declare as payload-required evidence the
commutation quotes and receipt evidence versions their hot books cite, as the
state record did, so a host seals with prepare, store, and commit while they
are hot. When a book is archived its declaration ends, and
`prepare_fiscal_archive` refuses a book whose citations are newer than the last
sealed cut ([Required evidence and sealing](#the-fiscal-archive-g-15)), so every
archived citation is provable from a sealed segment. A long run therefore no
longer carries every citation it ever made. This is a deliberate change from
0.15.0, where a receipt's citations stayed required for the life of the state.

**Tamper.** Activation refuses: a ledger, office, or observer that does not
validate; a record whose key does not match its content (office key, scope ID,
bucket); any broken cross-record invariant; derived fields that differ from
`into_records`; a queue or parked slot naming an unknown observer; a root
whose archive head, receipts, or handles are inconsistent. Format tests pin the
new record shapes and the report schema hash.

## Public API

Breaking, under the same-version policy. Added, changed, or removed in
`canwu-fiscal`. Phase 1 brings the action, receipt, observer, archive, and
reason changes on the single record (with `expected_procedure_revision` kept);
Phase 2 brings the record types, the batch, per-target revisions, and the
audit over records.

- **State model.** `FiscalState` changes shape (root, offices, ledgers,
  jurisdictions, observers) with `validate`, `into_records`, `from_records`.
  New payload types `FiscalRoot`, `FiscalOffice`, `FiscalScopeLedger`,
  `FiscalCycleBook`, `FiscalBookStatus`, `FiscalJurisdiction`, `FiscalObserver`,
  and record types with reference helpers (`fiscal_root_reference`,
  `fiscal_office_reference(&EntityRef)`, `fiscal_ledger_reference(scope)`,
  `fiscal_jurisdiction_reference(region)`, `fiscal_observer_reference(id)`).
  `FiscalStateRecord`, `fiscal_state_reference`, `compute_aggregates`, and
  `recompute_derived` are removed; `load_fiscal_state` assembles the whole state
  (for tools and tests; it is `O(fiscal records)`).
- **Actions.** `FiscalActionRequest` is replaced by `FiscalActionBatch` and
  `FiscalActionEntry` (with `FiscalActionBatch::single`);
  `fiscal_action_command(&FiscalActionBatch)`. `GrantRemission`,
  `AuthorizeExecution`, and `RecordAudit` gain `scope_binding_id`. New variants
  `CloseAccountingCycle`, `RebindObserver`, `RemoveObserver`.
  `expected_procedure_revision` becomes `expected_revision` of the target.
- **Audit.** `audit_fiscal_records(&Canwu)` and `audit_fiscal_record_set(records)`
  check every cross-record invariant, in `O(fiscal records)`.
- **Reads for hosts.** `fiscal_target_revision(&Canwu, &FiscalActionTarget)` and
  `fiscal_recent_outcome(&Canwu, &FiscalActionTarget, action_id)` read a
  record's revision and a recent outcome without assembling the state. They are
  host reads of ground truth, like reading the 0.15.0 state's procedure
  revision; an actor-facing controller gets the revision through its option
  descriptor, as before.
- **Receipts.** `FiscalExecutionReceiptPacket` and `FiscalExecutionEvidence`
  gain `scope_binding_id`; `enqueue_execution_receipts(canwu, due_at, &[packet])`
  replaces `enqueue_execution_receipt`.
- **Observers and reports.** `FiscalObserverBinding` gains the optional actor,
  granularity, cadence, and `controlled_by`; `FiscalReportGranularity`,
  `FiscalReportCadence`; report types gain cycles, `withdrawn`, and an optional
  scope in a fact; `compute_projections` becomes `compute_projection(state,
  observer_id, at)`; `FiscalPlugin::with_report_holder_kinds`.
- **Archive.** `prepare_fiscal_archive`, `PreparedFiscalArchiveBatchV1` with
  `store_and_verify` and `validate`, `VerifiedFiscalArchiveCommitV1`,
  `enqueue_fiscal_archive`, `finalize_fiscal_archive_retention`,
  `archived_fiscal_record`, `audit_fiscal_archive`,
  `live_fiscal_archive_lookup_pages`, `fiscal_archive_lookup_key_digest`,
  `FiscalArchiveStore`, `FiscalArchiveError` (with `UnsealedCitations`),
  `FiscalArchiveKeyV1`, `FiscalArchiveRecordV1`, the object types, and the
  `FISCAL_ARCHIVE_*` namespaces. No `resource_*` name or resource type appears
  in the fiscal API.
- **Constants and reasons.** The caps of [Every fiscal cap](#every-fiscal-cap);
  removed constants are deleted, not deprecated. New stable reasons:
  `FISCAL_REVISION_EXPIRED`, `FISCAL_REVISION_UNKNOWN`, `FISCAL_CYCLE_CLOSED`,
  `FISCAL_CYCLE_ARCHIVED`,
  `FISCAL_RECORD_ACTION_LIMIT`, `FISCAL_PARTITION_KEYS_EXHAUSTED`,
  `FISCAL_HOT_BOOKS_EXHAUSTED`, and the receipt refusal reasons above;
  `FISCAL_STATE_BUDGET_EXHAUSTED` becomes `FISCAL_RECORD_BUDGET_EXHAUSTED` (an
  action's target or derived record would pass its byte budget).
- **Book objects** (`FiscalAssessment`, `FiscalRemission`,
  `FiscalExecutionRequest`, `FiscalAuditFinding`) gain `source_action_id`;
  `FiscalExecutionReceipt` keeps its accepted ingress.
- **Dependents.** `canwu-ming-fiscal-reference` and its tests move to the new
  API; `canwu-fiscal` depends on `canwu-resource` for the archive index.

## Measurement plan

Phase 0 is measured by `record-write-profile`
([Phase 0 results](#phase-0-results)). Phases 1 and 2 add a harness binary,
`fiscal-world-profile` in `benchmarks/performance-harness`, through the public
API only, deterministic from a seed, with generated identifiers
(`scope.000017`, `office.0003`) and no application vocabulary. It runs
`audit_fiscal_records` after every boundary it settles (outside the timed
part), so every profile is also a check of the cross-record invariants. Phase 1
measures what its single record allows (the reference run and the 0.15.0
comparison below); the presets marked for scale are Phase 2's.

**Generator.** Partitions of institutions with heavy-tailed scope counts (the
largest 6,000), a configurable number of central institutions and regional
offices per partition, observers per partition (one at institution
granularity seeing all of it, one at scope granularity per regional office),
one to three mechanisms per scope, annual or monthly cycles, burst or spread
arrival, the archive cadence, and a catalog of one region per partition.

**Presets.** `smoke` (20 partitions, 2,000 scopes); `world` (200 partitions,
50,000 scopes, 4,200 institutions, 4,000 observers, annual); `world-monthly`;
`world-wide` (one institution per scope); `decades` (the annual world for 100
cycles with an archive commit cadence, and the monthly world for 10 years).

**Measured, elapsed and allocations in separate release builds,** median and
p95 over samples, per boundary kind:

- idle boundary (one tick of another plugin, nothing fiscal due);
- assessing boundaries of 64, 1,024, 4,096, 16,384, and 50,000 entries (burst);
- a spread day of the annual and of the monthly world;
- receipt boundaries of 1,024 and 16,384 receipts;
- a close boundary of 50,000 books;
- report boundaries with 64, 134, and 4,000 observers due (with their drains),
  records and bytes published;
- archive: prepare, store and verify, enqueue, and the settling boundary, for
  commits of 256 and 4,096 books;
- the engine's cost of a ledger write, as Phase 0 measured a generic record's,
  for the ledgers Phase 2 actually writes;
- fiscal record bytes (total and largest per kind), snapshot bytes, the largest
  record against the paged-checkpoint page, restore time (activation), and
  exact replay of a 30-day window;
- archive growth at 10, 25, 50, and 100 years: objects and bytes written,
  lookup pages reachable and superseded, miss and hit lookup time, and
  `audit_fiscal_archive` time.

**Before.** 0.15.0 runs only inside its caps: at most 4,096 scope bindings, 64
observers, 4,096 assessments, 16,384 outcomes, 64 actions a boundary. The
before-run builds the same generator against 0.15.0 at the largest world it
admits (one cycle of 4,096 scopes, 64 observers) and records per-boundary time
for assessing (64 actions per boundary), receipt, report, and idle boundaries,
and the point where each cap stops it (the 4,097th assessment rejected, the
16,385th outcome refused at admission, a 65th observer refused at activation).
The 0.15.0 `fiscal-profile` baselines are the second before-reference. The
copy of the generator that compiles against 0.15.0 is described in the
proposal and not kept in the tree, as the resource proposal did.

Baselines are recorded under `benchmarks/baselines/` as
`2026-10-<dd>-fiscal-world-<preset>-<mode>.json`.

## Acceptance tests

Durable tests are named by contract under `crates/extensions/canwu-fiscal/tests/`.
Each fails to compile or fails on v0.15.0, and each is bite-checked by
reverting its fix in the working tree.

**Phase 1** (single state record):

| Test | What it holds |
| --- | --- |
| `fiscal_action_idempotency.rs` | A re-send refused while its outcome is in the ring (`IdempotencyConflict`) and after eviction (`FISCAL_REVISION_EXPIRED`); a revision above the current one refused (`FISCAL_REVISION_UNKNOWN`); two commands with one action ID admitted before either settles apply once (the second is skipped); a batch naming an ID twice is refused; an eviction inside one boundary lets no copy through. |
| `fiscal_cycle_archive.rs` | Close, archive (several blobs), both watermarks refuse a new book, a hot book below the office watermark is unaffected, institution totals archived once by the commit that empties the cycle in any order, lookup of a book and of institution totals, a book with citations newer than the last sealed cut refused at preparation, stale and duplicate commits terminate, acknowledgement twice is a no-op, commits settle before actions, sealed run = unsealed fork = restore from sealed segments = exact replay. |
| `fiscal_archive_without_store.rs` | With an archive store that fails every read, every boundary of a run that closes, archives, settles receipts, and publishes reports has the same receipts, records, and hashes as with a working store. |
| `fiscal_archive_lifetime.rs` | Past every 0.15.0 lifetime cap: more than 4,096 assessments, 16,384 outcomes, 8,192 execution requests and receipts, and 4,096 auditor proposals over many cycles and commits; nothing is refused for lifetime. The full size runs in a release build (`-- --ignored`); the debug suite runs the same pipeline for 8 rounds and holds that the state stops growing ([Phase 1 as implemented](#phase-1-as-implemented)). |
| `fiscal_observer_bindings.rs` | A government holder without a person; re-pointing keeps every other binding's lines and gives the new holder the whole report; the old holder's report is withdrawn; removal; line slots never reused; a dead holder's lines owed and published on revival. |
| `fiscal_receipt_refusals.rs` | Each receipt and historical-context refusal reason is an event, settles the packets after it, and changes nothing; no receipt or context packet fails a boundary. |
| Reference run (`canwu-ming-fiscal-reference`, `archive_century.rs`) | 100 simulated years with an archive cadence: the state stays inside 32 MiB and does not grow with the run, and its size over the run is recorded ([Phase 1 as implemented](#phase-1-as-implemented)). |

**Phase 2** (partitioned records):

| Test | What it holds |
| --- | --- |
| `fiscal_partitioned_state.rs` | Random sequences of batches, receipts, closes, archive commits, transitions, and context changes: after every boundary `audit_fiscal_records` passes and the records equal what `into_records` derives from the assembled state; restore and exact replay agree. |
| `fiscal_record_revisions.rs` | An action on one scope is not stale for activity on another; it is stale for its own scope; the overwrite rule within a boundary, including ledgers a transition changed; per-target action IDs. |
| `fiscal_action_batches.rs` | A batch of 4,096 entries over many scopes settles in one boundary with one outcome and one event each; per-record and per-boundary limits reject as outcomes; the role is re-checked after a rebind entry; an inadmissible entry refuses the command with its index and reason. |
| `fiscal_report_lines.rs` | Lines on observer records; publication of changed parts only (digests); more than the engine's per-boundary publications drain at the same instant with identical `as_of`; withdrawn parts when a cycle is archived. |
| Format and tamper (unit) | Record shapes, report schema hash, office key, and every activation refusal of the cross-record invariants. |

**Phase 3** (later): `fiscal_report_cadence.rs` (interval observers publish only
at their due times and only changed parts, queue pages, parked rotation) and
institution granularity.

Existing fixtures (`gap_g41`, `gap_g44`, `gap_g48`, `gap_g57`, `gap_g58`,
`fiscal_report_parts`, `fiscal_reports_after_seal`, the 0.16.0 package's
`fiscal_action_ingress_origin` and `fiscal_audit_appointment`) are moved to the
new API with their contracts; the ones whose contract this design changes
(the global revision, lifetime caps, report lines) are rewritten, and the
proposal lists each.

## Beyond fiscal: caps that block the target

Measured or read against the same world (50,000 scopes, about 4,200
institutions, 4,000 observers, daily boundaries for 100 years). Not designed
here; the owner decides which join this release. Sizes: S (a constant and its
tests), M (one crate, days), L (a crate's storage or settlement redesign), XL
(several crates or the engine's persistence).

| Area | Cap or behavior | Why it blocks | Rough fix |
| --- | --- | --- | --- |
| Engine knowledge | 64 publications per system per boundary; 10,000 records per boundary over all plugins | thousands of report holders need tens of same-instant boundaries | raise or redefine the per-system limit: S to M |
| Engine knowledge | superseded records stay in every holder's ledger for good (the 0.16.0 knowledge package removes their per-boundary cost, not their memory and save size) | 4,000 observers × monthly reports × 100 years is gigabytes held in memory and in saves | knowledge retention and archive: L |
| Engine record writes | measured in Phase 0: about 200 µs per written record plus 134 to 153 µs per KiB of structured JSON (34 µs per KiB for flat text), rising at 50,000 writes in one boundary; the encodings and hashes of the write account for a sixth to a quarter | a monthly day of the county-level world costs 3.5 to 4.5 s and a burst 25 s in the engine alone | a cheaper write path; where the time goes is not yet located, and the handling of the payload tree is the first suspect: L |
| Engine idle floor | measured in Phase 0: every boundary costs about 0.5 to 1.7 µs per stored domain record (35 to 113 ms at 65,536 records, 5 to 6 ms at 16,384), whichever kinds its systems declare | every boundary of a world with tens of thousands of records, of any plugin, pays tens of milliseconds before any work | per-boundary work proportional to what changed, not to the store: M to L (cause not yet located) |
| Engine persistence | confirmed in Phase 0: a record whose payload is larger than 1,151,023 bytes does not fit a 4 MiB paged-checkpoint page (`value_bytes` written as a JSON number array) | every extension whose single root record grows past it (resource, production, society, culture, law plan and directory, the fiscal catalog) cannot take a paged checkpoint | a compact byte encoding or chunked leaves: S to M |
| Engine sealing | `seal_evidence` requires an empty ingress queue, future-dated packets included | no plugin can keep a timer as a future packet | allow future-dated plugin ingress across seals: M |
| Resource (`ResourceLimitsV1`) | accounts 8,192 canonical, 262,144 hard; definitions and unit revisions 1,024; access grants 4,096 never archived; allocation candidates 2,048; reports 256 and grants 1,024; one root record decoded, validated, and journaled by every changing boundary (about 50 µs per hot record) | 50,000 scopes × several resources is about 250,000 accounts in one record rewritten every day | shard the root by account holder: XL |
| Production (G-18) | sites 2,048; facilities, work orders, executions, projects 4,096; outcomes 8,192; incident receipts 8,192 for life; 64 reports and 512 incidents a boundary; limits fixed in code; one root record | county-level production is tens of thousands of sites | the filed fix (at least 16,384, or shard by site, plus a batching contract): L |
| Society | no count caps, but its state, ledger, and pending records are whole roots rewritten at every settlement (daily by default), and the ledger keeps every outcome for good | cohorts for 50,000 scopes in one record, growing for 100 years | shard and archive the ledger: L to XL |
| Law (G-30) | sharded, but the plan and directory records grow with jurisdictions (default budget 512) and are decoded every boundary; 128 MiB global hot-state budget; 512 mutations a boundary; outbox ID blocks for life; no aggregate caseload (G-30 filed fix) | 50,000 jurisdictions push plan and directory past the record ceiling; caseload per county needs the aggregate | shard plan and directory, add the aggregate caseload record: M to L |
| Routing (G-24) and transport | no stored caps; every route query rebuilds the adjacency index and hashes the whole network; 10,000 expanded nodes by default; no recurring timetables (G-24 filed fix) | each query costs the size of a county-level network | a compiled, cached network index: M; recurring timetables: S to M |
| Movement | 1,024 orders, 256 per owner, 64 report holders a boundary, fixed; one root record | world-scale movement | L |
| Military | 4,096 records per kind, never deleted (a lifetime cap); an outcome ledger that grows for good | 100 years | archive: M to L |
| Technology | 5,000 records in total for life; every record loaded each boundary | 100 years at world scale | L |
| Information, correspondence | one record per object, never deleted; information loads all its records each active boundary | grows for 100 years | archive and indexed reads: M to L |
| Culture | author budgets (10,000 targets and cohorts) in one state root | county-level cultural targets | shard: L |

## What can still fail a boundary

In the fiscal extension, after this design:

- an invalid committed fiscal record (activation and every write exclude it);
- counter overflow (64-bit);
- from Phase 3 only, an engine refusal the plugin cannot foresee: a report
  published to a domain-entity holder that a system of another plugin retires
  in the same phase 13. The report system checks the holder when it runs, and
  the engine orders systems within a phase, so the residual window is a system
  of another plugin that both writes in phase 13 and retires knowledge holders.
  Phases 1 and 2 have no domain-entity holders, so no such case.

An action never fails a boundary: it is applied, rejected as its own outcome,
or dropped with `canwu.fiscal.action_dropped.v1` when even its rejection cannot
be recorded (its record at its byte budget). A receipt or historical-context
packet never fails one either: it settles or is refused with
`canwu.fiscal.ingress_rejected.v1`. Neither does an archive packet: a commit
or acknowledgement that is malformed or forged, a commit that does not bind
its selection, and a stale commit the head has no room for are refused with
the same event and change nothing. A report that cannot be published now
stays pending.

## Risks, open decisions, and questions for review

**Main risks.**

1. **The engine's cost of a record write and of a large store.** Phase 0
   measured both ([Phase 0 results](#phase-0-results)): about 200 µs per
   written record plus about 140 µs per KiB of structured JSON, and an idle
   floor of 0.5 to 1.7 µs per stored record in every boundary. With them a
   monthly day of the county-level world costs 3.5 to 4.5 s in the engine
   alone and an annual day 0.3 to 0.4 s. Phase 2 makes 0.16.0 able to run that
   world, which no earlier release could, but not at the player-lag
   diagnostics until the engine items under
   [Beyond fiscal](#beyond-fiscal-caps-that-block-the-target) are done.
2. **Size of the change.** Phases 1 and 2 replace the storage and settlement of
   the whole crate and land on top of the 0.16.0 fiscal and society package
   (appointments, refusals, drain, internal packet). The logical `FiscalState`
   with `into_records`, `audit_fiscal_records` after every boundary in tests and
   in the harness, and the partition test are the safety net.
3. **Invariants by construction.** Cross-record invariants are checked in full
   only at activation and by the audit; on writes they are maintained by
   construction with cheap local checks. A bug there corrupts silently until the
   next audit or restore, which is why every test and profile audits after every
   boundary.
4. **The paged-checkpoint ceiling.** Confirmed: a record whose payload is
   larger than 1,151,023 bytes cannot be written into a paged checkpoint. Phase
   1 keeps the state in one record, so a Phase 1 run whose state passes that
   size cannot take a paged checkpoint until Phase 2 partitions it. It affects
   every extension that keeps its state in one record, and a world catalog
   larger than the ceiling needs splitting.
5. **Engine knowledge limits.** Thousands of observers lean on same-instant drain
   boundaries until the engine's per-system publication limit changes; Phase 3
   waits for that.
6. **Crate dependency.** `canwu-fiscal` gains `canwu-resource` as a dependency for
   the index functions (hidden behind the fiscal archive trait), and the
   implementation needs the production package's change to `canwu-resource` in
   the integration branch (or cherry-picks it unchanged).

**Open decisions taken conservatively** (reverse any of them in review):

- A historical-context change no longer makes actions stale (Phase 2; Phase 1
  keeps the global revision, so it still does there).
- External-operation uniqueness is checked within a scope's hot books, not
  across the world.
- An action ID reused for a different decision is detected only while the
  earlier outcome is in the ring; action IDs are per target.
- Accounting-cycle IDs order by bytes; no explicit ordinal.
- Observer bindings can be re-pointed and removed at runtime, not installed, and
  their visibility, granularity, and cadence are the scenario's.
- Due reports are published by the next event-bearing boundary at or after
  their due time; no configurable report-system cadence (Phase 3).
- Audit findings are allowed on a closed book until it is archived.
- Outcomes beyond the 8-entry ring live only in their settled events, one event
  per settled entry.
- Every receipt and historical-context failure is a recorded refusal (event),
  not a boundary failure, and a refused receipt leaves no record in the state.
- The evidence versions an archived book cites stop being payload-required for
  the live run; preparation refuses a book whose citations are not yet sealed.
- Domain-entity report holders wait for Phase 3.

**Review record.** The independent review of the first version
("implement with changes, with phasing") raised three blocking findings,
nine changes, and seven notes. All were accepted:

| Finding | Where it is resolved |
| --- | --- |
| The office watermark was defined two incompatible ways | [Two watermarks](#accounting-cycles-open-close-archive); cross-record invariants 7 and 8 |
| At-most-once had two holes (same-boundary copies; the floor's off-by-one) | [Idempotency without lifetime memory](#idempotency-without-lifetime-memory) |
| The cost gate measured the wrong quantity (leaf and branch pages, affected-record validation, boundary hash) | [Cost model](#cost-model) with [Phase 0 results](#phase-0-results); a ring of 8 holding no reason |
| Missing cross-record invariants, cheap per-write checks, a host audit, the audit after every boundary | [The logical state and its validation](#the-logical-state-and-its-validation) |
| Ledgers changed by `ApplyTransition` | [Revision per record](#revision-per-record) |
| Report-queue slots keyed by (due time, page); publication of only the parts a queue entry names | [When reports are published](#when-reports-are-published) |
| Parked observers re-checked at a bounded rate | [Bindings](#bindings) (Phase 3 rotation) |
| Required evidence of archived books | [Required evidence and sealing](#the-fiscal-archive-g-15) |
| Whole-command refusal names the entry and reason | [Throughput](#throughput) |
| The resource index behind a fiscal-owned trait and error | [Machinery, reused](#machinery-reused) |
| Domain-entity holders out of the first phases | [Bindings](#bindings) |
| The event shape | [Throughput](#throughput): one event per settled entry |
| Notes: action IDs per target, a stale flood, prepare and enqueue in one gap, drains and sealing, an idle-boundary number, superseded lookup pages, the question count | the sections named above, [Phase 0 results](#phase-0-results), and [Host guidance](#machinery-reused) |
| Phasing | [Phases](#phases), as decided by the owner |

**Questions that remain open for review.**

1. Is the scope ledger the right shard, now that Phase 0 shows a write's cost
   follows the record's JSON values? Would per-(scope, cycle) book records, or a
   compact book encoding, be worth their bookkeeping from the start of Phase 2?
2. Does any client depend on looking up an old outcome by action ID alone, now
   that only the event keeps it?
3. Does dropping the context revision from staleness (Phase 2) lose a
   protection anyone relies on?
4. Should external-operation uniqueness be global, at the price of a hot index
   or archive reads?
5. Is a fixed `MAX_FISCAL_OBSERVERS` (65,536) an acceptable answer to the filed
   "configurable" fix?
6. Should a refused receipt leave a record in the state (the 0.16.0 package
   left this open), now that a ledger could hold one per request?
7. Should the engine changes that Phase 0 points to (the per-write path and the
   per-boundary cost of a large store) be scheduled before the county-level
   world is used at monthly cycles?

## Documentation delta

For the orchestrator, after implementation:

- `docs/architecture.md` (fiscal procedure): the partitioned records, per-record
  revisions, batch command, cycles and watermarks, the archive, observers with
  optional actors, report cadence and lines on observer records, and that no
  boundary reads the fiscal archive.
- `docs/versioning.md` (0.16.0 entry): the format items of
  [Persistence](#persistence-replay-seal-independence-and-formats), the removed
  lifetime caps, and the open engine items of
  [Beyond fiscal](#beyond-fiscal-caps-that-block-the-target) as known limits.
- `docs/terminology.md` and both website term pages: candidate terms
  "scope ledger" (范围账), "accounting cycle book" (会计周期账册), "fiscal
  archive" (财政归档), "report cadence" (报告节律), and "institution holder"
  (机构持有者), to be confirmed against the term sheet's style.
- `crates/extensions/canwu-fiscal/README.md` and
  `docs/proposals/fiscal-throughput-and-authority.md`: brought up to date with
  the contracts this design changes.
