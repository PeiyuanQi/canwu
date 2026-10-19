# Movement Leg Delay and Current Incident Evidence

Status: implemented in 0.19.0. It closes downstream gap G-64 (no operation
delays the arrival of a departed leg) and engine finding E-44 of
`opus-progress.md` (incident evidence checked against retained evidence). It
changes `canwu-movement` only; `canwu-transport`, `canwu-sim`, and `canwu-api`
are untouched. Same version only: no compatibility work with 0.18.0 bytes or
hashes.

"Today" means 0.18.0 as it is on `main`.

## Contents

1. [Problem](#problem)
2. [Decisions](#decisions)
3. [Implementer choices](#implementer-choices)
4. [Incident evidence: current, not replay-stable](#incident-evidence-current-not-replay-stable)
5. [Persistence and formats](#persistence-and-formats)
6. [What can still fail a boundary](#what-can-still-fail-a-boundary)
7. [Public API delta](#public-api-delta)
8. [Durable tests](#durable-tests)
9. [Documentation delta](#documentation-delta)
10. [Terminology candidates](#terminology-candidates)
11. [Known limits](#known-limits)
12. [Review record](#review-record)

## Problem

**G-64.** Once a leg departs, nothing can move its arrival. `depart`
(`lifecycle.rs`) schedules the arrival due at the actual departure plus the
planned duration and stores it as the order's `pending_due`; `CompleteLeg`
arrives the leg now; `Reroute` is refused while a leg is `Departed` ("a
departed leg must arrive or fail before a reroute"); and `FailLeg` followed by
`Reroute` re-runs the leg as an interruption with new itinerary and leg ids.
A host can therefore decide a hold (a flooded ford, a delayed escort) only
before departure. The validation required a departed leg to have an arrival
due but never checked its time.

**E-44.** Incident operations checked their evidence with
`SimulationView::domain_record_version_evidence_exists` (through
`ViewEvidence::version_exists`). For a superseded record version that answer
depends on retained evidence: an unsealed run and exact replay find the
version, while a sealed run finds it only if the continuation closure kept
it. A boundary decision (apply or refuse the incident) could therefore differ
between a sealed run and its replay. An order's `delivery_attempt` was checked
with the same function.

## Decisions

From the owner's decisions of 2026-10-19 (`opus-progress.md` §6):

- **D1.** A new movement operation `DelayLeg { execution, leg, until, reason }`
  replaces the arrival due of a departed current leg.
- **D2.** `until` is an absolute instant, not before the boundary time nor the
  arrival scheduled at departure, so a hold can be lengthened, shortened, or
  lifted, never make the journey faster.
- **D3.** It is requested by an owner or operator command, or by an incident
  with evidence that is a current version, each with a short reason.
- **D4.** Any number of delays; only the current due is kept (no history).
- **D5.** Owner and operator reports show the hold; remote observers learn of
  it only at arrival.
- **D6.** A later leg whose booking expires meanwhile fails through the
  existing recorded leg failure (`capacity_unavailable`).
- **D7.** `Reroute` stays refused on a departed leg.
- **D8 (E-44).** Incident evidence must be a current or replay-stable version;
  this package chooses **current** ([below](#incident-evidence-current-not-replay-stable)).

## Implementer choices

Names, codes, and the check order are the implementer's, for the owner to
confirm after the release.

- **Operation.** `MovementOperation::DelayLeg { execution, leg, until, reason }`.
  A command from the order's owner or operator, or a `movement_incident_v1`
  incident (the incident allow-list gains it).
- **The hold.** `MovementLegHoldV1 { leg, scheduled_arrival_at, until, reason,
  held_at }`, kept as `MovementOrderRecordV1::hold`, one per order. Each
  accepted delay replaces it; it clears when the leg arrives (by its due or by
  `CompleteLeg`), fails, or the execution closes.
- **The bound is recorded, not remembered.** `scheduled_arrival_at` is the
  arrival scheduled when the leg departed: the leg's `actual_departure_at`
  plus its planned duration (the same `planned_duration` helper `depart`
  now uses). It is derived from immutable leg and plan data, stored in the
  hold for reports, and validated against the derivation, so repeated delays
  cannot drift below it.
- **A new invariant.** `MovementState::validate` now checks a departed leg's
  arrival due time: it equals the scheduled arrival, or the hold's `until`
  when a hold exists; a hold names the departed leg, keeps the derived
  scheduled arrival, has `until >= scheduled_arrival_at`, `held_at` between
  the departure and `until`, and a bounded reason; no hold exists without a
  pending arrival due. Every path of the plugin satisfies it (a departure
  always schedules `at + duration`).
- **A lift keeps the hold.** Lifting is `until == scheduled_arrival_at`; the
  hold stays with that `until` (and its reason and time) until the leg
  arrives, so the owner's report shows when and why the hold was lifted and a
  report's `observed_as_of` never moves backwards.
- **Re-stating a hold** with the same `until` is accepted and updates the
  reason and time; it schedules one more due packet, which the packet already
  queued makes stale, or the other way round (both are identical).
- **Check order and codes**, each a recorded rejection on the operation's
  outcome:
  1. incident only: the operation is on the incident allow-list
     (`Unauthorized`), and its evidence is the current version of its record
     (`Evidence`; an undeclared kind is `Evidence` as before);
  2. the execution is known (`NotFound`) and the command holder is its owner
     or operator (`Unauthorized`);
  3. the reason is a non-empty, trimmed label of at most 256 bytes, the
     bound of `FailLeg` and `Cancel` (`InvalidOperation`);
  4. `leg` is the current leg and is `Departed` (`InvalidState`);
  5. `until` is no earlier than the settling boundary and no earlier than the
     scheduled arrival (`InvalidOperation`);
  6. `until` is within `MovementLimitsV1::time_bound_minutes`
     (`LimitExceeded`);
  7. the candidate runtime validates (`InvalidState` otherwise).
- **Same-boundary order.** Operations settle before due packets in one
  boundary, so a delay settled in the boundary of the arrival it replaces
  wins: the arrival packet no longer equals `pending_due` and is skipped. The
  stale packet of an earlier hold is skipped the same way at its time.
- **Explicit arrival is unchanged.** `CompleteLeg` still arrives a held leg
  now (an owner or operator asserting the arrival), and `FailLeg` still fails
  it; both clear the hold. The "never faster" bound applies to `DelayLeg`.
- **Reports.** `MovementReportDetailV1::hold` carries the hold to the owner and
  operator; `held_at` is an event time of detailed reports only, so a delayed
  remote observer's report never changes for a hold, and it learns of one
  only from the leg's `arrived_at`.
- **E-44's fix extends to the order's `delivery_attempt`.** The finding
  names incident operations, but an `Order`'s `delivery_attempt` was checked
  through the same function (`ViewEvidence::version_exists`) and had the same
  defect: a superseded attempt version was accepted by an unsealed run and by
  exact replay and refused by a sealed run. No boundary decision may depend
  on retained evidence (`opus-progress.md` §7), so the delivery attempt must
  now be the current version of its record too, refused otherwise as a
  recorded `Evidence` rejection ("the delivery attempt must be the current
  version of its record"). Consequence: an order whose delivery-attempt
  record is revised between the order's submission and its settlement is
  refused; the host resubmits it with the new version. Rejected
  alternative: accept any `attempt.version <= current.version`, as a
  reroute's `condition_exists` does; it is replay-stable too, but it gives up
  the check of the cited version's `established_by` (an invented source
  would pass) and would admit an attempt version the application has since
  replaced. The orchestrator accepted this extension; the owner is asked to
  confirm it. The authority basis already had to be current, and a reroute's
  `ExternalCondition` reads only the record's current version number, so
  after this change no movement decision reads retained evidence.
- **Linked transfers.** A delay creates no itinerary revision and no leg, so a
  resource transfer's `TransportExecutionLink` to the execution and leg stays
  valid.

## Incident evidence: current, not replay-stable

`ViewEvidence::version_is_current` reads
`SimulationView::domain_record_version_is_current`: the cited version must be
its record's version in the boundary's overlay or current state, with its
establishment checked. Both candidates are replay-stable; they differ only in
what else they admit. `replay_stable_domain_record_version` would also admit a
superseded initial-scenario version, and would keep admitting a committed
version that an earlier system of the same boundary has just superseded.

Current is chosen because:

1. **The owner's decision for D3 says current.** One rule for every incident
   operation is simpler than a separate rule for delays.
2. **No asymmetry.** Replay-stable would accept a superseded version only if
   the scenario created it, and refuse every superseded version made by a
   boundary. Both mean the application has since changed the record; a rule
   that tells them apart by origin has no domain meaning.
3. **Same rule as the authority basis.** An order's `authority_basis` already
   must be current (`current_record`); incident evidence, which also asserts
   a present fact, joins it, and so does the delivery attempt.
4. **Replay-stable by construction.** The current version is authoritative
   state, not retained evidence: a version whose establishing boundary is
   sealed still resolves while it is current, and a same-boundary proposal is
   read from the overlay alike in every run.

The cost: an application that revises its evidence record between reporting
an incident and its settlement must cite the new version, or report after the
revision. An incident settles in the boundary its ingress is due in (the next
boundary for host ingress queued at `now`), so the window is the delay the
reporter chose.

## Persistence and formats

| Item | 0.18.0 | Implemented |
| --- | --- | --- |
| `MovementOperation` (command and incident payloads) | 10 variants | adds `delay_leg { execution, leg, until, reason }` |
| `MovementOrderRecordV1` (in the runtime record `canwu.movement:runtime`) | — | `hold: Option<MovementLegHoldV1>`, omitted when `None`, so a runtime with no hold encodes as before |
| `MovementState::validate` | arrival due exists | arrival due time checked (above); applied at activation, at each load of the runtime, and to every candidate |
| `MovementReportDetailV1` (report payload) | — | `hold: Option<MovementLegHoldV1>`, omitted when `None`; a report without a hold, its digest included, is byte-identical |
| `MOVEMENT_SEMANTIC_HASH` | `2d3c7d8e…a383` | `6cf78f1f…550e`, BLAKE3 of `canwu-movement plugin semantic hash 0.19.0: DelayLeg holds a departed leg; cited versions must be current` |
| `movement_report` knowledge schema hash | `31cf5c80…1e3a` | `81cf8978…4f11`, BLAKE3 of `canwu-movement movement_report knowledge schema 0.19.0: MovementReportDetailV1::hold`; the schema id stays at version 1 |
| `movement_incident_v1` descriptor description | "…leg failure, reroute, or handoff" | "…leg failure, delay, reroute, or handoff" |
| Snapshot, continuation, journal formats; `canwu-transport` records | — | unchanged |

The semantic hash changes, so a run saved by 0.18.0 with the movement plugin
does not restore; under the same-version policy none is expected to. A
scenario that installs a runtime root with a departed leg
(`MovementState::into_initial_record`) must now give it the arrival due at
the leg's departure plus its planned duration, or a hold whose `until` is no
earlier and equals the arrival due.

## What can still fail a boundary

Nothing new. Every refusal of `DelayLeg` and of non-current evidence is a
recorded rejection on the operation's outcome; the boundary stands.

- "movement due work precedes its boundary" (`plugin.rs`) stays unreachable
  for a delay: the operation refuses `until` before the boundary, and the
  candidate validation refuses a hold whose `until` precedes its `held_at`
  (checked by mutation: with the operation's bound removed, the same request
  is still a recorded `InvalidState` rejection).
- The held arrival is ordinary due work: if it cannot apply, the leg fails
  with `due_settlement_failed`, as today.
- What could already fail a movement boundary still can: a runtime record that
  fails to decode, validate, or encode (a corrupted or hand-edited root), and
  the runtime revision overflowing.

## Public API delta

`canwu-movement` only:

- `MovementOperation::DelayLeg { execution, leg, until, reason }` (new
  variant; `MovementOperation::execution` returns its execution).
- `MovementLegHoldV1` (new, re-exported from the crate root).
- `MovementOrderRecordV1::hold` and `MovementReportDetailV1::hold` (new public
  fields).
- `MOVEMENT_SEMANTIC_HASH` (new value).
- Behaviour: `MovementIncidentV1::evidence` and
  `MovementOrderRequestV1::delivery_attempt` must be current versions;
  `MovementState::validate` checks the arrival due time.

The new variant and fields break exhaustive matches and struct literals
downstream, as any 0.x minor release may. No `canwu-api` or `canwu-sim`
change, so no `gated!` change.

## Durable tests

`crates/extensions/canwu-movement/tests/gap_g64_movement_delay_leg.rs`:

1. **`gap_g64_movement_delay_leg`**: two movements a > b > c, ten minutes a
   leg from minute 10; the second books its b > c crossing for 20 to 30. A
   delay before departure is `InvalidState`. At 12, delays ending at 11 and
   at 15 are `InvalidOperation`, one naming the next leg `InvalidState`, a
   257-byte reason `InvalidOperation`, a person without a grant
   `Unauthorized`, all recorded with the boundary standing. The owner holds
   the first movement until 40 and the second until 60; a reroute of the
   held leg is `InvalidState`; the operator shortens the first hold to 30;
   after the incident record is revised, an incident delay citing its first
   (initial-scenario) version is `Evidence` and one citing the revision holds
   until 36; the owner lifts it (until 20, the scheduled arrival). A third
   movement held until 50 has its held leg failed by the operator at 12:
   applied, the hold cleared, and a following `Reroute` from where it stopped
   applies (it settles at 25). At 30, a
   hold of the second movement ending at 25 (after the scheduled arrival,
   before now) is `InvalidOperation`. The run saved there, with the second
   hold pending, and restored matches the original in state and checkpoint
   hash at 45 and in checkpoint hash at 95. The lifted
   movement arrives at 20 and settles at 30, the stale packets skipped; the
   held one arrives at 60, after its booking window, and its next leg fails
   with `capacity_unavailable`, its booking `Expired`. Owner and operator
   reports show each hold when it was set (`observed_as_of` 12, 14, 16, 18)
   and none after arrival; the remote observer's reports change only at 0,
   10, and 60 for the held movement, though its view passed the hold's time
   at 42 while the leg was held; exact replay matches.
2. **`e44_incident_evidence_is_current_alike_in_sealed_and_unsealed_runs`**:
   the incident record is revised to version 2 and then 3 by boundaries; the
   run seals, after which the sealed run no longer knows version 2 and its
   unsealed twin does (asserted). In both, an `Order` whose delivery attempt
   cites version 2 is an `Evidence` rejection and one citing version 3 is
   applied; a `FailLeg` and a `DelayLeg` citing version 2 are `Evidence`
   rejections and a `DelayLeg` citing version 3 is applied; the movement states and checkpoint hashes agree,
   the held leg arrives at 150 in both, and exact replay of the sealed run
   equals both.

Each was checked inline (not committed) to fail under a plausible regression:

| Mutation | Test 1 | Test 2 |
| --- | --- | --- |
| evidence read back to `domain_record_version_evidence_exists` (0.18.0) | fails | fails |
| evidence read as `replay_stable_domain_record_version` | fails | passes |
| no boundary-time bound on `until` | fails | passes |
| no scheduled-arrival bound on `until` | fails | passes |
| hold time an event time of remote reports | fails | passes |
| hold missing from report detail | fails | passes |
| `DelayLeg` not on the incident allow-list | fails | fails |
| hold kept after the leg arrives | fails | fails |
| hold kept after the leg fails (`fail_leg` clears only `pending_due`) | fails | passes |
| delivery attempt read back to `domain_record_version_evidence_exists` | passes | fails |

The existing `gap_g19_movement_lifecycle_plugin` passes unchanged.

## Documentation delta

For the release documentation pass (canonical page and both website
languages each):

- `docs/architecture.md`, `canwu-movement` paragraph: the operation list
  becomes "`Order`, `StartLeg`, `CompleteLeg`, `FailLeg`, `DelayLeg`,
  `Reroute`, `RecordHandoff`, `RequestBooking`, `Cancel`, or `OfferPool`";
  "Seizures and reconciliations are accepted only from application systems
  through the `movement_incident_v1` ingress with an exact evidence record of
  a declared kind" becomes "…with the current version of an evidence record
  of a declared kind". Add after the phase-7 sentence: "`DelayLeg` holds a
  departed leg: it replaces the leg's arrival due with an absolute `until`
  no earlier than the boundary or the arrival scheduled at departure, so a
  hold can be lengthened, shortened, or lifted but never makes the leg
  faster; the owner and operator see the hold in their reports, and a
  delayed remote observer learns of it only from the arrival."
- `docs/end-state.md`, the 0.13.0 movement paragraph: "arrive only as
  application incidents that cite exact evidence" becomes "…that cite the
  current version of an evidence record", and add "Since 0.19.0 an owner,
  operator, or incident can hold a departed leg (`DelayLeg`)."
- `website/src/content/docs/{,en/}architecture/routing-movement.mdx`: the
  `MovementOperation` row adds `DelayLeg`; the `MovementIncidentV1` row
  becomes "An application-reported `FailLeg`, `DelayLeg`, `Reroute`,
  `RecordHandoff`, or `Reconcile` that cites the current version of an
  evidence record"; step 3 adds "delayed a departed leg"; step 5 adds "the
  arrival time of a departed leg"; "Leg timing" adds: "A `DelayLeg` holds the
  current departed leg until an absolute `until`, no earlier than the
  boundary or the arrival scheduled at departure (its departure plus its
  planned duration). It replaces `pending_due`, so later delays lengthen,
  shorten, or lift the hold, and only the latest is kept. A delay settled in
  the boundary of the arrival it replaces wins. A refused delay is a
  recorded rejection. A later leg whose booking expires meanwhile fails with
  `capacity_unavailable` when it is due to depart. `Reroute` stays refused
  while a leg is departed." The reports paragraph adds the hold for the owner
  and operator. Chinese: 路段滞留 per the candidates below.
- `docs/versioning.md`, the release entry: the operation; "incident evidence
  and an order's delivery attempt must now be the current version of their
  record (a superseded version is refused even while retained), so a sealed
  run and its replay decide alike; an order whose delivery-attempt record is
  revised before it settles is refused, and the host resubmits it with the
  new version"; the format rows above; "a movement runtime whose departed
  leg's arrival due is not its departure plus its planned duration, and
  that holds no `MovementLegHoldV1`, is now invalid: a scenario-installed
  runtime root (`MovementState::into_initial_record`) with such a leg is
  refused at load, and so is any restored runtime"; the known limits below.
- `docs/terminology.md` and both website term pages: the rows below; the
  `movement lifecycle extension` row may add "and holds departed legs".
- The engine-user skill's movement notes, if any, the same two sentences.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| leg hold | `MovementOperation::DelayLeg`, `MovementLegHoldV1`, `MovementOrderRecordV1::hold` | 路段滞留 | A hold on a departed leg that moves its arrival to an absolute instant no earlier than the arrival scheduled at departure; later holds lengthen, shorten, or lift it, only the latest is kept, and only the owner and operator see it before the leg arrives. Not 延误 alone, which in prose means any lateness. |
| scheduled arrival | `MovementLegHoldV1::scheduled_arrival_at` | 排定到达时刻 | The arrival a departed leg was scheduled for when it departed: its actual departure plus its planned duration. A hold never ends before it. |

## Known limits

- **Explicit arrival.** `CompleteLeg` arrives a held leg at once, before its
  scheduled arrival if the owner or operator says so, as in 0.18.0.
- **Stale packets.** Each `DelayLeg` schedules one due packet and leaves the
  packet it supersedes queued until that packet's own due time, which can be
  as late as the time bound (`MovementLimitsV1::time_bound_minutes`). Until
  then the superseded packet is pending ingress like any other: a seal and a
  continuation checkpoint pin it, `pending_ingress()` lists it, and at its
  time it wakes a boundary (one that would otherwise be empty), where it is
  skipped without a trace as stale, as stale packets of explicit operations
  are today. Their number is bounded per live execution by its operation
  outcomes (below), but they accumulate over a long run with many holds.
- **"Any number" is bounded by outcome quotas.** Each delay is one operation
  with one durable outcome, kept while its execution lives: 1,024 per command
  holder and 16,384 per runtime (commands are refused at admission past
  their share, and an incident past the cap emits
  `canwu.movement.operation_dropped.v1`).
- **Hold history.** Only the latest hold is kept (D4); the reports already
  published keep each earlier one in the owner's and operator's knowledge.
- **A booking expiry republishes a detailed report** without moving
  `observed_as_of` (0.18.0 behaviour: an expiry is not an event time).
- **Evidence revised before settlement.** An incident whose evidence record is
  revised before the incident settles is refused; the application cites the
  revision or reports after it.

## Review record


The code was reviewed once by an independent reviewer and the fixes once in a
delta review; neither found a blocking issue. Accepted findings:

- No test failed a held leg: a `fail_leg` that left the hold in place passed
  every test. A test now fails a held leg and reroutes it.
- Requiring the order's `delivery_attempt` to be current is correct under the
  rule that no decision may depend on retained evidence, but goes beyond the
  owner's wording for E-44 and was untested. A sealed-and-unsealed test now
  covers it, and the extension is listed for the owner to confirm.
- The stale-packet known limit now states that superseded due packets stay
  queued, pinned, and listed until their time.
- A scenario-installed departed leg whose arrival due is neither its
  scheduled arrival nor its hold is now refused at load; the versioning
  notes state it.
- Optional: an assertion message in the sealed-and-unsealed test, and one
  sentence of this proposal aligned with the versioning notes.
