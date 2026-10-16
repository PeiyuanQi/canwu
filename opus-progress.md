# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-10-16 at Canwu **0.16.0** (tag `v0.16.0`, commit `2a3a1f7`; `main` at `42bd3c7` with the approved terms). This file is the to-do list of engine
> gaps that a downstream application has filed against the published crates. The application is Celestial Mandate
> (CM); its gap ledger (implementation document `59`, in the CM repository) gives each row its id (`G-nn`) and the
> full evidence. Priorities are the ledger's severities: High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.16.0 closed G-61 (continuation checkpoint), G-59 (`Send` runtime), phases 1 and 2 of G-15 / G-16 (world-scale
> fiscal), and all eighteen engine-side findings of the 0.15.0 run (E-01 to E-18). What remains: phase 3 of G-15 /
> G-16 (§2), the series-scale rows (§4.1), and the engine-side findings of the 0.16.0 run (§4.2).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | 0.16.0 is published: all 24 library crates on crates.io, tag `v0.16.0`, GitHub release, website deployed (canwu.org shows 0.16.0 in both languages). 0.16.0 is the only live version: 0.15.0 was yanked on 2026-10-16 (0.14.0 on 2026-10-14). Downstream still pins `=0.14.0` for `canwu-api`, `canwu-fiscal`, `canwu-production`, and `canwu-resource` and has **adopted neither 0.15.0 nor 0.16.0**. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.16.0`); nothing written by an earlier release is read; no migration. |
| Proposals of 0.16.0 (all "implemented in 0.16.0") | `docs/proposals/`: `knowledge-ledger-cost-and-authentication.md` (E-04, E-06), `fiscal-and-society-limits.md` (E-10, E-11, E-16, transfer settlement knob, auditor appointment), `resource-refusals-without-stalls.md` (E-01, E-02, E-13 to E-15), `production-archive-volume.md` (E-07, E-12), `law-information-military-correctness.md`, `world-scale-fiscal.md` (G-15, G-16 phases 1 and 2), `record-store-ceiling-and-cost.md`, `continuation-checkpoint.md` (G-61, E-05, E-08, E-09), `reference-archive-and-replay-profile.md` (E-17, E-18), `command-admission-history-index.md`, `resource-root-write-cost.md` (E-03), `send-runtime.md` (G-59). |
| Branches and worktrees | Only `main`, locally and on the remote. Untracked here: this file and `continuation-gate.patch` (the local continuation gate harness, see §4.2 E-27). An empty `.worktrees/integration` folder may remain if a process still held it at cleanup; delete it when free. |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.16.0`; nothing from 0.14.0 or 0.15.0 loads. (2) Saves: write a continuation checkpoint instead of a full save; seals work with work in flight; a fork can be saved on a worker thread (`Canwu` is `Send`; archive providers are `Arc<dyn … + Send + Sync>`). (3) Fiscal is record- and batch-based (`FiscalActionBatch`, per-record revisions, `into_records`), with a fiscal archive and rebindable observers. (4) Never declare initial-scenario record versions as payload-required; a command can no longer schedule a delayed `EnqueuePluginIngress`. (5) Refusals that used to fail a boundary are recorded rejections, skipped packets with trace events, or deferrals; read outcomes. Read the "Known limits" list in the 0.16.0 entry of `docs/versioning.md` first. |

## 2. High

| Gap | Crate | Problem at 0.16.0 | Fix | Downstream effect until fixed |
| --- | --- | --- | --- | --- |
| **G-15 / G-16, phase 3** (observers at scale) | `canwu-fiscal` | Phases 1 and 2 removed every lifetime cap and measured a world of 50,000 scopes, 4,200 institutions, and 4,000 observers (idle boundary about 1 ms; an ordinary day about 0.26 s with reports). Report publication is limited to 64 holders per boundary, so 4,000 observers drain over about 63 boundaries at one instant, and a burst of 50,000 assessments takes about 7 s in one boundary. | Three caps meter publication: the engine's `KnowledgeLimitsV1::batches_per_system_boundary` (64, one holder per batch; the cap that binds), fiscal's `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024), and the engine's `records_per_boundary` (10,000). A drain boundary costs about 74 ms (the burst's 62 drains: 4.6 s), not yet profiled. Suggested package, in order: (1) profile one drain boundary (queue-page scan, row assembly, digests, `knowledge_heads`, engine insert); (2) engine: replace the 64-batch count with a measured budget of records and holder entries per boundary, and lift fiscal's 1,024 to match, so 4,000 observers fit one boundary; (3) engine: shared publication (one batch to many holders, records stored once, a head entry per holder), and fiscal publishes each changed scope part once to every observer bound to that scope (holder-side assembly); (4) `Interval` cadence with per-observer offsets as the world-scale default, `OnChange` for the few observers that need it; (5) coarser rows for upper observers (institution or jurisdiction rollups); (6) retire superseded report records after a seal, so held reports stop growing. Gate on the world preset: 4,000 observers due at one instant publish in one boundary, with a stated report cost, and held knowledge stays flat over simulated years. | A downstream world with thousands of report holders sees reports arrive over many same-instant boundaries. |

## 3. Medium

No row filed by the downstream ledger is open. See §4.2 for engine-side findings of Medium severity (E-19 to E-21).

## 4. Low

No individual row filed by the downstream ledger is open outside §4.1.

### 4.1 Low, series scale (deferred until a downstream series build needs them)

| Gap | Crate | Limit | Fix |
| --- | --- | --- | --- |
| G-17 | `canwu-resource` | One completion lease per irreversible request, so 500+ households need 500+ leases. The single-step lease cuts each to 3 terminal records and 1 lookup, but each active lease still reserves 16 slots. | A `ResourceConsumptionBatchRequestV1` under one lease, one outcome per item; propose only if profiling shows the lease budget binding. |
| G-18 | `canwu-production`, `canwu-resource` | Canonical caps: 2,048 processes and sites; 4,096 facilities, work orders, executions, and mutations per boundary. (0.16.0 raised resource archive candidates to 16,384.) | Raise caps to at least 16,384 or shard by site scope; a batching contract for monthly settlement mutations. |
| G-24 | `canwu-routing` | `TraversalModel::Departures` slots are absolute times (about 1,700 slots per connection for a 14-month ferry schedule). | `TraversalModel::Recurring { period, offsets, duration }` and an optional time-windowed `EdgeConditionOverlay`. |
| G-30 (also G42-1) | `canwu-law` | Per-proceeding records only, a 128 MiB state ceiling, no aggregate caseload. | An `AggregateCaseloadRecord { jurisdiction, period, intake_by_class, resolved_by_class, review_queue_by_level, clearance_bp, evidence }` written in phase 7 from a host `caseload_intake_v1` packet. |
| G-34 | `canwu-society` | No cohort-to-cohort relation primitive. | An `IntergroupRelationEdge`; propose only when a second downstream consumer exists. |

Recorded but not proposed: G-33 (`OrganizationRelation.relation` is an untyped string; downstream validation is enough) and G-38 (the `canwu-military` shape; relevant only if downstream ever reconsiders adopting it).

### 4.2 Found during the 0.16.0 run (engine-side; not numbered by the downstream ledger)

Each was found by a worker or an independent reviewer and left out of the release on purpose; most are stated in the
0.16.0 "Known limits" list of `docs/versioning.md`. `E-nn` ids are local to this file and continue the 0.15.0 run's
numbering. Severity is this file's suggestion.

| Id | Sev. | Crate | Problem at 0.16.0 | Fix |
| --- | --- | --- | --- | --- |
| E-19 | Medium | `canwu-sim` | Every command's `CommandTransactionCheckpoint::capture` clones the people, letters, armies, plugin components, and scheduled actions maps: about 0.15 ms per command in the 50,000-scope fiscal world, half of what admission still costs after the command-history index. | Persistent (shared) maps for those collections, or capture only what a command touches. |
| E-20 | Medium | `canwu-sim` | Plugin components, entities, and persons are still copied every boundary; `fork()` takes about 140 ms at 65,536 records; restoring a large unsealed snapshot is slow (1.09 GB in 231 s after a world-scale fiscal burst). | Persistent maps for the remaining per-boundary copies; hosts use continuation checkpoints for saves meanwhile. |
| E-21 | Medium | `canwu-sim` | A paged checkpoint can describe up to 512 MiB per record whatever few distinct bytes the provider holds (a leaf may name one chunk many times), so restoring an untrusted checkpoint can allocate far more than it supplies; the compact checkpoint body is capped at 4 MiB per envelope page. | A host-supplied restore budget for assembled bytes (new public API); lift the envelope page cap. |
| E-22 | Low | reference crates | The force-supply / economy package archive keeps every maintenance receipt and refuses more than 8,192 per root (0.16.0 refuses host-side at enqueue instead of failing a boundary); the economy root fills to its 8,192 hot operation outcomes and then stops. | Fold acknowledged receipts into a digest chain; archive hot outcomes as resource does. |
| E-23 | Low | `canwu-production` | A well-formed completion, continuation, archive commit, or retention acknowledgement that another plugin schedules and that fails production's checks still fails a boundary; receipts of duplicate or stale commits stay until acknowledged (only the 256 MiB root cap bounds them); any admitted production packet, even a refused one, triggers the incident and report systems (a host can cause incident draws with junk packets); settlement rebuild costs O(selected × hot outcomes). | Recorded refusals for plugin-scheduled packets; count duplicates per handle; skip incident draws for refused packets; an outcomes-by-execution index. |
| E-24 | Low | `canwu-resource` | An archive commit costs about 25 µs per archived record (16,384 records about 0.5 s in one boundary); the live root is still decoded whole once per operation; finished leases stay in the live root until their last receipt is archived; force-supply acknowledgements read the whole root; a host `Expire` naming a wrong `current_boundary` is not refused at enqueue. | Cheaper commits; split the live state; a scan-free force-supply settlement. |
| E-25 | Low | `canwu-law` | `DelegatedAuthority` records grow for the life of a run; `reject_stale_pending_proposals` is unbudgeted (about D²/B checks over a deferred pile-up, bounded by `max_procedures`); the `max_provider_packets` default of 4,096 is unreviewed. | Retire delegations on expiry; budget the stale-proposal pass; confirm the default. |
| E-26 | Low | `canwu-fiscal` | Host archive operations assemble the whole fiscal state (about 1 s at world scale); the sealed cut for archive preparation is host-supplied; external-operation ids are checked for reuse only within one scope's open books; resolved auditor proposals are not archived (64 per policy); no differential test against the phase 1 single-record model. | A runtime-supplied sealed cut; record-scoped host archive helpers. |
| E-27 | Low | test infrastructure | The continuation gate (every flow sealed before every host step, restored from continuation checkpoints at 2^k boundaries, exactly replayed at 3·4^k, compared in lockstep) is a local harness only: `continuation-gate.patch` next to this file, last applied on the integrated tree before the release commit. `Simulation::new` accepts initial-scenario records whose declarations can never seal. | Owner decision (2026-10-16): commit the gate as a test-only feature marked as a release test, **not run in CI**, and give the release skill a pre-release test step that runs it (see §6). Separately, refuse undeclarable initial records at load. |

## 5. Next steps

1. **low priority: Downstream adopts 0.16.0** directly from 0.14.0: move the four pins to `=0.16.0`, rebuild scenarios with the 0.16.0 authoring helpers, switch saves to continuation checkpoints (G-61) saved from a fork on a worker thread (G-59), move fiscal actions to batches, and delete each fallback its ledger rows name (G-40, G-44, G-49, G-50, G-52 to G-54, G-56, G-58 from 0.15.0; G-15, G-16, G-59, G-61 from 0.16.0). The ledger records each adoption.
2. **G-15 / G-16 phase 3** (observers at scale), against the world preset of `fiscal-world-profile`.
3. The Medium findings E-19 to E-21, by downstream need; E-21's restore budget before any host restores checkpoints from untrusted sources.
4. Each change set follows the 0.16.0 pattern: one worktree and branch per package under `.worktrees/`, a proposal, durable tests that bite, an independent review before integration, the continuation gate on the integrated tree, then one release commit with the shared docs, the website, and the skills.
5. **Before the next release (E-27):** commit the continuation gate from `continuation-gate.patch` as a test-only feature marked as a release test (not in CI; mark the deliberate declared-after-seal test as expected so a passing gate is green), and add the pre-release test step to `canwu-contributor-release` (§6). Then delete the patch file.

## 6. Open decisions (owner)

Decided on 2026-10-14 and carried out in 0.16.0: the society transfer knob (batch default, one by one optional), the
term fixes (无定位人物, 当前知识记录, 边界耗时明细, 一步式完成租约), auditor appointment (appointer, nominators,
petitioners), G-15 High with the county-level world target, continuation checkpoint in 0.16.0, world-scale fiscal
through phase 2 in 0.16.0, and §4.2 findings staying engine-side.

Decided on 2026-10-16 and on `main` (42bd3c7, docs and website only; crate READMEs need none): the Chinese terms for
the 0.16.0 concepts, as proposed except eight: 固定证据 pinned evidence, 有依据事实 provenance fact, 最新日志 journal
frontier, 操作定址抽样明细 keyed-draw ledger, 社群输入队列 society ingress queue, 请议方 petitioner, 范围明细 scope
ledger, 行动批次 fiscal action batch.

Decided on 2026-10-16 (owner):
- **The 0.16.0 contract choices are confirmed as shipped:** production and resource do not declare initial-scenario
  versions (the engine keeps refusing such declarations); a command may not schedule a delayed
  `EnqueuePluginIngress`; law refuses evidence citations of ingress the run can no longer declare; the fiscal test of
  65,537 actions in one boundary stays `#[ignore]` (release only, not in CI) and runs before each release.
- **Continuation gate (E-27):** "commit but mark as release test no need in ci, and add release skill that run test
  before release." The gate is committed as a test-only feature, marked as a release test, and not run in CI. The
  release skill gets a pre-release test step. A release skill already exists (`.agents/skills/canwu-contributor-release`,
  with its `.claude/skills` loader); its verification list has only fmt, clippy, the workspace tests, and the
  `canwu-debug` check. The step adds what the 0.16.0 release ran by hand: `cargo doc`, the public examples, the Rust
  1.88 check, the performance-harness checks (fmt, clippy, test, test with `allocation-counting`), the
  release-only `#[ignore]` tests (`cargo test --release … -- --ignored`), and the continuation gate (only the
  deliberate declared-after-seal test may fail). All run once, before the release commit is tagged.

Open: none.

## 7. Working rules

- Follow `AGENTS.md`: change-surface map, independent review for persistence, replay, determinism, and performance changes, tests only as durable evidence, and one coherent commit per change set when a commit is asked for.
- Test in proportion to the change; the integrating agent runs the full verification once for the integrated whole.
- Keep application vocabulary out of the core: proposals describe generic contracts, and downstream ids (`G-nn`) appear only as "downstream gap G-nn".
- A plugin boundary decision must not depend on retained evidence: sealing removes it and exact replay keeps it. Read record versions through `replay_stable_domain_record_version`, knowledge to supersede through `knowledge_heads`, and treat `knowledge_changes_by_correlation*` as evidence reads. Never declare initial-scenario record versions as payload-required.
- An expected refusal is a recorded rejection, a skipped packet with a trace, or a deferral, never a failed boundary: a failed boundary cannot be stepped past, and due ingress cannot be cancelled. Every new operation states which refusals can still fail a boundary.
- Never iterate an `im` ordered map or set in reverse and never use `get_prev` / `lookup_prev` (im 15.1 drops elements past three tree levels).
- Same version only: no compatibility work with an earlier release's bytes or hashes.
- Releases, tags, crates.io publication, and yanks are the owner's.
- When a row ships, delete it here, record it in its proposal and `docs/versioning.md`, and the downstream ledger records the adoption.
