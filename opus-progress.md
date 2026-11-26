# Opus progress — downstream engine gaps / Opus 进度记录

> Updated 2026-11-23 at Canwu **0.24.0**, the latest published release (tag `v0.24.0`, release commit `7f53466` on `main`, published 2026-11-23 UTC; 0.23.0, tag `v0.23.0`, release commit `fd6ac8b`, stays published). This file is the to-do list of engine gaps that a downstream application has filed
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
> declared run's seat could not move to an heir); 0.23.0 closes all three. It then filed **G-98** (2026-11-13: no
> ticket a person in custody decides), **G-99** (2026-11-15: no condition fall or repair of a worn facility),
> **G-110** and **G-111** (2026-11-18: no expansion project; no capacity change on an open account), and **G-100 to
> G-102** (2026-11-19: one engagement per command group, a garrison's surrender, order confusion by step length);
> 0.24.0 closes all seven. Also open: the known limits the 0.18.0 to
> 0.24.0 packages left on purpose (§4.2, engine findings E-34 to E-54, E-56, E-58, E-59, and E-60 to E-66; E-55 and
> E-57 are fixed), two limits the downstream met and worked around (§4.3), and the decisions this run took for the
> owner to confirm (§6).

## 1. Where things are

| Item | State |
| --- | --- |
| Release | **0.24.0 is published** (2026-11-23 UTC, 2026-11-22 PT: all 24 library crates on crates.io in one workspace publish with `--registry crates-io` from the tagged commit, after a clean dry run and CI green on Linux, macOS and Windows; the annotated tag `v0.24.0` on the release commit `7f53466`; the GitHub release; the website deployed from `main`; a registry-only consumer builds and runs against `=0.24.0`. Prepared 2026-11-22, a pre-1.0 minor release; owner's request of 2026-11-22: finish the open gap changes, release to remote `main`, update the website documents, clean up, and make sure the website is released). The packages were merged into `release/0.24.0` after review: `304d172` (G-111, branch `fix/g111`, merge `6ed92ae`), `903f584` (G-98, `fix/g98`, merge `78d1262`), `3c143f0` (G-99 and G-110, `fix/g99-g110`, merge `34e085f`), and `aa2f9fd` (G-100 to G-102, `fix/g100-g102`, merge `a1cdcbb`). No package has a proposal; the contract is the 0.24.0 entry of `docs/versioning.md`, the rustdoc, and the crate READMEs. Formats: commitment 8 → 9 (`canwu.commitment.decisions.v5`), decision archive 2 → 3, resource state 6 → 7, production archive blob 3 → 4, military `SCHEMA_VERSION` 2 → 3; the resource, production, and military semantic hashes change; the snapshot, checkpoint/evidence-journal, paged-checkpoint, and continuation-checkpoint formats keep their numbers. Verification: by the owner's rule for this run (2026-11-22, "only test after all fixes are in") the workers ran only fmt and Clippy, and the integrator ran the full list once on the integrated tree with the 0.24.0 version and docs (2026-11-22): fmt; Clippy with warnings denied, plain and with the continuation gate; the workspace tests, 897 passed, 0 failed, 3 ignored; the debug client; rustdoc with warnings denied; the four public examples; the minimum Rust version; the performance-harness fmt, Clippy, tests (4) and allocation contract (6); the release-only scale tests in a release build, 3 passed; the continuation gate across the workspace, 897 passed with the two tests that expect its panic; and the website build (87 pages). The two docs reviews (accuracy against the code, website copy) were applied before it. The tag, the crates.io publication, the GitHub release, and the website deployment followed the verified release commit. 0.23.0 (tag `v0.23.0`, `fd6ac8b`), 0.22.1, 0.22.0, 0.21.0, and 0.20.0 stay published and are not yanked; 0.19.0 was yanked on 2026-10-24. The downstream pins `=0.23.0` since CM 0.121.0 and moves to `=0.24.0` by its own adoption plan; its next free ledger id is G-112. |
| Policy | **Same version only** (owner, 2026-10-14): Canwu crates depend only on the exact same version of each other (`=0.24.0`); nothing written by an earlier release is read; no migration. |
| Proposals of 0.24.0 | None of its own: the four packages were built from the downstream's gap rows. `docs/proposals/military-engagements.md` is updated in place for G-100 to G-102. 0.23.0's proposals are `technology-restore-validation.md` (G-95), `fiscal-batch-sets-and-aggregated-evidence.md` (G-96), and `seat-rebinding.md` (G-97); earlier releases' proposals are listed in their entries of `docs/versioning.md`. |
| Branches and worktrees | `main` only (0.24.0). After the publication the release branch `release/0.24.0` (local and remote), the package branches `fix/g98`, `fix/g99-g110`, `fix/g100-g102`, and `fix/g111`, the docs branch `docs/0.24.0`, their worktrees, the tag worktree, and the git-ignored run folder `.worktrees/run-0.24` were removed (owner's request of 2026-11-22). |
| Release procedure | `.agents/skills/canwu-contributor-release/SKILL.md` and `docs/releasing.md`. The orchestrator runs the full verification list once on the integrated release tree and the pre-release tests (the public examples, the performance-harness checks, the release-only `#[ignore]` scale tests in a release build, the continuation gate) before tagging (the counts go in the release row). |
| What the adopter must know first | (1) Pin every Canwu crate at `=0.24.0`; the exact engine-version check refuses 0.23.0 saves, a commitment at format 8 is refused by its format, and a run that registers the resource, production, or military plugin is also refused by that plugin's semantic hash. (2) Source changes: `custody_addressed: None` on every `DecisionTicketDraft` and `DecisionTicket` literal and `cancelled_custody_tickets: Vec::new()` on `BoundaryPersonAvailabilityChange`; the new arms `SetAccountCapacity` (`ResourceOperationRequestV1`, `ResourceOperationKind`), `RecordFacilityCondition` (`ProductionOperation`), `Expand { .. }` (`FacilityProjectKind`, no longer `Copy`), `OpenGroupEngagements`, `SurrenderSiege`, and `Surrendered`; military literals add `group: None` and, on each side, `confusion_carry: 0` and `confusion_paid_to` at the opening time, and a rulebook renames `cohesion.confusion` to `confusion_per_day` with the cost of one source per day of fighting. (3) A captor's question: open the ticket with `custody_addressed: Some(DecisionCustodyAddress::new(captor))` for the captive's own controller, with no command option, and apply the answer in the host; read endings from all three lists: `cancelled_custody_tickets` (`decision_maker_dead`, `custody_ended`, `custody_transferred`), `cancelled_tickets` for a captive who goes missing, and `cancelled_controller_tickets` on the change of another person the ticket's controller acts for; a custodian change within one polity is a transfer and cancels. (4) A store that grows: `enqueue_resource_account_capacity` citing the works' current record, sent while no completion lease locks the account's current revision (an account receiving a production execution's output stays locked for the execution's run); it replaces a second "annex" account. (5) Facilities: `RecordFacilityCondition` citing the host's own current record with its provider-evidence digest, `lifecycle_after` by the 250 ‰ line; `Repair` straight on a `Degraded` or `Damaged` facility; one `Expand` per capacity level, listing every capability. (6) Military: `OpenGroupEngagements` replaces a one-engagement stand-in for a split side in the field (siege assaults are not grouped, so a stand-in for a split besieging side stays); `SurrenderSiege` for a yielding garrison, read by the siege's `end_cause`. (7) The "Upgrading from 0.23.0" notes and "Known limits" of the 0.24.0 entry of `docs/versioning.md`; the 0.23.0 and earlier notes still apply to a host that skipped those releases. |

## 2. High

No row filed by the downstream ledger is open.

## 3. Medium

**G-118** (filed 2026-11-25, `canwu-fiscal`): a scope ledger keeps every settled execution request and receipt of
its hot books, so each ledger write rewrites the scope's history (the downstream measured 1.6 KB to 15.8 KB per
ledger over 14 months and 234.9 MB of boundary evidence). Built on branch `fix/g118` for 0.25.0, not released: a
request fulfilled to its quantity moves, with its receipts, into one of `FISCAL_SETTLED_BUCKETS` (8) per-scope
`canwu.fiscal:settled` records; the book keeps per-assessment settled figures; an archive commit merges the entries
back into the archived book and empties the buckets (the 0.25.0 entry of `docs/versioning.md`). The downstream
dropped the row's conditional part (b): `CloseAccountingCycle` already closes a book with an outstanding assessment.

No other row filed by the downstream ledger is open. The seven rows 0.24.0 closes carry no severity in the ledger rows as
copied for this run (each names the downstream wave it is wanted by, W5 to W7 or later), so they are recorded here:
G-98 (a ticket a person in custody decides, filed 2026-11-13), G-99 (a facility's condition fall and a repair on a
degraded or damaged facility, 2026-11-15), G-110 (an expansion project, 2026-11-18), G-111 (a capacity change on an
open account, 2026-11-18), and G-100 to G-102 (one engagement per command group, a surrender end cause, order
confusion prorated by step length, 2026-11-19). All are closed by 0.24.0 (the 0.24.0 entry of `docs/versioning.md`;
for G-100 to G-102 also `docs/proposals/military-engagements.md`).

G-95 (no technology validation on a continuation-restored or sealed
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

### 4.2 Known limits left by the 0.18.0 to 0.24.0 packages (engine-side; not numbered by the downstream ledger)

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
| E-63 | Low | `canwu-decision`, `canwu-sim` (0.24.0) | A custody-addressed ticket offers no command option (command admission still refuses the captive as issuer), so the host applies the answer. The mark is set only at `Open`. The controller-authority exemption covers only a controller whose authority person is the decision maker; a ticket whose controller acts for another person who becomes unavailable is cancelled with `controller_authority_unavailable` in that person's `cancelled_controller_tickets`. A captive who goes `Missing` is cancelled with `decision_maker_unavailable` in `cancelled_tickets`, not in `cancelled_custody_tickets`, so a host reads all three lists. `Hostage` is available, so a captive who becomes a hostage of the same custodian ends the ticket with `custody_ended`; a change between `Captive` and `Detained` under the same custodian keeps it. A transfer is any changed custodian: the engine cannot tell a move within one polity from one to another. A mismatched custodian at `Open` is `InvalidDecision`, with no code of its own. | A command option answered through a host-named issuer, or a custody reason for `Missing`, only if a host needs them. |
| E-64 | Low | `canwu-resource` (0.24.0) | `SetAccountCapacity` is the host's only; the engine checks its evidence's currency and kind, not what it says (as for a rescope, E-47). A completion lease that locks the account's current revision refuses it, so an account receiving a production execution's output cannot be resized during that execution's run. A lowering counts the escrow of every open transfer naming the account at either end, also one on its way back to another account, so it can refuse a capacity the stock would fit; other credits may still fill the room a lowering left for stock in flight. A closed account's capacity cannot change. The rule is stricter than the gap row's "`capacity_after` at or above the balance" (§6 item 15). | Count only the escrow that can still be credited to the account, if a host meets the refusal. |
| E-65 | Low | `canwu-production` (0.24.0) | `RecordFacilityCondition` only lowers a condition and takes no `Repairing`, planned, or retired facility; a worn `Operational` facility above 250 ‰ records `Degraded` before a `Repair` is admitted. Its evidence check is the record's currency, the provider-evidence digest, and a non-empty capability: any record kind, the binding's `kind` and `available_quantity` unchecked, and never what the record says. Every completed project, an `Expand` included, returns the facility `Operational` at 1,000 ‰, so an expansion also restores the condition. One open project per facility generation: no expansion during a repair. `Expand` names the whole capacity map (at most 64 capabilities, none dropped or lowered). Pre-existing (0.23.0 has the same rule), now stated in the README and the 0.24.0 entry: while any of the allocations that overlap at a time is `Reserved`, their sum is held to the condition-adjusted capacity (`FacilityAsset::usable_capacity`); once all are `Consumed` the bound is the nominal capacity. `StartExecution` accepts only `Reserved` allocations and turns them `Consumed` before the state is validated, and it checks neither the facility's condition nor its lifecycle, so an execution whose allocations overlap no reserved one starts within the nominal capacity on a worn or `Damaged` facility too; a host that wants a worn facility to take less work sizes its allocations by `usable_capacity`. Whether `StartExecution` should check condition and lifecycle is passed to the CM adoption plan as a possible next gap row; nothing is promised here. | A partial restore (a project that completes below 1,000 ‰), or a condition and lifecycle check at `StartExecution` and a capacity check on consumed allocations, if the downstream files the row. |
| E-66 | Low | `canwu-military` (0.24.0) | Siege assaults are not grouped (a siege's forces are not free), so a besieging side split into command groups fights one assault and a host's stand-in for that case stays. Every command group of an engagement group is one owner's; the shared side takes no joiner; a command group whose opening share of the shared men would be 0 is refused. A shared-side input or order, and a profile revision from either side, advances the `host_revision` of every open engagement of the group. A held engagement of a group closes only when the host resumes or abandons it: after a group-wide `Abandon` by the shared defenders of a grouped relief lifts the siege, a held member keeps the besiegers engaged until then. The day of `confusion_per_day` is `siege.day_minutes`; a resumed engagement starts its confusion afresh, so a held span costs nothing (a resumed member of a group keeps the shared side's paid-up-to time); a confusion remainder below one point is never charged once the engagement closes; a step records no separate trace of the confusion cost. An assault closed by a surrender reads `AttackerPrevailed` with `site_fell`, as a storm: the siege's `end_cause` tells them apart. The synthetic rulebook keeps the figure 50 under the new per-day unit. | Grouped assaults (a siege side with command groups) if the downstream's siege design asks; a confusion term on `StepV1`. |

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

1. **G-118 on the 0.25.0 line:** branch `fix/g118` is built, reviewed (two independent reviews and their delta
   reviews), and tested (the workspace tests and the fiscal crates under the continuation gate); the downstream
   proposes G-117 (`Retire` / `Delete` of a create-only domain record, `canwu-sim`) for the same line. Integration,
   the release's full verification, and publication wait for the owner.
2. **The downstream's adoption of 0.24.0** follows its own plan (its ledger records the adoption of G-98 to G-102,
   G-110, and G-111 at the `=0.24.0` pin) and the rows it files against 0.24.0. G-94's real check is still its
   `Daily` p99 past campaign month 22; if its clearing commits show E-59's re-hash, measure how old the cleared
   entries are before choosing anything.
3. Then the §4.2 limits by downstream need: **E-50** first if the downstream runs cross-plugin completions through a
   long campaign (the cap of 1,024 ended participants); **E-49** if a host wants malformed correspondence commands
   recorded rather than validated before sending; E-41's force-supply deferral if a host runs the reference economy
   near its byte cap; E-46 to E-47, the military limits (E-51), the 0.21.0 limits (E-52 to E-54), E-59, and the
   0.23.0 and 0.24.0 limits (E-60 to E-66) as the downstream's slices ask; §4.3 if the downstream asks.
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

Decisions the 0.24.0 packages took (G-98 to G-102, G-110, G-111), each recorded in the 0.24.0 entry of
`docs/versioning.md`, for the owner to confirm:

14. **G-98's shape.** The host sets the mark at `DecisionMutation::Open` (`DecisionTicketDraft::custody_addressed`, a
    `DecisionCustodyAddress { custodian }`), as the run-11 corrected contract asks. Decided beyond the row: the mark
    refuses `DecisionAction::Command` options (the captive's answer could never be issued as a command); a missing
    captive keeps `decision_maker_unavailable` in `cancelled_tickets` (the row is silent); the same custodian's
    `Captive` ↔ `Detained` change keeps the ticket, and `Hostage` ends it with `custody_ended`; the controller
    exemption is narrow (only the decision maker as authority person); a mismatched custodian is
    `InvalidDecision` with no new error code; the custody pass of the sweep runs only for a person with an open
    custody-addressed ticket. Commitment format 9 (`decisions.v5`) and decision archive format 3; the snapshot,
    journal, paged, and continuation formats keep their numbers.
15. **G-111's shape, stricter than the row.** A tuple variant around a request struct with an `operation_key` and
    `expected_account_revision`, as every resource operation has; `capacity_after: Option<u64>` (`None` removes the
    bound); host-only ingress like a rescope. Beyond the row's "admitted when `capacity_after` ≥ the balance": a
    lowering must also leave room for the escrow of every open transfer naming the account; a closed account is
    refused (a rescope accepts one); a completion lease locking the account's current revision refuses the change.
16. **G-99's and G-110's shape.** `RecordFacilityCondition` carries a `ProductionEvidenceBinding` whose record may be
    of any kind, checked for currency and digest after the authority and fall rules; a record that changes nothing
    is refused; a `Repairing` facility takes none. `Expand`'s `capacity_after` is a per-capability map (the asset's
    capacity is one), at most 64 entries, none dropped or lowered; an `Expand` completes `Operational` at 1,000 ‰
    like every project, also from `Degraded`, `Damaged`, or `Repairing`. Fixed with them: a facility leaves the
    incident due index when it stops being eligible, and a completed project pins only its result's site and, at
    the result's generation, its capacity.
17. **G-100 to G-102's shape.** G-100: one command, `OpenGroupEngagements`, opens every
    engagement of the group; the shared forces' men are split by largest remainder at every step, the shared
    side's cohesion, shock, and order-confusion state are one figure copied to every open member, shared-side
    inputs and orders apply to the whole group, and the group keeps one step grid (a profile revision on one
    revises and resumes every open member); an opening share of 0 is refused and a member whose share falls to 0
    skips its step; siege assaults are not grouped. G-101: a surrender end cause (`SurrenderSiege`, `SiegeEndCause::Surrendered`) rather
    than a release from the assault, which could empty a side and still close as a storm. G-102: the rulebook's
    `confusion_per_day` with a carried remainder and a paid-up-to time (`SideStateV1::confusion_carry`,
    `confusion_paid_to`), charged by the plugin after the resolver for the minutes since the side was last
    charged, rather than a finer cohesion unit or a `StepProfileV1` field; the day is `siege.day_minutes`; the
    reference content keeps the figure 50 under the new unit. Military `SCHEMA_VERSION` 3.
18. **No proposals for 0.24.0.** The packages were built from the gap rows under the owner's request to finish the
    open gap changes; the contract is the 0.24.0 entry of `docs/versioning.md`, the rustdoc, and the crate READMEs,
    with `docs/proposals/military-engagements.md` updated in place.

**Terms for the owner to confirm** (0.24.0; in `docs/terminology.md` and both website term pages, marked for the
owner in `docs/terminology.md` until confirmed):

| English | Proposed 中文 | Code |
| --- | --- | --- |
| custody-addressed ticket | 在押人物票据 | `DecisionTicketDraft::custody_addressed`, `DecisionCustodyAddress` |
| account capacity change | 账户容量变更 | `ResourceOperationRequestV1::SetAccountCapacity`, `enqueue_resource_account_capacity` |
| facility condition record | 设施状况记录 | `ProductionOperation::RecordFacilityCondition` |
| expansion project | 扩建项目 | `FacilityProjectKind::Expand` |
| command group | 指挥组 | `CommandGroupV1` |
| engagement group | 交战组 | `EngagementGroupV1`, `OpenGroupEngagements` |
| siege surrender | 围城投降 | `SurrenderSiege`, `SiegeEndCause::Surrendered` |
| daily confusion cost | 每日号令混乱代价 | `CohesionRuleV1::confusion_per_day` |

The pages also use, without a term row, 看管方 for a custodian (already on the integration page) and 号令混乱 for
order confusion.

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
