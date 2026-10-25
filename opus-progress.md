# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-10-25 at Canwu **0.20.0** (tag `v0.20.0`, release commit `a82ffeb` on `main`, published 2026-10-24),
> with the rows the downstream filed when it adopted 0.20.0 (CM 0.89.0).
> This file is the to-do list of engine gaps that a downstream application has filed against the published crates.
> The application is Celestial Mandate (CM); its gap ledger (implementation document `59`, in the CM repository) gives
> each row its id (`G-nn`) and the full evidence. Priorities are the ledger's severities: High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.20.0 closed every row the downstream filed after adopting 0.19.0 (G-69 to G-82), the recorded design question
> G-38 (`canwu-military` rewritten as the engagement engine), engine finding E-48, and the correspondence half of
> E-41. Open: eight Low downstream rows filed at the downstream's 0.20.0 adoption (G-83 to G-90, §4); the known limits the 0.18.0 to 0.20.0 packages left on purpose (§4.2, with the
> three new findings E-49 and E-50, pre-existing and found by the 0.20.0 packages, and E-51, the military limits),
> two limits the downstream met and worked around (§4.3), and the decisions this run took for the owner to confirm
> (§6).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | 0.20.0 is published (2026-10-24): all 24 library crates on crates.io (one workspace publish from the tagged commit; the index lists each), the annotated tag `v0.20.0` on the release commit `a82ffeb`, the GitHub release, and the website deployed from `main` (canwu.org shows `=0.20.0` in both languages). 0.20.0 is the only live version: 0.19.0 was yanked on 2026-10-24. A registry-only consumer of `canwu-api`, `canwu-military`, and `canwu-military-reference-content` at `=0.20.0` builds and runs. Verification on the release tree (owner, 2026-10-24: no extensive testing): fmt, Clippy with warnings denied, the workspace tests (856 passed, 0 failed, 3 ignored), the debug client, the documentation build with warnings denied, the four public examples, the performance harness checked (not run), the website build, and CI on Windows, macOS, and Linux; not run: the workspace-wide continuation gate (each persistence package ran it on its own crates in review), the release-only `#[ignore]` scale tests, and the minimum-Rust-version check. Downstream pins `=0.19.0` (its locked build keeps resolving the yanked version) and moves to `=0.20.0` as its next change set. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.20.0` once released); nothing written by an earlier release is read; no migration. |
| Proposals of 0.20.0 (all "implemented in 0.20.0") | `docs/proposals/`: `correspondence-domain-parties.md` (G-72, G-78, E-41's correspondence half), `movement-holds-and-revocation.md` (G-73, G-79, G-80, E-48), `decision-deadlines-reassignment-and-reads.md` (G-70, G-81, G-82), `military-engagements.md` (G-38; supersedes `military-domain-design.md`), `resource-continuation-and-completion-reports.md` (G-74 to G-77), `calendar-marks-and-draw-retirement.md` (G-69, G-71). The 0.19.0 and 0.18.0 proposals are listed in their entries of `docs/versioning.md`. |
| Branches and worktrees | Only `main`, locally and on the remote; no 0.20.0 worktrees remain. |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`. The orchestrator runs the full verification list once on the integrated release tree and the pre-release tests (the public examples, the performance-harness checks, the release-only `#[ignore]` scale tests in a release build, the continuation gate) before tagging (the counts are in the release row). |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.20.0`; nothing earlier loads (commitment format 7 and continuation checkpoint format 4 refuse 0.19.0 saves by their format as well as their engine version). (2) Read the "Upgrading from 0.19.0" notes of the 0.20.0 entry of `docs/versioning.md` and its "Known limits": `DelayLeg` takes `retime_later_legs` and holds a leg before departure; `MovementLegHoldV1::scheduled_at` with `kind`; `RevokeBooking` and `MovementBookingRequestV1::replaces`; superseded leg dues leave `pending_ingress()`; `ExtendDeadline`, `Reassign`, `with_option_bias`, `decision_tickets_opened`, `open_decision_tickets_of`, `open_decision_tickets_of_controller`; `KnowledgeSeedReceipt::refusal` and `settled_at`, domain-entity correspondence parties, a dead person cannot carry; `canwu-military` is rewritten (rulebook, `MilitaryPlugin<R>`, `RegisterForce`, `PlaceForce`, `OpenEngagement`, `SupplyInput`, `Step`, `engagement_summary`); resource state format 6 and bucket formats 2, a continuation checkpoint taken mid-lease loads, report grants need holders that can hold knowledge, a reserved report without an observation head is given up; production reads asset bindings and application specs; technology references domain entities as records; calendar marks (`schedule_calendar_marks`, `BoundarySystemContract::calendar`) and keyed-draw retirement (`KeyedDrawRetirement`). (3) The 0.19.0 and 0.18.0 notes still apply. |

## 2. High

No row filed by the downstream ledger is open.

## 3. Medium

No row filed by the downstream ledger is open.

## 4. Low

Filed 2026-10-24 by the downstream's W3 wave end (CM 0.89.0, the 0.20.0 adoption): G-83 by its movement
package, G-84 from its G-70 adoption, G-85 to G-90 from its `canwu-military` adoption; each verified in the published
0.20.0 source. None blocks the downstream; each row names what it built or left meanwhile.

| Id | Sev. | Crate | Gap at 0.20.0 | Proposed fix | Downstream effect |
| --- | --- | --- | --- | --- | --- |
| G-83 | Low | `canwu-movement` | (filed 2026-10-24 by `45` W3 8C; verified in the published 0.20.0 source: `lifecycle.rs` `request_booking` refuses a booking once `execution.bookings.len() >= MovementLimitsV1::CURRENT.bookings_per_execution` (16), and `TransportExecution::request_booking` only appends — a released, expired, failed, revoked, or replaced booking stays in the list) An execution can book at most 16 lots over its whole life, closed and replaced ones counted, so a leg of more than 16 lots is short of hulls at once (the 徽商's whole 南京 stock, 20 lots on its 瓜洲 leg) and a leg held for its lots can re-book in place (G-79 `replaces`) only while its lost lots and their replacements stay within 16: the 3,000 石 convoy (8 lots) re-books once, the whole 瓜洲水次仓 haul (15 lots) never | **Limit:** count only the live bookings (`Requested`, `Confirmed`) against the bound, or move a closed or replaced booking out of the execution (its evidence kept), or size the bound for a hull-lot leg's re-bookings (a lot per hull, `45` §4.3) | **built around nothing:** a leg whose lots cannot be booked again is held for them to `maxWaitDays` and then fails `no_capacity` for its replan (`45` §12 "Canwu 0.20 movement", `Deviation (Canwu 0.20, G-83)`) |
| G-84 | Low | `canwu-decision` (utility stage) | (filed 2026-10-24 at W3's wave end from W3 7A's hand-back, the G-70 adoption; verified in the published 0.20.0 source: `policy.rs` `WeightedUtilityEvaluator::with_option_bias` keeps `option_biases: BTreeMap<String, i64>` on the evaluator by option id, so a bias is part of the policy and of its controller's identity, and `model.rs` `DecisionOption` carries `utility_inputs` and no bias) A bias that differs from ticket to ticket cannot ride the actor's one controller: `41` §5.2's `Mediate` acceptance bias (`clamp(score − 60, −20, +20)`, one value per party and ticket) needs one doctrine controller per bias value (`cm-diplomacy:<actor>:<class>:accept_p<n>` / `accept_m<n>`, up to 41 per actor and class), and `48`'s per-ticket `aiWeightHints` would need the same | **Option field:** an optional `bias: i64` on `DecisionOption`, written by the ticket's drafter, added to the evaluator's own bias for that option and recorded on `DecisionOptionEvaluation::bias` beside the factor lines, so per-ticket biases share the actor's doctrine controller | **built around nothing new:** W3 7A's controller per bias value (`41` §12 "Canwu 0.20.0 adoptions"; §5.9 G-70) |
| G-85 | Low | `canwu-military` | (filed 2026-10-24 at W3's wave end from W3 8A's hand-back; verified in the published 0.20.0 source by W3 8A: `settle.rs` `queue_report` pushes a report item onto `CombatRegistryV1::report_queue` for every stepped engagement, siege, and crossing, and `report.rs` `publish_engagement_reports` (phase 13) drains the queue and rewrites the registry, whatever the subject's holders) The combat registry is written twice on every stepping boundary, the report queue filled in phase 7 and drained in phase 13, even when no report names a holder (CM names none, `44` §12's 8A note: about 0.24 KB a write, two a siege day) | **Queue rule:** queue a report item only for a subject whose side lists a holder, or keep the queue outside the registry record | **built around nothing:** CM declares no report holders and pays the two writes (`44` §12's 8A instrument column) |
| G-86 | Low | `canwu-military` | (filed 2026-10-24 at W3's wave end from 8A; verified in the published 0.20.0 source by W3 8A: `rulebook.rs` `CombatRulebookV1::kinds` holds one rule per `EngagementKind` with one `max_minutes`, and `settle.rs` closes an engagement once `minutes_between(opened_at, at) >= max_minutes`) `Assault` has one duration, so it cannot be both the multi-day wall assault and the one-day gate assault the mapping names it (`docs/reviews/canwu-adoption/2026-10-24-canwu-0.20-military-mapping.md`) | **Field:** an optional `max_minutes` on `OpenEngagementV1`, bounded by the kind's, or a second assault kind | `Adopted (design)` in `44` §12's 8A note: the gate assault runs as the engine's `Field` kind with the wall closed (gate terrain and frontage, the 2500 cell, one day) |
| G-87 | Low | `canwu-military` | (filed 2026-10-24 at W3's wave end from 8A; verified in the published 0.20.0 source by W3 8A: `command.rs` `invest_site` requires every besieger and defender `is_free()` at the site, and a besieged garrison is not free; `ReviseSiegeV1` revises supply, profile, work, the fleet, a gate, the storm, and a sortie, never the besiegers; `JoinEngagement` has no siege counterpart) A siege cannot take a second column: a second investment of an invested place is refused, and no command joins a force to an open siege | **Command:** `JoinSiege { siege, expected_host_revision, side, forces }`, as `JoinEngagement` | `Deviation (Canwu 0.20, G-87)` in `44` §12's 8A note: a second column's `BesiegePlace` at an invested place is refused 城已被围 (4A merged the columns) |
| G-88 | Low | `canwu-military` | (filed 2026-10-24 at W3's wave end from 8A; verified in the published 0.20.0 source by W3 8A: `command.rs` `ReviseCrossing { crossing, expected_host_revision, interceptor, night }`) A crossing's defending bank is fixed when `OrderCrossing` is read: `ReviseCrossing` revises the interceptor and the night, never the defenders, so a force that reaches the far bank after the order does not resist the landing | **Field:** `defenders: Option<Vec<ForceId>>` on `ReviseCrossing`, checked as `OrderCrossing` checks them | `Deviation (Canwu 0.20, G-88)` in `44` §12's 8A note: the bank defenders are those at the far bank when the order is read |
| G-89 | Low | `canwu-military` | (filed 2026-10-24 at W3's wave end from 8A; verified in the published 0.20.0 source by W3 8A: `command.rs` `PlaceForce` refuses a force that is not `is_free()` — "a force engaged, in a siege, or crossing cannot be placed") A besieged defender cannot leave its siege: CM cannot place a garrison force that marches off, so it still stands in the siege, while a besieger that leaves lifts it (`LiftSiege` when none stays) | **Command or field:** a defender's release from a siege (`ReviseSiege { release: forces }`, or `PlaceForce` admitting a defender, which leaves the siege's defenders) | `Deviation (Canwu 0.20, G-89)` in `44` §12's 8A note: a besieged defender that marches off still stands in the siege |
| G-90 | Low | `canwu-military` | (filed 2026-10-24 at W3's wave end from 8A; verified in the published 0.20.0 source by W3 8A: `command.rs` `Step { up_to }` is a trusted host command — "steps the work due by `up_to` in the next boundary at once, after the apply packets admitted with it" — and `plugin.rs` `with_step_cadence(Some(cadence))` instead schedules a step packet due at once in each boundary of that cadence with work due) Stepping costs boundaries the `Daily` boundary does not carry: on a day with no other day-open batch the `Step` takes the batch's command boundary and the cut that applies it, so a siege, fight, or crossing day settles 3–4 boundaries where a quiet day settles 1 (`44` §12's 8A instrument column) | **Ingress:** a step the host queues as ingress due at a named time (the next 00:00), admitted and stepped by that `Daily` boundary after the day's apply packets | **built around nothing:** the day's orders and the `Step` ride the day-open batch (on a day with another batch they share its boundaries; `63` §5.2 rule 3's `canwu-military` row) |

### 4.1 Recorded, not proposed

None. G-38 (the `canwu-military` shape contradicted the downstream design) is closed by 0.20.0's engagement engine
(`docs/proposals/military-engagements.md`); its criterion of a second independent host adopting the crate is open
(§4.2, part of the military limits).

### 4.2 Known limits left by the 0.18.0, 0.19.0, and 0.20.0 packages (engine-side; not numbered by the downstream ledger)

Each is stated in the "Known limits" of its release in `docs/versioning.md` and in its proposal. `E-nn` ids are local to this
file and continue the earlier numbering. Severity is this file's suggestion; none blocks the downstream adoption.

| Id | Sev. | Crate | Limit | Possible fix |
| --- | --- | --- | --- | --- |
| E-34 | Low | `canwu-resource` | Demands, legs, transfers, leases, and report grants are still decoded whole in the live root; a pooled demand attaches every account (~30 µs per account, so canonical accounts stay 8,192); the per-holder custodian index is one unsharded entry (~250 KB rewritten per account creation at 10,000 accounts); the 0.16.0 deferral limits stand (deferred plugin work retried about once a day and blocking the seal, an activated lease never expires, a `Requested` acquisition holds its reservation, a skipped output batch leaves `CompletedPendingOutputSettlement`). | Split the remaining live state; shard the custodian index; scheduled retries and lease expiry. |
| E-35 | Low | `canwu-production` | A batch item still copies and validates its root (~3.5 ms at 1 MB), so roots must be sized by scope; each observation prune scans the whole rollover table twice (heavy only at the maxima); production reports are still kept for life per holder; an activated lease that never runs stays in its root; roots come from the scenario only. | Per-root write sets; a key-indexed rollover; report retention per site subject. |
| E-36 | Low | `canwu-sim` | A paged restore still verifies the whole evidence prefix, so the continuation checkpoint stays the world-scale save; knowledge maps are rebuilt from the ledger at every paged checkpoint; the seal copies sealed records (~2.6 µs per index entry); restoring a 662 MB unsealed snapshot still takes ~29 s (evidence validation ~14 s); the restore budget bounds assembled bytes, not heap (~16× for adversarial JSON). | Incremental paged knowledge maps; evidence validation by sealed segment; a heap-aware restore meter. |
| E-37 | Low | `canwu-sim` | Queued commands admitted inside a boundary still refresh the checkpoint hash once per admission (early and cheap; E-31 follow-up); a publisher that does not read the publication budget and overshoots fails the boundary (undeclared-write class, by design). | Fold in-boundary admission refreshes into the boundary's hashing phase. |
| E-38 | Low | `canwu-fiscal`, `canwu-sim` | At the world due instant the view's `current_domain_record_version` costs ~75 ms (45,065 calls), and the engine's share of the due instant and its heap peak (306 MB) are not yet profiled. | A cheaper version read on the view; profile the engine share. |
| E-39 | Low | `canwu-law` | A law boundary at ~1,555 jurisdictions still decodes the plan once (~8 ms); directory object routes grow with live legal objects; the provider index costs ~9 µs per kept packet per boundary and keeps only the newest 256 (citation window); a delegation with no `expires_at` stays for the run. | A decoded-plan cache; directory sharding. |
| E-40 | Low | `canwu-society` | The society root is still one record (10 MB at 4,000 cohorts, about half materialized views); under the default `Keep` the ledger grows; the intergroup relations record is rewritten whole on each revision boundary. | Shard the root; archive the ledger. |
| E-41 | Low | reference crates (`canwu-force-supply-reference`) | Past the byte cap, the force-supply plugin's acknowledgements of resource and externality outcomes can still fail a boundary (should defer as `canwu-resource` does). The correspondence half (a malformed `recurring` connection payload in a carrier's knowledge failed the boundary) is closed by 0.20.0: a seed that states such a connection is refused with `KnowledgeSeedRefusal::InvalidConnection`; a malformed fact installed by the initial scenario or another publisher still fails the carrier's planning boundary, and correspondence still plans with plain `plan_route` (no `RoutingIndex` kept across planning reads). | Deferral for force-supply acknowledgements; a planning-snapshot cache keyed by the carrier's read-set digest if the index pays. |
| E-46 | Low | `canwu-resource` | Conservation totals are global across resources and units; the design asks for totals per resource and unit. Authority, not the identity check, stops minting. | Per-resource-and-unit totals when a consumer needs them. |
| E-47 | Low | `canwu-resource` (0.19.0) | The link history keeps 14 in-transit links (later intermediate links of a longer execution stay in the movement root only); an open transfer costs about 5.2 KB of live root at 16 links (about 43 MB at 8,192 transfers); a demand with more than `(max_archive_candidates − 1) / 2` legs (127 at the canonical 256) can never close; an operation key can be taken before any lease carries it, so issuers must not use guessable keys for provider operations; an unbound lease other than a tracked inflow authorizes whatever its targets allow; partial losses are capped only by the escrow; a rescope checks its evidence's currency and kind, not what it says. | A per-demand leg cap; key reservation by the issuer; a compact link encoding. |
| E-49 | Low | `canwu-correspondence` (pre-existing, found by the 0.20.0 package) | Many command-handler refusals are raised as `InvalidDomainRecord`, which the engine does not count as an expected command rejection (`is_expected_command_rejection`), so they fail the command's boundary instead of being recorded: on `initiate_correspondence_v1` a malformed key or profile, a zero execution or dispatch version, a deadline before admission, a prepared dispatch that is missing, stale, not prepared, not single-recipient, or of another channel profile, or a sender that does not match it; on `resolve_correspondence_v1` a missing operation, an overlapping information operation, a recovery the operation's state does not allow, or a non-terminal attempt or inactive dispatch. Authority refusals (the 0.20.0 carrier check included) are `InvalidAuthority` and recorded. A host that validates these before it sends never meets them. | Recode each as an expected command rejection (`InvalidAuthority`, or a new expected code) so a malformed command is recorded on its attempt. |
| E-50 | Medium | `canwu-resource` (pre-existing, found by the 0.20.0 package) | An external completion participant that ends `Released`, `Expired`, or `Rejected` never leaves `ExternalCompletionParticipantBookV1::grants`: each coordinator (production, the force-supply reference) acknowledges a participant's end by citing the resource root's current version and reading the participant from that version's live `grants`, and the resource plugin learns of no acknowledgement, so retiring one would break an acknowledgement still to come. After 1,024 have accumulated (`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL`), `grant` refuses every new participant for the run as a recorded `limit_exceeded`. Ended participants declare no evidence meanwhile. A long campaign with cross-plugin completions reaches the cap. | A retirement the coordinator triggers once it has acknowledged (a change to the production and force-supply coordinator protocols), or an acknowledgement the resource plugin can observe. |
| E-51 | Low | `canwu-military` (0.20.0) | No failure-mode model; two sides per engagement; no automatic contact; vessels are components; one wave size per crossing; reports are bands; retired records stay in the store and are validated at every load (about 2 KB per closed fight); an outcome signal whose target ingress no plugin registered fails the first closing boundary; no second independent host has adopted the crate yet (G-38's criterion (f)). | A per-wave size on `ReviseCrossing`; fidelity classes in reports; a second host adoption. |

### 4.3 Limits the downstream met and worked around (not filed)

| Id | Sev. | Crate | Limit | Downstream handling |
| --- | --- | --- | --- | --- |
| E-42 | Low | `canwu-movement` | Capacity pools are never retired, so a runtime holds at most 256 pools over its life. | Pools are opened sparingly; a long campaign may reach the limit. |
| E-43 | Low | `canwu-movement` | The holder-relative `movement_report` serves person holders only; institution owners get no report from it, and pool custodians learn booking identities from the host (they revoke by reading the runtime). | The application publishes its own movement knowledge for institutional holders. |

Also met downstream, both already in §4.2: E-34's activated lease that never expires (the application releases its
own leases) and E-37's publication-budget overshoot (the application's publishers reserve within the budget).

### 4.4 Engine facts the downstream design loop asked to confirm (answered 2026-10-19 from the 0.18.0 source; unchanged by 0.20.0)

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

1. **Wait for the downstream's adoption of 0.20.0** (its ledger records each row's adoption at the `=0.20.0` pin) and
   for the rows it files against 0.20.0. Nothing is filed at the time of writing.
2. Then the §4.2 limits by downstream need: **E-50** first if the downstream runs cross-plugin completions through a
   long campaign (the cap of 1,024 ended participants); **E-49** if a host wants malformed correspondence commands
   recorded rather than validated before sending; E-41's force-supply deferral if a host runs the reference economy
   near its byte cap; E-46 to E-47 and the military limits (E-51) as the downstream's military slice asks; §4.3 if the
   downstream asks.
3. Each change set follows the 0.18.0 pattern: one worktree and branch per package under `.worktrees/`, a proposal,
   durable tests that bite, an independent code review (two for persistence-heavy packages) and a delta review of the
   fixes, integration on a release branch, the pre-release tests of the release skill (the continuation gate
   included), then one release commit with the shared docs, the website (with its accuracy and copy reviews), and the
   skills.

## 6. Open decisions (owner)

Decisions this run took under the owner's instruction of 2026-10-24 (address every open downstream gap with the fix
its row recommends, following Canwu's own guidance; abstract the downstream's combat into `canwu-military` properly;
release only when all are fixed), each recorded where it lives and listed here for the owner to confirm:

1. **G-69's shape.** Taken as the row recommends: host-defined calendar marks `(cadence, calendar key)` on a calendar
   boundary (`schedule_calendar_marks`), readable on the view (`SimulationView::calendar_marks`) and carried by the
   boundary record and ledger, with an optional calendar key on a system's registration
   (`BoundarySystemContract::calendar`) so the system wakes only on its own calendar's boundaries. The core keeps no
   calendar. Rejected: a calendar inside `SystemCadence`; a calendar table in the run configuration
   (`docs/proposals/calendar-marks-and-draw-retirement.md`).
2. **The continuation checkpoint format moves from 3 to 4** for the keyed-draw retirement's restore rules (a
   checkpoint lists only the draws not retired at its time, and a restore refuses one that lists a retired draw); the
   proposal had kept the number since no field changes. One bump for the release, so a 0.19.0 continuation checkpoint
   is refused by its format as well as by its engine version.
3. **The military open questions** (`docs/proposals/military-engagements.md`, "Open questions"), shipped as designed:
   (a) the plugin owns a thin `force` record with the composition rules of §9 (sides by reference with host-applied
   deltas is the recorded rejected shape); (b) the plugin's default step cadence is the `Daily` wake cadence (`MilitaryPlugin::new()` sets
   `step_cadence` to `Some(SystemCadence::Daily)`; `with_step_cadence(None)` hands stepping to the trusted
   `Step { up_to }` command), and the reference starter keeps the default, which the tutorial teaches; (c) `Step` stays trusted-only (an issuer in the rulebook's `trusted_issuers`), not accepted from
   a side's controller; (d) `canwu-force-supply-reference` is kept and the two synthetic military profiles are deleted.
4. **The Chinese terms** of the release, in `docs/terminology.md` and both website term pages: 展期 (deadline
   extension), 改派 (ticket reassignment), 选项偏置 (option bias), 准入批次 (admission batch); 路段滞留 widened (leg
   hold), 排定时刻 (scheduled due, replacing 排定到达时刻 / scheduled arrival, now deprecated), 行程顺延量 (schedule
   shift), 预订撤销 (booking revocation), 预订替换 (booking replacement); 知识种子拒收 (seed refusal); 历法标记 (calendar
   mark), 历法键 (calendar key), 定址抽样退役期 (keyed-draw retirement), 已退役定址抽样 (retired keyed draw); the
   military terms 交战, 战斗步, 战斗规则册, 战斗步结算器, 围城, 渡河作战, 战斗条件输入, 战前预判 (accepted by the host-side
   review).
5. **Deviations from the rows, each explained in its proposal:** correspondence party roles admit a retired entity
   (`allow_retired: true`) so a correspondence record never blocks a party's retirement, and a dead person can no
   longer carry; `RevokeBooking` returns a booking as the new status `Revoked` (not `Failed` / `Released`) with
   `replaces` for the owner's re-request; the departure hold is the same `DelayLeg` operation with
   `retime_later_legs`, not a separate `HoldDeparture`, and the re-timing is applied when the hold ends by the time
   that elapsed; the option bias sits on `WeightedUtilityEvaluator` rather than `UtilityProfile` or `DecisionOption`;
   `Reassign` never changes the decision maker; G-75 takes the "archive the participant without a report" shape, not
   a completion-published report; G-77 declares identity-only evidence (no bodies kept); production refuses a wrong
   record kind before reading it rather than widening the read set.

Every decision through 0.19.0 is decided and confirmed by the owner and recorded in its proposals, its
`docs/versioning.md` entry, and the term sheets.

## 7. Working rules

- Follow `AGENTS.md`: change-surface map, independent review for persistence, replay, determinism, and performance changes, tests only as durable evidence, and one coherent commit per change set when a commit is asked for.
- Test in proportion to the change; the integrating agent runs the full verification once for the integrated whole, and the pre-release tests of the release skill (the continuation gate included) once before tagging.
- Keep application vocabulary out of the core: proposals describe generic contracts, and downstream ids (`G-nn`) appear only as "downstream gap G-nn".
- A plugin boundary decision must not depend on retained evidence: sealing removes it and exact replay keeps it. Read record versions through `replay_stable_domain_record_version`, knowledge to supersede through `knowledge_heads`, and treat `knowledge_changes_by_correlation*` as evidence reads. Never declare initial-scenario record versions as payload-required (refused at load since 0.18.0); declare the versions a hot record cites identity-only where no rule reads their bodies (0.20.0).
- An expected refusal is a recorded rejection, a skipped packet with a trace, or a deferral, never a failed boundary: a failed boundary cannot be stepped past, and due ingress cannot be cancelled. Every new operation states which refusals can still fail a boundary. A knowledge publisher reads the publication budget before it proposes, and checks its holder with `knowledge_holder_accepts_publication` before it reserves or publishes.
- Never iterate an `im` ordered map or set in reverse and never use `get_prev` / `lookup_prev` (im 15.1 drops elements past three tree levels); forward ranges with a lower bound, full forward iteration, and `get_min` / `get_max` are safe. Runtime state and journals are shared (`im`, `Arc`, `SharedJournal`): change them only through their copy-on-write methods.
- Same version only: no compatibility work with an earlier release's bytes or hashes.
- Workers in parallel worktrees use their own `target` dir and their own scratch file names (a shared `CARGO_TARGET_DIR` runs stale binaries; shared scratch names get overwritten).
- Releases, tags, crates.io publication, and yanks are the owner's.
- When a row ships, delete it here, record it in its proposal and `docs/versioning.md`, and the downstream ledger records the adoption.
