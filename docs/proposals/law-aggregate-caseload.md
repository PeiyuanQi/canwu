# Law: Aggregate Caseload, Plan Records at Scale, and Bounded Bookkeeping

Status: implemented in 0.18.0, in `canwu-law` and, for one item,
`canwu-information`. It closes downstream gap G-30 (and its twin G42-1): an
aggregate caseload record per jurisdiction and period. It also fixes the
law rows of the [Beyond fiscal](world-scale-fiscal.md#beyond-fiscal-caps-that-block-the-target)
table that cost a world of about 1,400 jurisdictions on every law boundary,
and the three parts of engine finding E-25: delegation records that grew for
the life of a run, the unbudgeted stale-proposal pass, and the unreviewed
`max_provider_packets` default. Same version only: every changed format is
versioned and nothing earlier is read.

The contracts it changes are [legal institutionalization](legal-institutionalization-framework.md),
[legal storage sharding and compaction](legal-storage-sharding-compaction.md),
and [law, information, and military correctness](law-information-military-correctness.md).

## Aggregate caseload (downstream gap G-30)

### Problem

The law extension models proceedings one by one: a `LegalCase` with its
findings and rulings, inside the legal ledger. A world that tracks routine
justice for about 1,400 counties needs counts, not proceedings: how many
cases of each class arrived and were disposed of in a month, how many wait
for review at each level above, and how well each county keeps up. Recording
each routine case as a `LegalCase` would put tens of thousands of hot
proceedings in the ledger, whose whole hot state is capped at 128 MiB and is
decoded per shard at every boundary that touches it, and would make a host
pay a proceeding's cost for a number.

### Decision

A plan may declare a caseload profile; a host reports aggregate caseload per
jurisdiction and period; the law admission system (phase 7) settles each
report into one record with its clearance rate and the stocks it carries.

```rust
LegalDefinition.caseload: Option<CaseloadProfileDefinition> // omitted when None
CaseloadProfileDefinition {
    case_classes: Vec<String>,        // 1..=64 canonical IDs, sorted at compile
    review_levels: Vec<String>,       // 0..=16
    retained_periods: u16,            // 1..=120 kept periods per jurisdiction
    max_reports_per_boundary: u32,    // 1..=16,384 reports examined per boundary
}
// packet caseload_intake_v1 (LAW_CASELOAD_INGRESS): {"reports": [CaseloadIntakeReport]}
CaseloadIntakeReport {
    jurisdiction: String,             // a compiled jurisdiction
    period: CaseloadPeriod { start, end },   // [start, end), end <= boundary time
    opening: Option<CaseloadStock>,   // sets the first period's stocks, checks later ones
    intake_by_class, resolved_by_class: BTreeMap<String, u64>,
    review_by_level: BTreeMap<String, CaseloadReviewFlow { entered, concluded }>,
    evidence: Vec<EvidenceRef>,
}
AggregateCaseloadRecord {             // record canwu.law/caseload_period
    jurisdiction, ordinal, period, intake_by_class, resolved_by_class,
    pending_by_class,                 // opening + intake - resolved, per class
    review_queue_by_level: BTreeMap<String, CaseloadReviewQueue { entered, concluded, pending }>,
    clearance_bp: Option<u32>,        // resolved * 10,000 / intake; None when intake is 0
    evidence, report: EvidenceRef,    // the carrying packet
    settled_at: SimTime,
}
LegalCaseloadHead { format_version, jurisdiction, settled_periods } // canwu.law/caseload_head
```

Settlement rules:

- Periods of a jurisdiction are contiguous: the first report may start
  anywhere; every later one starts where the newest settled period ends.
  A period of any length may cover a gap.
- Stocks are conserved: a class cannot resolve more than its opening pending
  plus its intake, and a review level cannot conclude more than its opening
  queue plus its entries. Review levels are queues of their own; the engine
  does not tie a level's entries to the classes resolved below it.
- Idempotent per (jurisdiction, period): a report that repeats the newest
  settled period exactly is already settled and changes nothing; one that
  names it with other content conflicts. This is decided first, comparing
  evidence as data, so a retry at a later boundary is already settled even
  when its evidence can no longer be cited; only a report that would settle
  has its evidence checked.
- Evidence: at most 16 references and 2,048 encoded bytes per report, after
  deduplication (`MAX_CASELOAD_EVIDENCE_PER_REPORT`,
  `MAX_CASELOAD_EVIDENCE_BYTES`).
- Zero counts are left out of records, so a sparse class costs nothing.
- Every examined report counts against `max_reports_per_boundary`; a report
  past it is refused, and a packet that carries more reports than the budget
  on its own is refused whole.

Each refused report changes nothing and is recorded with
`canwu.law.caseload_rejected.v1` (`LAW_CASELOAD_REJECTED_EVENT`), whose
summary names the packet, the report's index, the jurisdiction, the period,
and a stable code (`CaseloadRefusalReason::code`):

| Code | When |
| --- | --- |
| `caseload_jurisdiction_unknown` | the plan compiles no such jurisdiction |
| `caseload_class_unknown`, `caseload_level_unknown` | a count names an undeclared class or level |
| `caseload_period_invalid` | the period is empty or ends after the boundary |
| `caseload_period_not_next` | neither the newest period nor the next one |
| `caseload_period_settled` | an exact repeat of the newest period |
| `caseload_period_conflict` | the newest period with other content |
| `caseload_opening_mismatch` | a stated opening differs from the carried stocks |
| `caseload_resolved_exceeds_pending`, `caseload_review_exceeds_queue` | a stock would go negative |
| `caseload_count_overflow` | a count overflows `u64` |
| `caseload_evidence_budget` | more than 16 evidence references or 2,048 bytes of them |
| `caseload_evidence_unavailable` | cited evidence fails the legal evidence rule |
| `caseload_boundary_budget` | past `max_reports_per_boundary` |

A packet that does not decode, that the plan has no caseload profile for, or
that is larger than the budget is refused whole with
`canwu.law.ingress_rejected.v1`.

**Records and retention.** Per jurisdiction, one head
(`legal_caseload_head_reference`, id `caseload:<jurisdiction>`) and one record
per kept period in `retained_periods` slots
(`legal_caseload_period_reference`, id `caseload:<jurisdiction>:<slot>`); the
period with ordinal `n` is in slot `(n - 1) % retained_periods`, so a new
period overwrites the oldest kept one. A jurisdiction holds at most
`retained_periods + 1` records, and settling a report writes two small records
whatever the retention. A first draft kept every period of a jurisdiction in
one record; it cost 650 to 740 ms a monthly burst once twelve periods were
kept, because each report rewrote the whole history.

**Independence and evidence.** The caseload path reads the profile and the
jurisdiction index from the plan record by pointer, without decoding the plan,
and never loads the legal runtime; its records do not count toward the legal
state budget (they have their own bound, below).
Cited evidence is checked as legal mutation evidence is (an exact record
version must be current, proposed this boundary, or initial-scenario; an
ingress must be queued or admitted by this boundary); the records keep the
references as data and declare no identity evidence, so they never hold a seal
back. Every decision reads committed caseload records, the plan, and the
boundary's admitted packets, so a sealed run, its unsealed fork, a restore
from sealed segments, and exact replay decide alike. Activation checks every
head and period record against the plan: format, jurisdiction, the slot of
each kept ordinal, contiguity, the stock chain, canonical counts, and the
clearance rate.

**Bound.** The caseload records of a plan are bounded when the plan compiles.
A period record's payload is at most

    W(j) = 1,024 + len(j) + sum over classes of 3 × (len(class) + 26)
           + sum over levels of (len(level) + 96) + 2,048

bytes (fixed fields at their widest, the jurisdiction ID, each class in three
count maps with a 20-digit count, each review level with three counts, and the
evidence cap), and a head's at most `128 + len(j)`. A profile compiles only if

    sum over the plan's jurisdictions j of (128 + len(j) + retained_periods × W(j))

is at most `MAX_CASELOAD_STATE_BYTES` (256 MiB;
`CaseloadProfileDefinition::max_state_bytes`), and activation refuses period
records whose evidence exceeds the cap. Numbers: the widest profile (64
classes and 16 levels of 128-byte IDs, 128-byte jurisdiction IDs) has
`W = 36,352` bytes, so 120 kept periods fit 61 jurisdictions. The measured
profile (8 seven-byte classes, 3 review levels, twelve-byte jurisdiction IDs)
has `W = 4,187` bytes: 1,555 jurisdictions with 12 kept periods are bounded
at 78 MB (12.3 MB measured), and allow up to 41 kept periods; 50,000
jurisdictions allow one. The reviewer's worst case at the conservative
budgets (512 jurisdictions × 121 records of about 38 KB, 2.4 GB) no longer
compiles. A unit test checks that the widest record a report can settle
encodes within `W`.

**Aggregate or per proceeding.** The profile's classes are the classes a host
reports in aggregate. A case the host wants to follow on its own (a party a
player knows, an appeal to the capital) it also records as a `LegalCase`, and
still counts it in its class's intake, so the aggregate stays the complete
count. The two paths share nothing in the engine; a case can cite the
jurisdiction's current caseload period record version as evidence. A slot
record is overwritten by later periods and its ID names only its slot, so such
a case should carry the period and its ordinal in its own data.

Caseload records are ground truth: a host publishes what an actor learns of
them through its own knowledge, as for any other record.

### What can still fail a boundary

An invalid persisted caseload head or period record, a head that names a
missing period record, and `EvidenceContentUnavailable` from the engine (an
engine-rule violation).

### Measured

Release build, a plan of 1,555 jurisdictions with 1,400 county institutions,
eight classes and three review levels, every county reporting in one boundary
each month, twelve kept periods: 60 to 100 ms a monthly boundary for 1,400
reports (about 40 to 70 µs a report), flat from the first month to the
fourteenth; 730 bytes a period record, 12.3 MB of caseload records with twelve
periods kept; an idle boundary stays at 23 µs.

## Plan and directory records at 1,400 jurisdictions

### Problem

The Beyond fiscal table lists the law plan and directory records as decoded
every boundary and growing with jurisdictions. Measured before this change on
the plan above (release build):

| | Before | After |
| --- | ---: | ---: |
| Law records at activation | 3,118 | 6 |
| Plan record | 917 KB, rewritten every law boundary | 917 KB, written once |
| Directory record | 612 KB | 0.9 KB |
| A boundary that settles one legal mutation | 100.7 ms | 9.4 ms |
| The same with 13 jurisdictions | 1.8 ms | 1.1 ms |
| Activation | 256 ms | 157 ms |

Timings are medians of 15 boundaries on a shared machine; while other builds
loaded it, the 1,555-jurisdiction boundary after the change measured 9.5 to
15 ms.

Every compiled order and jurisdiction had its own shard and archive-head
record, empty or not, listed three times in the directory; the plan record
carried the ledger's counters and evidence declaration, so it changed at every
law boundary; and a boundary encoded or decoded the 917 KB plan about ten
times (header decode with two copies, the scoped load's empty runtime, its
budget re-count, the persistence image, the draft, and the comparison).

### Decision

- The plan record holds only the compiled plan, its hash, and its budgets,
  declares no evidence, and is written once, when the plan is installed. The
  ledger counters (`reserved_state_bytes`, `boundary_index`,
  `last_settled_at`, `next_outbox_sequence`, `next_source_ordinal`) move to
  the coordinator shard and the identity-evidence declaration to the
  directory. A scoped load must include the coordinator.
- `LegalRuntime`'s JSON form leaves the plan out (`#[serde(skip)]`); every
  load sets `LegalRuntime::plan` from the plan it is given after checking
  `plan_hash`, and a cold load checks that the plan record holds exactly that
  plan. The live boundary decodes the plan once, by reference, and drafts
  only the directory and the shards it loaded.
- An order or jurisdiction shard and its archive head are created when they
  first hold something. A unit that names a compiled order or jurisdiction
  whose shard does not exist yet is routed as before, and loading it loads
  nothing. A live boundary whose ledger gains a shard it did not load fails
  as a broken invariant rather than writing a directory that names a record
  it did not write.

The 128 MiB legal state ceiling is unchanged: with routine cases in aggregate
caseload records outside it, a world of this size keeps only salient cases in
the ledger. What remains per law boundary at this size is the single plan
decode, about 8 ms of the 9.4 ms, which grows with the plan's institutions
and jurisdictions; the directory's object routes still grow with hot legal
objects.

## Budgeted stale-proposal check (E-25)

Before settlement, the admission system rejects pending proposals whose host
compare-and-set no longer holds or that depend on a retired culture
generation. It checked every dirty procedure at every boundary that settled,
so a pile-up of `D` dirty procedures under a budget `B` settling over `D / B`
continuation boundaries made about `D^2 / B` checks. It now checks at most
`max_mutations_per_boundary` dirty procedures, in procedure-ID order (the
order settlement selects them in), and settlement leaves every dirty procedure
after the last one checked for a continuation boundary at the same time, so no
procedure is advanced at a boundary that did not check it. A chain checks each
dirty procedure once per boundary that reaches it: at most `D` checks plus the
re-checks of procedures the mutation budget deferred, at most `B` a boundary.

## Provider index default (E-25)

The legal provider index lives in the coordinator shard and in the evidence
declaration, which every boundary that settles legal work decodes and writes.
Measured on a small plan (release build), a law boundary costs:

| Recorded provider packets | 0 | 256 | 1,024 | 4,096 |
| --- | ---: | ---: | ---: | ---: |
| Law boundary | 1.1 ms | 2.8 ms | 10.0 ms | 40.0 ms |

About 9 µs per kept packet, on every law boundary (53 ms at 4,096 before the
plan-record change). The default of 4,096 made a full index the largest cost
of a law boundary, so the default is now 256 (`DEFAULT_PROVIDER_PACKETS`): a
host cites a provider packet within 256 provider packets of its admission, or
raises `max_provider_packets` knowing the cost. The index keeps the newest
packets and forgets the lowest ingress ID first, so a boundary that admits
more provider packets than it keeps forgets the oldest of them before its own
legal work runs, and a citation of a forgotten packet, even one admitted by
the same boundary, is refused with `canwu.law.ingress_rejected.v1`. Moving the index to a record
of its own, read only by boundaries that record or cite provider packets, is
left for a later change.

## Delegations leave their ledger when they expire (E-25, `canwu-information`)

### Problem

`canwu-information` recorded each delegation command a command grant names
in a create-only `DelegatedAuthority` record and never retired it, so the
records grew with every delegation command of a run. Deleting such records
would not bound them: the engine keeps a deleted record as a tombstone with
its payload.

### Decision

One record per command grant holds the grant's live delegations
(`DelegatedAuthority`, now `Versioned`, payload
`DelegatedAuthorityLedgerPayload { format_version, authority_grant,
delegations, expiring }`, at `delegated_authority_reference(grant)`):
delegations by command ID, and an expiry index (`DelegationExpiry
{ expires_at, command }`, by time then command). The lifecycle system, at
every boundary it already runs (no wake of its own), reads the first expiry by
pointer without decoding the ledger, and when it has come, removes every
delegation whose `expires_at` is at or before the boundary time; it records
admitted delegations in the same rewrite, and does not record a claim already
expired at admission. A delegation whose `expires_at` has come authorizes
nothing even before it is removed, so an interpretation must be admitted
while its delegation is in force, even if it is dated inside the window. A
claim with no `expires_at` stays for the run.

After expiry nothing about the delegation stays in state: the interpretations
that relied on it keep the hash of its claim (`authority_claim_hash` of their
operation), and its command stays in the evidence journal until a seal removes
it. The ledger is committed state and the commands read are the boundary's
own, so sealed and unsealed runs and exact replay decide alike. An
interpretation citing an expired delegation is rejected with
`invalid_authority`, as one citing an unrecorded command is; what can still
fail a boundary is an invalid ledger record (a plugin invariant).

## Persistence, replay, and formats

- `LEGAL_SHARD_PAYLOAD_FORMAT_VERSION` 1 to 2 (the plan record layout and the
  declaration's home); `LAW_SEMANTIC_HASH` changes; `law_record_schemas`
  returns 6 schemas (adds `caseload_head` and `caseload_period`, which declare
  no evidence); `LEGAL_CASELOAD_FORMAT_VERSION` 1. A plan without a caseload
  profile keeps its encoding and hash.
- `canwu-information`: the `delegated_authority` record changes payload and
  becomes `Versioned`; `DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION` 1; the
  plugin semantic hash changes.
- No engine format changes.

## Public API

- `canwu-law`, new: `CaseloadProfileDefinition`, `CaseloadPeriod`,
  `CaseloadStock`, `CaseloadReviewFlow`, `CaseloadReviewQueue`,
  `CaseloadIntakeReport`, `AggregateCaseloadRecord`, `LegalCaseloadHead`,
  `LegalCaseloadState`, `CaseloadRefusalReason`, `LegalCaseloadHeadRecord`,
  `LegalCaseloadPeriodRecord`, `LAW_CASELOAD_INGRESS`, `LAW_CASELOAD_HEAD`,
  `LAW_CASELOAD_PERIOD`, `LAW_CASELOAD_REJECTED_EVENT`,
  `LEGAL_CASELOAD_FORMAT_VERSION`, `MAX_CASELOAD_CLASSES`,
  `MAX_CASELOAD_REVIEW_LEVELS`, `MAX_CASELOAD_RETAINED_PERIODS`,
  `MAX_CASELOAD_REPORTS_PER_BOUNDARY`, `MAX_CASELOAD_EVIDENCE_PER_REPORT`,
  `MAX_CASELOAD_EVIDENCE_BYTES`, `MAX_CASELOAD_STATE_BYTES`,
  `DEFAULT_PROVIDER_PACKETS`, `CaseloadProfileDefinition::{new,
  max_period_record_bytes, max_state_bytes}`, `caseload_head_record_id`,
  `caseload_period_record_id`,
  `legal_caseload_head_reference`, `legal_caseload_period_reference`,
  `enqueue_caseload_intake`, `load_legal_caseload`; fields
  `LegalDefinition::caseload` and `CompiledLawPlan::caseload`;
  `CompiledLawPlan` implements `Default`.
- `canwu-law`, changed: `LawBudgets::max_provider_packets` defaults to 256;
  `LegalRuntime`'s serialized form omits `plan`, and `LegalRuntime::from_state`
  sets it from the plan it is given; `LegalDefinition` gains a field (struct
  literals add `caseload: None`).
- `canwu-information`, new: `DelegatedAuthorityLedgerPayload`,
  `DelegationExpiry`, `DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION`; changed:
  `delegated_authority_reference(authority_grant)` takes only the grant, and
  `DelegatedAuthority`'s payload is the ledger.

## Tests

- `crates/extensions/canwu-law/tests/law_caseload.rs`:
  `aggregate_caseload_settles_conserves_and_refuses_without_failing_the_boundary`
  (conservation, clearance, thirteen refusal codes in order, the whole-packet
  budget refusal, slot overwrite at two kept periods, exact replay);
  `caseload_decides_alike_in_sealed_and_unsealed_runs` (the fourteenth code;
  a retry of a period settled before the seal whose cited packet is no longer
  citable is already settled, not refused for its evidence, and a new period
  citing a packet the seal may have dropped is refused, alike in a sealed run,
  its unsealed fork, and a restore that validates the caseload records; the
  run seals again; its replay equals the fork); and
  `activation_refuses_forged_caseload_records` (a broken stock chain, swapped
  slots, and a headless period record each block activation, which every
  restore runs).
- `crates/extensions/canwu-law/src/caseload.rs`:
  `the_widest_caseload_records_fit_their_bound` (unit test).
- `crates/extensions/canwu-law/tests/legal_ingress_refusal.rs`:
  `stale_proposal_checks_stay_within_the_boundary_budget` (five stale
  proposals under a budget of two are rejected two, two, and one per boundary;
  without the budget all five go in one boundary, and without the selection
  rule three are advanced unchecked and stay open).
- `crates/extensions/canwu-information/tests/runtime_adapter.rs`:
  `an_expired_delegation_leaves_its_ledger_and_authorizes_nothing`.
- Updated for the new layout: `legal_persistence_exports_independently_versioned_format8_shards`
  (8 records, no empty jurisdiction shard),
  `live_proposal_mutation_is_settled_inside_kernel_record_cas` (the plan
  record stays at version 1),
  `law_plugin_activation_rejects_a_forged_identity_evidence_declaration`
  (forges the directory's declaration), the schema counts, and the
  information delegation test and schema unit test.

## Deviations from the filed fix

- The filed shape has one `AggregateCaseloadRecord` per jurisdiction and
  period; it is kept, with `ordinal`, `pending_by_class`, the review flows,
  the carrying `report`, and `settled_at` added, and stored in a fixed ring of
  slots with a head per jurisdiction so records stay bounded.
- `clearance_bp` is `Option<u32>`: absent when nothing arrived.
- The filed fix has salient `LegalCase`s cite the aggregate period; no new
  field was added: a case may cite the period record's current version as
  evidence, and the two paths stay independent.
- Only the newest period is idempotent: a repeat of an older one is refused as
  not next, since its record may have been overwritten.

## Known limits

- A law boundary on a plan of 1,555 jurisdictions and 1,400 institutions
  still decodes the plan once (about 8 ms); the directory's object routes grow
  with hot legal objects.
- The provider index still costs about 9 µs per kept packet on every law
  boundary.
- A delegation claim without `expires_at` stays in its ledger for the run, and
  a ledger is decoded whole when it changes or is cited.
- Caseload history beyond `retained_periods` is the host's to keep.
