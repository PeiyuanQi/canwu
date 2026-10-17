# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-10-16 at Canwu **0.17.0** (tag `v0.17.0`, commit `5fc7e04` on `main`). This file is the to-do list of
> engine gaps that a downstream application has filed against the published crates. The application is Celestial
> Mandate (CM); its gap ledger (implementation document `59`, in the CM repository) gives each row its id (`G-nn`) and
> the full evidence. Priorities are the ledger's severities: High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.17.0 closed G-15 / G-16 phase 3 (fiscal observers at scale) and committed the continuation gate as a release test
> (the first part of E-27). No downstream row of High or Medium severity is open. What remains: the series-scale rows
> (§4.1) and the engine-side findings of the 0.16.0 and 0.17.0 runs (§4.2).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | 0.17.0 is published: all 24 library crates on crates.io, tag `v0.17.0`, GitHub release, website deployed (canwu.org shows 0.17.0 in both languages). 0.17.0 is the only live version: 0.16.0 was yanked on 2026-10-16 (0.15.0 and 0.14.0 earlier). Downstream still pins `=0.14.0` for `canwu-api`, `canwu-fiscal`, `canwu-production`, and `canwu-resource` and has **adopted none of 0.15.0, 0.16.0, or 0.17.0**. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.17.0`); nothing written by an earlier release is read; no migration. |
| Proposal of 0.17.0 ("implemented in 0.17.0") | `docs/proposals/fiscal-observers-at-scale.md` (G-15 / G-16 phase 3: publication budget, shared publication, retention, report cadence, institution granularity, parked rotation; its "As implemented" section lists every deviation). `docs/proposals/world-scale-fiscal.md` records phases 0 to 2. |
| Branches and worktrees | Only `main`, locally and on the remote. The continuation gate is committed as the `continuation-gate` feature of `canwu-api` (a release test, never enabled by CI or applications). |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`: CI runs fmt, clippy, workspace tests, and the `canwu-debug` check; once before tagging, the pre-release tests (doc with warnings denied, the four examples, the Rust 1.88 check, the harness checks, `cargo test --release --workspace -- --ignored` about 15 min, clippy with the gate feature, and the gate `cargo test --workspace --features canwu-api/continuation-gate` about 36 min; only the deliberate declared-after-seal test panics, as expected). |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.17.0`; nothing from 0.14.0 to 0.16.0 loads. (2) Knowledge publication: `PublishKnowledge` takes groups (wrap an old directive as one `Explicit` group of one holder), a publisher reads `knowledge_publication_budget()` before it proposes, every knowledge schema names a `retention` (`Keep` keeps 0.16.0 behavior), `KnowledgePublished` lists holders. (3) Fiscal: observer bindings take `cadence` (`OnChange` default, `Interval` with `staggered`) and `granularity` (`Scope` default, `Institution`); set `historical_context.updated_at` before adding interval observers; the report share (`with_report_budget`), report retention (`with_report_retention`, default `RetireWhenSuperseded`: closed-cycle reports are not kept unless `Keep`), and wake cadence (`with_report_cadence`) are fixed for a run. (4) Saves: a continuation checkpoint, saved from a fork on a worker thread; a paged checkpoint keeps the whole knowledge ledger in one 4 MiB envelope page, too small for thousands of report holders. (5) The 0.16.0 items still hold: fiscal actions in batches, never declare initial-scenario record versions as payload-required, refusals are outcomes. Read the "Known limits" list of the 0.17.0 entry of `docs/versioning.md` first. |

## 2. High

No row filed by the downstream ledger is open.

## 3. Medium

No row filed by the downstream ledger is open. See §4.2 for engine-side findings of Medium severity (E-19 to E-21, E-28).

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

### 4.2 Found during the 0.16.0 and 0.17.0 runs (engine-side; not numbered by the downstream ledger)

Each was found by a worker, a reviewer, or a profile and left out of its release on purpose; most are stated in the
"Known limits" lists of `docs/versioning.md`. `E-nn` ids are local to this file. Severity is this file's suggestion.

| Id | Sev. | Crate | Problem at 0.17.0 | Fix |
| --- | --- | --- | --- | --- |
| E-19 | Medium | `canwu-sim` | Every command's `CommandTransactionCheckpoint::capture` clones the people, letters, armies, plugin components, and scheduled actions maps: about 0.15 ms per command in the 50,000-scope fiscal world. | Persistent (shared) maps for those collections, or capture only what a command touches. |
| E-20 | Medium | `canwu-sim` | Plugin components, entities, and persons are still copied every boundary; `fork()` takes about 140 ms at 65,536 records; restoring a large unsealed snapshot is slow (1.09 GB in 231 s after a world-scale fiscal burst in 0.16.0). | Persistent maps for the remaining per-boundary copies; hosts use continuation checkpoints for saves meanwhile. |
| E-21 | Medium | `canwu-sim` | A paged checkpoint can describe up to 512 MiB per record whatever few distinct bytes the provider holds, so restoring an untrusted checkpoint can allocate far more than it supplies; the compact checkpoint body is capped at 4 MiB per envelope page. | A host-supplied restore budget for assembled bytes (new public API); lift the envelope page cap (see E-32). |
| E-22 | Low | reference crates | The force-supply / economy package archive keeps every maintenance receipt and refuses more than 8,192 per root; the economy root fills to its 8,192 hot operation outcomes and then stops. | Fold acknowledged receipts into a digest chain; archive hot outcomes as resource does. |
| E-23 | Low | `canwu-production` | A plugin-scheduled completion, continuation, archive commit, or retention acknowledgement that fails production's checks still fails a boundary; duplicate or stale commit receipts stay until acknowledged; any admitted production packet, even a refused one, triggers the incident and report systems; settlement rebuild costs O(selected × hot outcomes). | Recorded refusals for plugin-scheduled packets; count duplicates per handle; skip incident draws for refused packets; an outcomes-by-execution index. |
| E-24 | Low | `canwu-resource` | An archive commit costs about 25 µs per archived record; the live root is still decoded whole once per operation; finished leases stay in the live root until their last receipt is archived; force-supply acknowledgements read the whole root; a host `Expire` naming a wrong `current_boundary` is not refused at enqueue. | Cheaper commits; split the live state; a scan-free force-supply settlement. |
| E-25 | Low | `canwu-law` | `DelegatedAuthority` records grow for the life of a run; `reject_stale_pending_proposals` is unbudgeted; the `max_provider_packets` default of 4,096 is unreviewed. | Retire delegations on expiry; budget the stale-proposal pass; confirm the default. |
| E-26 | Low | `canwu-fiscal` | Host archive operations assemble the whole fiscal state (about 1 s at world scale); the sealed cut for archive preparation is host-supplied; external-operation ids are checked for reuse only within one scope's open books; resolved auditor proposals are not archived (64 per policy); no differential test against the phase 1 single-record model. | A runtime-supplied sealed cut; record-scoped host archive helpers. |
| E-27 | Low | `canwu-sim` | `Simulation::new` accepts initial-scenario records whose declarations can never seal. (The committed continuation gate, the other half of E-27, shipped in 0.17.0.) | Refuse undeclarable initial records at load. |
| E-28 | Medium | `canwu-sim` | At world scale the continuation checkpoint grows about 3.8 MB per simulated month (172 MB after month 1, 293 MB after month 24; restore 58 s): knowledge and fiscal state stay flat, but the request index (`RequestIndex`: commands, queued commands, ingress provenance) keeps every accepted command's outcome, about 440 bytes per command. | Retire command outcomes past an admission horizon (or archive them) so a sealed run's request index stays bounded. |
| E-29 | Low | `canwu-sim` | A simulated month of the world profile takes 73 s, about 111 s by month 5, then creeps about 1 s a month (129 s by month 24). Not located; one guess is the seal rebuilding the request maps (`continuation.rs` ~758). | Profile a late month against an early one. |
| E-30 | Low | `canwu-fiscal`, `canwu-sim` | The world due instant (all 4,000 observers, one boundary) costs 2.5 s, of which fiscal 1.8 s against about 0.6 s expected, with a 472 MB heap peak for 15.7 MB of payload (about 680 allocations per report record; per-record time 190 µs at the due instant against 75 µs on a spread day). | Sample-profile the due-instant row: `serde_json::Value` trees for parts, per-observer record loads through the view, per-record `knowledge_heads`, working-set effects. |
| E-31 | Low | `canwu-sim` | A mid-boundary `ScheduleIngress` (`append_ingress`, `ingress.rs` ~1182) calls `refresh_checkpoint_hash`, computing every dirty commitment root early: 195 ms after a boundary of 56,891 writes. | Defer the refresh to the boundary's own hashing phase (touches the ingress identity contract; needs a review). |
| E-32 | Low | `canwu-sim` | A paged checkpoint keeps the whole knowledge ledger in one 4 MiB envelope page (E4 of the 0.17.0 design, deferred), so a world with thousands of report holders saves only as a continuation checkpoint. | Page the knowledge ledger in the paged checkpoint (bodies chunked, as the record store does). |
| E-33 | Low | `canwu-sim`, publishers | The publication budget is shared in dispatch order with no per-plugin reservation: a publisher that runs earlier and spends the whole budget at every same-instant boundary keeps information and fiscal deferring (no first-party plugin does), and a publisher that does not read the budget and overshoots fails the boundary (the undeclared-write class, by design). | Per-plugin reservations only if a downstream composition needs them. |

## 5. Next steps

1. **low priority: Downstream adopts 0.17.0** directly from 0.14.0: move the four pins to `=0.17.0`, add a `retention` to every knowledge schema it registers (`Keep` keeps today's behavior), wrap `PublishKnowledge` directives as groups and read the publication budget, rebuild scenarios with the 0.16.0 / 0.17.0 authoring helpers, switch saves to continuation checkpoints (G-61) saved from a fork on a worker thread (G-59), move fiscal actions to batches, choose fiscal report cadence, granularity, share, and retention, and delete each fallback its ledger rows name (G-40, G-44, G-49, G-50, G-52 to G-54, G-56, G-58 from 0.15.0; G-15, G-16, G-59, G-61 from 0.16.0 and 0.17.0). The ledger records each adoption.
2. **E-28** (request-index growth of the continuation checkpoint) before any long world-scale campaign relies on saves; then the Medium findings E-19 to E-21 by downstream need (E-21's restore budget before any host restores checkpoints from untrusted sources).
3. Each change set follows the 0.17.0 pattern: a proposal reviewed by an engine reviewer and a host-side reviewer before work starts, one worktree and branch per package under `.worktrees/`, durable tests that bite, an independent code review before integration, the pre-release tests of the release skill (the continuation gate included) on the integrated tree, then one release commit with the shared docs, the website (with its copy review), and the skills.

## 6. Open decisions (owner)

Decided on 2026-10-14 and 2026-10-16 and carried out in 0.16.0 and 0.17.0: the society transfer knob, the 0.16.0 terms,
auditor appointment, G-15 High with the county-level world target, continuation checkpoint and world-scale fiscal
phases 1 and 2 in 0.16.0, §4.2 findings staying engine-side, the 0.16.0 contract choices, and the continuation gate as
a release test not run in CI with a pre-release test step in the release skill (done in 0.17.0).

Taken in the 0.17.0 run by the orchestrator, under the owner's instruction to do the next step and release, and
confirmed by the owner on 2026-10-16; each is recorded in `docs/proposals/fiscal-observers-at-scale.md`:
- Superseded report entries retire **at publication** (`RetireWhenSuperseded`), not after a seal as the 0.16.0 row
  suggested: retirement tied to a seal would make sealed and unsealed runs differ and break the lockstep gate.
- Retention is a schema-level policy plus the fiscal knob `with_report_retention` (default `RetireWhenSuperseded`);
  host-driven retirement for `Keep` schemas is deferred.
- One `KnowledgePublished` event per directive, listing its holders.
- The engine budget is sized so fiscal's default share of one half publishes one changed cycle of the world preset in
  one boundary; overshoot by a publisher that does not read the budget fails the boundary (undeclared-write class).
- Paging the knowledge ledger in the paged checkpoint (E4) is not in 0.17.0 (now E-32).
- Office-aligned shared report parts were designed and withdrawn on the profile's evidence; per-observer parts kept,
  courts at institution granularity on the world preset.
- `with_report_cadence` sets a wake cadence (a separate waking system), because the engine refuses two phase-13
  writers of the fiscal records.
- 0.16.0 was yanked after 0.17.0 was verified on crates.io, following the one-live-version practice.

Decided on 2026-10-16 (owner), the Chinese terms of the 0.17.0 concepts, on `main` in the docs and website:
知识记录正文 knowledge record body (short form 正文, 记录正文 on the knowledge page beside an information item's
正文; the owner first wrote 知识记录主体, then chose 正文 because 主体 already names a record's subjects), 持有人条目
holder entry, 持有人本地记录号 holder-local record ID, 共享发布 shared publication, 发布预算 publication budget,
知识留存策略 knowledge retention, 已清退条目 cleared entry, 报告节律 report cadence, 报告日程记录 report schedule,
机构粒度 institution granularity, 报告份额 report share, 搁置轮询 parked rotation, 唤醒节奏 wake cadence.

Open: none.

## 7. Working rules

- Follow `AGENTS.md`: change-surface map, independent review for persistence, replay, determinism, and performance changes, tests only as durable evidence, and one coherent commit per change set when a commit is asked for.
- Test in proportion to the change; the integrating agent runs the full verification once for the integrated whole, and the pre-release tests of the release skill (the continuation gate included) once before tagging.
- Keep application vocabulary out of the core: proposals describe generic contracts, and downstream ids (`G-nn`) appear only as "downstream gap G-nn".
- A plugin boundary decision must not depend on retained evidence: sealing removes it and exact replay keeps it. Read record versions through `replay_stable_domain_record_version`, knowledge to supersede through `knowledge_heads`, and treat `knowledge_changes_by_correlation*` as evidence reads. Never declare initial-scenario record versions as payload-required.
- An expected refusal is a recorded rejection, a skipped packet with a trace, or a deferral, never a failed boundary: a failed boundary cannot be stepped past, and due ingress cannot be cancelled. Every new operation states which refusals can still fail a boundary. A knowledge publisher reads the publication budget before it proposes.
- Never iterate an `im` ordered map or set in reverse and never use `get_prev` / `lookup_prev` (im 15.1 drops elements past three tree levels).
- Same version only: no compatibility work with an earlier release's bytes or hashes.
- Releases, tags, crates.io publication, and yanks are the owner's.
- When a row ships, delete it here, record it in its proposal and `docs/versioning.md`, and the downstream ledger records the adoption.
