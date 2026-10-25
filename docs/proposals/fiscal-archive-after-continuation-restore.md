# Fiscal Archive after a Continuation Restore

Status: implemented in 0.21.0. One change to a host-facing read in
`canwu-sim`, reached through `canwu-api`, and the documentation of the fiscal
archive helpers in `canwu-fiscal` that read it (downstream gap G-91). No
state, record, checkpoint, or plugin format changes. Same version only: no
compatibility work with 0.20.0 bytes or hashes.

## Problem

A host that saves only by continuation checkpoint and never seals cannot
archive a closed book that cites a version established since its last
restore. Verified in the 0.20.0 source:

- `Simulation::sealed_evidence_cursor` (`canwu-sim`
  `runtime/persistence.rs`) answers the runtime's archived cursor, "where its
  last seal ended, or the settled cut of the continuation checkpoint it was
  restored from". `restore_continuation` (`runtime/continuation.rs`) sets
  the archived cursor to the checkpoint's settled cut.
- `prepare_fiscal_archive` (`canwu-fiscal` `plugin.rs`) passes that cursor to
  `FiscalState::prepare_archive_batch` (`archive.rs`), which refuses a
  selected book that cites a version a boundary after the cut established
  (`FiscalArchiveError::UnsealedCitations`).
- `prepare_fiscal_archive`'s own note says a run that retains all its
  evidence refuses none, and the reason it gives for the refusal is a sealed
  segment, which a continuation restore does not have.

So every book closed in a session from one restore is refused. It waits in
its ledger until a later restore's cut passes it, and a ledger that closes a
book a month fills its `MAX_FISCAL_HOT_BOOKS_PER_LEDGER` (64) hot books about
five years after the restore; the next book's open is then rejected with
`FISCAL_HOT_BOOKS_EXHAUSTED`.

## What the refusal protects

**On a run that sealed.** While a book is hot its ledger record declares
every boundary-established version the book cites (its assessments'
commutation quotes and its receipts' evidence) as a payload-required
evidence continuation (`records.rs` `ledger_continuation`). That declaration
is what keeps each version provable inside the run once the boundary that
established it is sealed: a seal pins the body of a declared version that is
no longer current (`PinnedEvidence::record_versions`), keeps an archived
evidence receipt for it (`reachable_archived_receipt_references`), and every
checkpoint, compact or continuation, carries what the declaration needs; a
restore requires every declared version to exist at its cut
(`continuation_needs`, `ArchiveNotReady` otherwise).

Archiving a book takes it out of its ledger, so the ledger's next version
declares nothing for it. From then on the run keeps nothing of those
versions: the next seal drops their receipts and pins, a continuation
checkpoint carries none of them, and no rule reads them (no boundary reads
the archive store). The archived book names each citation by identity only
(`DomainRecordVersionRef`: record, version number, establishing boundary; no
body and no digest), so what the cited quote or evidence said, and that it
was established, is recorded only in the boundary record that established
it. On a sealing run that record leaves the live run at a seal into a
segment the host keeps, authenticated by the run's segment manifest. The
refusal makes sure the record is already in such a segment when the
declaration ends: a book is archived only when every citation's boundary is
at or below the run's last seal.

What is lost without it: a citation newer than the last seal sits in the
retained tail. It is provable from the live run, and it is in a segment
after the next seal, unless the host writes a continuation checkpoint first
and restores from it. The closure of that checkpoint no longer covers the
version, so the restored run, and every segment the host keeps, lacks the
boundary that established it: the archived book cites evidence nothing the
host holds proves any more.

**On a run restored from a continuation checkpoint.** The checkpoint carries
current state and the continuation closure: for every declared version that
is neither current nor initial, its membership, and its body when it is
declared payload-required. It carries no segment manifest and no archived
receipts, and the restore installs none (`archived_segment_headers` and
`archived_evidence_receipts` are empty). The restore validates the versions
current records declare against what the checkpoint carries, and nothing
else. Such a run therefore holds no sealed segment at all: everything before
its base exists only as the closure.

The 0.20.0 check treats the base as a seal. It passes every citation at or
below the base, none of which is in a segment this run or its checkpoint
knows, and refuses every citation above it, which sits in the retained tail
exactly as on a run that has never sealed. A refused book is archived after
the next restore, when its citations are below the new base and in no more
of a segment than before. The check protects nothing on such a run; it only
delays the archive, and the delay is what fills the ledger.

When a continuation-restored run seals, its segments start at its base and
its own manifest authenticates them. From then on it is a sealing run, and
the protection applies to citations past its own last seal.

## Invariant

- A run that has sealed since it was created or restored archives a book
  only when every version the book cites was established at or below its
  last seal. Unchanged from 0.20.0.
- A run that has not sealed since it was created or restored refuses no
  citation: its archived books' citations reach a sealed segment only at its
  next seal. A continuation restore counts as not sealed. This is the rule
  0.18.0 gave every `Canwu`, every never-sealed `CompactedCanwu`, and every
  run restored with its whole journal, now applied to the one restore it
  missed.
- The refusal stays a host-facing preparation refusal. No boundary reads the
  cut, so a sealed run, its unsealed fork, a restore, and the exact replay
  settle alike.

## Design

`sealed_evidence_cursor` answers where the run's own last seal ended:

```rust
// canwu-sim, impl Simulation (and the delegating CompactedSimulation method)
pub fn sealed_evidence_cursor(&self) -> Option<EvidenceCursor> {
    let evidence = &self.state.evidence;
    (!evidence.archived_segment_headers.is_empty()).then_some(evidence.archived)
}
```

Only a seal adds a segment header, every seal that moves anything adds one,
and no restore installs one (a checkpoint-and-journal restore rehydrates the
whole history at the zero cut; a continuation restore starts with none). So
the headers are the run's own seals, and when there is one the archived
cursor is where the last of them ended. The answer is unchanged for every run
except one restored from a continuation checkpoint that has not sealed since:
it answered the checkpoint's settled cut and now answers `None`.
`CompactedCanwu::continuation_base()` still reports where such a run's
history begins; it is set once at the restore and kept after the run's first
seal, so where the retained tail starts is the last seal's end or, before
one, the base (see [Upgrading notes](#upgrading-notes)).

`canwu-fiscal` is unchanged in code: `prepare_fiscal_archive` and
`enqueue_fiscal_archive` read the cut through `CanwuRun::sealed_evidence_cursor`
as before. Its note, the `UnsealedCitations` rustdoc, and the crate README
now say the same thing as the read: refused past the run's last seal; a run
that has not sealed since it was created or restored, a continuation restore
included, refuses none.

### Alternatives not taken

- **Record in the continuation checkpoint whether its cut was sealed**, so
  that a host that seals at each save keeps the refusal through its first
  session after a restore. A continuation checkpoint is a function of state
  and history at its cut, byte-equal whether the run that wrote it sealed or
  not (the continuation gate), so it cannot carry the writer's seals.
- **Let the archive commit carry what its books' citations need** (the
  bodies of the cited versions) so that no cut has to cover them. The blob
  would hold other domains' record bodies (up to 1,024 receipts a book, each
  with its evidence), settlement would have to bind bodies it reads only
  through pins (a replay-stable read answers only current and initial
  versions), and the blob format, its budgets, and the commit's binding
  would change. A sealing host already has its segments, and a host that
  keeps no segments has nothing a body copy would be checked against. Larger
  than the gap.
- **A seal a continuation host makes and discards.** Already possible, but
  the segment is thrown away, so the refusal would protect nothing, and the
  host would pay a seal for each archive commit.

## Persistence and replay

Nothing persisted changes: no snapshot field, checkpoint, continuation
checkpoint, segment, record, schema, plugin descriptor, or hash. The read is
derived from the segment manifest and the archived cursor the runtime already
holds. Forks answer as their source does. A continuation checkpoint written
by a run that archived books citing versions after its base no longer
carries those versions (the ledger no longer declares them) and restores;
the restored run reads the archived books from the store through its
archive head.

## Refusals

`FiscalArchiveError::UnsealedCitations` is unchanged in shape and meaning on
a run that has sealed. A continuation-restored run no longer meets it until
it seals. The other preparation refusals (`Backpressure`,
`NothingToArchive`, `Mismatch`) are unchanged.

## What can still fail a boundary

Nothing new. The cut is read only by host preparation; no boundary system
reads it.

## Known limits

- A host that saves only by continuation checkpoint and never seals keeps no
  segment, so the versions its archived books cite are in nothing it holds
  once their books leave the run: the archived book names them by identity.
  This was already true for every citation at or below a restore's base, and
  for every citation of a never-sealed run's first session.
- A host that seals at each save archives, in its first session after a
  continuation restore and until its first seal there, books whose citations
  its next seal puts in a segment rather than one that already holds them,
  as in a fresh run's first session. Seal at each save, before the
  continuation checkpoint, keeps every archived citation in a segment.
- A host that took `UnsealedCitations` as "not yet" keeps a harmless arm: it
  no longer fires on a continuation-restored run, only on a run that sealed.

## Upgrading notes

A host that restores from continuation checkpoints and never seals archives
every closed book again; its ledgers no longer fill after a restore. A host
that read `sealed_evidence_cursor()` for where a run's retained evidence
begins gets the 0.20.0 answer with
`run.sealed_evidence_cursor().or_else(|| run.continuation_base().map(|base| base.cut))`,
which needs a `CompactedCanwu` (`continuation_base` is not on `CanwuRun`;
a base at the zero cut answers `Some` of it, where 0.20.0 answered `None`).
`continuation_base()` alone is not that answer: it is set once at the restore
and kept after the run's first seal, so it names where the run's history
began, not where its retained tail starts. A sealing host changes nothing.

## Public API delta

- `Simulation::sealed_evidence_cursor`, `CompactedSimulation::sealed_evidence_cursor`,
  `Canwu::sealed_evidence_cursor`, `CompactedCanwu::sealed_evidence_cursor`,
  and `CanwuRun::sealed_evidence_cursor`: same signatures; the answer is the
  end of the run's own last seal, `None` until a run seals after it is
  created or restored (a continuation restore included). Rustdoc updated.
- `CompactedCanwu::journal_segment_since`: rustdoc only (its lower bound is
  the first evidence the run retains, the continuation base before a
  continuation-restored run seals).
- `canwu-fiscal`: rustdoc of `prepare_fiscal_archive` and
  `FiscalArchiveError::UnsealedCitations`.

## Durable tests

- `canwu-fiscal` `fiscal_cycle_archive::a_continuation_restored_run_archives_citations_newer_than_its_base_until_it_seals`:
  a run restored from a continuation checkpoint answers `None`, archives a
  closed book whose commutation quote a boundary after the restore
  established, writes a continuation checkpoint after the commit that
  restores with the same checkpoint hash, and reads the archived book, its
  citation included, from the store through the restored head. The restored
  run then seals, and a book citing a quote newer than that seal is refused
  with `UnsealedCitations` until a second seal covers it. Under 0.20.0 the
  first archive is refused.
- `canwu-fiscal` `fiscal_cycle_archive::a_book_citing_evidence_newer_than_the_last_seal_is_refused_until_it_is_sealed`
  (unchanged): what the rule keeps for a run that sealed.
- `canwu-api` `continuation_checkpoint::a_continuation_restored_run_continues_in_lockstep`
  now pins `None` for both continuation restores until their own first seal,
  then the end of each seal, beside the unsealed, sealed, and full-save runs.

## Documentation delta

- `docs/architecture.md` (the fiscal archive paragraph), `crates/extensions/canwu-fiscal/README.md`,
  and the rustdoc above: updated in this change.
- For the release: `docs/versioning.md` (the release notes below); the
  website pages `architecture/fiscal.mdx` in both languages (the
  "sealed cut comes from the run" item says a continuation restore answers
  its settled cut and is refused after a restore: replace with the text
  below); `developer/persistence.mdx` in both languages is still true.
  `docs/terminology.md` and both terminology pages are still true ("the
  run's own sealed cut").

Replacement for the fiscal page's item (English):

> The sealed cut comes from the run. A book citing an evidence version
> established after the run's last seal is refused at preparation
> (`FiscalArchiveError::UnsealedCitations`) until a seal covers it, because
> archiving a book ends the ledger's payload-required declaration of the
> quotes and receipt evidence it cites. The helpers read that cut through
> `sealed_evidence_cursor()` (on `Canwu`, `CompactedCanwu`, and any
> `CanwuRun`): the end of the run's last seal, and `None` until a run seals
> after it is created or restored. A run that has not sealed (every `Canwu`,
> a `CompactedCanwu` that has never sealed, a run restored with its whole
> journal or from a continuation checkpoint, until its next seal) refuses
> nothing: a continuation restore holds no sealed segment, so its base is
> not a seal. A sealing host therefore **seals at each save**, before it
> writes a continuation checkpoint, if every archived citation is to lie in
> a sealed segment; a host that saves only by continuation checkpoint and
> never seals archives freely in every session.

(Chinese):

> 封存切点来自模拟运行本身。引用了运行最近一次封存之后建立的证据版本的账册，会在准备时被拒绝（`FiscalArchiveError::UnsealedCitations`），直到某次封存覆盖它为止，因为归档账册会结束范围明细对它引用的折纳报价和凭证证据的载荷保留声明。辅助函数通过 `sealed_evidence_cursor()`（`Canwu`、`CompactedCanwu` 和任何 `CanwuRun` 都有）读取这个切点：运行最近一次封存的终点；运行在创建或恢复之后尚未封存时为 `None`。尚未封存的运行（每个 `Canwu`、从未封存的 `CompactedCanwu`、带完整日志恢复或从接续检查点恢复且尚未再次封存的运行）不会拒绝任何账册：接续恢复不持有任何已封存的日志段，所以它的起点切点不算封存。会封存的上层应用要想让每处已归档的引用都落在已封存的日志段里，就应当在**每次存档时先封存**，再写接续检查点；只用接续检查点存档、从不封存的上层应用，在每个会话里归档都不会遇到这种拒绝。

## Terminology candidates

No new public term. The existing wording is kept and made exact:

| English | 中文规范写法 | Meaning |
| --- | --- | --- |
| sealed cut | 封存切点 | Where a run's last seal ended (`sealed_evidence_cursor`); a run that has not sealed since it was created or restored has none. |
| continuation base | 起点切点 | The settled cut of the continuation checkpoint a run was restored from (`continuation_base`); not a seal. |

## Release notes text (for `docs/versioning.md`)

- **A continuation restore is not a seal** (downstream gap G-91;
  [proposal](proposals/fiscal-archive-after-continuation-restore.md)).
  `sealed_evidence_cursor()` answers where the run's own last seal ended and
  is `None` until a run seals after it is created or restored; a run restored
  from a continuation checkpoint answered the checkpoint's settled cut in
  0.20.0. So `prepare_fiscal_archive` no longer refuses, as
  `UnsealedCitations`, a book citing a version established after a
  continuation restore on a run that has not sealed since; a run that has
  sealed is refused past its own last seal as before.

Upgrading from 0.20.0: a host that saves only by continuation checkpoint
archives its closed books in every session again (a "not yet" arm for
`UnsealedCitations` may stay; it no longer fires there). A host that read
`sealed_evidence_cursor()` for where a run's retained evidence begins gets
the 0.20.0 answer with
`run.sealed_evidence_cursor().or_else(|| run.continuation_base().map(|base| base.cut))`
on a `CompactedCanwu` (`continuation_base` is not on `CanwuRun`);
`continuation_base()` alone stays set after the run's first seal. Nothing
persisted changes.
