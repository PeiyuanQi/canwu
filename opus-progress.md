# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-10-25 at Canwu **0.21.0** (release in progress on `release/0.21.0`; the release row below is completed by
> the orchestrator at the tag). This file is the to-do list of engine gaps that a downstream application has filed
> against the published crates. The application is Celestial Mandate (CM); its gap ledger (implementation document `59`,
> in the CM repository) gives each row its id (`G-nn`) and the full evidence. Priorities are the ledger's severities:
> High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.21.0 closes every row the downstream filed after adopting 0.20.0: G-91 (Medium, from its B33 fix) and G-83 to G-90
> (Low, from its 0.20.0 adoption). No downstream row is open. What remains: the known limits the 0.18.0 to 0.21.0
> packages left on purpose (§4.2, engine findings E-34 to E-51), two limits the downstream met and worked around
> (§4.3), and the decisions this run took for the owner to confirm (§6).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | **0.21.0 — to be completed by the orchestrator at the tag:** the release commit, the annotated tag `v0.21.0`, the crates.io publication of all 24 library crates in the publish order of `docs/releasing.md`, the GitHub release, the website deployment (both languages showing `=0.21.0`), the registry-only consumer smoke test, and the verification counts (the full list, the pre-release tests, the continuation gate, the `#[ignore]` scale tests, the minimum-Rust-version check). 0.20.0 (tag `v0.20.0`, release commit `a82ffeb`, published 2026-10-24) stays the only live version until then; 0.19.0 was yanked on 2026-10-24. Downstream pins `=0.20.0` (CM 0.89.0) and moves to `=0.21.0` as its next change set. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.21.0` once released); nothing written by an earlier release is read; no migration. |
| Proposals of 0.21.0 (all "implemented in 0.21.0") | `docs/proposals/`: `decision-ticket-option-bias.md` (G-84), `movement-kept-bookings.md` (G-83), `fiscal-archive-after-continuation-restore.md` (G-91), `military-engagement-revisions.md` (G-85 to G-90; extends `military-engagements.md`). The 0.20.0, 0.19.0, and 0.18.0 proposals are listed in their entries of `docs/versioning.md`. |
| Branches and worktrees | `release/0.21.0` (worktree `.worktrees/release-0.21.0`) integrates the decision, movement, fiscal, and military packages (the military package's final commit `0d7ae1c` on `g21-military`, `d480572` on the release branch); `release-0.21.0-docs` carries the shared release documentation. The orchestrator deletes every 0.21.0 branch and worktree after the release, leaving `main` alone locally and on the remote. |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`. The orchestrator runs the full verification list once on the integrated release tree and the pre-release tests (the public examples, the performance-harness checks, the release-only `#[ignore]` scale tests in a release build, the continuation gate) before tagging (the counts go in the release row). |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.21.0`; nothing earlier loads (commitment format 8 and decision archive format 2 refuse 0.20.0 saves and archive blobs by their format as well as their engine version; a military payload at schema version 1 and a movement or military run by its semantic hash). (2) Read the "Upgrading from 0.20.0" notes of the 0.21.0 entry of `docs/versioning.md` and its "Known limits": `DecisionOption::bias` / `with_bias` (a per-ticket bias on the option, one controller per decision maker and class; struct literals add `bias: 0`); kept bookings (`bookings_per_execution` 64 kept at once, `bookings_per_runtime` 16,384, closed and replaced bookings leave the execution, `LimitExceeded` at the bound, a closed booking's final status from reports or the record history); a continuation restore is not a seal (`sealed_evidence_cursor()` is `None` until the restored run seals, so a continuation-only host archives in every session; a "not yet" arm for `UnsealedCitations` may stay); military `JoinSiege`, `ReviseSiegeV1::release`, `ReviseCrossing::defenders`, `OpenEngagementV1::max_minutes`, the timed step (`timed_step_ingress`, queued after each daily boundary instead of `Step`), no report queued for a subject without holders, `SCHEMA_VERSION` 2. (3) The 0.20.0, 0.19.0, and 0.18.0 notes still apply. |

## 2. High

No row filed by the downstream ledger is open.

## 3. Medium

No row filed by the downstream ledger is open. G-91 (a continuation restore counted as a seal for the fiscal archive's
citation check) is closed by 0.21.0 (`docs/proposals/fiscal-archive-after-continuation-restore.md`).

## 4. Low

No row filed by the downstream ledger is open. G-83 (`docs/proposals/movement-kept-bookings.md`), G-84
(`docs/proposals/decision-ticket-option-bias.md`), and G-85 to G-90 (`docs/proposals/military-engagement-revisions.md`)
are closed by 0.21.0.

### 4.1 Recorded, not proposed

None. G-38 (the `canwu-military` shape contradicted the downstream design) is closed by 0.20.0's engagement engine
(`docs/proposals/military-engagements.md`); its criterion of a second independent host adopting the crate is open
(§4.2, part of the military limits).

### 4.2 Known limits left by the 0.18.0 to 0.21.0 packages (engine-side; not numbered by the downstream ledger)

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
| E-51 | Low | `canwu-military` (0.20.0, 0.21.0) | No failure-mode model; two sides per engagement; no automatic contact; vessels are components; one wave size per crossing; reports are bands; retired records stay in the store and are validated at every load (about 2 KB per closed fight); an outcome signal whose target ingress no plugin registered fails the first closing boundary; no second independent host has adopted the crate yet (G-38's criterion (f)). 0.21.0 adds: a command admitted by the timed step's boundary (queued as ingress for its instant or proposed during it) applies in the next cut, after the step, so a day's orders still cost their boundaries; report holders are fixed at the opening, so a subject opened without them never queues a report; joined and released forces change the siege from its next siege day, and a besieger that joins during a relief stands beside it; a fight's duration is fixed at its opening and a fight the plugin opens takes its kind's; revised crossing defenders do not enter a fight in progress. | A per-wave size on `ReviseCrossing`; fidelity classes in reports; a second host adoption. |
| E-52 | Low | `canwu-movement` (0.21.0) | Closed bookings are not in the live runtime (a booking that closes in the boundary it was decided appears in no report); a replaced booking's identity is free again once its replacement leaves; the per-execution bound of 64 counts the leg under way's consumed lots and lost lots not yet replaced (two legs of 32 lots); one runtime-wide bound of 16,384 without a per-owner quota; each `RequestBooking` scans every kept binding for a `replaces` naming its identity; an `OfferPool` window can be narrowed past a consumed booking that left; a report near 64 kept bookings and 64 legs with maximal labels can pass the 64 KiB knowledge payload limit and is withheld. | A per-owner quota and a `replaces` index if a host meets them. |
| E-53 | Low | `canwu-decision` (0.21.0) | A trace records one bias per option and its ticket part is the bias on the ticket version the trace names (not recoverable for a deferring trace after a later `ReplaceOptions`, since hot history keeps only the latest version); only the shipped utility evaluator reads the ticket bias (rule, human, external, and LLM policies ignore it, `ExternalDecisionRequest` does not carry it); a policy built on `WeightedUtilityEvaluator` cannot opt out; the bias is unbounded (the host clamps when it drafts). | A per-version option record in hot history, or a two-part evaluation field, if a host needs the split after a re-bias. |
| E-54 | Low | `canwu-fiscal`, `canwu-sim` (0.21.0) | A continuation-only host keeps no segment, so the versions its archived books cite are in nothing it holds once the books leave the run (the archived book names them by identity); a host that seals at each save archives, in its first session after a continuation restore and until its first seal there, books whose citations the next seal puts in a segment rather than one that already holds them. | Seal at each save before the continuation checkpoint (already the documented practice); an archive commit that carries cited bodies was rejected as larger than the gap. |

### 4.3 Limits the downstream met and worked around (not filed)

| Id | Sev. | Crate | Limit | Downstream handling |
| --- | --- | --- | --- | --- |
| E-42 | Low | `canwu-movement` | Capacity pools are never retired, so a runtime holds at most 256 pools over its life. | Pools are opened sparingly; a long campaign may reach the limit. |
| E-43 | Low | `canwu-movement` | The holder-relative `movement_report` serves person holders only; institution owners get no report from it, and pool custodians learn booking identities from the host (they revoke by reading the runtime). | The application publishes its own movement knowledge for institutional holders. |

Also met downstream, both already in §4.2: E-34's activated lease that never expires (the application releases its
own leases) and E-37's publication-budget overshoot (the application's publishers reserve within the budget).

### 4.4 Engine facts the downstream design loop asked to confirm (answered 2026-10-19 from the 0.18.0 source; unchanged by 0.21.0)

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

1. **Finish the 0.21.0 release** (orchestrator): run the full verification list and the pre-release tests once on
   the integrated tree, bump the workspace version and every `=0.21.0` pin (the README pins,
   `developer/integration.mdx`, `HomePage.astro`), build the website once with its accuracy and copy reviews, make the
   release commit, tag, publish in the order of `docs/releasing.md`, deploy the website, and complete the release row
   above. Then the downstream moves to `=0.21.0` and its ledger records the adoption of G-83 to G-91.
2. Then the §4.2 limits by downstream need: **E-50** first if the downstream runs cross-plugin completions through a
   long campaign (the cap of 1,024 ended participants); **E-49** if a host wants malformed correspondence commands
   recorded rather than validated before sending; E-41's force-supply deferral if a host runs the reference economy
   near its byte cap; E-46 to E-47, the military limits (E-51), and the 0.21.0 limits (E-52 to E-54) as the
   downstream's slices ask; §4.3 if the downstream asks.
3. Each change set follows the 0.18.0 pattern: one worktree and branch per package under `.worktrees/`, a proposal,
   durable tests that bite, an independent code review (two for persistence-heavy packages) and a delta review of the
   fixes, integration on a release branch, the pre-release tests of the release skill (the continuation gate
   included), then one release commit with the shared docs, the website (with its accuracy and copy reviews), and the
   skills.

## 6. Open decisions (owner)

Decisions the 0.21.0 run took under the owner's instruction of 2026-10-25 (address every open downstream row, release,
deploy the website, and move the downstream to the new release), each recorded in its proposal and listed here for the
owner to confirm:

1. **G-83's bounds.** `bookings_per_execution` counts what an execution keeps at once (closed and replaced bookings
   leave it) and is raised from 16 to 64, so a leg of more than 16 lots books them at once with room for a later leg's
   lots while the current leg is under way; a new runtime-wide bound `bookings_per_runtime` of 16,384 (0.20.0's implied
   maximum of 1,024 orders × 16) keeps the worst-case runtime record where it was. Rejected: counting only `Requested`
   and `Confirmed` against the bound while keeping the rest (the record still grows for the execution's life), a
   tombstone or digest chain, moving consumed bookings out at departure, a per-leg bound
   (`docs/proposals/movement-kept-bookings.md`).
2. **G-84's shape.** The per-ticket bias sits on `DecisionOption` (`bias: i64`, `with_bias`) beside the policy's own
   `with_option_bias`; the evaluator records the sum on the existing `DecisionOptionEvaluation::bias` (not two fields),
   the guarded policy's semantic hash does not cover ticket biases, and the bias is unbounded (the host clamps). It
   revises 0.20.0's decision 5 ("the option bias sits on `WeightedUtilityEvaluator` rather than `UtilityProfile` or
   `DecisionOption`"): the policy's standing bias stays there; the ticket's rides the option. The decision commitment
   moves to `canwu.commitment.decisions.v4` (commitment format 8) and the decision archive to format 2
   (`docs/proposals/decision-ticket-option-bias.md`).
3. **G-89's release is on `ReviseSiege`**, not `PlaceForce`: `ReviseSiegeV1::release` takes forces off either side of a
   siege under the siege's `expected_host_revision`, so every change to a siege's sides is a siege command, and a
   besieger that joined with `JoinSiege` can leave again; a release that would leave no besieger is refused
   (`LiftSiege` ends a siege), and one that leaves no defender lets the place fall `Abandoned` with nobody dispersed.
   Rejected: `PlaceForce` admitting a defender (`docs/proposals/military-engagement-revisions.md`). With it, from the
   same proposal: `OpenEngagementV1::max_minutes` rather than a second assault kind (G-86); the timed step as a public
   plugin ingress the host queues (`military_timed_step_v1`) rather than a delayed `Step` command, the commands its
   boundary admits applying in the next cut after the step (G-90); a join during the siege's assault or sortie that
   also enters that fight, and one owner refused on both sides of a siege by `JoinSiege` and `InvestSite` (G-87);
   `ReviseCrossing::defenders` replacing the list under `OrderCrossing`'s checks, the issuer controlling every
   defender it removes unless trusted or the crossing force's controller (G-88); no report queued for a subject whose
   sides name no holder, the queue staying in the registry (G-85).
4. **G-91's shape: a continuation restore is not a seal.** `sealed_evidence_cursor()` answers where the run's own last
   seal ended and is `None` until a run seals after it is created or restored, so the fiscal archive's citation check
   refuses only past a cut the run itself sealed; `canwu-fiscal` changes no code. Rejected: recording in the
   continuation checkpoint whether its cut was sealed (a checkpoint is byte-equal whether the writer sealed or not),
   letting the archive commit carry the cited bodies (larger than the gap), a seal the host makes and discards
   (`docs/proposals/fiscal-archive-after-continuation-restore.md`). A host that read the cursor for where retained
   evidence begins combines it with `continuation_base()` (versioning notes).
5. **The Chinese terms** of the release, in `docs/terminology.md` and both website term pages: 选项偏置 (option bias)
   widened to cover `DecisionOption::bias`; 票据选项偏置 (ticket option bias); 在持预订 (kept booking); 已了结预订 (closed
   booking); and the military terms 定时战斗步 (timed step), 加入围城 (siege join), 撤出围城 (siege release), 交战时限
   (engagement time limit), marked for the owner in `docs/terminology.md` until confirmed. The fiscal wording is kept and
   made exact (封存切点 is the run's own last seal; 起点切点 is not a seal); no new fiscal term.
6. **E-55 fixed in the release.** The release's continuation gate (never run workspace-wide for 0.20.0) failed 11
   `canwu-economy-reference` tests, the same on 0.20.0: the unpublished `canwu-force-supply-reference` root cited
   earlier versions of itself without declaring them, so a seal dropped them and the resource's later declaration
   was refused (`ArchiveNotReady`). Fixed in the reference crate, not the engine (the engine's refusal is its
   designed guard): the root declares those versions identity-only while a live acquisition, grant, or certificate
   cites them, and refuses a citation of a version it has not written; its root format moves 2 → 3. Reviewed and
   delta-reviewed; the whole gate passes.

Every decision through 0.20.0 is decided and confirmed by the owner and recorded in its proposals, its
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
