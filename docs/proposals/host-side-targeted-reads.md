# Host-Side Targeted Reads

Status: implemented in 0.22.0. It closes downstream gaps G-92
(`canwu-api` with `canwu-knowledge`) and G-93 (`canwu-resource`): a host can
read a few current knowledge records, or a few named accounts and records of
the resource root, at the cost a boundary system pays for the same read. It
changes `canwu-knowledge`, `canwu-sim`, `canwu-api`, and `canwu-resource`.
No state, record, snapshot, checkpoint, hash, or plugin format changes. Same
version only: no compatibility work with 0.21.0 bytes or hashes.

## Problem

Inside a boundary both reads are indexed: `SimulationView::knowledge_heads`
walks a holder's index of current records, and `ResourceRootReader::load`
decodes the live root and attaches only the bucketed records a reader names.
A host has neither (verified in the 0.21.0 source):

- **G-92.** `CompactedCanwu::admin_query_knowledge` and
  `CanwuViewer::query_knowledge` call `KnowledgeSnapshot::query_current`,
  which hashes the holder's whole history into the read cut's
  `holder_projection_root`, and `query_records` checks every entry of the
  holder's ledger before it filters. Every host question about a holder costs
  O(ledger). `CanwuRun` has no knowledge read but `admin_query_knowledge`.
- **G-93.** `ResourceRootReader::load` takes only a `SimulationView`. Every
  host helper of `query.rs` calls `resource_state`, which walks every domain
  record of the run to list the buckets (`RunSource::bucket_references`) and
  decodes the live root and every bucket; a read after any change decodes
  everything again.

## Decision

Give the run handles the reads a boundary has, reading the committed state:

1. `knowledge_heads(holder, kind, subjects)` with the view's contract, and
   `knowledge_current_records(holder, kind, subjects)`, an iterator of the
   same current records as holder-relative `KnowledgeRecordView`s, newest
   first in publication order (global ID, not `learned_at`). Neither builds a projection or a hash; a host that needs a page
   bound to a read cut (a cursor) keeps `admin_query_knowledge`.
2. `ResourceRootReader::load_run(run)` with the reader's existing
   `attach`, `attach_live`, and `attach_custodian_accounts`, plus
   `attach_all` (the whole root, as `resource_state` reads it).
   `resource_state` and the key-closed `query.rs` helpers read through it.
3. An incremental re-attach: `ResourceRootReader::keep` turns a reader into
   an owned `KeptResourceRoot`, and `ResourceRootReader::reattach(run, kept)`
   repeats the kept reader's requests on the run's current root, re-reading
   only the buckets whose record is not the one the kept root decoded.
4. Two supporting run-handle reads: `domain_record_shared(reference)`, the
   run's own shared copy of a record, and `domain_records_of_kind(kind)`, the
   current records of one kind in reference order.

## Invariant

- For the same committed state, each host read answers exactly what the same
  read in a boundary system answers before anything is staged in it:
  `knowledge_heads` what `SimulationView::knowledge_heads` returns;
  `knowledge_current_records` the records a `CurrentHeads` page of
  `knowledge_records` returns when it names every schema version of the kind
  and the subjects, in another order;
  a reader from `load_run` the state `ResourceRootReader::load(view)` builds
  for the same requests.
- A re-attached reader equals a reader loaded fresh from the same run with
  the same requests, whatever happened between: steps, a fork that diverged,
  or a restore into another run. That includes failure: a request that
  failed on the kept reader is repeated, so the re-attach fails where a fresh
  reader fails.
- A bucket counts as unchanged only when the run holds the very allocation
  the kept root decoded (`Arc::ptr_eq` of `domain_record_shared`). The store
  replaces a record's `Arc` on every write and the kept root holds a clone,
  so an equal pointer means equal content in every run and fork; a version
  number does not (a diverged fork reuses version numbers), and a restored
  run holds new allocations, so it is re-read in full.
- The trait states the contract: `domain_record_shared` returns the
  allocation `domain_record` reads. The reader checks it (`ptr::eq`) and
  treats any other answer, a fresh copy or one cached by version, as no
  shared copy: read correctly, nothing reused.

## Ownership and affected surfaces

| Surface | Change |
| --- | --- |
| `canwu-knowledge` | `HolderLedger::current_record_views` (the views of `current_records`) |
| `canwu-sim` | the four reads on `Simulation` and `CompactedSimulation`, over the existing index, the record store's `get_shared`, and its ordered key pages |
| `canwu-api` | inherent methods on `Canwu` and `CompactedCanwu`; four required `CanwuRun` methods; `&self` only, so `gated!` is unchanged |
| `canwu-resource` | the reader's source is a run or a view; `KeptResourceRoot`; `resource_state` and eleven helpers read through the reader |
| `canwu-debug`, examples | none |

The reads stay on the run handles, never on `SimulationView` (which has its
own) or `CanwuViewer`. The API delta check gains a feature that calls
`knowledge_current_records` on a `CanwuViewer` and must not compile.

## API surface

```rust
// canwu-knowledge
impl HolderLedger {
    pub fn current_record_views<'a>(&'a self, kind: &'a KnowledgeRecordKind,
        subjects: &'a [KnowledgeSubject]) -> impl Iterator<Item = KnowledgeRecordView> + 'a;
}
// canwu-sim (Simulation, CompactedSimulation) and canwu-api (Canwu, CompactedCanwu)
pub fn knowledge_heads(&self, holder: &KnowledgeHolderRef, kind: &KnowledgeRecordKind,
    subjects: &[KnowledgeSubject]) -> Result<KnowledgeHeads, CanwuError>;
pub fn knowledge_current_records<'a>(&'a self, holder: &'a KnowledgeHolderRef,
    kind: &'a KnowledgeRecordKind, subjects: &'a [KnowledgeSubject])
    -> impl Iterator<Item = KnowledgeRecordView> + 'a;
pub fn domain_record_shared(&self, reference: &DomainRecordRef) -> Option<Arc<DomainRecord>>;
pub fn domain_records_of_kind(&self, kind: &DomainRecordKind)
    -> impl Iterator<Item = &DomainRecord> + '_;
// CanwuRun: the same four, the iterators as Box<dyn Iterator<..> + '_ / 'a>.

// canwu-resource
impl<'r> ResourceRootReader<'r, 'r> {
    pub fn load_run<H: CanwuRun + ?Sized>(run: &'r H) -> Result<Option<Self>, CanwuError>;
    pub fn reattach<H: CanwuRun + ?Sized>(run: &'r H, kept: KeptResourceRoot)
        -> Result<Option<Self>, CanwuError>;
}
impl ResourceRootReader<'_, '_> {
    pub fn attach_all(&mut self) -> Result<(), CanwuError>;
    pub fn keep(self) -> KeptResourceRoot;
    pub fn buckets_decoded(&self) -> usize;
}
pub struct KeptResourceRoot { /* private */ }   // Clone, Debug, Send; state()
```

`knowledge_heads` refuses more than `relations_per_record` subjects with
`KnowledgeLimitExceeded`, as the view does; it returns at most that many
global IDs, ascending, the newest when more match (`truncated`).
`knowledge_current_records` is unbounded: the caller takes what it needs and
pays for what it takes. Sorting its views by `(learned_at, id)` gives the
order of `admin_query_knowledge`.

A reader records what it was asked (`attach_all`, the terminal and live
keys, the custodians) as sets; `reattach` repeats them. The answer does not
depend on order: each request attaches a closure keyed by identity.

## Authority

Trusted host only. The knowledge reads return any holder's records with
their payloads, and the resource reader ground truth for every custodian.
Their rustdoc says, as `pending_ingress` does, that the host selects its own
items (the records of the holder whose actor it shows, a holder's own
accounts) before anything reaches an actor, whose reads remain
`CanwuViewer::query_knowledge` and the holder-relative `resource_report`.

## Determinism

The reads change nothing and depend only on committed state. Allocation
identity decides only whether a decode is reused, never an answer: the reused
decode is the same function of the same bytes. `buckets_decoded` does depend
on the process's history (forks share records, a reload copies them): a
diagnostic, never a simulation input. The converted helpers keep their
signatures. With `canwu-resource`'s `verify-targeted-reads` feature, off by
default, each also reads the whole root and panics on a different answer.
Canwu's tests enable it through path-only dev-dependencies (`canwu-resource`
on itself, and `canwu-production`, `canwu-economy-reference`,
`canwu-force-supply-reference`, not published), so `cargo test --workspace`
checks every targeted read in every suite that calls them; an application
build has no such check, cost, or panic.

## Cost and complexity

- `knowledge_heads`: the view's walk, O(log n) per match plus skipped
  candidates, at most `relations_per_record + 1` matches; no ledger check, no
  hash. `knowledge_current_records`: the same per record taken, plus one
  payload clone.
- `domain_records_of_kind`: a binary search of the key pages plus the
  records of the kind; `domain_record_shared`: one lookup and an `Arc` clone.
- `load_run` and a targeted attach: the live root's decode plus, per named
  record, its bucket (2 to 4 entries) and the entry. `attach_all`: lists the
  buckets by kind instead of walking every record.
- `reattach`: one shared-record lookup per bucket the kept reader read. With
  the live root and every such bucket unchanged it decodes nothing. After the
  live root changes (nearly every step that settles resource work) it decodes
  the live root and the changed or new buckets, but attaches every record
  the requests reach again from the kept decodes and re-runs `attach_all`'s
  checks: O(attached records), not O(changed). A host that needs O(changed)
  reads targeted records.
- Measured by the review in a debug build, on a root of 1,644 buckets (700
  extra accounts, 300 hot rejections), five calls each: a whole read 65 ms;
  `load_run` with one attach 0.13 ms; a targeted helper with the whole-root
  check (now behind the feature) 64 ms; a re-attach with nothing changed
  5.4 ms; one after a step that changed 2 buckets 22 ms. Release builds were
  not measured.
- Memory: a kept root holds the state it read, the decoded entries, and an
  `Arc` per bucket it read; a changed bucket's old version stays alive until
  the next `reattach` releases it.

## Persistence and replay

Nothing persisted changes: no snapshot field, checkpoint, journal, record,
schema, plugin descriptor, semantic hash, or format number. The reads are
outside every boundary; a sealed run, its unsealed fork, a continuation
restore, and an exact replay answer alike (tested). A kept root is host
memory, never saved; after a restore the host loads afresh or re-attaches,
which re-reads everything.

## What can still fail

No boundary: the reads are host-side. `knowledge_heads` refuses more than 64
subjects. A reader fails, as the view's does, on a malformed live root or
bucket it reads; `attach_all` also on ledgers that disagree with the stored
entries or a broken demand closure. The converted helpers no longer run
those whole-root checks: a host that relied on a helper to detect a
corrupt root calls `resource_state` or `validate_resource_runtime`.
`reattach` fails where a fresh read fails, and returns `None` when the run
has no resource root.

## Alternatives not taken

- **A hashless `admin_query_knowledge`.** The page still checks and filters
  the whole ledger, and its result carries the read cut it would no longer
  compute.
- **Compare bucket versions.** Cheaper to state, wrong across a diverged fork
  or another run with the same version numbers.
- **Compare bucket payloads.** Exact, but O(root bytes) on every re-attach.
- **A second reader type for runs.** The view reader's closure code is the
  thing to share; one type keeps the methods and their meaning identical.

## Known limits

- The ten in-boundary `knowledge_records` pages the downstream lists still
  hash the holder's history; this package changes no `SimulationView` read.
- Reuse needs the run's own record allocations: a kept root from a view
  reader, or from another process, re-reads everything.
- `resource_operation_outcome_by_id`, `latest_resource_fulfillment`,
  `exact_resource_completion_certificate`, `resource_report`, and
  `resource_observation_witness` still read the whole root: no key names
  what they scan, or their checks read records no key reaches.

## Tests

- `canwu-api` `gap_g92_api_host_knowledge_reads`: a probe system, before any
  publisher of its boundary, records `knowledge_heads` and a `CurrentHeads`
  page for a superseding line under a `Keep` and a `RetireWhenSuperseded`
  schema, a line past the truncation bound, and a two-subject filter; before
  every boundary the host reads the same on a full run, a run that seals
  every boundary, a continuation restore, and a snapshot reload, through
  `&dyn CanwuRun`, and must match. The records come newest first; 64
  subjects are read and 65 refused.
- `canwu-resource` `gap_g93_resource_host_reader`: the same probe for the
  resource root (a custodian's accounts, a changing account, named
  outcomes), across those runs; the converted helpers against the whole
  root; a kept whole reader re-attached after a new live bucket, a changed
  bucket, and buckets an archive commit empties equals `resource_state` and
  decodes exactly the buckets whose records changed, none when nothing
  changed, and everything after a restore. The original and a fork each
  write account `left` once with different floors, so its bucket and the
  live root differ under the same versions (asserted); the re-attach to the
  fork equals the fork's whole root. A third test reads through a handle
  with a malformed bucket: a reader kept after a failed request fails again
  on re-attach as a fresh one does, then reads correctly from the engine's
  run; a handle that returns fresh shared copies is read correctly with
  every bucket decoded.
- Checked to fail (not committed): version equality in place of
  `Arc::ptr_eq` (the fork's state differs); a fast path that ignores a failed
  request (the re-attach returns `Ok`); an off-by-one heads bound.

## Upgrading notes

An implementation of `CanwuRun` outside `canwu-api` adds the four methods;
`domain_record_shared` returns the allocation `domain_record` reads. A host
replaces its own memo of decoded roots and current heads with `keep` /
`reattach` and the knowledge reads, and a host that used a helper to detect
a corrupt root calls `validate_resource_runtime`.

## Documentation delta

In this change: `docs/architecture.md` (the resource reader paragraph, the
host knowledge reads, the `CanwuRun` contract), `docs/terminology.md` (rows
below), `docs/versioning.md` (the 0.22.0 entry), the rustdoc. For the
release pass, in both languages: the website term pages (the rows below);
`architecture/resources-production.mdx` (the host reader, keep and
re-attach, the helpers' lost integrity checks), `architecture/knowledge-information.mdx`
(the host knowledge reads), `developer/persistence.mdx` (a kept root is never
saved; after a restore everything is read again); the engine-user skill's
host notes (read named records with `load_run`; keep and re-attach a whole
root).

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| kept resource root | `KeptResourceRoot`, `keep`, `reattach` | 已读资源根 | What a host's reader read, kept without the run; a re-attach decodes only what changed. Not 在持预订 (kept booking). Marked for the owner. |
| run handle contract | `CanwuRun` | 运行句柄契约 | Existing row; adds the knowledge reads, records by kind, the shared copy. |
| current knowledge records | `knowledge_current_records` | 当前知识记录 | Existing row; adds the host reads. |

## Review record

One independent review ("ready after fixes"). Accepted and fixed: the
whole-root check of the converted helpers moved from every debug build to the
opt-in `verify-targeted-reads` feature (it made each helper cost a whole read
in downstream debug builds and could panic there); the fork test now meets
the version hazard (same version, different content) and fails a version
comparison on the answer, not the decode count; the `domain_record_shared`
contract is stated on the trait and checked by the reader; a reader kept
after a failed request no longer re-attaches through the fast path; the
G-92 test checks the order, a `RetireWhenSuperseded` schema, and the limit
at 64; this record, the measurements, and the documentation delta; the
rustdoc's page and order wording; a guard that drops kept buckets when the
bucket counts differ. Not changed: a view reader still records its requests
(negligible), and `ResourceRootReader` is invariant in its first lifetime.
