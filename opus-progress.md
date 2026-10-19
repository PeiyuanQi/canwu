# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-10-19 at Canwu **0.18.0** (tag `v0.18.0`, commit `ea85ca7` on `main`). This file is the to-do list of
> engine gaps that a downstream application has filed against the published crates. The application is Celestial
> Mandate (CM); its gap ledger (implementation document `59`, in the CM repository) gives each row its id (`G-nn`) and
> the full evidence. Priorities are the ledger's severities: High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.18.0 closed every row this file held after 0.17.0: the series-scale rows G-17, G-18, G-24, G-30 (with G42-1),
> G-33, and G-34, and engine findings E-19 to E-33. Since then the downstream adopted 0.18.0 and filed seven new rows
> while building on it (G-62 to G-68, 2026-10-17 and 2026-10-18; §3 and §4), each verified in the published 0.18.0
> source. Also open: one recorded design question (G-38), the known limits the 0.18.0 packages left on purpose
> (§4.2), two limits the downstream met and worked around (§4.3), and three engine findings of the 0.19.0 source read
> (§4.5). The two engine facts the downstream design loop asked to confirm are answered (§4.4). On 2026-10-19 the
> owner decided the shape of every fix for 0.19.0 (§6).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | 0.18.0 is published: all 24 library crates on crates.io, tag `v0.18.0`, GitHub release, website deployed (canwu.org shows 0.18.0 in both languages). 0.18.0 is the only live version: 0.17.0 was yanked on 2026-10-17 (0.14.0 to 0.16.0 earlier). Downstream pins `=0.18.0` since 2026-10-17 (its release 0.77.0) and has adopted G-44, G-47 to G-54, and G-56 to G-61 (G-46 is superseded by G-61); the other released rows wait for the downstream slices that consume them. The downstream plans to pin the next release as the first change set of its next build wave, after the rows below are fixed. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.18.0`); nothing written by an earlier release is read; no migration. |
| Proposals of 0.18.0 (all "implemented in 0.18.0") | `docs/proposals/`: `recurring-traversal.md` (G-24), `society-intergroup-relations.md` (G-33, G-34), `law-aggregate-caseload.md` (G-30, E-25), `request-outcome-retention.md` (E-28, part of E-29), `fiscal-archive-and-due-instant-cost.md` (E-26, E-30), `engine-load-and-budget-guards.md` (E-27, E-31, E-33), `resource-series-scale.md` (G-17, G-18, E-24), `reference-archive-receipts.md` (E-22), `production-series-scale.md` (G-18, E-23), `runtime-state-copies.md` (E-19, E-20), `paged-restore-budget-and-seal-cost.md` (E-21, E-29, E-32). |
| Branches and worktrees | Only `main`, locally and on the remote. |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`; the pre-release tests ran once on the 0.18.0 release commit (workspace 821 passed; release-only `--ignored` 3 passed; continuation gate 821 passed, 0 failed). |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.18.0`; nothing earlier loads. (2) Read the "Upgrading from 0.17.0" notes of the 0.18.0 entry of `docs/versioning.md` and its "Known limits" first: the knowledge publication contract of 0.17.0 (groups, budget read, `retention` on every schema), request-outcome retention (a late retry of a retired request is `RequestOutcomeRetired`; treat it like `IdempotencyConflict`), resource roots installed with `into_records` (accounts and grants live in live buckets), production scope roots and the key rule (no `/` in primary-root keys, `{scope}/` in scope roots), fiscal `prepare_fiscal_archive(run, …)` and "seal at each save", paged restores take a `PagedRestoreBudget`, society settlement writes its root once, law `max_provider_packets` default 256 and delegations that expire. (3) Saves: a continuation checkpoint saved from a fork on a worker thread (forks are O(1); the first boundary after a fork pays the journal copy while the fork is alive). |

## 2. High

No row filed by the downstream ledger is open.

## 3. Medium

Filed 2026-10-18; each verified in the published 0.18.0 source. Nothing is built around them downstream: the
application ships the limitation and states it.

| Id | Crate | Gap at 0.18.0 | Proposed fix | Downstream effect |
| --- | --- | --- | --- | --- |
| G-64 | `canwu-movement` | No operation delays the arrival of a departed leg: `Reroute` is refused while a leg is `Departed` (`lifecycle.rs` `reroute`: "a departed leg must arrive or fail before a reroute"), `CompleteLeg` only arrives a leg now, and `FailLeg` then `Reroute` re-runs the leg as an interruption. | `DelayLeg { execution, leg, until }`, or `Reroute` admitting a departed current leg whose replacement keeps its departure and moves its arrival, rescheduling the leg's arrival due. | An under-way leg is never held; holds are decided before departure only. |
| G-65 | `canwu-resource` | A transfer cannot lose part of its escrow: `CompleteTransfer { Lose }` settles on `transfer.escrow`, the whole remaining quantity (`runtime.rs` `complete_transfer`). | `Lose { quantity }` (partial) leaving the remainder in escrow at the next revision, or a `SplitTransfer` before departure. | Medium once the application turns on drawn transit losses; until then no transit loss is drawn. A cargo would have to be booked as several transfers. |
| G-67 | `canwu-resource` | An installed account's `place_scope` cannot change, and `CreateAccount` refuses a place scope from a tracked command. After an application carries a store's contents to another place, the account keeps its old place scope, so `AcceptLocal` refuses a same-place transfer at the new place. | A holder or host operation that re-scopes an account to a new place, with evidence (the carriage that moved it). | A moved store cannot be paid or drawn at its new place until it returns. |
| G-68 | `canwu-resource` | Value from outside the modelled accounts cannot be credited: `Credit { source: ExternalInflow }` is refused as a holder command (`validate_resource_authority`), needs a completion certificate as provider ingress (`validate_local_provider_participant`), and no lease can be issued at the packet's own boundary time. There is no inflow counterpart to `ExternalOutflow`. | An `ExternalInflow` credit that a holder (or a host adapter) may request with its evidence, the counterpart of `ExternalOutflow`. | A monthly inflow from an off-ledger source is recorded but never booked. |

## 4. Low

Filed 2026-10-17 (G-62) and 2026-10-18; each verified in the published 0.18.0 source.

| Id | Crate | Gap at 0.18.0 | Proposed fix | Downstream effect |
| --- | --- | --- | --- | --- |
| G-62 | `canwu-api` (`CompactedCanwu`), `canwu-sim` | After a continuation restore a host cannot read the payload or id of ingress queued before the save and still pending: `restore_continuation` keeps ingress at or below the settled cut out of the retained journal and pins the pending ones in `RuntimeEvidence.pinned.ingress`; `CompactedCanwu::ingress_log` returns only the retained tail; `boundaries_between` returns queued boundaries with counts per kind, not records; `SimulationView::ingress` reads a pinned record only inside a boundary system. | `CompactedCanwu::pending_ingress()` (or `ingress_record(id)` resolving pinned records), returning the queued `IngressRecord`s a continuation checkpoint pins, as a live run's `ingress_log` holds them. | After a load the host cannot list its own pending commands (a "sent" line is missing until the command applies), and an adapter that deduplicates by payload must rely on the engine's idempotency. Nothing is lost. |
| G-63 | `canwu-resource` | A transfer carried by a multi-leg execution cannot record a completed intermediate leg or the handoff after it: `advance_transfer` admits only `PendingDispatch → InTransit`, `InTransit → ArrivalPending`, and `→ ReturnPending`; an `InTransit → InTransit` advance with a new `TransportExecutionLink { leg_execution, handoff }` is `InvalidLifecycle`. | Admit `TransferProgressV1::InTransit` from `InTransit` when the link names a later leg or a new handoff of the same execution, appending the evidence. | The transfer's custody evidence names only its first and last legs; the legs between are in the movement root only. |
| G-66 | `canwu-sim` / `canwu-resource` | Two completion leases cannot both lock one external record in the same instant, so two `AcceptLocal` completions that share a record (the two legs of one exchange) cannot settle on the same day. | Let two completion leases hold one external record's lock in the same instant when they complete disjoint transfers, or a combined completion. | The second leg of an exchange lands a day after the first. |

The source read of 2026-10-19 narrows G-66: the lock lasts from activation to completion, not for an instant, and a
host can settle several boundaries at one simulated time (the G-12 acceptance test settles two at minute 4), so the
second leg can settle at the same instant one boundary later; the day comes from settling one boundary per day. The
owner declined the engine change (§6); 0.19.0 documents the workarounds.

### 4.1 Recorded, not proposed

G-38 (the `canwu-military` shape contradicts the downstream design; relevant only if downstream ever reconsiders adopting `canwu-military`; the ledger files no fix).

### 4.2 Known limits left by the 0.18.0 packages (engine-side; not numbered by the downstream ledger)

Each is stated in the 0.18.0 "Known limits" of `docs/versioning.md` and in its proposal. `E-nn` ids are local to this
file and continue the earlier numbering. Severity is this file's suggestion; none blocks the downstream adoption.

| Id | Sev. | Crate | Limit at 0.18.0 | Possible fix |
| --- | --- | --- | --- | --- |
| E-34 | Low | `canwu-resource` | Demands, legs, transfers, leases, and report grants are still decoded whole in the live root; a pooled demand attaches every account (~30 µs per account, so canonical accounts stay 8,192); the per-holder custodian index is one unsharded entry (~250 KB rewritten per account creation at 10,000 accounts); the 0.16.0 deferral limits stand (deferred plugin work retried about once a day and blocking the seal, an activated lease never expires, a `Requested` acquisition holds its reservation, a skipped output batch leaves `CompletedPendingOutputSettlement`). | Split the remaining live state; shard the custodian index; scheduled retries and lease expiry. |
| E-35 | Low | `canwu-production` | A batch item still copies and validates its root (~3.5 ms at 1 MB), so roots must be sized by scope; each observation prune scans the whole rollover table twice (heavy only at the maxima); production reports are still kept for life per holder; an activated lease that never runs stays in its root; roots come from the scenario only. | Per-root write sets; a key-indexed rollover; report retention per site subject. |
| E-36 | Low | `canwu-sim` | A paged restore still verifies the whole evidence prefix, so the continuation checkpoint stays the world-scale save; knowledge maps are rebuilt from the ledger at every paged checkpoint; the seal copies sealed records (~2.6 µs per index entry); restoring a 662 MB unsealed snapshot still takes ~29 s (evidence validation ~14 s); the restore budget bounds assembled bytes, not heap (~16× for adversarial JSON). | Incremental paged knowledge maps; evidence validation by sealed segment; a heap-aware restore meter. |
| E-37 | Low | `canwu-sim` | Queued commands admitted inside a boundary still refresh the checkpoint hash once per admission (early and cheap; E-31 follow-up); a publisher that does not read the publication budget and overshoots fails the boundary (undeclared-write class, by design). | Fold in-boundary admission refreshes into the boundary's hashing phase. |
| E-38 | Low | `canwu-fiscal`, `canwu-sim` | At the world due instant the view's `current_domain_record_version` costs ~75 ms (45,065 calls), and the engine's share of the due instant and its heap peak (306 MB) are not yet profiled. | A cheaper version read on the view; profile the engine share. |
| E-39 | Low | `canwu-law` | A law boundary at ~1,555 jurisdictions still decodes the plan once (~8 ms); directory object routes grow with live legal objects; the provider index costs ~9 µs per kept packet per boundary and keeps only the newest 256 (citation window); a delegation with no `expires_at` stays for the run. | A decoded-plan cache; directory sharding. |
| E-40 | Low | `canwu-society` | The society root is still one record (10 MB at 4,000 cohorts, about half materialized views); under the default `Keep` the ledger grows; the intergroup relations record is rewritten whole on each revision boundary. | Shard the root; archive the ledger. |
| E-41 | Low | reference crates, `canwu-correspondence` | Past the byte cap, the force-supply plugin's acknowledgements of resource and externality outcomes can still fail a boundary (should defer as `canwu-resource` does); correspondence plans with plain `plan_route` (no edge conditions or compiled index) and a malformed `recurring` connection payload in a carrier's knowledge still fails the boundary as `InvalidDomainRecord`. | Deferral for force-supply acknowledgements; correspondence adopts `RoutingIndex` and refuses malformed connection knowledge as a recorded rejection. |

### 4.3 Limits the downstream met and worked around (not filed)

| Id | Sev. | Crate | Limit at 0.18.0 | Downstream handling |
| --- | --- | --- | --- | --- |
| E-42 | Low | `canwu-movement` | Capacity pools are never retired, so a runtime holds at most 256 pools over its life. | Pools are opened sparingly; a long campaign may reach the limit. |
| E-43 | Low | `canwu-movement` | The holder-relative `movement_report` serves person holders only; institution owners get no report from it. | The application publishes its own movement knowledge for institutional holders. |

Also met downstream, both already in §4.2: E-34's activated lease that never expires (the application releases its
own leases) and E-37's publication-budget overshoot (the application's publishers reserve within the budget).

### 4.4 Engine facts the downstream design loop asked to confirm (answered 2026-10-19 from the 0.18.0 source)

1. Transition timing: **wrong as stated.** A transition manifest commits in the boundary it is ready in, never one
   boundary later. Its coordinator chooses `ready_at`, not the engine: a manifest registered in phase 7 may be ready in
   the same boundary; one registered in phase 10 or 12 must name a later boundary, at most 1,024 ahead. At `ready_at`
   the participants stage in phase 10, the kernel audits in phase 11, and the writes commit in that same boundary ("a
   manifest never spans two boundaries", `canwu-sim` `runtime/transitions.rs`, module docs and `ready_at_is_valid`).
   Only writes with `NextBoundary` visibility become readable from the next boundary. A boundary is a sequence number,
   not a day.
2. Correspondence planned legs: **yes for a trusted host adapter, no for a restricted viewer.** The adapter reads
   `typed_domain_record(&run, &correspondence_operation_ref(key))` and decodes a `CorrespondenceOperationRecord`, whose
   `route_plan.legs` hold each leg's connection, nodes, mode, and planned departure and arrival, beside the
   execution's legs and itinerary revisions. That is ground truth (dispatch publishes nothing to the sender or the
   carrier), so it must not reach an actor as it is. `CanwuViewer` has no domain-record read. The actor-relative way is
   to plan again from the actor's own knowledge (`planning_snapshot_from_knowledge_result` over
   `planning_knowledge_query()`, then `plan_route`), which matches the engine's plan only while the actor's knowledge is
   what the carrier planned with.

### 4.5 Engine findings of the 0.19.0 source read (2026-10-19)

| Id | Sev. | Crate | Finding at 0.18.0 | Fix |
| --- | --- | --- | --- | --- |
| E-44 | Medium | `canwu-movement` | Incident operations check their evidence with `domain_record_version_evidence_exists` (`lifecycle.rs` incident check, `plugin.rs` `ViewEvidence`), whose answer differs between a sealed run and its exact replay for a superseded version, so a boundary decision depends on retained evidence. | 0.19.0: incident evidence must be a current or replay-stable version. |
| E-45 | Low | `canwu-resource` | A `Credit { ExternalInflow }` outcome keeps no evidence (`runtime.rs` `credit`), unlike production credits and outflow, against the immutable-outcome-evidence rule. | 0.19.0, with G-68. |
| E-46 | Low | `canwu-resource` | Conservation totals are global across resources and units; the design asks for totals per resource and unit. Authority, not the identity check, stops minting. | Left; per-resource-and-unit totals when a consumer needs them. |

## 5. Next steps

1. **Build 0.19.0 as decided in §6**: G-62 to G-68, E-44, and E-45, the Medium rows first (G-64, G-65, G-67, G-68).
   The downstream has finished the build wave that filed them and pins the next release as the first change set of
   its next wave (owner ruling, 2026-10-18). Send the §4.4 answers with the release notes.
2. The §4.2 limits and E-46 by downstream need; E-41's force-supply deferral first if a host runs the reference
   economy near its byte cap; §4.3 if the downstream asks.
3. Each change set follows the 0.18.0 pattern: one worktree and branch per package under `.worktrees/`, a proposal, durable tests that bite, an independent code review (two for persistence-heavy packages) and a delta review of the fixes, integration on a release branch, the pre-release tests of the release skill (the continuation gate included), then one release commit with the shared docs, the website (with its accuracy and copy reviews), and the skills.

## 6. Open decisions (owner)

Decided and carried out through 0.18.0: the society transfer knob, the 0.16.0 and 0.17.0 terms, auditor appointment,
G-15 High with the county-level world target, continuation checkpoint, world-scale fiscal, §4.2 findings staying
engine-side, the 0.16.0 contract choices, the continuation gate as a release test, and the decisions of the 0.17.0
run (confirmed 2026-10-16).

Taken in the 0.18.0 run by the orchestrator under the owner's instruction to fix every remaining gap and finding and
release (2026-10-16), and confirmed by the owner on 2026-10-17; each is recorded in its proposal:
- G-34 and G-17 were implemented although the ledger filed them as conditional ("second consumer", "profile first").
- G-33 is a society relation vocabulary (empty = unrestricted); intergroup-relation regions are identifiers, not record
  references, so no edge pins another plugin's entity.
- Delegations expire: an interpretation admitted at or after `expires_at` is refused even if dated inside the window.
- `max_provider_packets` default lowered from 4,096 to 256.
- Resource canonical limits were kept at what the measured costs support (accounts 8,192, allocation candidates and
  dirty demands 2,048; hard maxima raised); hot buckets hold 2 records; every host `Expire` is refused at enqueue.
- Production roots are sharded by host-assigned scope with a key prefix rule.
- Paged restores take a required `PagedRestoreBudget` parameter (default 2 GiB for a host's own checkpoints).
- 0.17.0 was yanked after 0.18.0 was verified on crates.io, following the one-live-version practice.

Decided on 2026-10-17 (owner), the Chinese terms of the 0.18.0 concepts, as on `main` in `docs/terminology.md` and
both website term pages: 请求结果保留期 request outcome retention, 已退役请求结果 retired request outcome, 社群结果留存策略
society outcome retention, cohort 类别 cohort class, 群体间关系 intergroup relation, 案件量配置 caseload profile, 汇总案件量
aggregate caseload, 结案率 clearance rate, 发布预留 publication reservation, 委托账 delegation ledger, 活动桶 live bucket,
保管人索引 custodian index, 消费批次 consumption batch, 周期性班次 recurring departure, 连接条件 edge condition, 路由索引
routing index, 恢复预算 restore budget, 组装字节数 assembled bytes.

Decided on 2026-10-19 (owner, all as the orchestrator recommended from a source read), the shape of 0.19.0. Names,
error codes, and the order of checks are the orchestrator's, recorded in each proposal for the owner to confirm after
the release:
- Scope: 0.19.0 closes G-62 to G-68, E-44, and E-45, and carries the §4.4 answers; §4.2, §4.3, and E-46 stay out.
  0.18.0 is yanked once 0.19.0 is verified on crates.io.
- G-64: a new movement operation `DelayLeg { execution, leg, until, reason }` replaces the arrival due of a departed
  current leg. `until` is an absolute instant, not before the boundary time nor the arrival scheduled at departure, so
  a hold can be lengthened, shortened, or lifted, never make the journey faster. It is requested by the owner or
  operator command or by an incident with evidence (a current version), each with a short reason; any number of
  delays, only the current due kept. Owner and operator reports show the hold; remote observers learn of it at
  arrival. A later leg whose booking expires meanwhile fails through the recorded leg failure. `Reroute` stays refused
  on a departed leg.
- G-65: `RecordLoss` may take part of a transfer's escrow (controller command or provider ingress); the transfer keeps
  its state and the rest of its escrow; `CompleteTransfer` stays terminal; the demand is credited only with what is
  accepted, so a loss reopens its remainder; allowed in every non-terminal state. No `SplitTransfer`.
- G-63: an `InTransit → InTransit` advance is admitted for the same execution with a new leg (not lower than the
  current one) or a new handoff. The transfer keeps an ordered link history, capped at 16, through completion and into
  the archive; past the cap a middle advance is a recorded rejection; arrival, return, and completion are always
  admitted.
- G-67: a host-only `RescopeAccount` with the expected account revision, the new place scope (or none), and an evidence
  version current at settlement, kept in the outcome. Refused (recorded) while the account is the source or
  destination of an unfinished transfer or a lease holds its current revision; reservations, demands, and grants carry
  over. A holder never sets a place scope.
- G-68: `Credit { ExternalInflow(evidence) }` is admitted as a custodian command under a host-issued single-step lease,
  the mirror of `ExternalOutflow`, and its outcome keeps the evidence (E-45). No engine cap; conservation totals stay
  global (E-46).
- G-62: `pending_ingress()` on the `CanwuRun` trait (both handles) iterates the queued `IngressRecord`s in queue order,
  unfiltered, for the trusted host only (never on `SimulationView` or `CanwuViewer`); no `ingress_record(id)`. It also
  covers a sealed run, where the gap appears as well.
- G-66: declined. The release documents the two ways around it: settle the second leg in a further boundary at the same
  instant, or give each leg its own handover record. No shared lock and no combined exchange acceptance.

Open: none.

## 7. Working rules

- Follow `AGENTS.md`: change-surface map, independent review for persistence, replay, determinism, and performance changes, tests only as durable evidence, and one coherent commit per change set when a commit is asked for.
- Test in proportion to the change; the integrating agent runs the full verification once for the integrated whole, and the pre-release tests of the release skill (the continuation gate included) once before tagging.
- Keep application vocabulary out of the core: proposals describe generic contracts, and downstream ids (`G-nn`) appear only as "downstream gap G-nn".
- A plugin boundary decision must not depend on retained evidence: sealing removes it and exact replay keeps it. Read record versions through `replay_stable_domain_record_version`, knowledge to supersede through `knowledge_heads`, and treat `knowledge_changes_by_correlation*` as evidence reads. Never declare initial-scenario record versions as payload-required (refused at load since 0.18.0).
- An expected refusal is a recorded rejection, a skipped packet with a trace, or a deferral, never a failed boundary: a failed boundary cannot be stepped past, and due ingress cannot be cancelled. Every new operation states which refusals can still fail a boundary. A knowledge publisher reads the publication budget before it proposes.
- Never iterate an `im` ordered map or set in reverse and never use `get_prev` / `lookup_prev` (im 15.1 drops elements past three tree levels); forward ranges with a lower bound, full forward iteration, and `get_min` / `get_max` are safe. Runtime state and journals are shared (`im`, `Arc`, `SharedJournal`): change them only through their copy-on-write methods.
- Same version only: no compatibility work with an earlier release's bytes or hashes.
- Workers in parallel worktrees use their own `target` dir and their own scratch file names (a shared `CARGO_TARGET_DIR` runs stale binaries; shared scratch names get overwritten).
- Releases, tags, crates.io publication, and yanks are the owner's.
- When a row ships, delete it here, record it in its proposal and `docs/versioning.md`, and the downstream ledger records the adoption.
