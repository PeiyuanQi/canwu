# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-11-13 at Canwu **0.23.0** (prepared on branch `release/0.23.0`, awaiting publication; §1). This file is the to-do list of engine gaps that a downstream application has filed
> against the published crates. The application is Celestial Mandate (CM); its gap ledger (implementation document `59`,
> in the CM repository) gives each row its id (`G-nn`) and the full evidence. Priorities are the ledger's severities:
> High, Medium, Low.
>
> This file holds the current state only. When a release closes a row, delete the row and record the change in its
> proposal and in `docs/versioning.md`.
>
> 0.21.0 closes every row the downstream filed after adopting 0.20.0: G-91 (Medium, from its B33 fix) and G-83 to G-90
> (Low, from its 0.20.0 adoption). Since then the downstream filed **G-92 and G-93** (Medium, 2026-10-26, from its
> day-cost package); 0.22.0 closes both. It then filed **G-94** (Medium, 2026-10-29: in-boundary knowledge pages cost
> the holder's history); 0.22.1 closes it. Since then it proposed **G-95** (Medium, 2026-11-05: no technology
> validation on a run restored from a continuation checkpoint), **G-96** (Low for its first chapter, Medium at series
> scale, 2026-11-06: one fiscal round per office and one evidence record per scope), and **G-97** (from its build: a
> declared run's seat could not move to an heir); 0.23.0 closes all three. Also open: the known limits the 0.18.0 to
> 0.23.0 packages left on purpose (§4.2, engine findings E-34 to E-54, E-56, E-58, E-59, and E-60 to E-62; E-55 and
> E-57 are fixed), two limits the downstream met and worked around (§4.3), and the decisions this run took for the
> owner to confirm (§6).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | **0.23.0: prepared, awaiting publication** (2026-11-13, a pre-1.0 minor release; owner's authorization: "once done, and released to crates.io and on the canwu website"). The release commit is on the local branch `release/0.23.0` (worktree `.worktrees/rel`), on top of the three merged packages: `6108737` (G-95, branch `fix/g95`), `8d819f7` (G-96, `fix/g96`), and `99b4fcb` (G-97, `fix/g97`). Not yet pushed, tagged, published, or deployed; the owner does that, then fills in this row. It closes G-95 (`docs/proposals/technology-restore-validation.md`), G-96 (`docs/proposals/fiscal-batch-sets-and-aggregated-evidence.md`), and G-97 (`docs/proposals/seat-rebinding.md`); no format number changes, the technology and fiscal semantic hashes change, and the new seat-rebinding fields are optional and written only by a run that rebinds. Verification on the release tree: fmt, Clippy with warnings denied, the workspace tests (883 passed, 0 failed, 3 ignored; 0.22.1's 867 and the packages' 16), the debug client, the documentation build with warnings denied, the four public examples, the minimum Rust version (1.88), the performance-harness checks (4 and, with allocation counting, 6 passed), the release-only `#[ignore]` scale tests in a release build (3 passed), the continuation gate across the workspace (883 passed, 0 failed, 3 ignored; the two expected panics: the first gate run failed on the G-95 test `a_noncanonical_payload_continuation_is_refused_on_both_handles`, whose scenario record names a version only boundary 2 creates, so the release pass added `canwu-technology`'s never-enable `continuation-gate` hook and that test now expects the gate's panic beside `a_packet_first_declared_after_its_seal_is_not_ready`), the package contents (`LICENSE`, `NOTICE`, and `README.md` in all 24), a clean `cargo publish --workspace --registry crates-io --dry-run --locked` (all 24 packaged and verified in dependency order, uploads aborted), the third-party notices regenerated with cargo-about 0.8.4 (unchanged: no dependency changed), and the website build, with a copy review of the changed pages in both languages. 0.22.1 (tag `v0.22.1`, `7a7feff`), 0.22.0, 0.21.0, and 0.20.0 stay published and are not yanked; 0.19.0 was yanked on 2026-10-24. The downstream pins `=0.22.1` since CM 0.97.0 and moves to `=0.23.0` once it is published; its next free ledger id is G-98. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.23.0`); nothing written by an earlier release is read; no migration. |
| Proposals of 0.23.0 (all "implemented in 0.23.0") | `docs/proposals/`: `technology-restore-validation.md` (G-95), `fiscal-batch-sets-and-aggregated-evidence.md` (G-96), `seat-rebinding.md` (G-97). Earlier releases' proposals are listed in their entries of `docs/versioning.md`. |
| Branches and worktrees | `main`; locally `release/0.23.0` (worktree `.worktrees/rel`) until 0.23.0 is published, and the merged package branches `fix/g95`, `fix/g96`, and `fix/g97` (worktrees `.worktrees/g95`, `.worktrees/g96`, `.worktrees/g97`), each to be removed after publication. On the remote, `main` and the merged cloud branch `claude/youthful-johnson-b38ta4`, left for the owner to delete. |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`. The orchestrator runs the full verification list once on the integrated release tree and the pre-release tests (the public examples, the performance-harness checks, the release-only `#[ignore]` scale tests in a release build, the continuation gate) before tagging (the counts go in the release row). |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.23.0`; the exact engine-version check refuses 0.22.1 saves, and a run that registers the technology or fiscal plugin is also refused by that plugin's semantic hash. (2) Source changes: add `parts: Vec::new()` to every `FiscalExecutionEvidence` struct literal; add the `SeatRebinding` arm to exhaustive matches on `IngressPayload` and `BoundaryIngressKind`, and `seat_rebindings: Vec::new()` / `rebound_seat: None` to literals of `BoundaryRecord`, `BoundaryReceipt`, and `SimulationSnapshot`. (3) Restore continuations through `from_continuation_checkpoint_verified` with the stored continuation hash (the authoritative state hash does not cover the seat binding in force), then call `canwu_technology::validate_technology_runtime_on(&run)` on a run with the technology plugin. (4) Technology content: a new operation's exact citation of a superseded version that no current technology record holds is rejected (cite the current version), and records near the 16 KiB payload limit grow by the versions their continuations add. (5) Optional: fiscal batch sets (`fiscal_action_set_command`, and an own-name controller with `DecisionAuthority::Actor` and the same person as command subject, which holds that person's fiscal roles in any set command a ticket offers it) and aggregated evidence (one record, one receipt per scope); seat rebinding (`rebind_seat`; read the binding in force with `seat_binding()`). (6) The "Upgrading from 0.22.1" notes and "Known limits" of the 0.23.0 entry of `docs/versioning.md`; the 0.22.1 and earlier notes still apply to a host that skipped those releases. |

## 2. High

No row filed by the downstream ledger is open.

## 3. Medium

No row filed by the downstream ledger is open. G-95 (no technology validation on a continuation-restored or sealed
run, Medium, proposed 2026-11-05), G-96 (one fiscal round per office and one evidence record per scope, Low for the
downstream's first chapter and Medium at series scale, proposed 2026-11-06), and G-97 (a declared run's seat could not
move to an heir, proposed by the downstream's build) are closed by 0.23.0
(`docs/proposals/technology-restore-validation.md`, `docs/proposals/fiscal-batch-sets-and-aggregated-evidence.md`,
`docs/proposals/seat-rebinding.md`; the 0.23.0 entry of `docs/versioning.md`).

G-94 (in-boundary knowledge pages cost the holder's history, filed 2026-10-29) is closed by 0.22.1
(`docs/proposals/bounded-knowledge-pages.md`; the 0.22.1 entry of `docs/versioning.md`).

G-92 and G-93 (host-side knowledge and resource reads, filed 2026-10-26) are closed by 0.22.0
(`docs/proposals/host-side-targeted-reads.md`; the 0.22.0 entry of `docs/versioning.md`).

G-91 (a continuation restore counted as a seal for the fiscal archive's citation check) is closed by 0.21.0
(`docs/proposals/fiscal-archive-after-continuation-restore.md`).

## 4. Low

No row filed by the downstream ledger is open. G-83 (`docs/proposals/movement-kept-bookings.md`), G-84
(`docs/proposals/decision-ticket-option-bias.md`), and G-85 to G-90 (`docs/proposals/military-engagement-revisions.md`)
are closed by 0.21.0.

### 4.1 Recorded, not proposed

None. G-38 (the `canwu-military` shape contradicted the downstream design) is closed by 0.20.0's engagement engine
(`docs/proposals/military-engagements.md`); its criterion of a second independent host adopting the crate is open
(§4.2, part of the military limits).

### 4.2 Known limits left by the 0.18.0 to 0.23.0 packages (engine-side; not numbered by the downstream ledger)

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
| E-56 | Low | `canwu-resource` (0.22.0) | A re-attach after the live root changes (nearly every step that settles resource work) decodes the live root and the changed buckets but attaches every record its requests reach again from the kept decodes: O(attached records), not O(changed). Reuse needs the run's own record allocations: a kept root from a view reader, or from another process, re-reads everything. | A host that needs work in proportion to what changed reads targeted records. |
| E-58 | Low | `canwu-resource` (0.22.0) | `resource_operation_outcome_by_id`, `latest_resource_fulfillment`, `exact_resource_completion_certificate`, `resource_report`, `resource_observation_witness`, and the validators still read the whole root: no key names what they scan, or their checks read records no key reaches. | Not proposed. |
| E-59 | Low | `canwu-knowledge` (0.22.1) | A page that filters by subject reads the entries of its schemas and time range that lack the subject (only schema, `learned_after`, the cursor, `learned_at_or_before`, and `CurrentHeads` are seeks), and a page naming no schema seeks once per schema the holder ever held. A commit that clears or removes an entry re-hashes the holder's views after the hash copy before it (one per 256 views), once per holder, paged or not: clearing an old entry under a long history costs that history once (10,000 entries: 14 ms in a release build); an existing body given below the holder's highest global ID costs the same. Each admission hashes its view (about 3 µs), decoding re-hashes every view (median +14% on a 3.7 MB ledger), memory +10% per entry and 9 KB per holder. | The chunked view root (`docs/proposals/bounded-knowledge-pages.md`, alternatives) bounds the re-hash but changes the read-cut root and every hash over a stored cut; not taken (§6 item 9). Revisit if the downstream's clearing commits show the re-hash. |
| E-60 | Low | `canwu-technology` (0.23.0) | `validate_technology_runtime_on` does not replay provenance: operation outcomes before the run's history cut are not recomputed. Before the cut, a consumed intent's provider ingress and finalizer change are not read; what the run held there rests on the continuation hash the host stored, or on the sealed segments. A forged identity of an older version of a current record that obeys the identity rule (not version zero, its record current, older than the current version and from a boundary no later than it) cannot be told from a real one without the full journal, so the plugin and both validators accept it on every handle, and its time is the forged boundary's, which can be earlier than the record existed; when owner-authorized maintenance updates a record earlier in the boundary, the full validator's replay may refuse a forged identity of it that the plugin accepted (technology registers no maintenance). `TechnologyRestoreAccess` is an open trait: an outside implementor must add any read a later release adds. The two consumed-intent checks are duplicated (about 100 lines). | A host that needs provenance keeps a full save and runs `validate_technology_runtime`. Declaring generic citations identity-only would refuse forged identities but declares far more than the exact closure; not taken (`docs/proposals/technology-restore-validation.md`, alternatives). Merge the consumed-intent checks when the refusal order may change. |
| E-61 | Low | `canwu-fiscal`, `canwu-ming-fiscal-reference` (0.23.0) | A controller whose authority is an actor and whose command subject is that same person holds the person's fiscal roles in any set command a ticket offers it, even a one-batch set (the host decides which tickets offer one). A set has one issuer: offices held by different persons take one set per person, and where those persons decide through decisions, each person's decision still takes its own round. The reference integration's boundary system does not yet skip aggregated evidence, so aggregated evidence sent to it as raw plugin ingress can still fail that boundary (its enqueue helper refuses it). Aggregated evidence saves evidence records and ingress, not ledger writes: each scope still takes its own receipt. Receipts citing aggregated evidence leave the per-scope path's books but for their citations, so the ledger records and hashes differ from it (the named departure from the gap row's equal state hash). | Skip aggregated evidence inside the reference integration's boundary as a recorded refusal; a multi-issuer set only if a host needs one round across persons. |
| E-62 | Low | `canwu-sim`, `canwu-api` (0.23.0) | A run with direct command history cannot rebind its seat (`MixedCommandIngress`); read-only and replay-purpose runs cannot rebind (`InteractionReadOnly`), so a replay-purpose run keeps its configured binding. A boundary-window summary counts a seat rebinding under the command class (filter player commands on kind). One rebinding per seat may be queued at a time, and a retry after its boundary is refused at the call as stale. A rebinding changes the person, controller, and permission profile only, never the seat or its institution, and cancels tickets only when the person changes, for the seat's controllers that act for the previous person. A seat bound to an institution without a person cannot be rebound (`InvalidAuthority`). A continuation's authoritative state hash does not cover the binding in force (restore through `from_continuation_checkpoint_verified` with the stored continuation hash). `CanwuRun` does not gain `rebind_seat`. | Binding the seat in the control commitment was rejected (the state hash would cover seat policy; `docs/proposals/seat-rebinding.md`, alternatives). Widen a limit only if a host meets it. |

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

1. **Publish 0.23.0** (the owner): push the release commit to `main`, tag `v0.23.0`, publish the 24 crates, create
   the GitHub release, and deploy the website; then fill in the release row (§1) and remove the 0.23.0 worktrees and
   the merged `fix/g95`, `fix/g96`, and `fix/g97` branches.
2. **Wait for the downstream's adoption of 0.23.0** (its ledger records the adoption of G-95, G-96, and G-97 at the
   `=0.23.0` pin) and for the rows it files against 0.23.0. G-94's real check is still its `Daily` p99 past campaign
   month 22; if its clearing commits show E-59's re-hash, measure how old the cleared entries are before choosing
   anything.
3. Then the §4.2 limits by downstream need: **E-50** first if the downstream runs cross-plugin completions through a
   long campaign (the cap of 1,024 ended participants); **E-49** if a host wants malformed correspondence commands
   recorded rather than validated before sending; E-41's force-supply deferral if a host runs the reference economy
   near its byte cap; E-46 to E-47, the military limits (E-51), the 0.21.0 limits (E-52 to E-54), E-59, and the
   0.23.0 limits (E-60 to E-62) as the downstream's slices ask; §4.3 if the downstream asks.
4. Each change set follows the 0.18.0 pattern: one worktree and branch per package under `.worktrees/`, a proposal,
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

Decisions the 0.22.0 package took (G-92 and G-93, `docs/proposals/host-side-targeted-reads.md`), for the owner to confirm:

7. **The Chinese term 已读资源根** (kept resource root), in `docs/terminology.md` and both website term pages, marked for
   the owner in `docs/terminology.md` until confirmed.
8. **`verify-targeted-reads` is opt-in.** The whole-root cross-check of the targeted resource helpers is a feature of
   `canwu-resource`, off by default and on only in Canwu's own tests, chosen over an always-on check (which made each
   helper cost a whole read in downstream debug builds and could panic there).

Decided by the owner for 0.22.1 (G-94, `docs/proposals/bounded-knowledge-pages.md`):

9. **Hash continuity over a bounded clearing cost** (owner, 2026-11-05, "release it" on the branch as built). The read-cut
   root keeps its material, kept as a running hash, so no stored read cut or hash over one changes; the price is E-59's
   one-time re-hash when an entry leaves. The chunked view root, which bounds that cost and changes the root, was not
   taken.

Decisions the 0.23.0 packages took (G-95 to G-97), each recorded in its proposal, for the owner to confirm:

10. **G-95's identity rule.** The technology plugin and both validators accept a cited version the run no longer holds
    by its identity (a committed boundary established it, not version zero, its record current, older than the
    current version and from a boundary no later than it), so sealed, restored, and uninterrupted runs decide alike; a
    forged identity that obeys the rule is accepted (E-60). A record's payload continuation also declares what each
    supersedable cited version declares. The restore validator reads through a trait in `canwu-technology`
    (`TechnologyRestoreAccess`), not new `CanwuRun` methods.
11. **G-96's shape.** Only the set command admits the own-name decision (the batch command keeps 0.22.1's rule), and
    receipts citing aggregated evidence record that one version, so the books equal the per-scope path's but for
    their citations and the state hash differs: a named departure from the gap row's equal state hash.
12. **G-97's shape.** A seat rebinding is host ingress, not a command, decided before any other ingress its boundary
    admits; only life state refuses a person (a detained or captive heir may hold the seat); the binding in force is
    in the checkpoint hash and the boundary chain, not in a commitment root or the authoritative state hash.
13. **The Chinese terms** of the release, in `docs/terminology.md` and both website term pages: 行动批次组 (fiscal
    action batch set), 本人名义决策 (decision in one's own name), 汇总执行证据 (aggregated execution evidence), 证据分项
    (evidence part), and 席位改绑 (seat rebinding), marked for the owner in `docs/terminology.md` until confirmed.

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
