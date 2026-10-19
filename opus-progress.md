# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-10-19. The published release is Canwu **0.18.0** (tag `v0.18.0`, commit `ea85ca7` on `main`).
> **0.19.0 is merged to `main`** (release commit `89ac2a9`) and the website deploys from it; the tag, the crates.io
> publication, the GitHub release, and the yank of 0.18.0 are the owner's to finish (§5). This file is the to-do list of engine gaps that a downstream
> application has filed against the published crates. The application is Celestial Mandate (CM); its gap ledger
> (implementation document `59`, in the CM repository) gives each row its id (`G-nn`) and the full evidence.
> Priorities are the ledger's severities: High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.19.0 closes every row the downstream filed after adopting 0.18.0 (G-62 to G-68; G-66 by documentation, as the
> owner decided) and engine findings E-44 and E-45 (§3). Delete those rows when 0.19.0 is published. Still open: one
> recorded design question (G-38), the known limits the 0.18.0 and 0.19.0 packages left on purpose (§4.2), two limits
> the downstream met and worked around (§4.3), and E-46. The two engine facts the downstream design loop asked to
> confirm are answered (§4.4). The owner decided the shape of every 0.19.0 fix on 2026-10-19 and confirmed the
> choices the run made inside those decisions, and the Chinese terms, the same day (§6).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | 0.18.0 is published: all 24 library crates on crates.io, tag `v0.18.0`, GitHub release, website deployed (canwu.org shows 0.18.0 in both languages). 0.18.0 is the only live version: 0.17.0 was yanked on 2026-10-17 (0.14.0 to 0.16.0 earlier). Downstream pins `=0.18.0` since 2026-10-17 (its release 0.77.0) and has adopted G-44, G-47 to G-54, and G-56 to G-61 (G-46 is superseded by G-61); the other released rows wait for the downstream slices that consume them. 0.19.0 (workspace version `0.19.0`, every pin `=0.19.0`) is on `main` since 2026-10-19 (release commit `89ac2a9`, CI green on Windows, macOS, and Linux), and the website deploys from `main`; not yet tagged or on crates.io. The downstream pins 0.19.0 as the first change set of its next build wave. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.19.0` once released); nothing written by an earlier release is read; no migration. |
| Proposals of 0.19.0 (all "implemented in 0.19.0") | `docs/proposals/`: `movement-delay-leg.md` (G-64, E-44), `pending-ingress-read.md` (G-62), `resource-transit-place-and-inflow.md` (G-63, G-65, G-67, G-68, E-45; G-66 documented). The 0.18.0 proposals are listed in the 0.18.0 entry of `docs/versioning.md`. |
| Branches and worktrees | `main` (0.19.0: `034e558` movement, `8e07403` pending ingress, `2d68fb0` resource, `89ac2a9` release). The merged session branch `claude/sweet-bell-yqcrl9` is still on the remote: the session's git proxy refused branch deletion and tag pushes, so the owner deletes it (§5). No worktrees. |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`. The full list and the pre-release tests ran once on the 0.19.0 release tree (2026-10-19, Linux, without debug info): fmt, clippy, workspace tests (841 passed, 0 failed, 3 ignored), `canwu-debug` check, rustdoc with `-Dwarnings`, the four examples, `+1.88.0` check, the performance harness (fmt, clippy, tests 4 and 6 with allocation counting), the continuation gate (clippy, and tests 841 passed, 0 failed, the expected panic included), and the release-only `--ignored` tests: the 65,537-action fiscal boundary and the persisted law boundary probe passed; the million-candidate law probe (`production_runtime_archive_prepare_scales_to_one_million_candidates`) could not run on this 15 GB machine, killed by the kernel at about 13.3 GB resident, and 0.18.0 is killed the same way at about 13.6 GB, so it is the machine, not the release. **Run that probe on the release machine before tagging** (`cargo test --release --locked -p canwu-law --test law_contract -- --ignored`). Dependencies did not change, so the third-party notices were not regenerated. Every package had an independent code review (two for resource) and delta reviews until no blocking finding remained; the website copy had an independent readability review. |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.19.0`; nothing earlier loads. (2) Read the "Upgrading from 0.18.0" notes of the 0.19.0 entry of `docs/versioning.md` and its "Known limits": `DelayLeg`; incident evidence and an order's delivery attempt must be the current version (resubmit an order whose attempt record changed); a scenario-installed departed leg needs the arrival due at departure plus planned duration; `pending_ingress()` after a seal or load (trusted host only; keep your own items); resource state format 5; `RecordTransferLoss`; the link history; `enqueue_resource_account_rescope`; a tracked inflow needs a lease the host bound with `CompletionOperationBindingV1`; a tracked command whose lease belongs to another plugin is refused at admission; demands are credited with what is accepted, allocation passes count escrow in flight, and `AmendDemand` cannot go below fulfilled + reserved + in flight; for G-66, settle the second leg one boundary later at the same instant or give each leg its own handover record. (3) The 0.18.0 notes still apply. |

## 2. High

No row filed by the downstream ledger is open.

## 3. Closed by 0.19.0 (prepared, not yet published)

Each row was filed 2026-10-17 or 2026-10-18 and verified in the published 0.18.0 source; the owner decided each fix
on 2026-10-19 (§6). Delete this section when 0.19.0 is published; the downstream ledger records each adoption.

| Id | Sev. | Crate | Gap at 0.18.0 | 0.19.0 |
| --- | --- | --- | --- | --- |
| G-64 | Medium | `canwu-movement` | No operation delays the arrival of a departed leg. | `DelayLeg { execution, leg, until, reason }` holds the current departed leg; `until` is absolute and never before the boundary or the arrival scheduled at departure. |
| G-65 | Medium | `canwu-resource` | A transfer cannot lose part of its escrow. | `RecordTransferLoss` takes part of the escrow and keeps the transfer open; the demand is credited with what is accepted. |
| G-67 | Medium | `canwu-resource` | An account's `place_scope` cannot change. | Host-only `RescopeAccount` (`enqueue_resource_account_rescope`) with current evidence. |
| G-68 | Medium | `canwu-resource` | Value from outside the modelled accounts cannot be credited. | Tracked `Credit { ExternalInflow(evidence) }` under a lease the host bound to exactly that credit (`CompletionOperationBindingV1`). |
| G-62 | Low | `canwu-api`, `canwu-sim` | After a seal or a continuation restore a host cannot read its pending ingress. | `pending_ingress()` on `CanwuRun`, `Canwu`, and `CompactedCanwu`. |
| G-63 | Low | `canwu-resource` | A multi-leg transfer cannot record its intermediate legs. | `InTransit → InTransit` advances and an ordered link history (16 entries) kept into the archive. |
| G-66 | Low | `canwu-sim`, `canwu-resource` | Two completion leases cannot lock one external record at once. | Declined by the owner; documented: settle the second leg one boundary later at the same instant, or give each leg its own handover record. The "a day later" came from settling one boundary per day. |
| E-44 | Medium | `canwu-movement` | Incident evidence was read through a retained-evidence check. | Incident evidence and an order's delivery attempt must be the current version. |
| E-45 | Low | `canwu-resource` | An `ExternalInflow` outcome kept no evidence. | The outcome keeps its evidence. |

## 4. Medium and Low

No row filed by the downstream ledger is open beyond §3.

### 4.1 Recorded, not proposed

G-38 (the `canwu-military` shape contradicts the downstream design; relevant only if downstream ever reconsiders adopting `canwu-military`; the ledger files no fix).

### 4.2 Known limits left by the 0.18.0 and 0.19.0 packages (engine-side; not numbered by the downstream ledger)

Each is stated in the "Known limits" of its release in `docs/versioning.md` and in its proposal. `E-nn` ids are local to this
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
| E-46 | Low | `canwu-resource` | Conservation totals are global across resources and units; the design asks for totals per resource and unit. Authority, not the identity check, stops minting. | Per-resource-and-unit totals when a consumer needs them. |
| E-47 | Low | `canwu-resource` (0.19.0) | The link history keeps 14 in-transit links (later intermediate links of a longer execution stay in the movement root only); an open transfer costs about 5.2 KB of live root at 16 links (about 43 MB at 8,192 transfers); a demand with more than `(max_archive_candidates − 1) / 2` legs (127 at the canonical 256) can never close; an operation key can be taken before any lease carries it, so issuers must not use guessable keys for provider operations; an unbound lease other than a tracked inflow authorizes whatever its targets allow; partial losses are capped only by the escrow; a rescope checks its evidence's currency and kind, not what it says. | A per-demand leg cap; key reservation by the issuer; a compact link encoding. |
| E-48 | Low | `canwu-movement` (0.19.0) | Each `DelayLeg` leaves the due packet it replaces queued until that packet's time (pinned through seals and continuation checkpoints, listed by `pending_ingress()`, waking an otherwise empty boundary); they accumulate on a long run with many holds. | Cancel the superseded due when a hold replaces it. |

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

## 5. Next steps

1. **Owner: finish publishing 0.19.0** from a machine with crates.io and tag-push access, on the release commit
   `89ac2a9`: run the million-candidate law probe (§1), push the annotated tag `v0.19.0`, publish the 24 crates in the
   order of `docs/releasing.md`, run a registry-only consumer smoke test, publish the GitHub release (send the §4.4
   answers with its notes), yank 0.18.0, and delete the remote branch `claude/sweet-bell-yqcrl9`. Then delete §3 and
   record the release here.
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

Taken in the 0.19.0 run by the orchestrator inside the decisions above, each recorded in its proposal, and confirmed
by the owner on 2026-10-19:
- G-65 is a sibling operation `RecordTransferLoss`, not a transfer target of `RecordLoss`, so the account-loss path is
  unchanged; it has the same controller-command and provider-ingress paths.
- G-68's "host-issued lease" is enforced by a **lease operation binding** (`CompletionOperationBindingV1`: operation
  kind, quantity, optionally the transfer disposition), carried in the certificate digest. A tracked inflow requires a
  lease bound to exactly that credit; a holder cannot write a binding; a tracked command is refused at admission when
  its lease's grants or expected participants name another plugin. The code review showed the plain lease let a
  custodian mint under a lease issued for another operation.
- E-44 extends to an order's `delivery_attempt`, which must also be the current version: an attempt record revised
  before the order settles refuses the order, and the host resubmits.
- Behaviour changes found necessary in review, beyond the rows: a demand is credited with what is accepted;
  allocation passes count escrow in flight (0.18.0 could allocate it twice); `AmendDemand` cannot go below fulfilled +
  reserved + in flight; an ended demand's closure waits for the transfers that draw on it; archived loss identities are
  refused; a closure without room is a skip; a departed leg's arrival due must match its scheduled arrival or hold, so
  a scenario that installs one otherwise is refused at load.
- The Chinese terms of the 0.19.0 concepts, as in `docs/terminology.md` and both website term pages: 路段滞留 leg hold,
  排定到达时刻 scheduled arrival, 待处理输入 pending ingress (the sheet's existing rendering, used instead of the
  proposal's 待准入输入), 转移部分损耗 partial transfer loss, 运输关联历史 link history, 中间路段 intermediate leg,
  账户改换地点 account rescope, 外部流入入账 external inflow credit, 租约操作绑定 lease operation binding; and in website
  prose 资格信封 for `EligibilityEnvelopeV1`.

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
