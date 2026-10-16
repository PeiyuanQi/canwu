# Fiscal Observers at Scale

Status: proposed for 0.17.0; reviewed once (engine and host-side reviews,
"implement with changes"), with every finding resolved in
[Review record](#review-record) and the 0.16.0 drain profile folded in
([Profile results](#profile-results)). Phase 3 of
[world-scale fiscal state](world-scale-fiscal.md), which shipped Phases 0 to
2 in 0.16.0. It answers the rest of downstream gap G-15/G-16: report
publication to thousands of holders at one instant, report cadence, coarser
rows for upper observers, parked-holder rotation, and held knowledge that
stops growing. It changes the generic knowledge ledger (`canwu-knowledge`,
`canwu-sim`, `canwu-api`), every plugin that publishes knowledge, and the
fiscal extension (`canwu-fiscal`, `canwu-ming-fiscal-reference`, the fiscal
profiles under `benchmarks/`).

The contracts it builds on: the knowledge ledger of
[knowledge ledger cost and authentication](knowledge-ledger-cost-and-authentication.md)
(persistent holder ledgers, the current-record index, commitment format 5),
the relation rule and `knowledge_heads` of
[same-instant supersession](same-instant-supersession-and-unplaced-persons.md),
sealing with work in flight and the continuation checkpoint of
[continuation checkpoint](continuation-checkpoint.md), and the fiscal records,
report queue, and report lines of
[world-scale fiscal state, Phase 2 as implemented](world-scale-fiscal.md#phase-2-as-implemented).
Same version only: nothing written by 0.16.0 is read by 0.17.0, so every
format this design changes is versioned and none is migrated.

Every section leads with its decision. "Today" means 0.16.0 as it is on
`main`; "proposed" means this design. Costs are diagnostics: the owner's rule
is player lag, judged by a human at each speed.

## Contents

1. [The problem and the gate](#the-problem-and-the-gate)
2. [Decisions on one page](#decisions-on-one-page)
3. [Today's publication path, as read in the code](#todays-publication-path-as-read-in-the-code)
4. [Engine publication budget](#engine-publication-budget)
5. [Shared publication and one copy per body](#shared-publication-and-one-copy-per-body)
6. [Knowledge retention](#knowledge-retention)
7. [Fiscal reports: per-observer parts kept, office parts rejected](#fiscal-reports-per-observer-parts-kept-office-parts-rejected)
8. [Report cadence](#report-cadence)
9. [Coarser rows for upper observers](#coarser-rows-for-upper-observers)
10. [Parked holders, domain-entity holders, and the report-system cadence](#parked-holders-domain-entity-holders-and-the-report-system-cadence)
11. [Every remaining cap](#every-remaining-cap)
12. [What can still fail a boundary](#what-can-still-fail-a-boundary)
13. [Persistence, hashing, replay, and formats](#persistence-hashing-replay-and-formats)
14. [Public API delta](#public-api-delta)
15. [Terminology candidates](#terminology-candidates)
16. [Documentation delta](#documentation-delta)
17. [Acceptance tests](#acceptance-tests)
18. [Measurement plan and gate](#measurement-plan-and-gate)
19. [Profile results](#profile-results)
20. [Packaging for two workers](#packaging-for-two-workers)
21. [Engine findings outside this package](#engine-findings-outside-this-package)
22. [Risks and open questions](#risks-and-open-questions)
23. [Review record](#review-record)

## The problem and the gate

On the `world` preset of `fiscal-world-profile` (200 polities, 50,000 scopes,
4,200 institutions, 4,000 observers), Phase 2 publishes every observer's
report on change, one knowledge batch per observer, and the engine lets one
system publish at most 64 batches per boundary. A day on which every
observer's institutions change drains over about 63 same-instant boundaries.
The profile ([Profile results](#profile-results)) measured the burst's 62
drains at 4.2 s (a median drain 56 ms, 17 ms fiscal and 38 ms engine, of
which 21 ms is one engine defect), and the same work in one boundary with
every cap lifted at 4.7 s with +1.9 GB of peak heap. Held knowledge grows
by about 12,400 records and 65 MB of JSON per cycle, so monthly cycles over
100 years would reach 7 to 78 GB.

**Gate** (owner, `opus-progress.md` §2), on the `world` preset with courts at
`Institution` granularity and regional officials at `Scope`:

1. Every observer due at one instant, owing one dirty cycle, publishes in
   **one** boundary; its cost is stated as a diagnostic. With the numbers of
   this design, that is about 9,600 bodies (4,000 summaries, about 5,200
   scope parts, about 400 institution parts), about 44 MB of canonical JSON
   and 18 MB of payload. Several dirty cycles owed at once (about 5,600
   parts a cycle) take more than one boundary by design, through the same
   drain packet as today.
2. Held knowledge stays flat over simulated years.

The three caps that meter publication today, and what binds:

| Cap | Where | Value | Effect when exceeded today |
| --- | --- | ---: | --- |
| `KnowledgeLimitsV1::batches_per_system_boundary` | `canwu-sim/src/runtime/knowledge.rs:38`, checked in `validate_boundary_proposal` (`settlement.rs:2208`) | 64 publications per system per boundary, one holder each | the boundary fails with `KnowledgeLimitExceeded` |
| `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` | `canwu-fiscal/src/model.rs:114`, used by `Publisher` in `report.rs:296` | 1,024 report records per boundary | fiscal stops and schedules the drain packet |
| `KnowledgeLimitsV1::records_per_boundary` | `knowledge.rs:39`, checked in `stage_knowledge_publications` (`settlement.rs:1692`) | 10,000 records over all plugins | the boundary fails with `KnowledgeLimitExceeded` |

Fiscal never reaches the engine's two failing checks because `Publisher::room`
stops at 64 publications and 1,024 records; the price is the drains. The
profile shows that lifting the caps alone does not reach the gate cheaply:
the one-boundary run is slower than the drains, because the engine copies
and encodes each record five or more times and resolves initial-scenario
evidence by a linear scan.

## Decisions on one page

| Item | Decision |
| --- | --- |
| (a) Budget | The per-system count of batches is removed. A boundary has one publication budget of **payload bytes, holder entries, and record bodies**, shared by every publisher in dispatch order; a system reads what is left with `SimulationView::knowledge_publication_budget()`. Exceeding it still fails the boundary, in the undeclared-write class; a system that reads first can always stay inside deterministically. Fiscal's default share is half. |
| (b) Shared publication | One `PublishKnowledge` directive carries **groups** (holders, drafts); every body is stored **once** and each holder gets a **holder entry** (its own local ID and supersession). The same `Arc` body serves the overlay, the ledger, and the journal; a body is encoded once and hashed once per boundary; index keys are digests, not subject strings. |
| (f) Retention | A schema declares `Keep` or `RetireWhenSuperseded`; under the latter an entry a new entry supersedes is retired **at publication**, in the same boundary, live and replay alike, never at a seal (the opus-progress step said "after a seal"; seal-tied retirement would make sealed and unsealed state differ and break the lockstep gate). Fiscal exposes the choice as `with_report_retention`, default `RetireWhenSuperseded`. |
| (c) Fiscal assembly | **Per-observer parts and lines are kept** (today's model). Office-aligned shared parts were designed and rejected: with courts at `Institution`, no part on the world preset has two holders, and the office model needs more records, more fiscal writes, and a generation protocol. The gate is met by the budget, one copy per body, retention, and fiscal cost cuts. `fiscal_reports` is unchanged in shape; the projection invariant is restated for `Interval`. |
| (d) Cadence | `Interval { every, offset }` aligned to `SimTime::EPOCH`, with `FiscalReportCadence::staggered`, is the world-scale default; `OnChange` stays. Due times live in the queue pages; a schedule record keeps each page's earliest due time and the parked rotation; the root keeps the earliest of all. No future-dated packet. A summary is republished at every due time, so silence never means "no report". |
| (e) Coarser rows | `Institution` granularity: one row per visible institution and mechanism, summed over the office's books of the cycle. Jurisdiction rollups are not added. |
| (g) Other Phase 3 | Parked holders are re-checked in rotation, 16 per boundary. Domain-entity holders stay deferred. The report system keeps `EventDriven` and gains `with_report_cadence`. |
| E0 | The engine indexes initial-scenario record versions for evidence resolution: the one defect the profile found, 40 % of a first-publication drain's engine time. |
| Not in 0.17.0 | Paging the knowledge ledger in the paged checkpoint (E4): the continuation checkpoint is the world-scale save. |

## Today's publication path, as read in the code

Implemented in 0.16.0; this is what the design changes.

**Directive and staging.** `BoundaryDirective::PublishKnowledge { holder,
visibility, producer_correlation, records, summary }` (`boundary.rs:182`) is
accepted from a phase-4 or phase-13 system. Systems of a phase run in the
registry's dispatch order (`settlement.rs:409`), which is the non-decreasing
`(phase, plugin, system)` order restore validation enforces on the boundary's
knowledge changes (`validation.rs:3994`). `validate_boundary_proposal`
(`settlement.rs:2208`) counts the directives of one proposal and fails the
boundary above 64; per draft it checks the grant, the schema, `validate_draft`
(holder eligibility, confidence, relations, subjects, a 64 KiB payload
measured as `serde_json::to_vec(&draft.payload).len()`, `knowledge.rs:352`),
the origin evidence (`validate_proposal_evidence_reference`,
`settlement.rs:2885`: a version established by this boundary resolves when it
is among the boundary's staged record changes), the relation rule
`validate_relation_targets`, and a duplicate of the canonical `(holder,
draft)` kept as bytes in a `BTreeSet`. `stage_knowledge_publications`
(`settlement.rs:1663`) sums the boundary's records, fails above 10,000,
allocates one global `KnowledgeRecordId` per draft in staging order, clones
each same-boundary record into the overlay, and builds a
`BoundaryKnowledgeChange` per directive. `commit_knowledge_publications`
(`settlement.rs:1766`) inserts every record into a shared clone of the
persistent `HolderLedgers`, invalidates the knowledge commitment, and appends
one `KnowledgePublished { holder, record_count }` event per directive
(`event_payloads.rs:92`), whose audience is that one holder
(`event_audience`, `mod.rs:2084`). The journal clones the records again into
`BoundaryRecord::knowledge_changes`, and the boundary hash and the state
commitment each encode them once more.

**Ledger.** `HolderLedgers` (`canwu-knowledge/src/ledger.rs`) files each
record under its holder and global ID, keeps a by-ID map and a namespace
count, and each `HolderLedger` keeps an index of current records by kind and
by subject, keyed by the full `KnowledgeRecordKind` and `KnowledgeSubject`
values. A record is current when no record of the holder names it in
`supersedes`. Nothing is removed in a run.

**Holder-local IDs.** `holder_local_ids` (`canwu-knowledge/src/lib.rs:593`)
numbers a holder's records 1..n by position at read time;
`CanwuViewer::audit_knowledge_record` reads `values().nth(index)`
(`canwu-api/src/lib.rs:2481`). Removing a record would renumber the later
ones.

**Commitment.** The knowledge root (format 5) is `H(actors sub-root, records
sub-root)`, the records sub-root a running hash of every record in global ID
order (`KnowledgeRecordAccumulator`, `state.rs:721`) that relies on records
never changing or leaving. The boundary hash covers every field of
`knowledge_changes` (`hashing.rs:1039`).

**Reads.** `SimulationView::knowledge_heads` (`view.rs:318`) reads the
committed index plus the boundary's overlay, never evidence;
`knowledge_changes_by_correlation*` (`view.rs:1180`) scans retained boundary
evidence. `CanwuViewer::query_knowledge` (`canwu-api/src/lib.rs:2413`) answers
only the principal's holder.

**Holders.** `validate_holder_for_publication` (`knowledge.rs:512`) admits an
existing person who is not dead, an existing army or government, or an
active undeleted domain entity whose schema allows holders;
`SimulationView::knowledge_holder_accepts_publication` (`view.rs:139`) is the
same check for systems.

**Persistence.** `KnowledgeSnapshot::records` is a list of `{ holder, records }`
entries validated by `validate_snapshot_records` (`knowledge.rs:395`), which
requires every `supersedes` and `contradicts` target to resolve within the
holder (`knowledge.rs:495`); the continuation checkpoint embeds the same
snapshot and `validate_continuation_state` (`validation.rs:569`) runs the same
check; boundary evidence is validated by `validate_boundary_knowledge_changes`
(`validation.rs:3970`), which replays the relation rule phase by phase over a
by-ID map. A paged checkpoint keeps the whole knowledge ledger inside the
4 MiB envelope page (`record-store-ceiling-and-cost.md`, "Not done").

**Evidence resolution.** A reference to a record at its initial-scenario
version is resolved by `initial_domain_record_matches` (`validation.rs:2855`),
a scan of every initial record: about 320 µs each at 67,990 records, under
1 µs once the record has been rewritten.

**Sealing.** `seal_evidence` proceeds with work in flight: pending ingress is
pinned in the continuation closure (`persistence.rs:2233`). What a
future-dated packet still costs is a boundary the host is forced to settle at
its due time, a pin in every continuation checkpoint until then, and the fact
that due ingress cannot be cancelled. A mid-boundary `ScheduleIngress`
(the drain packet's `append_ingress`, `ingress.rs:1182`) calls
`refresh_checkpoint_hash`, which computes every dirty commitment root early.

**Fiscal.** `materialize_reports` (`canwu-fiscal/src/report.rs:265`) reads the
root's queue pages (every non-empty page, in every drain), sorts the queued
observers by (due time, ID), and `publish_observer` computes each observer's
owed parts from its own rows, compares each part's digest (over the rendered
rows, at the observer's confidence) with the one the observer record keeps,
and publishes the summary and the changed parts to the one holder, each record
superseding the holder's current record on that line through `knowledge_heads`
(`Publisher::draft`, `report.rs:201`); `Publisher::publish` (`report.rs:231`)
is fiscal's only constructor of the directive, and `report.rs:295` its only
reader of the 64-batch count. Settlement queues, for each changed `(office,
rank, cycles)`, every observer of the office (`Working::office_observers`,
`working.rs:747`). Every observer is `OnChange`; cadence and granularity were
not added to `FiscalObserverBinding` (`model.rs:1631`). Fiscal's cost at 4,000
observers is about 1.1 s: ledger reads 0.35 s, index lookups 0.1 s,
payload encoding with `serde_json::to_value` 0.25 s, digests 0.1 s, queue-page
rescans 0.27 s per burst.

## Engine publication budget

**Decision.** `KnowledgeLimitsV1` loses `batches_per_system_boundary` and
gains a per-boundary budget of payload bytes and holder entries beside the
body count; a directive is bounded in bodies and entries; a system reads the
remaining budget, with the per-directive bounds, through the view. 0.17.0
may break the struct, and does.

```rust
// canwu-sim/src/runtime/knowledge.rs (proposed)
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KnowledgeLimitsV1 {
    pub schemas_per_plugin: usize,           // 256, unchanged
    pub records_per_batch: usize,            // 1_000 bodies per directive, unchanged
    pub holder_entries_per_batch: usize,     // 4_096, new
    pub records_per_boundary: usize,         // 32_768 bodies (was 10_000)
    pub holder_entries_per_boundary: usize,  // 32_768, new
    pub payload_bytes_per_boundary: usize,   // 64 MiB of new bodies' payloads, new
    pub payload_bytes_per_record: usize,     // 65_536, unchanged
    pub relations_per_record: usize,         // 64, unchanged
    pub text_bytes: usize,                   // unchanged
    pub max_page_size: u32,                  // unchanged
    pub queries_per_batch: usize,            // unchanged
    pub ids_per_direct_get: usize,           // unchanged
    pub default_page_size: u32,              // unchanged
    pub relation_trace_depth: u32,           // unchanged
    pub relation_graph_records: usize,       // unchanged
}
```

| Measure | What it counts | Bound per directive | Bound per boundary |
| --- | --- | ---: | ---: |
| payload bytes | `serde_json::to_vec(&draft.payload).len()` of each new body, the measure `validate_draft` already takes; public as `KnowledgeRecordDraft::payload_bytes()` | 64 KiB each | 64 MiB |
| holder entries | one per (holder, body) pair | 4,096 | 32,768 |
| record bodies | drafts stored once | 1,000 | 32,768 |

**Why bytes lead.** The profile fitted 3.3 µs per KiB of flat payload but 28
to 43 µs per canonical KiB of fiscal-shaped structured JSON, and a 1.5 KiB
summary against an 8 KiB part differ fivefold in cost; a count of records
cannot meter that. The numbers come from the gate arithmetic: one dirty cycle
of the world preset is about 9,600 bodies and entries and 18 MB of payload
(measured 1.9 KiB of payload per record, 4.4 KiB canonical), so fiscal's
default share of one half (16,384 bodies and entries, 32 MiB) publishes it in
one boundary, and the whole budget holds about three dirty cycles' parts.

**Per system.** No per-system share. The budget is spent in dispatch order:
phase 4 before phase 13, and within a phase in the registry's order
(`settlement.rs:409`, the `(phase, plugin, system)` order restore validation
enforces). Fairness between plugins is the host's composition problem; the
view read makes it deterministic, since a system sees exactly what earlier
phases and earlier systems of its own phase have taken.

**The read.**

```rust
// SimulationView (proposed)
pub struct KnowledgePublicationBudget {
    pub records: usize,          // bodies this boundary may still store
    pub holder_entries: usize,   // entries it may still add
    pub payload_bytes: usize,    // bytes of new bodies it may still take
    pub records_per_batch: usize,        // the per-directive bounds, so a
    pub holder_entries_per_batch: usize, // publisher splits groups right
}
impl SimulationView<'_> {
    /// The publication budget left at this system's cut: the limits less what
    /// every proposal validated earlier in this boundary (earlier phases, and
    /// earlier systems of this phase) has taken. Needs no read declaration:
    /// the answer is the check the engine applies to this system's own
    /// proposal, as `knowledge_holder_accepts_publication` is.
    pub fn knowledge_publication_budget(&self) -> KnowledgePublicationBudget;
}
impl KnowledgeRecordDraft {
    /// The bytes the budget charges for this draft's payload.
    pub fn payload_bytes(&self) -> Result<usize, CanwuError>;
}
```

`validate_boundary_proposal` runs right after each system returns, so the
engine knows, system by system, what the phase has taken; the view is built
per system (`settlement.rs:461`) and carries the running counts.

**When a system exceeds it.** The boundary fails with `KnowledgeLimitExceeded`.
This is the undeclared-write class, not an expected refusal: the read is
exact, phase- and system-ordered, and public, so a publisher that overshoots
ignored what it was told, and a publication the engine cut would leave the
plugin's state writes of the same proposal (digests, queue entries)
believing the records were published, with no state-based way for the plugin
to learn otherwise. Every publisher reads the budget and defers what does not
fit, as fiscal does with its drain.

**Fiscal's share.** `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024) is
replaced by `FiscalReportBudget { bodies, entries, payload_bytes }`, default
half of `KnowledgeLimitsV1::CURRENT` (16,384, 16,384, 32 MiB), settable up or
down with `FiscalPlugin::with_report_budget(FiscalReportBudget)` and recorded
in the plugin descriptor; what fiscal publishes is the least of its share and
the engine's remaining budget. What does not fit stays queued and the drain
packet follows, as in 0.16.0.

**Every other publisher.** All seven adapt to the directive shape of
[Shared publication](#shared-publication-and-one-copy-per-body) (one group of
one holder, `Explicit`); the ones that pace themselves read the budget, in
bodies, entries, and bytes:

| Publisher | Today | Proposed |
| --- | --- | --- |
| `canwu-military` (`plugin.rs:2017`) | `take(batches_per_system_boundary)` commanders | takes commanders while the budget has bodies, entries, and bytes for the next commander's records; the rest get no report this boundary, as today |
| `canwu-movement` (`model.rs:66`, `plugin.rs:761`) | its own `MovementLimitsV1::report_holders_per_boundary` (64) | unchanged value (movement's own public struct); it also stops at the engine budget |
| `canwu-information` (`plugin.rs:61`) | `MAX_PUBLICATION_BATCHES` 64 per chunk | unchanged chunking; the chunk also stops at the engine budget |
| `canwu-technology` (`model.rs:39`) | `max_publications_per_boundary` 32 | unchanged; already a recorded rejection past it |
| `canwu-correspondence`, `canwu-resource`, `canwu-production` | a few directives per boundary | shape only; `visibility` applies to every group; `KnowledgeWriteGrant` is unchanged |

## Shared publication and one copy per body

**Decision.** A record body is stored once; each holder holds a holder entry
with its own stored local ID and its own supersession and contradiction
lists; one directive publishes groups of (holders, drafts). One canonical
encoding, one hash, and one `Arc` per body per boundary serve validation,
the overlay, the ledger, the journal, and both hashes. This is the data model
retention needs (entries leave; bodies leave when unheld) and the copy
reduction the profile asks for, whether or not a publisher ever shares a body.

### Data model in `canwu-knowledge`

```rust
/// What is published and stored once (proposed; replaces the stored form of
/// `KnowledgeRecord`).
pub struct KnowledgeRecordBody {
    pub id: KnowledgeRecordId,          // global, allocated in staging order
    pub schema: KnowledgeSchemaId,
    pub subjects: Vec<KnowledgeSubject>,
    pub payload: Value,
    pub as_of: Option<SimTime>,
    pub learned_at: SimTime,
    pub confidence_per_mille: u16,
    pub origin: KnowledgeOrigin,
}

/// One holder's hold on one body (proposed).
pub struct HolderEntry {
    pub record: KnowledgeRecordId,
    pub local: HolderKnowledgeRecordId,       // stored; from the holder's counter
    pub supersedes: Vec<KnowledgeRecordId>,   // this holder's, same kind
    pub contradicts: Vec<KnowledgeRecordId>,  // this holder's, same kind
}

/// A stored body with what the runtime derives from it once (proposed;
/// private to the ledger).
struct StoredBody {
    body: Arc<KnowledgeRecordBody>,
    digest: [u8; 32],      // blake3 of the one canonical encoding
    payload_bytes: u32,    // what the budget charged
}

pub struct HolderLedgers {
    bodies: OrdMap<KnowledgeRecordId, StoredBody>,             // shared store
    holders: OrdMap<KnowledgeHolderRef, HolderLedger>,
    holder_counts: OrdMap<KnowledgeRecordId, u32>,             // holders per body
    namespaces: OrdMap<String, usize>,                         // bodies per namespace
    holder_roots: OrdMap<KnowledgeHolderRef, [u8; 32]>,        // derived, cached
}

pub struct HolderLedger {
    entries: OrdMap<KnowledgeRecordId, StoredEntry>,  // entry + its 32-byte leaf hash
    by_local: OrdMap<HolderKnowledgeRecordId, KnowledgeRecordId>,
    next_local: u64,                                   // persisted; never restarts
    naming: OrdMap<KnowledgeRecordId, OrdSet<KnowledgeRecordId>>, // who names it
    heads: OrdMap<KindKey, NewestFirst>,                            // derived
    subject_heads: OrdMap<KindKey, OrdMap<SubjectKey, NewestFirst>>, // derived
    chunk_roots: OrdMap<u32, [u8; 32]>,                // derived, cached
}
```

`KnowledgeRecord` stays as the **assembled** form a reader gets: body fields
plus `holder`, `supersedes`, `contradicts` of one entry (`HolderLedger::get`
returns an owned `KnowledgeRecord` built from the `Arc` body and the entry;
`HolderLedger::body(id)` and `HolderLedgers::body(id)` return
`&Arc<KnowledgeRecordBody>`). `KnowledgeRecordView` is unchanged. Every
iterator stays forward-only; `NewestFirst` keys stay `Reverse<KnowledgeRecordId>`.

**Index keys.** `KindKey` and `SubjectKey` are 16-byte digests
(blake3 of the canonical kind or subject, domain-separated, truncated), so an
insert compares 16 bytes instead of strings of a role and a typed target.
`current_records(kind, subjects)` checks, on each candidate's body, that
`body.schema.kind == kind` and that the body carries every one of the exact
subjects asked for (as `CurrentRecords::next` checks subjects today), so a
digest collision can only make a walk visit one extra candidate, never
return a wrong record.

**Record IDs.** One global ID per body, allocated in staging order as today;
a shared body has one ID whichever holders hold it. A holder entry's local ID
comes from the holder's `next_local`, assigned in the order the boundary
stages entries for that holder, so local order equals global order within a
holder, and local IDs never move when an entry is retired.

**Empty holders.** A holder ledger whose last entry was retired stays in
`holders` while `next_local > 1`, is encoded as `{ holder, next_local,
entries: [] }`, and its chunk root is the root of an empty list; decoding
accepts an empty entry list exactly when `next_local > 1`. Local IDs
therefore never restart.

### The directive

```rust
// canwu-sim (proposed)
pub struct KnowledgePublicationGroup {
    /// Sorted, unique. Every record of the group is published to every holder.
    pub holders: Vec<KnowledgeHolderRef>,
    pub records: Vec<KnowledgeRecordDraft>,
}
pub enum KnowledgeSupersession {
    /// Each draft's `supersedes` and `contradicts` apply to each holder of its
    /// group. Allowed only for groups of one holder.
    Explicit,
    /// The engine computes, per holder, the holder's current records of the
    /// draft's kind that carry every subject of the draft, at the publisher's
    /// cut (the `knowledge_heads` rule, newest 64), and names them in the
    /// holder's entry. Drafts must carry empty `supersedes` and `contradicts`.
    /// A draft whose match is truncated (more than 64 heads) fails the
    /// boundary: a shared line is meant to have one head.
    CurrentOnSubjects,
}
BoundaryDirective::PublishKnowledge {
    visibility: StateVisibility,
    producer_correlation: Option<String>,
    supersession: KnowledgeSupersession,
    groups: Vec<KnowledgePublicationGroup>,
    summary: String,
}
```

A directive's bodies are the sum of its groups' drafts (at most
`records_per_batch`), its entries the sum over groups of holders × drafts (at
most `holder_entries_per_batch`); a publisher splits a group with many drafts
or holders across directives against both bounds. A single-holder group with
`Explicit` is exactly today's directive, so today's publishers port by
wrapping. One holder may appear in several groups of one directive but not
twice in one group; the same (holder, body) twice in one proposal is refused
as today.

**Validation** (`validate_boundary_proposal`): per group, every holder passes
`validate_holder_for_publication`; per draft, once per body, the schema grant,
visibility, shape, subjects, origin evidence, and **one canonical encoding**,
whose blake3 digest is the body's digest; the byte charge is defined once as
`serde_json::to_vec(&payload).len()`, computed by the engine and by
`KnowledgeRecordDraft::payload_bytes()` the same way (taken from the canonical
bytes only if byte identity with that encoding is proven; a test asserts the
two are equal); the proposal's duplicate check keeps the digests
in a set, not the bytes (today's `BTreeSet<Vec<u8>>` held 62 MB in the
uncapped boundary). Per (holder, draft), the relation rule for `Explicit`, or
the computed list for `CurrentOnSubjects`.

**Staging** allocates one ID per body and one local ID per (holder, body),
wraps each body in one `Arc`, and puts that `Arc` into the same-boundary
overlay (`settlement.rs:1737` clones the assembled record per holder today;
the overlay becomes `BTreeMap<KnowledgeHolderRef, BTreeMap<KnowledgeRecordId,
(Arc<KnowledgeRecordBody>, HolderEntry)>>`, and `current_heads_at_cut`
(`knowledge.rs:219`) reads entries). `BoundaryKnowledgeChange` holds the same
`Arc`s, so the journal references bodies rather than cloning them. **Commit**
inserts the `Arc`s and entries into the shared clone of the ledgers, computes
each entry's leaf hash from the cached body digest, and applies retention. The
boundary hash and the state commitment read the cached digests, so a body is
encoded once and hashed once per boundary.

**Copy targets** (measured today, uncapped boundary of 12,416 records,
62 MB canonical): validation encoded each record twice and kept the bytes;
staging cloned each record into the overlay; the ledger took another; the
journal cloned the whole change (0.46 of 0.76 s); hashing encoded each record
twice more (0.40 s); heap peak +1.9 GB. Proposed: one encoding, one `Arc`, one
digest; the journal, overlay, and ledger share it; the per-boundary heap peak
for knowledge is about the canonical bytes once plus the entries.

**Events.** One `KnowledgePublished { holders: Vec<KnowledgeHolderRef>,
record_count: u32, entry_count: u32 }` per directive, `holders` sorted and at
most `holder_entries_per_batch`, `affected` the same holders as entities;
`event_audience` resolves it to the new `EventAudience::KnowledgeHolders(Vec<_>)`,
so `visible_changes_since` of each holder sees it. `record_count` is the
directive's total; a host wanting a holder's own count reads its ledger
(`fiscal_reports` for fiscal). Today's one event per holder would be 4,000
events at the gate, each admitted as ingress by the next boundary.

### Per-holder supersession and `knowledge_heads`

A holder's current records are the entries no entry of the same holder names
in `supersedes`, as today; the index is over entries. `knowledge_heads(holder,
kind, subjects)` is unchanged in signature and meaning and keeps returning
global IDs. The relation rule is per holder: the target must be an entry of
the same holder and kind at the publisher's cut (`validate_relation_targets`
with `committed(id)` answering "the holder's entry for `id`").

### Queries and paging per holder

`query_records` walks the holder's entries, assembles views from bodies,
sorts by `(learned_at, local)`, and pages by that key, so a cursor stays valid
across retirements of other entries (today's `(learned_at, global)` order is
the same order within a holder). `holder_projection_root` hashes the holder's
assembled views as today. `audit_knowledge_record(holder, local)` reads
`by_local`. `knowledge_record_count_in_namespace` counts bodies.
`KnowledgeQueryResult` gains `retention: BTreeMap<KnowledgeSchemaId,
KnowledgeRetention>` over the schemas the query named, or, when it named
none, the schemas of the returned records, so a research host can tell when
`FullHistory` is the same as `CurrentHeads`.

### Actor-relative reads leak nothing

A holder sees local IDs, its own entry's relations as local IDs, the body's
schema, subjects, payload, times, and confidence. It cannot tell a shared
body from a private one: no field says how many holders hold it, and
`learned_at` is the boundary's time for every entry of the boundary. The
residual coupling of 0.16.0 (global IDs showing allocation gaps) ends, since
local IDs come from the holder's own counter.

### Validation of holders

Every holder of a group is checked at proposal validation with
`validate_holder_for_publication`; an ineligible holder fails the boundary,
as today. A publisher to many holders checks each with
`knowledge_holder_accepts_publication` (or, as fiscal does,
`person_availability`) and leaves an ineligible holder out. The window in
which a check and the publication disagree is the one Phase 2 named: another
plugin's phase-13 system retiring a domain-entity holder after the check;
person availability changes only in phases 7 and 10.

### Costs

Measured today (synthetic publisher, flat payload): 17.9 µs per holder batch
(14.7 µs the first touch of the holder's persistent ledger, 2.5 µs the
event), 13.1 µs per record (4.8 hashing, 4.1 ledger, 2.2 journal, 2.1
validation), 3.3 µs per KiB flat, 28 to 43 µs per canonical KiB of fiscal
JSON, 20 to 55 µs per index insert keyed by subject strings. Proposed, per
body: one validation, one encoding and digest, one `Arc`; per entry: one
insert into `entries` and `by_local`, index updates on 16-byte keys, one leaf
hash, one chunk rehash; per holder: the first touch and a holder root.
Memory: a body held by *n* holders costs one payload and *n* entries of about
80 bytes plus a 32-byte leaf hash.

## Knowledge retention

**Decision.** Retention is a schema property applied at publication: under
`RetireWhenSuperseded`, when holder *H*'s new entry supersedes its entry *S*,
*S* is retired from *H* in the same commit; when no holder holds *S*'s body,
the body is dropped. The live run and its exact replay both do it, because it
is part of the boundary's state transition and the published lists are in
the boundary's evidence. A seal changes nothing, so a sealed shadow run and
the unsealed run keep the same authoritative state hash and checkpoint hash
in lockstep, which is what the continuation gate compares.

**Deviation from the plan, recorded.** `opus-progress.md` §2 step (6) said
"retire superseded report records after a seal". Retirement tied to a seal
would make the sealed run's state differ from the unsealed run's, so their
state hashes and continuation checkpoints would differ and the lockstep
comparison the continuation gate performs would fail by construction; it
would also make a boundary decision (a head count, a `FullHistory` read)
depend on whether a seal happened. Retirement at publication keeps both runs
equal; the orchestrator accepted the deviation in this run, and the owner may
reverse it.

```rust
// canwu-sim (proposed), on PluginKnowledgeSchema
pub enum KnowledgeRetention {
    /// Every entry stays for the life of the run (today's behaviour; default).
    Keep,
    /// An entry leaves its holder when an entry of the holder supersedes it.
    RetireWhenSuperseded,
}
pub struct PluginKnowledgeSchema {
    pub id, pub schema_hash, pub writable, pub payload_schema, pub subjects, // as today
    pub retention: KnowledgeRetention, // new; part of the plugin descriptor
}
```

The policy is in the registered schema, so it is in the plugin descriptor and
its semantic identity: a run restores and replays only with a plugin whose
schema declares the same policy. A plugin that lets its host choose (fiscal's
`with_report_retention`) sets the schema's `retention` at registration from
the knob, so there is one policy per schema and the engine enforces it; two
plugins cannot register one schema. Every plugin keeps `Keep` except fiscal's
report schema by default; the namespace-count readers (`canwu-production`
`plugin.rs:1227`, `canwu-resource` `plugin.rs:2430`, `canwu-technology`
`plugin.rs:433`) keep `Keep` and their counts keep their meaning.

**What retiring an entry does.**

1. The entry leaves `entries`, `by_local`, the heads index, and `naming`; its
   chunk root is recomputed.
2. Every entry of the holder that names it in `supersedes` or `contradicts`
   drops that ID (found through `naming`), so "every relation of an entry
   resolves within its holder" holds at all times, and `query_records`'
   consistency check and snapshot validation (`knowledge.rs:495`) keep it
   unchanged. Under `RetireWhenSuperseded` the superseding entry's list is
   therefore empty right after commit; the published list stays in the
   boundary's evidence.
3. `holder_counts[body]` falls by one; at zero the body leaves `bodies` and
   its namespace count.

Retirement runs in `commit_knowledge_publications` after the boundary's
entries are inserted, in holder order then ID order. A boundary that both
supersedes *S* and, in a later phase, names *S* again resolves the later name
at its own cut as today; the commit retires *S* once and drops the later
name. An entry of a dead person is never retired: the engine refuses
publication to the dead (`knowledge.rs:554`), so nothing supersedes it; it
stays while the holder does, which is bounded by dead holders. An ex-holder
after a rebinding keeps its withdrawn report's parts for the same reason,
bounded by rebindings.

**What hosts and plugins see.** `CurrentHeads` queries are unchanged.
`FullHistory` of a retiring schema shows only current entries; `Keep` keeps
today's history, and the queried schema's `retention` is reported on the
result. `knowledge_changes_by_correlation*` keep their meaning: they return
what the boundary published, retired or not, from retained evidence, and stay
evidence reads. `visible_changes_since` is events, unchanged. Saves hold only
current entries.

## Fiscal reports: per-observer parts kept, office parts rejected

**Decision.** The report model of Phase 2 stays: a part is 16 consecutive
rows of one cycle of one observer's own report, named on the observer's own
record, with the observer's own digests, published only to that observer's
holder. What changes is cadence, granularity, retention, the budget, and the
per-record cost. `fiscal_reports`, `fiscal_reports_of_holder`,
`assemble_fiscal_reports`, the exact pending bitmap, `is_stale`, and the line
scheme (`1 + s * 1,024 + p` over 64 per-observer cycle slots) are unchanged.

**Office parts, designed and rejected.** The first draft of this design
re-cut parts along offices (16 ranks of one office in one office cycle slot,
at one precision tier and one granularity, named on the office record), so
that one body served every observer of the office. On the world preset with
courts at `Institution` and officials at `Scope`, no part has two holders: the
court's rows are institution rows and each office has one official, so
sharing saves nothing, while the office model needs per-(office, slot)
head records written by settlement (4,200 in a burst), per-tier digests to
keep coarse tiers from learning sub-bucket movement, a slot generation
protocol against reuse within one boundary, layout pages for summaries of
more than 512 offices, and a cursor that over-approximates staleness. Even
with courts at `Scope` (every scope seen by exactly two observers), office
alignment gives 5,753 parts over 50,000 rows against 8,416 over 95,065 rows,
about 47 % fewer part bytes, but more records in total (10,956 heads and
4,000 summaries). Per-observer parts with one copy per body, retention, and
`Institution` courts meet the gate with fewer records (about 9,600 against
13,400) and far less change. Generic shared publication stays in the engine
for the data model and for publishers whose holders do coincide.

**Row content.** Rows are rendered at the observer's confidence
(`report_row_of_totals`), the digest is over the rendered rows, so a coarse
observer is republished only when its own bucketed figures change; nothing
below its precision reaches it. Unchanged from Phase 2.

**What a boundary publishes for one observer.** As today
(`publish_observer`): the parts its queue entry names whose digest changed,
withdrawn parts of leaving cycles, and the summary. New: the summary is also
published, alone, at every `Interval` due time at which nothing changed, once
the report has been published at least once, so `as_of` advances and a court
can tell "no change" from "no report" (one body of about 1.5 KiB and one
entry per due observer, about 200 KB a day at 134 due a day).

**Projection invariant, restated.** A report assembled from the records the
holder holds at the end of any boundary in which the observer was published
equals `compute_projection(state, observer, as_of)` over the fiscal records of
that boundary, when the report is complete (`pending` empty). For `OnChange`
this is every boundary that changes something the observer sees, as in
0.16.0; for `Interval` it is the due boundary, and between due times the
report shows the state as of the last due boundary. The randomized partition
test compares each observer's assembled report against the projection
captured at the observer's `as_of` boundary. `is_complete` means nothing was
owed at the last publication.

**Cost cuts, from the profile** (fiscal 1.1 s at 4,000 observers):

| Item | Today | Proposed |
| --- | --- | --- |
| ledger reads for rows | 50,000 at 6.9 µs, one per (scope, cycle) through `RowCache`; with courts at `Scope` the court's rows were read again for the official (95,065 index lookups) | unchanged mechanism; courts at `Institution` read each office's books once and sum them, so the preset reads each book once (about 0.35 s) |
| payload encoding | `serde_json::to_value(record)` then the engine encodes again, about 20 µs a record (0.25 s) | fiscal builds the part's `Value` directly from rows without the intermediate `FiscalReportRecord` struct, and the engine encodes it once; the summary likewise |
| digests | one blake3 over `canonical_hash` of the rows per part (0.1 s) | digest over the rows' compact encoding built once with the payload |
| queue-page rescans | every non-empty page read in every drain (0.27 s per burst) | the schedule record names the pages with due entries; a boundary reads those |
| index lookups | `index_scope` per row | one index page decode per office per boundary, cached in `Working` |

**Rebinding and removal** keep the Phase 2 rule: the old holder gets a
withdrawn summary and keeps the records it was given; the new holder gets
the whole report (its `whole` queue entry). Two bindings of one holder are
two reports on two observer records, so there is no line conflict and no new
refusal.

## Report cadence

**Decision.** `FiscalObserverBinding` gains `cadence: FiscalReportCadence`
and `granularity: FiscalReportGranularity`, both with serde defaults
(`OnChange`, `Scope`), so a small game's scenario changes nothing. A
world-scale scenario gives its observers `Interval` with offsets spread over
the interval. No future-dated packet is scheduled.

```rust
#[derive(Default)]
pub enum FiscalReportCadence {
    #[default]
    OnChange,
    /// Due at `offset + k * every` for k >= 0, aligned to `SimTime::EPOCH`;
    /// `offset < every`, both a whole number of minutes.
    Interval { every: SimDuration, offset: SimDuration },
}
impl FiscalReportCadence {
    /// `Interval` with `offset = (index * every.minutes() / stride) % every`,
    /// `stride` the number of offsets wanted (30 for one a day over a month).
    pub fn staggered(every: SimDuration, index: u64, stride: u64) -> Self;
}
```

Calendar cadences (lunar months, fiscal years, era-dependent intervals) are
not expressible: `every` is a fixed duration. A calendar host uses `OnChange`,
or an interval that approximates its calendar, or rebinds observers when the
calendar shifts.

| Rule | Decision |
| --- | --- |
| due times | `offset + k * every`; the first due time of a binding is the first aligned time at or after its creation |
| what is published at a due time | the parts its queue entry names whose digest changed, withdrawn parts, and the summary (always, see above); `as_of` is the due boundary's time |
| where due times live | only in the queue pages. `FiscalReportQueueEntry { due_at, cycles, whole, republish, parked }` keeps the 0.16.0 fields and one due time: `due_at` is when the entry is served (for `OnChange`, the boundary that queued it; for `Interval`, the next aligned time). No second due field exists; the fiscal worker reports the final field names, and this row is adjusted to them |
| the `Interval` rule | an `Interval` observer's queue entry **always exists**: created with `due_at` at the binding's first aligned time when the binding is added (`add_observer`, or at activation for an authored state), and re-created with the next aligned due time whenever the report system handles it, whether or not it published anything. `merge_queue_entry` keeps the existing `due_at` of an `Interval` entry and unions the queued cycles and parts (`whole` and `republish` or-ed), so settlement reads no observer record to queue a change. The audit checks exactly one entry per active `Interval` binding, and none past removal |
| the schedule record | `canwu.fiscal:report-schedule`, identity `primary`: `page_due: BTreeMap<page, SimTime>` (each page's earliest due entry; at most 4,096 entries), `parked_cursor: Option<String>`; written when a page's earliest due changes |
| the root | `FiscalRoot::earliest_due: Option<SimTime>`, the least of `page_due`; derived, so an idle boundary reads the root alone |
| the audit | `audit_fiscal_records` checks that `page_due` equals the pages' entries and `earliest_due` equals the least of `page_due` |
| what runs the report system | the fiscal report system itself, in every boundary of its cadence; a report due at `t` is published by the first such boundary at or after `t` |
| staleness | an `Interval` report never shows a change between two due times; `pending` is the exact bitmap of 0.16.0; `is_stale` keeps its meaning |
| `OnChange` | settlement queues the office's observers with due `now`, as today |

**Why no future-dated packet.** Sealing no longer needs the queue empty
(`persistence.rs:2233`), but a packet due at `t` forces the host to settle a
boundary at exactly `t`, is pinned in every continuation checkpoint until
then, and cannot be cancelled when its observer is rebound or removed before
`t`. A due time in a queue entry costs none of that.

**World-scale default.** 4,000 observers on `every = 30 days` with
`staggered(every, index, 30)` are due about 134 a day: about 134 summaries and
the parts those observers owe. The gate case is every offset equal.

## Coarser rows for upper observers

**Decision.** `FiscalReportGranularity::Institution`: one row per (visible
institution, mechanism), its facts the sums of the office's books of the cycle
per partition key (unit, payment form), rendered at the observer's confidence.
Parts are cut as for scope rows (16 rows a part), so a court of 21
institutions with one mechanism each has 2 parts per cycle, not 2 to 375.

Why not jurisdiction rollups: a jurisdiction is a catalog region, not an
institution; a region's scopes belong to several institutions, so a rollup
needs a second index (region → offices → ranks), a second line family, and
totals that cross offices. A court that wants a regional view binds the
region's offices at `Institution` granularity and gets one row each.

**Cost and evidence.** An institution row is computed by summing the office's
books of the cycle, read through `RowCache` once per (scope, cycle) per
boundary (about 6.9 µs each): the largest office, 6,000 scopes, costs about
40 ms at its court's due time. A scope part cites the ledger versions its rows
were read from (16 at most); an institution part over up to 6,000 books
cannot cite them all within the 64 relations a record may carry, so it cites
the office record's current version and carries in its payload the number of
books it summed. That is weaker provenance than a scope part's, stated here
and in the README; the office version is committed state, so the citation is
replay-stable. If the profile shows the summing to be a hitch, incremental
totals maintained by settlement on the office cycle entry follow.

## Parked holders, domain-entity holders, and the report-system cadence

**Parked holders.** An observer whose person holder is dead keeps its queue
entry with `parked: true`, as today. Each run of the report system re-checks
the next `FISCAL_PARKED_CHECKS_PER_BOUNDARY` (16) parked entries after the
schedule record's `parked_cursor` in observer-ID order (`person_availability`,
one read each), wrapping round; a revived holder is published to when the
rotation reaches it, within `ceil(parked / 16)` such boundaries. Phase 2
re-checked every parked observer in every boundary. An engine read of "the
persons whose availability changed in this boundary" would do better, but it
is an evidence read (`BoundaryRecord::person_availability_changes`), so it is
not used.

**Domain-entity holders** stay deferred. The reason is unchanged from Phase 2
and verified in the engine: an observer record cannot reference such a holder
(a reference with `allow_retired: false` makes the other plugin's retirement
re-validate and fail), so the report system would read it before publishing,
and a phase-13 system of another plugin, ordered after the check, can retire
it in the same phase; the publication then fails the boundary
(`validate_holder_for_publication`, `knowledge.rs:535`). Making it safe needs
an engine rule that a publication to a holder retired in the same phase is
skipped and recorded, and a state-based way for the publisher to learn the
skip. Nothing in the world target needs it.

**Report-system cadence.** The report system stays
`SystemCadence::EventDriven` in phase 13 (`plugin.rs:378`): it runs in every
boundary that admitted an event or ingress (`boundary_has_event_ingress`,
`settlement.rs:2118`), which includes every same-instant drain boundary, and
a daily host whose boundaries carry events runs it daily. A host whose
boundaries can pass without any event sets
`FiscalPlugin::with_report_cadence(SystemCadence)` (recorded in the plugin
descriptor like `SocietyPlugin::with_settlement_cadence`), and the system
then runs in every boundary that carries that cadence as well as in
event-bearing ones; a due report waits at most until the next such boundary.
An idle run of the report system costs one root read. `EventDriven` stays the
default because the drain boundary at the same instant must run it, and a
host that marks no cadence on that boundary would otherwise strand owed work.

## Every remaining cap

Fiscal, after this design. Unchanged caps of
[world-scale fiscal state, Every fiscal cap](world-scale-fiscal.md#every-fiscal-cap)
are not repeated.

| Cap | 0.16.0 | Becomes | Why |
| --- | ---: | ---: | --- |
| `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` | 1,024 | removed; `FiscalReportBudget` default half the engine's (16,384 bodies, 16,384 entries, 32 MiB), the least of it and the engine's remaining budget, settable with `with_report_budget` | one dirty cycle of the world preset fits the default share |
| Observers published per boundary | 64 (engine) | the budget | the per-system count is gone |
| `MAX_FISCAL_REPORT_CYCLE_SLOTS` | 64 | 64 | unchanged |
| `MAX_FISCAL_REPORT_PARTS_PER_CYCLE`, `FISCAL_REPORT_ROWS_PER_PART` | 1,024, 16 | unchanged | |
| `MAX_FISCAL_OBSERVER_ROWS` | 16,384 | 16,384 at `Scope`; at `Institution` one row per (institution, mechanism), so at most 4,096 × mechanisms | |
| `FISCAL_REPORT_QUEUE_CYCLES`, `FISCAL_REPORT_QUEUE_PARTS` | 16, 64 | unchanged | |
| `FISCAL_REPORT_QUEUE_PAGES` | 4,096 | 4,096 | the schedule record names each page's earliest due time |
| `FISCAL_PARKED_CHECKS_PER_BOUNDARY` | all | 16 | a fixed cost per boundary |
| Schedule record bytes | — | about 4,096 × 40 bytes, under the 1 MiB record budget | one entry per page |
| `with_report_budget` | — | at most the engine's limits | a host raises or lowers fiscal's share |
| Summary bodies per due day | — | one per due observer | the confirmation at a due time |

Engine caps are in [Engine publication budget](#engine-publication-budget);
the per-body payload (64 KiB), relations (64), and page size (1,000) are
unchanged.

## What can still fail a boundary

Engine, knowledge:

- a publication past the remaining budget (`KnowledgeLimitExceeded`): the
  undeclared-write class, since the read is exact and ordered; a plugin that
  did not read `knowledge_publication_budget()` or read it and overshot;
- a holder ineligible at the publisher's cut (`InvalidKnowledgeHolder`): a
  plugin that did not check, or a domain-entity holder retired in the same
  phase by another plugin (the Phase 2 window, now stated for every
  publisher);
- an `Explicit` relation that does not resolve (`KnowledgeRecordNotFound`), a
  cross-kind relation, a draft with relations under `CurrentOnSubjects`, a
  `CurrentOnSubjects` match that is truncated, a group with a duplicate
  holder or more than one holder under `Explicit` (`InvalidKnowledgeRecord`):
  plugin bugs;
- counter overflow (64-bit).

Retention never fails a boundary: retiring an entry is a state transition
with no precondition. What the policy requires of validation is in
[Persistence](#persistence-hashing-replay-and-formats).

Fiscal, after this design: an invalid committed fiscal record (excluded by
activation and every write), counter overflow, and nothing new: an observer
with an unreachable holder is parked, owed work past the budget waits for the
drain, a report with no hot cycle is not published, and no domain-entity
holder exists. An action, receipt, context, or archive packet never fails a
boundary, as in Phase 2.

## Persistence, hashing, replay, and formats

**Formats that change**, each versioned, none migrated:

| Item | 0.16.0 | 0.17.0 | What changed |
| --- | ---: | ---: | --- |
| Snapshot format | 8 | 9 | `KnowledgeSnapshot::records` encodes `{ bodies: [...by ID], holders: [{ holder, next_local, entries: [{ record, local, supersedes, contradicts }] }] }`; decoding refuses a body no holder holds, an entry naming a missing body, a local ID out of order or at or above `next_local`, an empty entry list with `next_local == 1`, a relation that does not resolve within the holder, a duplicate holder |
| `COMMITMENT_FORMAT_VERSION` | 5 | 6 | the knowledge root is `H("canwu.commitment.knowledge.v3", { actors, ledger })`, `ledger` the chunked commitment below; the boundary hash covers the new `knowledge_changes` shape; the `KnowledgeRecordAccumulator` is removed |
| Checkpoint/evidence-journal format | 5 | 6 | `BoundaryKnowledgeChange` carries bodies and per-holder entries; one `KnowledgePublished` event per directive with a holder list |
| Paged-checkpoint envelope | 5 | 6 | the envelope's compact body encodes snapshot format 9; the ledger stays inside the 4 MiB envelope page (E4 is later work) |
| Continuation checkpoint | 1 | 2 | embeds snapshot format 9 |
| `PluginDescriptor` | — | — | `PluginKnowledgeSchema::retention` is in the descriptor and its hash |
| `fiscal_report` knowledge schema | 4 | 5 | `granularity` and `cadence` in the summary; the institution part's `books_summed` |
| `FISCAL_RUNTIME_SCHEMA_VERSION` | 5 | 6 | binding fields, the `Interval` meaning of a queue entry's `due_at`, the schedule record, root `earliest_due` |

**Chunked commitment.** Each entry has a 32-byte leaf hash, computed at
insert and cached: `blake3("canwu.knowledge.entry.v1", local, record, body
digest, supersedes, contradicts)`, the body digest cached with the body. Chunk
*i* of a holder covers its entries with local IDs `[256 i, 256 i + 256)`; its
root is blake3 over the leaf hashes in local order (8 KB at most, binary, no
JSON). A holder's root is blake3 over its chunk roots in chunk order; an
empty holder's root is blake3 over nothing under the domain. The ledger root
is blake3 over `(blake3(holder), holder root)` pairs in holder order. Costs:
a touched entry rehashes its chunk (8 KB) and its holder (32 bytes a chunk);
a publishing boundary rehashes the ledger root over every holder (64 bytes a
holder: 0.26 MB at 4,000 holders, 4 MB at the 65,536 cap, about 2 ms at
blake3 speed); adding a holder costs its own root and the same ledger-root
pass, nothing is re-chunked. `knowledge_commitment_root` recomputes the same
value from scratch at load and must agree. The actor sub-root is unchanged.

**Paging (E4, not in 0.17.0).** The knowledge ledger stays in the envelope
page, so the paged checkpoint remains unavailable to a world-scale run and
the continuation checkpoint is its save; the design for later work is one
content-addressed page per body (unchanged bodies not stored again) and one
page per 256 holders' entries. The restore budget of E-21 is separate.

**Exact replay** re-runs the plugins from the journal
(`replay_exact_journal`, `replay.rs:169`) and compares revision and checkpoint
hash; the engine computes `CurrentOnSubjects` lists from state at the same
cut, and retirement is the same state transition, so a replay reaches the
same ledger. **Restore validation** (`validate_boundary_records`,
`validation.rs:3003`, and `validate_boundary_knowledge_changes`, `:3970`)
builds a `HolderLedgers` progressively instead of the by-ID map and, per
boundary: checks IDs and local IDs in order, every holder eligible at its
cut, every `Explicit` relation resolving at the publisher's cut, every
`CurrentOnSubjects` list equal to what the rule gives from the ledger built so
far (a tampered list is refused); then applies the commit and the policy
(retires superseded entries, drops their names through `naming`, drops bodies
at zero holders); at the end, the ledger built must equal the snapshot's.
`validate_snapshot` (`:640`) then checks a ledger of current entries only,
which is smaller. **Tamper**: besides the decoding refusals above, a retained
entry that the policy says was retired, a `next_local` below an entry's local
ID, or a chunk or holder root that does not match is refused at load; the
continuation checkpoint is verified against its `continuation_hash` as today.

**Seal independence.** No boundary decision reads retained evidence:
supersession comes from the index, retention from the policy, due times from
the queue pages and the schedule record, and the parked rotation from the
schedule record. The continuation gate (every mutation on a sealed shadow,
restored from continuation checkpoints, compared by state hash and checkpoint
hash) is the acceptance check.

## Public API delta

Breaking, under the same-version policy.

**`canwu-knowledge` / `canwu-api` re-exports.** New `KnowledgeRecordBody`,
`HolderEntry`; `KnowledgeRecord` becomes the assembled form (same fields;
`HolderLedger::get` and the iterators return it by value; `HolderLedger::body`,
`HolderLedgers::body` return `&Arc<KnowledgeRecordBody>`);
`HolderLedgers::insert(body, entries)`, `remove_entry(holder, id)`,
`remove_body(id)` replace `insert`/`remove`; `HolderLedger::next_local()`,
`local_of(id)`, `id_of(local)`; `HolderLedgers::body_count()`,
`entry_count()`; `GenericKnowledgeLedger::insert_records` takes the same.
`KnowledgeQueryResult` gains `retention: BTreeMap<KnowledgeSchemaId,
KnowledgeRetention>`.
`KnowledgeRecordView`, `KnowledgeQuery`, `KnowledgeCursor`, `KnowledgeSnapshot`
(shape only), and `MAX_KNOWLEDGE_PAGE_SIZE` are unchanged.

**`canwu-sim` / `canwu-api`.** `KnowledgeLimitsV1` as above
(`batches_per_system_boundary` removed; three fields added);
`BoundaryDirective::PublishKnowledge { visibility, producer_correlation,
supersession, groups, summary }`; `KnowledgePublicationGroup`,
`KnowledgeSupersession`; `KnowledgeRetention` and
`PluginKnowledgeSchema::retention`; `SimulationView::knowledge_publication_budget`
and `KnowledgePublicationBudget`; `KnowledgeRecordDraft::payload_bytes`;
`BoundaryKnowledgeChange { plugin, system, phase, producer_correlation,
visibility, summary, records: Vec<Arc<KnowledgeRecordBody>>, holders:
Vec<HolderPublication { holder, entries: Vec<HolderEntry> }> }`;
`BoundaryReceipt::knowledge_record_count` counts bodies and
`knowledge_entry_count` is added (`knowledge_batch_count` stays the directive
count); `KnowledgePublished { holders, record_count, entry_count }`;
`EventAudience::KnowledgeHolders(Vec<KnowledgeHolderRef>)` in `canwu-event`
(`KnowledgeHolder(_)` stays for other uses). `knowledge_heads`,
`knowledge_records`, `knowledge_record_count_in_namespace`,
`knowledge_changes_by_correlation*`, `CanwuViewer::query_knowledge`,
`query_holder_knowledge`, `audit_knowledge_record`, and
`CanwuRun::admin_query_knowledge` keep their signatures. Format and version
constants as in the table. E0 adds no public type.

**`canwu-fiscal`.** `FiscalObserverBinding` gains `granularity` and `cadence`
(serde defaults); `FiscalReportCadence` with `staggered`,
`FiscalReportGranularity`, `FiscalReportBudget`; `FiscalReportQueueEntry`
keeps its fields with the `Interval` meaning of `due_at`; new record
`FiscalReportSchedule`
(`canwu.fiscal:report-schedule`) with `fiscal_report_schedule_reference()`;
`FiscalRoot::earliest_due`; `FiscalReportSummary` gains `granularity` and
`cadence`; `FiscalReportPart` gains `books_summed: Option<u32>`;
`FiscalReportRow` is unchanged (institution rows carry the institution and
mechanism, no scope); `FiscalPlugin::with_report_cadence`,
`with_report_budget`, `with_report_retention`; the constants of
[Every remaining cap](#every-remaining-cap); `compute_projection`,
`fiscal_reports`, `fiscal_reports_of_holder`, `assemble_fiscal_reports`
unchanged in signature. `FiscalState::add_observer` queues the binding's first
due time.

**Other extensions.** No public type changes; each publisher's directive
construction changes.

## Terminology candidates

To confirm against `docs/terminology.md` and both website term pages; the
Chinese is proposed in the term sheet's style. The existing rows 报告周期槽位
(cycle slot), 报告分段 (report part), 报告队列页 (report-queue page), and
观察者记录 (observer record) stay valid, since the per-observer model is kept;
the report-queue page row gains the due time and the schedule record. 机构持有者
(institution holder) remains the Phase 2 candidate for the government holder
shipped in 0.16.0, unchanged by this design.

| 中文规范写法 (proposed) | English | Code | Meaning |
| --- | --- | --- | --- |
| 共享发布 | shared publication | `KnowledgePublicationGroup`, `KnowledgeSupersession::CurrentOnSubjects` | One directive publishes the same record bodies to many holders; each body is stored once and each holder gets its own entry and supersession. |
| 知识记录本体 | knowledge record body | `KnowledgeRecordBody` | The holder-independent part of a knowledge record: schema, subjects, payload, times, confidence, origin. |
| 持有人条目 | holder entry | `HolderEntry` | One holder's hold on one body: its local ID and its own supersession and contradiction lists. |
| 持有人本地记录号 | holder-local record ID | `HolderKnowledgeRecordId`, `HolderEntry::local` | The ID a holder sees for one of its records, from the holder's own counter; it never changes and never names another holder's record. |
| 发布预算 | publication budget | `KnowledgePublicationBudget`, `knowledge_publication_budget` | What a boundary may still spend on knowledge: payload bytes, holder entries, bodies. |
| 知识留存策略 | knowledge retention | `KnowledgeRetention` | A schema's rule for when an entry leaves its holder: kept for the run, or cleared when superseded. |
| 已清退条目 | cleared entry | — | An entry that left its holder under the schema's retention; its body leaves when no holder holds it. Not 退休 (retired), which the term sheet uses for a culture target. |
| 报告节律 | report cadence | `FiscalReportCadence` | When an observer's report is due: on every change, or at aligned interval times. |
| 报告间隔 | report interval | `FiscalReportCadence::Interval`, `staggered` | The `every` and `offset` of an interval cadence, aligned to the epoch. |
| 报告日程记录 | report schedule | `FiscalReportSchedule` | The record of each report-queue page's earliest due time and the parked rotation. |
| 机构粒度 | institution granularity | `FiscalReportGranularity::Institution` | One report row per visible institution and mechanism. |
| 报告份额 | report share | `FiscalReportBudget`, `with_report_budget` | The part of the engine's publication budget fiscal uses in one boundary. The prose term is "report share"; the code identifiers keep "budget". |

## Documentation delta

For the orchestrator, after implementation:

- `docs/architecture.md`, "Knowledge model": bodies and holder entries,
  shared publication, the budget read, retention, the chunked commitment, the
  initial-version index; "Fiscal procedure": cadence, granularity, the
  schedule record, the budget, retention knob.
- `docs/versioning.md`: the 0.17.0 entry with the format table above and the
  known limits that remain (the knowledge ledger in the envelope page; the
  institution part's provenance; the mid-boundary commitment refresh).
- `docs/terminology.md` and both website term pages: the candidates above.
- `crates/extensions/canwu-fiscal/README.md` "Reports": cadence, granularity,
  the report share, the schedule, the summary at a due time, the loss under
  `RetireWhenSuperseded` (a closed cycle's figures leave the holder's ledger
  once superseded; a host that needs them keeps `Keep` or its own archive),
  and that `with_report_retention`, `with_report_cadence`, and
  `with_report_budget` are recorded in the plugin descriptor: they are chosen
  before a run's first boundary, and changing any of them needs a new run (a
  restore, an exact replay, or a fork of an existing run with a plugin of
  another setting is refused as a plugin-identity mismatch).
- `docs/proposals/world-scale-fiscal.md`: amend the Phase 3 statements this
  design changes: "Bindings" (domain-entity holders stay deferred, not Phase
  3), "What an observer is told" (255 slots and `1 + s * 4,096 + p` are the
  as-implemented 64 and `1 + s * 1,024 + p`), "What can still fail a
  boundary" (the domain-entity window is stated for every publisher, Phase 3
  adds no such holder), and the retirement step (at publication, not after a
  seal, with the reason in [Knowledge retention](#knowledge-retention)).
- `canwu-api` examples (`plugin`, `phased_boundary`) and the engine-user skills
  under `agent-interface`: the directive shape and the budget read.
- Website mirrors of every changed page, in both languages, with the
  readability review before deployment.

## Acceptance tests

Durable tests that bite, each a contract or a failure path:

Engine:

1. **Shared publication, per-holder supersession.** One group to three
   holders, two of which hold an earlier line record and one of which holds
   none: the body is stored once (`body_count`), each holder's entry
   supersedes exactly its own earlier record (`CurrentOnSubjects`), the third
   supersedes nothing; `knowledge_heads` per holder agrees; a holder's views
   show local IDs and nothing about the others; a truncated match fails.
2. **Budget read and refusal.** Two phase-13 systems: the first reads the
   budget and publishes to it; the second reads the remainder and stays
   inside; a third variant overshoots in bytes alone and the boundary fails
   with `KnowledgeLimitExceeded` while a run without it commits;
   `payload_bytes()` equals what the engine charged. The answers are the same
   live, restored, and replayed.
3. **Retention.** A `RetireWhenSuperseded` schema: after *n* publications to
   a holder on one line, the holder holds one entry and the ledger one body
   for that line; a body shared by two holders leaves only when both
   superseded it; a `Keep` schema beside it keeps everything; the superseding
   entry's relations are empty after commit while the boundary evidence
   carries the published list; `FullHistory` shows only current entries and
   the result says so; `knowledge_changes_by_correlation` still returns the
   boundary's publication; a holder emptied by retention keeps its
   `next_local`, and its next entry's local ID continues.
4. **Lockstep under retention** (continuation gate harness, release test): a
   run that publishes a retiring schema across seals and continuation
   restores equals its unsealed twin boundary by boundary in state hash and
   checkpoint hash, and replays exactly.
5. **Local IDs and cursors.** A paged query whose cursor was taken before a
   retirement of another entry continues correctly; `audit_knowledge_record`
   by local ID returns the same record before and after; a tampered snapshot
   with a local ID at or above `next_local`, a relation to a cleared ID, a
   body with no holder, or an empty holder with `next_local == 1` is refused.
6. **Commitment.** A load recomputes the chunked root and agrees with the
   runtime's cached chunks after publications and retirements; one changed
   entry deep in a chunk changes the root; the same ledger built in another
   order gives the same root; the tamper variants of (5) change it.
7. **Events.** One `KnowledgePublished` per directive naming every holder;
   each holder's `visible_changes_since` sees it; a non-holder's does not.
8. **One copy** (allocation-counting build, performance harness): a
   publication of *n* bodies of *b* bytes allocates the canonical encoding
   once per body, and the journal's change shares the ledger's `Arc`s
   (`Arc::ptr_eq`).
9. **Initial-version evidence** (E0): a publication citing an initial-scenario
   record version resolves in a store of 65,536 initial records with an
   allocation count independent of the store's size, and a wrong version is
   refused as today.

Fiscal:

10. **Cadence.** An `Interval` observer with changes between its due times is
    published only at the due time with `as_of` the due boundary; one with no
    change gets its summary alone, `as_of` advanced; an `OnChange` observer is
    published in the changing boundary; the root's `earliest_due` lets an idle
    boundary read one record (allocation count); a due time with no boundary
    is served by the first event-bearing boundary after it, or by the next
    boundary of `with_report_cadence`; `staggered` places offsets as stated;
    `page_due` and `earliest_due` pass the audit after every boundary.
11. **Projection invariant** (the randomized partition test, extended): every
    complete report assembled at the end of a boundary in which its observer
    was published equals the projection captured at that boundary, for
    `OnChange` and `Interval`, `Scope` and `Institution`.
12. **Budget cut.** A report cut by `with_report_budget` leaves the exact
    pending bitmap, `is_stale` is right for every fact, and the drain
    completes it; a share above the engine's limits is refused at plugin
    construction.
13. **Institution granularity.** A court's institution rows equal the sums of
    its office's books at the court's confidence; a change of one book
    republishes one institution part; the part cites the office version and
    counts the books summed.
14. **Parked rotation.** With 40 parked observers and 16 checks a boundary, a
    revived holder is published to within three report-system boundaries, and
    a sealed run does it as its replay does.
15. **Retention knob.** Under `with_report_retention(Keep)` a closed cycle's
    parts stay readable through `FullHistory` after the empty parts supersede
    them; under the default they are cleared; the descriptor differs.
16. **Seal and restore** (existing tests kept): a sealed run supersedes what
    its replay does; a fiscal run restored from a continuation checkpoint
    continues publishing.

Run inline, not committed: the 4,000-observer instant (it is the profile).

## Measurement plan and gate

`fiscal-world-profile` (`benchmarks/performance-harness/src/bin/fiscal-world-profile.rs`),
`world` preset with courts at `Institution` and officials at `Scope`, release
build, medians of three samples, with `audit_fiscal_records` after every
boundary of the last sample, as in Phase 2; new rows in its JSON and in
`benchmarks/baselines/`:

| Measurement | What it records | Gate |
| --- | --- | --- |
| **Due instant** | after the burst, every observer due at one instant owing one dirty cycle: boundaries at the instant, settlement ms, fiscal / engine split, engine phases (validation, staging, knowledge, commit, journal, hashing), bodies, entries, payload and canonical bytes, events, heap peak | **exactly one boundary**; its cost and heap stated |
| Three cycles owed | the same with three dirty cycles | more than one boundary, by design; the count stated |
| Spread day | 134 observers due (offsets over 30 days): the same columns | a routine day stays tens of milliseconds of fiscal time |
| Courts | the 200 courts due, including the 6,000-scope polity | cost of the largest institution part stated |
| Burst, receipts, close, archive | as Phase 2, with `Interval` observers | each at or below its Phase 2 number |
| **Held knowledge over simulated years** | 24 monthly cycles (open, assess, close, archive) with monthly `Interval` observers: bodies, entries, payload bytes, continuation-checkpoint bytes after each cycle | **flat** after the second cycle |
| Idle boundary | with 4,000 `Interval` observers, nothing due | one root read; no measurable fiscal time |
| Restore | continuation checkpoint bytes, restore time, and the time of the knowledge part of restore validation after the burst and after 24 cycles | stated |
| First publication | the first boundary that publishes every authored observer's report | the evidence scan of E0 gone from the engine phases |

**Expected cost after the changes, as diagnostics** (to be measured, not
claimed): the gate boundary at about 9,600 bodies and 44 MB canonical, about
1.2 to 1.5 s and a heap peak of about +0.3 GB, against today's 3.1 to 4.2 s of
capped drains and 3.8 to 4.7 s with +1.9 GB uncapped. What buys it: E0
removes 1.2 s of evidence scanning from a first publication and 60 ms from a
spread day; one copy per body removes the overlay clone (0.42 s), the journal
clone (0.46 s), one of two encodings in validation, and one of two hash
encodings (about 0.2 s); digest index keys cut ledger inserts (0.5 to 0.68 s)
by half or more; courts at `Institution` cut the rows from 95,065 to about
54,000 and the parts from 8,416 to about 5,600; direct part encoding and the
single index decode cut fiscal's 1.1 s to about 0.6 s. The `smoke` preset
gets the same rows at 2,000 scopes so the suite stays runnable while
iterating.

## Profile results

Measured 2026-10-16 on `main` f6c5488 with throwaway instrumentation (report,
raw data, and patch in the orchestrator's `p3-profile` folder; AMD Ryzen 7
7800X3D, release build, medians of three after a warm-up). The numbers this
design rests on:

**One drain of the burst (62 drains; first publication / republication).**
Wall 55.7 / 37.2 ms (sums 4,230 / 3,151 ms). Fiscal 17.3 / 17.6 ms:
queue-page scan and decode 5.2 / 4.4, row assembly 4.9 / 5.3 (ledger reads
3.1 to 3.7), digests 1.1, summary and drafts 2.1 / 2.5 (payload to JSON 1.7,
`knowledge_heads` 0.04 to 0.45), writes 1.6 / 1.7. Engine 38.0 / 18.8 ms:
validation 22.6 / 2.4 (of which evidence references 20.6 / 0.1), stage 1.8,
ledger insert 2.6 / 3.2, events 0.15, commit 2.5, journal 4.5 / 4.7 (the
`knowledge_changes` clone 2.4 / 2.6), boundary hash plus state hash 2.8 /
3.3. Per drain: 64 holders, a median 164 records (2.6 a holder), 311 KiB of
payload, 723 KiB canonical (4.4 KiB a record), 1,085 evidence references,
129 record writes, 1,573 queue pages decoded (2,541 in the first), about
2,000 allocations a record, heap peak +25 MB. The 64-batch cap binds in every
drain; neither record cap binds. The "74 ms" of Phase 2 is the mean; the
median is 56 ms.

**The engine defect.** Evidence citing a record at its initial-scenario
version is resolved by a scan of all 67,990 initial records
(`initial_domain_record_matches`, `validation.rs:2855`), about 320 µs a
reference; every first summary cites its observer record, so each first drain
pays 64 scans (20.6 of 38 ms engine), the uncapped first publication 1.2 s,
and a spread day about 60 of its 112 ms. Once the record is rewritten the
lookup is under 1 µs. This is E0.

**Engine cost per holder, record, and KiB** (synthetic publisher, 12
configurations): 17.9 µs a holder batch (14.7 the first touch of the holder's
persistent ledger, 2.5 the event); 13.1 µs a record (4.8 hashing, 4.1 ledger,
2.2 journal, 2.1 validation); 3.3 µs a KiB of flat payload; superseding
+0.6 µs a record; 20 prior generations held +0 %. Fiscal-shaped records cost
far more per byte (330 to 390 allocations a KiB): about 115 µs a record in
drains, 28 µs a canonical KiB, rising with the boundary's size to 219 µs a
record and 43 µs a KiB in the 62 MB boundary. Index inserts run 20 to 55 µs a
record, keyed by full subject values.

**All 4,000 observers in one boundary** (caps lifted locally; first
publication / republication): 4,000 holders, 12,416 records, 62 MB canonical;
wall 4,730 / 3,826 ms; fiscal 984 / 1,104, engine 3,750 / 2,722; validation
1,553 / 265 (evidence 1,222 / 12), stage 420 / 446, ledger insert 507 / 681,
journal 757 / 812 (clone 455 / 541), hashing 396 / 400, commit of 6,562
writes 119 / 128; heap peak +1.91 GB, net retained +1.03 GB; the same work
capped took 4,230 / 3,151 ms, so one boundary is 12 to 21 % slower than the
drains. Fiscal side of the republication: rows 509 ms (50,000 ledger reads at
6.9 µs, 95,065 index lookups at 1 µs), payload to JSON 252 ms (20 µs a
record), digests 101 ms, record writes 48 ms. Nothing else broke; the largest
report (376 records) is under the 1,000-a-batch limit.

**Settling boundaries.** The burst's settling boundary (4,719 ms) spends
about 72 ms on reports, plus about 195 ms in phase 13 that is not report work:
the drain packet's `append_ingress` refreshes every dirty commitment root
early (`ingress.rs:1182`), after 56,891 writes. A spread day costs 112 ms of
its 166 ms in reports, 60 ms of it the evidence scan.

**Held knowledge.** After the burst 12,416 records held, 65.6 MB of knowledge
JSON, a 202 MB continuation checkpoint; +12,416 records, +65.5 MB, and about
+89 MB of checkpoint per cycle (24,832 / 131 MB / 292 MB after one more cycle,
49,664 / 262 MB / 469 MB after three). Current records: 4,000 summaries of
about 1.5 KB and 8,416 parts of about 7 KB; 55 % of the bytes are envelope
(subjects, evidence, origin). Per-boundary cost does not grow with history
(drain median 41.5 to 41.9 ms over three cycles). With retention, held
knowledge stays at about 12,400 records and 65 MB; without it, monthly cycles
over 100 years reach 7.3 GB (summaries alone) to 78 GB.

**Sharing on the preset.** Every scope is seen by exactly two observers
(court and official). Office-aligned sharing gives 5,753 parts over 50,000
rows against 8,416 over 95,065 (about 47 % fewer part bytes) but needs 10,956
part heads and 4,000 summaries, more records than today's 12,416; see
[Fiscal reports](#fiscal-reports-per-observer-parts-kept-office-parts-rejected).

**Two engine assumptions retention breaks**, both covered in E1 and E3 and in
[Persistence](#persistence-hashing-replay-and-formats): the
`KnowledgeRecordAccumulator` is an append-only hash in global-ID order
(`state.rs:705`), replaced by the chunked commitment; snapshot validation
requires every `supersedes` target to resolve (`knowledge.rs:495`), kept true
by dropping cleared IDs from the naming entries.

## Packaging for two workers

Two packages with disjoint files, started in parallel now in two worktrees;
each is one branch, durable tests, an independent review, and one commit. The
integrating agent runs the full verification, the profile gate, and the
continuation gate on the integrated tree.

### Engine package (L)

Owns every file under `crates/model/canwu-knowledge/`,
`crates/runtime/canwu-sim/`, `crates/api/canwu-api/`,
`crates/model/canwu-event/` (the audience variant), and these publisher and
test files that construct or read the directive, the receipt, or the limits:

| Part | Size | Files |
| --- | --- | --- |
| E0 initial-version index | S | `crates/runtime/canwu-sim/src/runtime/validation.rs` (`initial_domain_record_matches` and its three call sites, `validation.rs:2890`, `:2937`, `:2969`) and the runtime state that holds it: an index from `(record reference, version)` to the initial record, built once per scenario, held in an `Arc` that `fork()` and snapshot validation share, so no fork or restore rebuilds it; no format change; allocation-count test |
| E1 ledger | M | `crates/model/canwu-knowledge/src/{lib.rs,ledger.rs}`: bodies, entries, stored local IDs, digest index keys, `naming`, retention, empty holders, leaf and chunk hashes, the new encoding and its refusals |
| E2 directive, budget, one copy | M | `canwu-sim/src/runtime/{boundary.rs,knowledge.rs,settlement.rs,view.rs,event_payloads.rs,mod.rs,state.rs}`: groups, `CurrentOnSubjects`, `KnowledgeLimitsV1`, the budget read and `payload_bytes`, one encoding and `Arc` per body, the overlay of entries, one event per directive with the holder list, `EventAudience::KnowledgeHolders` in `crates/model/canwu-event/src/lib.rs` |
| E3 commitment and validation | M | `canwu-sim/src/runtime/{hashing.rs,state.rs,validation.rs,persistence.rs,continuation.rs}`: the chunked commitment (format 6), the progressive-ledger restore validation with the policy, the format constants (snapshot 9, journal 6, envelope 6, continuation 2), tamper refusals |
| E5 publishers and readers | S each | `crates/extensions/canwu-military/src/plugin.rs`, `canwu-movement/src/plugin.rs`, `canwu-information/src/plugin.rs`, `canwu-correspondence/src/plugin.rs`, `canwu-resource/src/plugin.rs`, `canwu-production/src/plugin.rs`, `canwu-technology/src/plugin.rs`; `crates/extensions/canwu-law/tests/law_contract.rs`; `crates/api/canwu-api/tests/{continuation_checkpoint.rs,gap_g06_sim_evaluation_trace.rs,gap_g49_sim_unplaced_person.rs,gap_g58_sim_same_instant_supersession.rs,governance_transition_conformance.rs}`; `crates/api/canwu-api/examples/*`; `crates/runtime/canwu-sim/src/runtime/tests/{knowledge.rs,knowledge_ledger.rs,knowledge_change_commitment.rs,mod.rs,persistence_contracts.rs}`, `crates/runtime/canwu-sim/tests/{information_flow_kernel.rs,api-delta/src/main.rs}`; `crates/tools/canwu-debug/src/main.rs` (reads `knowledge_record_count`, unchanged name); `benchmarks/performance-harness/src/main.rs`, `src/bin/continuation-profile.rs`, `src/bin/knowledge-ledger-profile.rs` |
| E5f the mechanical fiscal edits | S | in `crates/extensions/canwu-fiscal/src/report.rs`: the body of `Publisher::publish` (today `report.rs:231` to `:247`) becomes one group of one holder with `Explicit`; the initializer `publications_left: KnowledgeLimitsV1::CURRENT.batches_per_system_boundary` (today `report.rs:295`) becomes the literal `64`. In the fiscal tests that read engine shapes (`KnowledgeRecord`, the ledger, the receipt counts): minimal edits to the hunks at `tests/support/mod.rs:730` to `:745`, `tests/gap_g58_fiscal_same_instant_reports.rs:243` to `:250`, `tests/fiscal_report_parts.rs:996`, `tests/fiscal_reports_after_seal.rs:33` to `:40`, and `tests/fiscal_cycle_archive.rs:969`. Nothing else in fiscal is touched, so the 0.16.0 fiscal crate and its tests compile and pass on the engine branch with today's drains |

### Fiscal package (M)

Owns every file under `crates/extensions/canwu-fiscal/` and
`crates/integrations/canwu-ming-fiscal-reference/`, and
`benchmarks/performance-harness/src/bin/{fiscal-world-profile.rs,fiscal-profile.rs}`,
`benchmarks/README.md`, and the fiscal files of `benchmarks/baselines/`.

| Part | Size | Content | Needs the engine package? |
| --- | --- | --- | --- |
| F1 cadence and schedule | M | `FiscalReportCadence` with `staggered` and serde defaults, `granularity` on the binding, the `Interval` queueing rule (the always-present entry, `merge_queue_entry`, the re-queue at the next aligned time), the schedule record and `page_due`, the root's `earliest_due`, the audit, the summary at a due time, `with_report_cadence`; reading only the due pages | no |
| F2 institution granularity | S | institution rows, parts, their evidence and `books_summed` | no |
| F3 parked rotation | S | `FISCAL_PARKED_CHECKS_PER_BOUNDARY` and `parked_cursor` | no |
| F4 cost cuts | S | direct part encoding, one index decode per office per boundary, digest over the compact encoding | no |
| F5 budget, directive, retention | S | `FiscalReportBudget`, `with_report_budget`, the budget read, groups with `Explicit` (a group per holder, split against the per-directive bounds), `with_report_retention` setting the schema's `retention`, `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` removed | yes |
| F6 profiles | S | the measurement rows, the gate preset (courts `Institution`), the `smoke` rows | F5 for the gate |

**Until the engine lands**, F1 to F4 are built and reviewed on 0.16.0's engine
with `Publisher::publish` and `Publisher::room` exactly as they are today
(the per-holder directive under the 64-batch and 1,024-record limits, with
drains). Fiscal's only directive constructor stays `Publisher::publish` and
its only reader of the limits stays `Publisher::room` and the initializer at
`report.rs:295`, so F1 to F4 never touch the lines E5f changes in `report.rs`.
On the fiscal branch, every test read of an engine shape (a holder's ledger,
a `KnowledgeRecord`'s fields, the receipt's knowledge counts) goes through
small helpers in `tests/support/mod.rs`, so the five test hunks E5f edits on
the engine branch are, on the fiscal branch, calls of those helpers.

**Merge plan.** (1) The engine branch merges first (E0 to E3, E5, E5f),
reviewed; its fiscal edits keep fiscal compiling. (2) The fiscal branch
(F1 to F4) merges onto it; the overlaps are the two E5f hunks in `report.rs`
and the five test hunks. For `report.rs` the merge takes the engine branch's
two hunks and the fiscal branch's text everywhere else; for the tests the
fiscal worker keeps its helper versions and ports the helper bodies to the
engine shapes by hand as part of F5 (a small hand-merge, accepted). A
conflict elsewhere can arise only if F1 to F4 moved `Publisher::publish` or
the `publications_left` initializer, which they do not. (3) The fiscal worker
then writes F5 on the integrated tree (replacing `Publisher::publish`,
`Publisher::room`, and the initializer with the budget read and groups,
deleting `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`, porting the test helpers),
small and reviewed. (4) F6, the profile gate, the continuation gate (release
test), the shared docs, terms, website, and skills; one release commit.
Merge, do not rebase shared history.

**The engine API the fiscal package codes against** (names fixed here so the
two workers do not meet):

| API | Shape |
| --- | --- |
| `BoundaryDirective::PublishKnowledge` | `{ visibility, producer_correlation: Option<String>, supersession: KnowledgeSupersession, groups: Vec<KnowledgePublicationGroup>, summary }` |
| `KnowledgePublicationGroup` | `{ holders: Vec<KnowledgeHolderRef> /* sorted, unique */, records: Vec<KnowledgeRecordDraft> }` |
| `KnowledgeSupersession` | `Explicit` \| `CurrentOnSubjects` |
| `SimulationView::knowledge_publication_budget()` | `KnowledgePublicationBudget { records, holder_entries, payload_bytes, records_per_batch, holder_entries_per_batch }` |
| `KnowledgeRecordDraft::payload_bytes()` | `Result<usize, CanwuError>`, the bytes the budget charges |
| `KnowledgeLimitsV1::CURRENT` | `records_per_batch` 1,000, `holder_entries_per_batch` 4,096, `records_per_boundary` 32,768, `holder_entries_per_boundary` 32,768, `payload_bytes_per_boundary` 64 MiB; no `batches_per_system_boundary` |
| `PluginKnowledgeSchema::retention` | `KnowledgeRetention::{Keep, RetireWhenSuperseded}`, default `Keep` |
| `KnowledgePublished` | `{ holders: Vec<KnowledgeHolderRef>, record_count: u32, entry_count: u32 }`; audience `EventAudience::KnowledgeHolders(holders)` |
| `BoundaryReceipt` | `knowledge_batch_count` (directives), `knowledge_record_count` (bodies), `knowledge_entry_count` |
| `SimulationView::knowledge_heads`, `knowledge_holder_accepts_publication`, `person_availability` | unchanged |
| `CanwuViewer::query_knowledge`, `CanwuRun::admin_query_knowledge`, `KnowledgeQuery`, `MAX_KNOWLEDGE_PAGE_SIZE` | unchanged; `KnowledgeQueryResult::retention: BTreeMap<KnowledgeSchemaId, KnowledgeRetention>` added |

## Engine findings outside this package

Recorded for `opus-progress.md` §4.2; neither is needed for the gate, which
removes the drains that trigger the first:

- **A mid-boundary `ScheduleIngress` refreshes every dirty commitment
  root.** The drain packet's `append_ingress` (`ingress.rs:1182`) calls
  `refresh_checkpoint_hash`, which costs 195 ms after the burst's 56,891
  writes (0.6 ms in a normal drain). The fix, deferring the refresh to the
  boundary's own hashing phase, touches the ingress path's identity contract
  and is not small enough to add here unproven; a worker that finds it safe
  may take it with E2, else it is listed as a known limit.
- **Paging the knowledge ledger in the paged checkpoint** (E4), as stated
  under [Persistence](#persistence-hashing-replay-and-formats).

## Risks and open questions

**Risks.**

1. **The expected cost is an estimate.** The gate boundary's 1.2 to 1.5 s
   assumes the copy reduction and index keys remove what the profile
   attributes to copies and string keys; the profile gate measures it, and
   the budget numbers come down if a human finds the hitch too long.
2. **Size of the engine change.** Bodies and entries touch the ledger, the
   directive, staging, commit, validation, hashing, four formats, and seven
   publishers. The randomized ledger test, the progressive restore
   validation, and the continuation gate are the safety net.
3. **Summing institution rows** over large offices at a court's due time
   (about 40 ms for 6,000 scopes) and the weaker provenance of an institution
   part; incremental totals follow if the profile shows a hitch.
4. **Retention loses history by default, and the knobs are run-bound.** A
   host that reads a closed cycle's figures after they leave must set
   `with_report_retention(Keep)`; the README says so. `with_report_retention`,
   `with_report_cadence`, and `with_report_budget` are recorded in the plugin
   descriptor, so they are chosen before a run's first boundary and changing
   any of them needs a new run: a restore, an exact replay, or a fork of an
   existing run with a plugin of another setting is refused.
5. **Interval reports show the state as of the last due boundary.** A host
   that needs the change in the boundary that made it binds `OnChange`, at
   the cost Phase 2 measured.

**Open questions for the owner** (rulings of this run in the review record;
the owner may reverse): none new. The orchestrator ruled on paging (E4 later),
retention (schema policy plus the fiscal knob; host-driven retirement for
`Keep` schemas deferred), one event per directive, and fiscal's default share
(half, with the engine budget sized from the gate arithmetic).

**Open questions for reviewers.**

1. Does any plugin or host depend on `FullHistory` of fiscal reports or on
   the positional meaning of holder-local IDs?
2. Is `KnowledgeQueryResult::retention` the right place to tell a research
   host that `FullHistory` equals `CurrentHeads`, or should the schema listing
   carry it?
3. Should the summary at a due time be a binding flag (`confirm_unchanged`)
   rather than always on, for hosts whose observers are many and whose
   holders never read `as_of`?

## Review record

The first version was reviewed by an engine reviewer ("implement with
changes": three blocking findings, eight changes, seven notes) and a
host-side reviewer ("not ready until B1, B2 fixed": two blocking findings,
eight changes, six notes); the orchestrator ruled on the open questions. Each
finding is accepted, rejected with a technical reason, or made moot by the
per-observer decision, and the section that resolves it is named.

| Finding | Resolution |
| --- | --- |
| Engine B1 office cycle slots reused before their lines clear; `last_seen` and revision across cycles | Moot: office parts, office slots, heads, and `last_seen` are withdrawn; per-observer slots are allocated and freed by the report system itself in one phase, as in 0.16.0 ([Fiscal reports](#fiscal-reports-per-observer-parts-kept-office-parts-rejected)). The generation fix was adopted in the office draft before the model was dropped. |
| Engine B2 one event per directive breaks `event_audience` | Accepted: `KnowledgePublished { holders, record_count, entry_count }`, `EventAudience::KnowledgeHolders`, both in the API delta ([Events](#shared-publication-and-one-copy-per-body)). |
| Engine B3 packages not disjoint; name every file; who rewrites `Publisher::publish` | Accepted: every file listed; E5f names the two fiscal hunks the engine worker changes; F1 to F4 never touch them; F5 replaces them after the merge ([Packaging](#packaging-for-two-workers)). |
| Engine C1 gate arithmetic and budget size | Accepted with the ruling: budget 32,768 bodies, 32,768 entries, 64 MiB payload, from one dirty cycle at about 9,600 bodies and 18 MB; fiscal's default share one half; several cycles owed take more than one boundary; system order defined ([Engine publication budget](#engine-publication-budget)). |
| Engine C2 chunked-commitment canonicalization cost | Accepted: 32-byte leaf hashes cached per entry, binary blake3 over 256 leaves, holder roots, a ledger root over holder roots; no re-chunking when a holder is added ([Persistence](#persistence-hashing-replay-and-formats)). |
| Engine C3 empty holders and `next_local` | Accepted ([Empty holders](#shared-publication-and-one-copy-per-body)). |
| Engine C4 schedule record size; due times in four places | Accepted: due times only in queue pages; the schedule record keeps `page_due` and the parked cursor; the root the least; the audit checks the chain ([Report cadence](#report-cadence)). |
| Engine C5 projection invariant vs `Interval` | Accepted: restated for the boundary in which the observer was published; the randomized test compares at `as_of` ([Fiscal reports](#fiscal-reports-per-observer-parts-kept-office-parts-rejected)). |
| Engine C6 restore validation under retention | Accepted: the validator applies the policy per boundary over a progressive ledger and compares the final ledger; a restore measurement row added ([Persistence](#persistence-hashing-replay-and-formats)). |
| Engine C7 byte budget publicly measurable; per-directive bounds in the read; undeclared-write class | Accepted: `KnowledgeRecordDraft::payload_bytes()`, the bounds on `KnowledgePublicationBudget`, the class stated ([Engine publication budget](#engine-publication-budget), [What can still fail a boundary](#what-can-still-fail-a-boundary)). |
| Engine C8 cut E4 | Accepted per the ruling; listed as later work. |
| Engine N1 report system every boundary by default | Partly: `EventDriven` stays the default because the same-instant drain boundary must run the system; `with_report_cadence` runs it in the host's cadence as well ([Report-system cadence](#parked-holders-domain-entity-holders-and-the-report-system-cadence)). |
| Engine N2 overlay stores `Arc` and entries | Accepted ([Staging](#shared-publication-and-one-copy-per-body)). |
| Engine N3 `last_seen` size | Moot (no `last_seen`). |
| Engine N4 dead persons' entries; namespace-count readers keep `Keep` | Accepted ([Knowledge retention](#knowledge-retention)). |
| Engine N5 refuse only `truncated` | Accepted per the ruling ([The directive](#shared-publication-and-one-copy-per-body)). |
| Engine N6 and host C1 default share | Reconciled per the ruling: half, with the budget sized from the gate. |
| Engine N7 groups split against the per-directive caps | Accepted (F5). |
| Host B1 coarse tiers learn sub-bucket movement | Moot and accepted in effect: parts are per observer and the digest is over the rendered rows at the observer's confidence, as in 0.16.0; no tier-independent digest exists ([Row content](#fiscal-reports-per-observer-parts-kept-office-parts-rejected)). |
| Host B2 no retention opt-out; closed cycles erased | Accepted: `with_report_retention`, default `RetireWhenSuperseded`, in the descriptor through the schema's `retention`; the loss stated for the README; host-driven retirement for `Keep` schemas deferred per the ruling ([Knowledge retention](#knowledge-retention)). |
| Host C2 defaults, epoch, `staggered`, calendar cadences | Accepted ([Report cadence](#report-cadence)). |
| Host C3 = engine C5 | Accepted. |
| Host C4 silence at a due time | Accepted: the summary is republished at every due time once the report exists, one body and one entry ([What a boundary publishes](#fiscal-reports-per-observer-parts-kept-office-parts-rejected)). |
| Host C5 tier or granularity collision on rebinding | Moot: two bindings of one holder are two observer records with their own lines; no collision exists. |
| Host C6 cursor over-approximates staleness | Accepted by keeping the exact 0.16.0 bitmap; the cursor is withdrawn. |
| Host C7 contradictions with world-scale-fiscal.md; the retirement deviation | Accepted: the amendments listed in [Documentation delta](#documentation-delta); the deviation and its reason recorded in [Knowledge retention](#knowledge-retention). |
| Host C8 terminology | Accepted: 已清退条目 replaces 退役知识记录; the cycle-slot, report-part, queue-page, and observer-record rows stay valid under the per-observer model; 机构持有者's status stated ([Terminology candidates](#terminology-candidates)). |
| Host N1 `record_count` is the directive's total | Accepted. |
| Host N2 expose retention to research hosts | Accepted: `KnowledgeQueryResult::retention`; reviewer question 2 asks whether the schema listing is better. |
| Host N3 an ex-holder's parts never clear | Accepted and stated as bounded ([Knowledge retention](#knowledge-retention)). |
| Host N4 military checks bodies and bytes; `visibility` per group; grants unchanged | Accepted ([Every other publisher](#engine-publication-budget)). |
| Host N5 same-boundary evidence reference | Verified: `validate_proposal_evidence_reference` (`settlement.rs:2891`) accepts a version this boundary established when it is among the boundary's staged record changes; the office version an institution part cites is committed state in any case ([Coarser rows](#coarser-rows-for-upper-observers)). |
| Host N6 `omitted_offices` silently | Moot (no office cap; `omitted_cycles` stays as in 0.16.0). |
| Owner Q1 to Q4 | Ruled by the orchestrator in this run as recorded in [Decisions on one page](#decisions-on-one-page) and [Knowledge retention](#knowledge-retention). |
| Profile items 1 to 8 | Folded: E0 ([Packaging](#packaging-for-two-workers)); copy targets and index keys ([Shared publication](#shared-publication-and-one-copy-per-body)); the byte-led budget and the expected cost ([Engine publication budget](#engine-publication-budget), [Measurement plan](#measurement-plan-and-gate)); office parts re-checked and rejected ([Fiscal reports](#fiscal-reports-per-observer-parts-kept-office-parts-rejected)); the two retention assumptions ([Profile results](#profile-results)); fiscal cost cuts (F4); the mid-boundary refresh ([Engine findings outside this package](#engine-findings-outside-this-package)). |

The delta reviews of the second version found no blocking design hole and
asked for these, all accepted and stated as decided (the workers code to
them):

| Finding | Resolution |
| --- | --- |
| Host D1 and engine D2 one `Interval` queueing rule; the queue entry's due field defined precisely | Accepted: the entry always exists, is created at the binding's first aligned time and re-created at the next aligned time whenever handled, published or not; `merge_queue_entry` keeps an `Interval` entry's `due_at` and unions cycles and parts; settlement reads no observer record; the audit checks exactly one entry per active `Interval` binding; `due_at` is the one due field and `next_due` is dropped; the fiscal worker reports the final field names and the row is adjusted ([Report cadence](#report-cadence)). |
| Host D2 the three fiscal knobs are run-bound | Accepted: stated in Risk 4 and in the README delta ([Risks](#risks-and-open-questions), [Documentation delta](#documentation-delta)). |
| Host D3 "report share" (报告份额), not "report budget" | Accepted; the code identifiers keep "budget" ([Terminology candidates](#terminology-candidates)). |
| Host D4 `KnowledgeQueryResult::retention` type | Accepted: `BTreeMap<KnowledgeSchemaId, KnowledgeRetention>` over the schemas the query named, or the returned records' schemas when it named none ([Queries and paging](#shared-publication-and-one-copy-per-body), [Public API delta](#public-api-delta)). |
| Engine D1 (blocking) fiscal tests read engine shapes | Accepted: E5f edits the five test hunks minimally on the engine branch; the fiscal branch routes those reads through helpers in `tests/support/mod.rs`; at merge the fiscal worker keeps its helpers and ports their bodies in F5 ([Packaging](#packaging-for-two-workers)). |
| Engine D3 the byte charge defined once | Accepted: `serde_json::to_vec(&payload).len()`, computed the same way by the engine and `payload_bytes()`, from canonical bytes only if byte identity is proven, with a test asserting equality ([Validation](#shared-publication-and-one-copy-per-body)). |
| Engine D4 digest index keys check kind and exact subjects | Accepted ([Index keys](#shared-publication-and-one-copy-per-body)). |
| Engine D5 E0 index in an `Arc` shared by `fork()` and snapshot validation, three call sites, no format change | Accepted ([Packaging](#packaging-for-two-workers)). |
