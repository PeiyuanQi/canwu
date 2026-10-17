# Request Outcome Retention

Status: implemented on branch `e28-request-retirement` for the release
after 0.17.0; not released. The design was reviewed once (engine review
"implement with changes", host-side review "ready with changes") and the code
once ("commit after fixes"); every finding is resolved in
[Review record](#review-record), and [As implemented](#as-implemented) is
authoritative where the code differs from the text below. It answers engine
finding E-28 of `opus-progress.md` (the continuation checkpoint grows with
every tracked command a run has settled) and the part of E-29 (a simulated
month's wall time creeping) that shares its cause. It changes `canwu-sim`
and `canwu-api` only. Same version only: no compatibility work with an
earlier release's bytes.

The contracts it builds on: tracked command requests, exact retries, and
expected revisions as [architecture](../architecture.md) states them; sealing
with work in flight and the continuation checkpoint of
[continuation checkpoint](continuation-checkpoint.md); the derived
command-history index of
[command admission history index](command-admission-history-index.md); and
the settled-boundary horizon of [boundary window](boundary-window.md), whose
"kept from `now` minus a horizon" rule this design reuses.

Every section leads with its decision. "Today" means 0.17.0 as it is on
`main`. Costs are diagnostics: the owner's rule is player lag, judged by a
human at each speed.

## Contents

0. [As implemented](#as-implemented)
1. [The problem, measured](#the-problem-measured)
2. [Decisions on one page](#decisions-on-one-page)
3. [What admission and idempotency read from old outcomes](#what-admission-and-idempotency-read-from-old-outcomes)
4. [The retention window](#the-retention-window)
5. [One rule, one resolver](#one-rule-one-resolver)
6. [Retired request IDs](#retired-request-ids)
7. [Answers before and after the window](#answers-before-and-after-the-window)
8. [Sealed, unsealed, restored, and replayed runs](#sealed-unsealed-restored-and-replayed-runs)
9. [Dependencies, kept receipts, and provenance facts](#dependencies-kept-receipts-and-provenance-facts)
10. [Formats](#formats)
11. [Restore validation and tamper](#restore-validation-and-tamper)
12. [Cardinality and cost](#cardinality-and-cost)
13. [What can still fail a boundary](#what-can-still-fail-a-boundary)
14. [Public API delta](#public-api-delta)
15. [What a host must know](#what-a-host-must-know)
16. [Terminology candidates](#terminology-candidates)
17. [Documentation delta](#documentation-delta)
18. [Acceptance tests](#acceptance-tests)
19. [Measurement plan and gate](#measurement-plan-and-gate)
20. [Profile results](#profile-results)
21. [E-29](#e-29)
22. [Non-goals, deferred work, and known limits](#non-goals-deferred-work-and-known-limits)
23. [Alternatives rejected](#alternatives-rejected)
24. [Open decisions](#open-decisions)
25. [Review record](#review-record)

## As implemented

- `canwu-sim` `policy.rs`: `RunConfiguration::request_outcome_retention`
  (omitted from the wire at the default), `with_request_outcome_retention`,
  `RunConfigurationSnapshot::request_outcome_retention()`,
  `DEFAULT_REQUEST_OUTCOME_RETENTION` (30 days),
  `MAX_REQUEST_OUTCOME_RETENTION` (366 days); validation refuses a negative
  or longer retention.
- `error.rs`: `ErrorCode::RequestOutcomeRetired` (`request_outcome_retired`),
  cross-referenced with `IdempotencyConflict` in its rustdoc.
- `state.rs`: `RuntimeEvidence::retired_command_requests`
  (`RetiredRequestIds`, an `Arc` of canonical ranges, binary-searched), the
  resolver `RuntimeEvidence::command_request_retired(request, kept_from)`,
  `retire_command_requests` (the seal's move), `command_outcome_time`,
  `ranges_contain`, and `retired_ranges_are_canonical` (zero is a valid
  member: admission records the rejected attempt of a request whose ID is
  zero, and it retires like any other).
- `ingress.rs`: `Simulation::request_outcomes_kept_from` and
  `command_request_retired`; `enqueue_command` and `cached_command_outcome`
  ask the resolver before any table or journal.
- `decision.rs`: `enqueue_decision` refuses a retired nested command ID with
  `RequestOutcomeRetired`; `command_request_id_is_in_use` and
  `command_request_id_is_unique_for_admitted_decision` count the ranges.
- `persistence.rs`: `evidence_dependencies` skips retired requests in every
  table and in the retained attempts and command ingress, and finds a
  retained attempt's command by ID instead of a scan; the seal calls
  `retire_command_requests` after `seal_command_history`.
- `continuation.rs`: `RetiredRequestRange`, `RequestIndex::retired_commands`,
  continuation format 3; the builder splits outcomes by the rule;
  `check_shape` runs `check_retired_requests` (ranges canonical, nothing kept
  in a range, outcomes inside the retention and at or below the cut, pending
  command requests listed and never retired).
- `canwu-api`: re-exports `RetiredRequestRange` and the two constants.
- Tests: [Acceptance tests](#acceptance-tests).

## The problem, measured

A run keeps, for every tracked command request it has ever settled, the
request's exact outcome so that a retry gets the same answer. A sealed run
holds these in `RuntimeEvidence::archived_command_requests` and
`archived_ingress_requests` (`state.rs`), and a continuation checkpoint
writes them as `RequestIndex::commands` and `queued_commands`. Neither ever
shrinks.

The `fiscal-world-profile` held-knowledge row (`--courts institution
--cadence interval --held-cycles N`) runs monthly cycles on a sealed run:
every scope assessed on day 1 and every book closed on day 29, one batch
command per institution each time, then the books archived and the evidence
sealed. Per-field sizes of its continuation checkpoint (0.17.0, smoke preset:
20 partitions, 420 institutions, 840 tracked commands a month), growth per
month from the fifth month:

| Field | Growth a month | Per tracked command |
| --- | ---: | ---: |
| `requests.commands` (one outcome per request) | 216 KB | 257 B |
| `requests.queued_commands` (one enqueue receipt per request) | 138 KB | 164 B |
| `ingress_provenance` (a `{"ingress": N}` fact per request's ingress) | 14.3 KB | 17 B |
| `evidence_times` (one run per distinct instant and journal) | 2.5 KB | — |
| fiscal domain records | 4 KB | — |
| everything else (knowledge, record versions, frontiers) | flat | — |

That is 438 bytes per tracked command, 98 % of the growth. The world preset
(200 partitions, 4,200 institutions, 8,400 tracked commands a month) grows
the same way ten times over: 172 MB after the first month, 293 MB after the
24th (3.8 MB a month), restored in 58 s. A host that keeps a world-scale
campaign for a simulated century would carry 4.5 GB of request outcomes.

The provenance facts are a side effect: every queued receipt declares its
ingress record as an identity dependency (`evidence_dependencies`,
`persistence.rs`), and every declared ingress keeps a provenance fact. The
same function also declares the attempt, the command, and every emitted event
of every outcome, so the dependency list and the kept archive receipts grow
by three entries per tracked command; [E-29](#e-29) measures what that costs
each seal.

## Decisions on one page

- **D1. Outcomes are kept for a window.** A tracked command request's outcome
  is kept for the run's **request outcome retention** of simulated time
  after its attempt: default 30 days, any value from zero to 366 days, set in
  `RunConfiguration` when the run is created and fixed for it. After that the
  request is **retired**.
- **D2. A retired ID stays used.** The run keeps the IDs of retired requests
  as canonical ranges. A later request with a retired ID, an exact retry or
  not, is refused as `RequestOutcomeRetired`, without evidence and without a
  revision, so an ID is still never reused and a request is still never
  accepted twice.
- **D3. One rule, never a seal.** A request is retired exactly when its
  attempt's time is before `now` minus the retention. One resolver applies
  that rule to the ranges, the archived tables, and the retained attempts,
  and every reader asks it first; a seal only moves retired outcomes into the
  ranges. Unsealed, sealed, continuation-restored, and replayed runs give the
  same answers and write the same continuation checkpoint.
- **D4. Retirement touches no authoritative state.** No state, event,
  record, persisted outcome, state hash, boundary hash, or journal changes.
  What changes is the non-persisted answer to a late retry, and the parts of
  a save that are derived from the request tables: the request index, the
  provenance facts and dependencies it caused, and the kept archive receipts
  those dependencies reached.
- **D5. Formats.** `RequestIndex` gains `retired_commands`; continuation
  checkpoint format 3. The retention is omitted from a run configuration at
  its default; no other format changes.
- **D6. Decision requests are unchanged** (deferred, with the decision
  ledger's own history; [Non-goals](#non-goals-deferred-work-and-known-limits)).
- **D7. E-29.** The request index's share of the seal's growth is bounded by
  D1. The rest of a world-scale seal is proportional to the sealed segment,
  not to history; it is recorded as a separate finding.

## What admission and idempotency read from old outcomes

Decision: nothing that changes state reads a settled outcome; the paths that
change state read only whether an ID is used, which retirement preserves.
The outcomes serve exact retries; the ID serves the refusal of reuse, which
must last for the run.

Every reader of the request tables and of retained attempts by request ID:

| Reader (today) | Asked by | What it needs |
| --- | --- | --- |
| `cached_command_outcome` (`ingress.rs`) | `process_command`; a queued command's admission at its boundary; a decision's nested command; replay (`replay_attempt_record`, `replay.rs`) | the original outcome of an exact retry; a used ID with other input refused (`IdempotencyConflict`) |
| `enqueue_command`'s receipt lookup and retained-ingress scan (`ingress.rs`) | `enqueue_command` | the original `IngressReceipt` of an exact re-enqueue; a used ID with other ingress content refused |
| `command_request_id_is_in_use` (`decision.rs`) | `enqueue_command`, `enqueue_decision`, a boundary-generated decision, replay ingress (`enqueue_replay_ingress_cut`) | whether the ID was ever used |
| `command_request_id_is_unique_for_admitted_decision` (`decision.rs`) | a decision's admission (a persisted decision rejection when false) | whether the ID was used by anything but that decision |
| `archived_request_was_queued` (`state.rs`) | the unqueued-history flag and the index's first unqueued attempt | whether a still-retained attempt's request was queued |
| `evidence_dependencies` (`persistence.rs`) | seals, checkpoints, the compact identity | identity of every outcome's attempt, command, and events, and of every receipt's ingress (no reader needs these: an exact retry is answered from the stored outcome) |
| `continuation_checkpoint` (`continuation.rs`) | saves | writes the tables |

`Simulation::submit` passes no request ID and reads nothing. Replay's two
readers see each journaled ID once, in a fresh run with no ranges.

The revision fence already makes a second acceptance impossible without any
history: a tracked request carries `expected_revision`, admission refuses it
unless that equals the current revision, and the revision strictly increases
with every accepted command, persisted rejection, and settled boundary. So
an exact retry of an old request could never be accepted again even if the
run forgot it. What the stored outcome adds is the answer: "accepted, with
this receipt" rather than "stale". That answer matters while a host can
plausibly retry, which is short; the refusal of a reused ID matters for the
life of the run, and needs only the ID.

No state-changing path reaches a settled outcome. A queued command's ID
cannot be in use when it is admitted, because `enqueue_command` refused it if
it was and its pending ingress holds it until admission; the same holds for a
decision's nested command. The one state-changing reader of the tables,
`command_request_id_is_unique_for_admitted_decision`, asks only whether the
ID is used, and a retired ID stays used. A cached answer, a collision, and a
retired answer are all returned without an attempt, a revision, or an event,
so they are not journaled and replay never sees them.

## The retention window

Decision: a request is retired at simulated time `now` exactly when its
attempt's time is before `kept_from(now, retention)`, the same saturating
"now minus a horizon" that the settled-boundary ledger uses
(`boundary_window.rs`). An outcome settled at `t` is therefore answered at
every instant up to and including `t + retention`.

- **Which time.** The attempt's `at`, which equals the outcome's
  `accepted_at` or `rejected_at`, so a sealed outcome carries its own time
  and no new field is stored. A request with no attempt (a queued command
  still pending, a decision's nested command whose decision did not admit
  it) is never retired.
- **Configuration.** `RunConfiguration::request_outcome_retention`, a
  `SimDuration` from `SimDuration::ZERO` to `MAX_REQUEST_OUTCOME_RETENTION`
  (366 days), default `DEFAULT_REQUEST_OUTCOME_RETENTION` (30 days), set with
  `RunConfiguration::with_request_outcome_retention`. It is omitted from the
  wire at the default, so a default configuration's semantic hash does not
  change. A run without a declared configuration (`CompatibilityV1`,
  `ManifestOnlyV1`, `LegacyUnspecified`) uses the default.
- **Fixed at creation.** The value is part of the run's identity because it
  decides what a save contains. It cannot be raised on restore: the outcomes
  past the old window are gone, and a checkpoint written under one value is
  not canonical under another. A host that may need a longer window sets it
  when it creates the run. (Shortening on restore could be allowed later;
  deferred.)
- **What it governs.** Only how long after settlement a lost answer can be
  recovered by retrying. It does not interact with saves: a restore moves
  `now` back with the rest of the state, requests from before the save keep
  the answers they had at the save, and requests made after the save are
  unknown to the restored run and admitted (or revision-refused) as new.
  The window is simulated time, so a wall-clock pause retires nothing, and a
  fast simulation speed retires sooner in wall time: at a speed where a
  simulated month passes in tens of seconds, a remote client with a long
  timeout can come back after the window. Choose the retention from the
  highest speed and the longest client retry or reconnect delay; a host whose
  clients can be away longer keeps its own outcome log keyed by request ID
  (the receipts it already received).
- **Why simulated time.** It is deterministic and independent of seals. A
  revision or attempt-count window would bound the table more tightly but
  would make the retry window shrink as the world gets busier.
- **Why 30 days and 366.** Thirty days covers every in-process and remote
  retry at ordinary speeds and keeps one month of outcomes at world scale
  (8,400, about 3.5 MB) where today keeps all of them. The cap matches the
  settled-boundary horizon's year; a host that keeps a year of outcomes pays
  for twelve months of them, and an unbounded setting would bring E-28 back
  for that host ([Open decisions](#open-decisions)). Zero is allowed: an
  outcome is answered only at the instant it settled, which for a
  canonical-ingress host means no retry after the next advance.

Time only moves forward, and a fork or restore carries `now` with the rest of
the state, so a retired request stays retired.

## One rule, one resolver

Decision: `RuntimeEvidence::command_request_retired(request, kept_from)`
answers the rule for one request ID, and every reader asks it before it
consults a table or a journal. The code that walks a whole table (the seal's
move, the checkpoint builder, and the dependency loops) applies the same
comparison, `at < kept_from`, to each outcome or attempt it visits.

It answers true when the ID is in the ranges, when the archived outcome
table holds an outcome for it that settled before `kept_from`, or when the
retained attempt for the request (found through the command-history index)
settled before `kept_from`. `Simulation::command_request_retired` supplies
`kept_from` from the run's `now` and retention.

Order matters for lockstep. `enqueue_command` answers an exact re-enqueue
first from the archived receipts and then by scanning retained and pinned
ingress records, which carry no attempt time. In an unsealed run a retired
request's ingress record is still retained, so a scan before the resolver
would return the original receipt while a sealed twin answers
`RequestOutcomeRetired`, and the release continuation gate, which compares
every result, would diverge. `enqueue_command`, `cached_command_outcome`,
`enqueue_decision`'s nested command, and `evidence_dependencies` therefore
ask the resolver first.

## Retired request IDs

Decision: the IDs of retired requests whose attempts lie at or below the
settled cut are kept as a sorted list of disjoint, non-adjacent inclusive
ranges; every reader of "is this ID used" consults them.

- **Representation.** `RetiredRequestRange { first, last }` over
  `CommandRequestId`; in the runtime `RetiredRequestIds`, an `Arc` of the
  sorted list that a fork shares and a seal or restore replaces whole, read
  by binary search (no reverse walk of an ordered map).
- **What goes in.** Only requests whose attempts lie at or below the cut. A
  retired attempt above the cut (an unadmitted direct command after
  `advance` moved time without a boundary) is carried in full as unadmitted
  evidence; the resolver retires it by its time all the same.
- **Exactness.** An ID in a gap between ranges was never used (or is still
  live) and is admitted as today. Nothing is approximate: no filter, no
  floor, no hash. The ranges cover every ID an attempt can carry, zero
  included: `process_command` refuses a zero request ID as an expected
  rejection and records the attempt, which retires like any other.
- **Cost.** One range per maximal run of consecutive retired IDs; see
  [Cardinality and cost](#cardinality-and-cost) for counter, revision-derived,
  and hashed IDs.

## Answers before and after the window

Decision: within the window every answer is today's; after it, every host
request that names the ID is refused as `RequestOutcomeRetired`, except the
boundary-generated decision path, which keeps its generic guard error.

| Request | Within the window (today) | After the window |
| --- | --- | --- |
| `process_command`, exact retry | the original `CommandOutcome` | `CommandOutcome::Rejected` with `RequestOutcomeRetired`, `attempt_id: None`, the current revision and time |
| `process_command`, same ID, other input | rejected, `IdempotencyConflict` | rejected, `RequestOutcomeRetired` |
| `enqueue_command`, exact re-enqueue | the original `IngressReceipt` | `Err(RequestOutcomeRetired)` |
| `enqueue_command`, same ID, other content | `Err(IdempotencyConflict)` | `Err(RequestOutcomeRetired)` |
| `enqueue_decision` whose nested command reuses the ID | `Err(IdempotencyConflict)` | `Err(RequestOutcomeRetired)` |
| a boundary-generated decision whose nested command reuses the ID | `Err(InvalidDecision)` | `Err(InvalidDecision)` (unchanged) |
| a fresh ID | admitted | admitted |

After retirement the engine no longer knows the request's input, so it
cannot tell an exact retry from a collision; the distinct code says what it
knows. `RequestOutcomeRetired` is a non-persisted refusal like a collision:
no attempt, no revision, no event. It is built directly as an outcome, not
raised through the admission error path, so it is not in
`is_expected_command_rejection`. It cannot arise inside a boundary, since a
queued request's ID is never used when it is admitted; if it did, it would
be an `Ok` rejection that leaves the boundary standing, as a collision is
today.

## Sealed, unsealed, restored, and replayed runs

Decision: every request table and every answer is a function of the attempts
the run has settled, the ingress journal, `now`, and the retention; no seal
enters it.

- **Unsealed run.** The retained attempts answer as today, but a retained
  attempt that is retired answers `RequestOutcomeRetired` and contributes no
  dependency.
- **Sealed run.** A seal extends the tables with the segment's attempts and
  ingress, computes the segment's history flags, rebuilds the
  command-history index, and then moves every archived outcome that is
  retired at its `now`, with that request's queued receipt, into the ranges.
  The invariant: a receipt leaves only with its own request's sealed
  outcome, so no retained attempt ever loses the receipt that marks it
  queued. (Every attempt made in a boundary is admitted by that boundary, so
  an unadmitted tracked attempt is never a queued one.) Between seals,
  outcomes keep crossing the window, and the resolver applies the rule to
  the outcomes still in the table.
- **Continuation checkpoint.** At the settled cut: `commands` lists the
  outcomes at or below the cut that are not retired at `now`;
  `retired_commands` lists, as canonical ranges, the IDs of the retired
  ones, whether the last seal moved them or not; `queued_commands` lists the
  receipt of every queued command request that is not in those ranges.
- **Restored run.** It takes the tables and the ranges as they are and asks
  the same resolver from then on. A restore from a full save (checkpoint
  journal, paged checkpoint, segments) rebuilds the tables from its journals
  and seals with the same rule at the same `now`.
- **Exact replay.** Replay feeds the journaled inputs; retirement changes no
  journaled input, outcome, or state, so replay is unchanged by
  construction, and a replayed run asked the same retry at the same instant
  gives the same answer. A full-history run that never seals keeps every
  attempt in memory as today; only its answers follow the window.

So an unsealed run, a run sealed at any instants, and a continuation-restored
run write byte-identical continuation checkpoints at the same instant, which
the lockstep test and the release continuation gate check.

## Dependencies, kept receipts, and provenance facts

Decision: a retired request contributes no evidence dependency, in a sealed
or an unsealed run.

Today `evidence_dependencies` declares, for every outcome in the table and
every retained tracked attempt, the attempt, the command, and its emitted
events, and for every queued receipt and retained command ingress record,
the ingress record. With this design only requests the resolver does not
retire do; decision ingress keeps its unconditional dependency (D6). The
consequences, each bounded by the window:

- The dependency list and its root in the compact identity
  (`compact_identity_hash`) shrink to the window. A checkpoint journal or a
  paged checkpoint rebuilt from segments recomputes the same list at the same
  `now`, and `ReconstructedCheckpoint::verify` and `compact_identity_hash`
  filter the kept receipts by current reachability, so they still match.
- The kept archive receipts (`archived_evidence_receipts`), which a seal
  keeps only for references some dependency reaches, stop growing with
  history.
- The provenance facts of command ingress records whose request is retired
  are not kept. A provenance read (`plugin_ingress_matches`) of such a sealed
  record then reports `EvidenceContentUnavailable` instead of `false`, as for
  any sealed ingress no one declared; a plugin that reads it declares it as
  an identity-only dependency, as the rule already says. No first-party
  plugin reads the provenance of command ingress. (Known limit: between
  seals the pins of the last seal still hold such a fact, so the read can
  answer `false` in a sealed run and fail in a run restored later; the same
  class as today's kept fact for an admitted packet.)

The same function found each retained accepted attempt's command by a linear
scan of the retained commands, `O(attempts × commands)` per seal and per
checkpoint; it now looks the command up by ID.

## Formats

| Item | 0.17.0 | Implemented | What changes |
| --- | --- | --- | --- |
| `RunConfiguration` | — | — | `request_outcome_retention: SimDuration`, `#[serde(default = ..., skip_serializing_if = ...)]` at 30 days; validated `0..=366 days`; `RUN_CONFIGURATION_FORMAT_VERSION` stays 2, as for `evaluation_limits` |
| `RequestIndex` | `commands`, `queued_commands`, `decisions`, `decision_commands` | adds `retired_commands: Vec<RetiredRequestRange>` | canonical ranges; `commands` holds only outcomes not retired at `state.now`; `queued_commands` holds no ID in a range |
| `CONTINUATION_CHECKPOINT_FORMAT_VERSION` | 2 | 3 | the request index above; hash domain stays `canwu.continuation-checkpoint.v1` |
| Snapshot, commitment, checkpoint-journal, paged envelope formats | 9, 6, 6, 6 | unchanged | no authoritative or journaled shape changes |
| Compact identity | — | — | its dependency root covers fewer dependencies once requests retire; same function of state |

The snapshot shape does not change. Under the same-version policy no earlier
save is supported; if a full save of 0.17.0 shape were loaded, its run
configuration would read the default window and its request tables, rebuilt
from its journals, would only change retry answers.

## Restore validation and tamper

Decision: the continuation restore refuses a request index that is not the
canonical one for its state, before building the runtime, with
`InvalidSnapshot`.

`ContinuationCheckpoint::check_shape` runs `check_retired_requests`:

1. Ranges: `first` nonzero, `first <= last`, strictly increasing and
   non-adjacent (`previous.last + 1 < next.first`).
2. No ID of `commands`, `queued_commands`, or `unadmitted.command_attempts`
   lies in a range. `decision_commands` is not checked: a retired nested
   command ID legitimately sits there and in a range.
3. Every `commands` entry carries its own request ID in the outcome, names
   an attempt at or below the cut, and settled inside the retention at
   `state.now` (`kept_from(now, retention) <= at <= now`, with the retention
   read from `state.run_configuration`). The rule covers settled outcomes
   only; unadmitted attempts are carried in full.
4. Every pending ingress record that carries a command request keeps it
   listed (a queued command's receipt in `queued_commands`, a decision's
   nested command in `decision_commands`), and none of those IDs lies in a
   range. Without this, a forged range over a pending decision's nested
   command would make its admission record a persisted decision rejection,
   and a forged range over a pending queued command would make its boundary
   admit it with no attempt.

The round trip that follows the restore (`verify_continuation_round_trip`)
already catches a live outcome past the window and a receipt inside a range;
it does not catch non-canonical ranges when nothing new retires, because the
builder then writes the ranges back as they came. Rule 1 is the only guard
for those, and rules 2 and 4 cover what the round trip cannot see. The continuation hash covers the whole
index, so any other alteration fails the hash. What no check can catch
without history is a dropped range (a retired ID made free) or a forged
range over IDs never used, or a live outcome moved into a range; only the
host's stored continuation hash, the trust anchor, covers those, as it covers
a forged pinned payload today.

## Cardinality and cost

| Quantity | Today | Implemented |
| --- | --- | --- |
| Outcomes and receipts kept | every tracked request ever settled | requests settled in the last window, plus pending ones |
| World profile, 30-day window | +8,400 a month (3.5 MB) | about 8,400 at a month-end save |
| Retired IDs, counter or revision-derived IDs | — | one range per run of consecutive IDs (two a month in the profile, whose IDs follow the revision) |
| Retired IDs, hashed or random IDs | — | one range per retired request, about 59 bytes of JSON for a 20-digit ID: 0.5 MB a month at world scale, 6 MB a simulated year, 0.6 GB a century (against 438 bytes each today) |
| Dependencies and kept receipts from requests | +25,200 a month at world scale | about one window's |
| Provenance facts from requests | +8,400 a month | about one window's |
| Lookup | `O(log n)` | `O(log n + log ranges)` |
| Seal | extends the tables | extends, then rebuilds the outcome and receipt tables without the retired entries, `O(window)`, and on a seal that retires something copies the ranges, `O(ranges)` (a few for consecutive IDs, one per retired request for hashed IDs) |
| Checkpoint build and restore | `O(history)` request rows | `O(window + ranges)` |

So the request index is flat for a host whose request IDs are consecutive
and bounded per window otherwise, but a host that hashes keys into request
IDs still grows, at about 59 bytes per request: smaller by seven times, not
bounded. The guidance is in [What a host must know](#what-a-host-must-know).

`evidence_times` (one run per distinct instant per journal, about 2.5 KB a
simulated month at smoke) and the fiscal records' own growth are outside the
request index and stay as they are; both are small and stated in the gate.

## What can still fail a boundary

Nothing new. `RequestOutcomeRetired` is returned to the host by
`process_command`, `enqueue_command`, and `enqueue_decision`; inside a
boundary it is unreachable, and an `Ok` rejection if reached. No new error
leaves a boundary failed.

## Public API delta

- `canwu-sim`, re-exported by `canwu-api`:
  - `ErrorCode::RequestOutcomeRetired` (`"request_outcome_retired"`).
  - `RunConfiguration::request_outcome_retention`,
    `RunConfiguration::with_request_outcome_retention`,
    `RunConfigurationSnapshot::request_outcome_retention()`,
    `DEFAULT_REQUEST_OUTCOME_RETENTION`, `MAX_REQUEST_OUTCOME_RETENTION`.
  - `RetiredRequestRange { first, last }` and
    `RequestIndex::retired_commands`.
  - `CONTINUATION_CHECKPOINT_FORMAT_VERSION` = 3.
- No new `&mut self` method on `Canwu`, so the continuation gate's shadow is
  untouched. A host reads the window with
  `run_configuration().request_outcome_retention()`.

## What a host must know

For the release entry and the engine-user skill (which has no text on
request IDs or retries today, so this is a new paragraph):

1. **Late retries.** An exact retry is answered with its original outcome or
   receipt while its attempt is inside the run's request outcome retention
   (default 30 simulated days); after that it is refused as
   `RequestOutcomeRetired`, as is any request that reuses the ID.
2. **Branch rule.** Every host branch that treats `IdempotencyConflict` as
   "this ID is already used, take another" must accept
   `RequestOutcomeRetired` the same way.
3. **Simulated time.** A wall-clock pause never retires anything; a high
   simulation speed retires sooner in wall time. Choose the retention from
   the highest speed and the longest client retry or reconnect delay, and set
   it when the run is created; it cannot be raised later.
4. **Remote clients.** A host whose clients may be away longer than the
   window keeps the outcomes it received, keyed by request ID, and answers
   those clients itself.
5. **Request IDs.** Allocate them so that they are consecutive: from a
   counter the run's state restores, or derived from the revision at which
   the host builds a batch (as the fiscal profile does: revision plus offset
   plus one), which a restore reproduces. Then retired IDs collapse into a
   few ranges. IDs hashed from keys cost about 59 bytes each for the life of
   the run.
6. **Archived evidence of retired requests.** After the next seal,
   `archived_evidence_receipt` and `load_archived_evidence_segment` no longer
   find a retired request's attempt, command, events, or ingress through the
   kept receipts, since no dependency reaches them; the host's own stored
   segments still hold them.
7. **Plugins and command ingress.** A plugin that reads the provenance of a
   command ingress record (`plugin_ingress_matches`) declares it as an
   identity-only dependency, as the evidence rule already requires; an
   undeclared read answers `false` while the record is retained and
   `EvidenceContentUnavailable` once a seal has removed it, which a sealed
   run and its unsealed twin would decide differently.
8. **Recovering a counter after a restore.** The next free ID is one more
   than the largest of `requests.retired_commands` (`last`),
   `requests.commands`, `requests.queued_commands`, `requests.decision_commands`,
   and the request IDs of `unadmitted.command_attempts` and of pending
   ingress in the checkpoint just restored. An exact retry is itself the
   safe status query: within the window it returns the outcome, after it
   `RequestOutcomeRetired`, and for a request the run never received it
   submits it, which is what the retry wanted.

## Terminology candidates

For the owner's term sheet; English first, Chinese proposed:

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| request outcome retention | `request_outcome_retention` | 请求结果保留期 | How long, in simulated time after its attempt, a tracked command request's exact outcome is kept to answer a retry. |
| retired request | `RequestOutcomeRetired`, `RetiredRequestRange` | 结果已退役的请求 (or 已退役请求标识) | A tracked command request past the retention: its outcome is no longer kept, its ID stays used for the run, and every later request with it is refused. Not 退休目标 (a culture term) nor 已清退条目 (knowledge). |

## Documentation delta

For the release documentation pass (canonical page and both website
languages each):

- `docs/architecture.md` and the website events/settlement pages: "Exact
  retries return the original outcome" becomes "within the run's request
  outcome retention; later, a used ID is refused as
  `RequestOutcomeRetired`"; the request-commitments paragraph names the
  retired ranges.
- `docs/proposals/continuation-checkpoint.md`, "Known limits": the request
  index no longer grows for ever for consecutive IDs; the keyed-draw ledger
  still does.
- `docs/versioning.md`: the release entry with the late-retry change in one
  sentence (old and new answer), the branch rule, the format row, and the
  known limits (hashed IDs, decision tables, `evidence_times`, the
  provenance read between seals).
- `docs/terminology.md` and both website terminology pages: the two rows
  above.
- `benchmarks/README.md`: the held-knowledge paragraph and the checkpoint
  row ([Profile results](#profile-results)).
- The engine-user skill (`agent-interface`, `canwu-engine-usage`): the
  points of [What a host must know](#what-a-host-must-know); the plugin rule
  of point 7 also goes into the contributor and developer skills.

These are left for the release documentation pass, as the 0.17.0 packages
left them for its release commit; until then they are an open item.

## Acceptance tests

Committed, each shown to fail under a plausible regression (mutations run
inline, not committed):

1. **`request_outcomes_retire_by_simulated_time_alike_in_every_run`**
   (`canwu-sim`, `tests/persistence_contracts.rs`): a declared run with a
   one-day retention; an accepted, a stale, and a zero-ID direct request
   (the last two recorded rejections). Inside the window an unsealed and a
   sealed run answer exact retries with the original outcomes and a reused
   ID with `IdempotencyConflict`, and write the same checkpoint; past it both
   answer `RequestOutcomeRetired` for all four without a new revision, before
   and after the seal that moves them, as does a run restored from the
   continuation checkpoint, which keeps no outcome and one range `0..=2`; the
   seal leaves the outcome table empty; the compact checkpoint restores from
   its segments; a fresh ID is admitted alike in every run. Fails if the seal
   stops moving outcomes, if archived lookups ignore the window, if retained
   attempts keep citing dependencies, if the builder stops splitting
   outcomes, or if the restore refuses a zero ID in a range.
2. **`a_continuation_restored_run_continues_in_lockstep`** (`canwu-api`,
   extended): over 50 days with the default 30-day window, every run
   (unsealed, sealed every five days, a full-save restore, and two
   continuation restores) answers an exact re-enqueue of every past dispatch
   with its receipt inside the window and `RequestOutcomeRetired` after it,
   refuses a retired ID with new content and a decision that nests it,
   changes no hash doing so, and writes byte-identical checkpoints whose one
   range names exactly the retired dispatches (consecutive IDs, so the range
   grows across seals and restores). Fails if lookups ignore the window, if
   retained command ingress keeps citing a retired request, if the builder
   stops splitting outcomes, or if `enqueue_decision` asks about use before
   retirement.
3. **`a_tampered_continuation_checkpoint_is_refused`** (extended): with a
   recomputed hash, an empty, unsorted, or adjacent range, a kept outcome
   inside a range, an outcome outside the retention, and a pending queued
   command whose receipt is dropped or whose ID is retired are each refused
   with `InvalidSnapshot`. Fails without restore rule 4.

Checked inline and not committed: the retention validation (negative and
367 days refused, 366 and zero accepted, the default omitted from the wire,
a non-default value round-tripping), and that the existing idempotency tests
(`typed_ingress_is_idempotent_revision_guarded_and_replayable`,
`compacted_live_journals_preserve_continuation_idempotency_and_exact_replay`)
pass unchanged, their retries falling inside the window.

## Measurement plan and gate

The `fiscal-world-profile` held-knowledge row, world preset, institution
courts, interval cadence, `--held-cycles 24`, after the change, against the
0.17.0 row recorded in `benchmarks/README.md`:

- **Gate.** The continuation checkpoint after month 24 is within 2 MB of its
  size after month 5 (0.17.0: +72 MB), its request index holds at most one
  window of outcomes, and its retired ranges are at most two per month.
- Restore time of the month-24 checkpoint (0.17.0: 58 s) and month wall time;
  the month time must show no upward trend from month 5 to month 24 beyond
  what the fiscal records' own growth explains (see [E-29](#e-29)).

## Profile results

The gate is met. `fiscal-world-profile`, world preset, institution courts,
interval cadence, `--held-cycles 24`, on the implementation branch, against
the 0.17.0 row of `benchmarks/README.md` (sizes are deterministic):

| After month | 0.17.0 checkpoint | Implemented |
| --- | ---: | ---: |
| 1 | 172 MB | 171.6 MB |
| 2 | | 188.5 MB |
| 4 | | 204.8 MB |
| 5 | | 205.45 MB |
| 10 | | 205.93 MB |
| 15 | | 205.99 MB |
| 20 | | 206.17 MB |
| 24 | 293 MB | 206.30 MB |

- From month 5 to month 24 the checkpoint grows by 0.85 MB in all (0.17.0:
  about 72 MB). The request index at month 24 holds 8,400 outcomes
  (2.25 MB) and 8,400 queued receipts (1.43 MB), the window of one month,
  and 46 retired ranges (1.4 KB), two a month; its 72 KB of growth since
  month 5 is longer numbers, not more rows. The rest of the growth is outside
  the request index: fiscal domain records 414 KB (each scope's ledger grows
  by about two bytes a month until month 10), knowledge 125 KB, current and
  known record versions 178 KB, `evidence_times` 50 KB, provenance facts
  8 KB.
- Months 1 to 5 grow by 34 MB as the fiscal records and the held knowledge
  fill, as in 0.17.0.
- Restoring the month-24 checkpoint took 75.9 s, and the month wall time
  ranged from 95 s to 274 s with no usable trend: sibling sessions ran cargo
  builds and test suites on the machine for the whole run (one test build
  ran out of memory), so neither is comparable with the quiet-machine
  0.17.0 figures (58 s; 73 s to 129 s).
- The E-29 evidence is therefore the deterministic counts. On the smoke
  preset over 12 months, the compact checkpoint's dependencies and kept
  archive receipts stay at 8,039 from month 1 to month 12; in 0.17.0 they
  grew by 2,520 a month (three per tracked command). Inside the window the
  request dependencies coincide with those the fiscal ledgers already
  declare for their recent outcomes, so the request index adds none.

## E-29

Decision: the request index causes part of the monthly creep, which D1
bounds; the seal's large fixed cost is a separate finding.

A breakdown of the world-scale seal (0.17.0, ms, month 1 and month 3; month
2 was disturbed by other load):

| Step of `seal_evidence` | Month 1 | Month 3 |
| --- | ---: | ---: |
| fork before the seal (copies the retained tail) | 1,210 | 1,809 |
| `evidence_dependencies` (dependencies) | 486 (135,139) | 524 (185,539) |
| continuation needs and pins | 173 | 249 |
| segment copy | 1,022 | 1,336 |
| archive index of the segment (records) | 6,347 (472,036) | 7,873 (476,628) |
| verify the segment just built | 11,767 | 14,401 |
| apply tables and drain journals | 597 | 711 |
| insert and retain archive receipts (kept) | 1,311 (131,407) | 1,885 (185,539) |
| the whole seal as the profile sees it, with drops | 27,629 | 40,178 |

The dependencies and kept receipts grow by 25,200 a month, three per tracked
command, all from the request index; rebuilding the receipt map and the
dependency list costs about 0.3 s more each month. With D1 both stay at one
window's size. The guess in `opus-progress.md` (the checkpoint builder
rebuilding the request maps, `continuation.rs` ~758) is real but outside the
month's time; D1 bounds it too.

Most of the seal, about 30 s at world scale, is proportional to the sealed
segment and does not grow with history: the archive index hashes every
sealed record, the verification hashes them all again, and the fork before
the seal copies the retained tail. It is a player-lag concern for a host
that seals on the simulation thread, and it is recorded as a new finding
(skip re-verifying a segment the engine just built; avoid the full copy
before the seal), not part of this design. Two smaller costs in today's
code are also outside the request index: `enqueue_command` scans every
retained and pinned ingress record for an exact re-enqueue (`O(retained
ingress)` per call), and the dependency scan this change replaced.

## Non-goals, deferred work, and known limits

- **Decision requests.** `RequestIndex::decisions` and `decision_commands`
  keep every decision request ID and receipt (about 200 bytes a decision).
  The decision ledger keeps its own compact archive receipt per archived
  attempt, so bounding decision history is its own design; the world profile
  has no decisions.
- **Hashed request IDs** stay unbounded at about 59 bytes each
  ([Cardinality and cost](#cardinality-and-cost)). A packed encoding of
  single-ID ranges (a varint or bitmap form) would cut that to about 8 bytes
  each; deferred until a host keeps hashed IDs at world scale.
- **A read-only request status** (`command_request_status(id)`): deferred.
  An exact retry is the status query a host needs, and a host recovering a
  counter reads the index of the checkpoint it restored.
- **The keyed-draw ledger** keeps one entry per drawn address for the life of
  the run (Part C of the continuation design); out of scope.
- **`evidence_times`** grows with distinct instants (2.5 KB a month at smoke).
- **The seal's segment cost** ([E-29](#e-29)).
- **The provenance read between seals** ([Dependencies](#dependencies-kept-receipts-and-provenance-facts)).
- **Archiving retired outcomes** for later lookup: a host that needs old
  outcomes keeps them from the receipts it received or from its sealed
  segments, which hold every attempt.

## Alternatives rejected

- **Expire by the request's own fence and allow reuse after the window.**
  Refuse any request whose `expected_revision` or `expected_time` is older
  than the window, keep nothing for retired requests, and admit a reused ID
  as new. Zero storage, but a request ID would no longer be unique in a
  run's history, which the attempt index, the decision-command uniqueness
  check, and snapshot validation all assume; and a host that derives IDs from
  keys and rebuilds a request with the current revision would have the same
  logical action admitted twice.
- **A floor instead of ranges.** Require request IDs to increase and keep
  the largest retired one. One number, but it refuses fresh IDs below the
  floor, which today's hosts may use.
- **A probabilistic filter of retired IDs.** Not exact.
- **A revision or count window.** Hard size bound, but the retry window
  would shrink as the world gets busier; the time window is stated instead.
- **Retire at a seal.** Simplest to write, but a sealed and an unsealed run
  would answer the same retry differently and write different checkpoints.
- **Answer a retired ID with `IdempotencyConflict`.** No host branch would
  change, but the code says "different input", which the engine no longer
  knows; the distinct code with the branch rule in the release entry was
  preferred.

## Open decisions

- **The default and the cap.** 30 days and 366 days are implemented; is an
  unbounded setting wanted for small runs that want today's behaviour?
- **The terms** ([Terminology candidates](#terminology-candidates)).
- **Hashed IDs.** Keep the stated cost, or pack single-ID ranges now?

## Review record

The first version was reviewed by an engine reviewer ("implement with
changes": six changes, six notes, no blocking finding) and a host-side
reviewer ("ready with changes": two blocking findings, six changes, five
notes). Each finding is accepted, rejected with a reason, or deferred, and
the section that resolves it is named.

| Finding | Resolution |
| --- | --- |
| Engine C1 two retirement rules (pure time vs "answered from its record" above the cut) | Accepted: the pure time rule everywhere; ranges are only a representation of attempts at or below the cut; rule 3 of the restore checks covers settled outcomes only ([Retired request IDs](#retired-request-ids), [Restore validation](#restore-validation-and-tamper)). |
| Engine C2 `enqueue_command` reader order breaks lockstep | Accepted: one resolver asked first by every reader ([One rule, one resolver](#one-rule-one-resolver)); the lockstep test retries every past dispatch in every run. |
| Engine C3 tamper claim wrong; pending decision and pending command checks missing | Accepted: restore rule 4; the tamper paragraph says what only the anchor covers ([Restore validation](#restore-validation-and-tamper)). |
| Engine C4 "nothing written by 0.17.0 is read" false for full saves | Accepted: reworded under [Formats](#formats); no snapshot format change. |
| Engine C5 seal order as an invariant; perform the move after the index rebuild | Accepted: the move runs after `seal_command_history`; the invariant is stated; the unadmitted-receipt rationale removed ([Sealed, unsealed](#sealed-unsealed-restored-and-replayed-runs)). |
| Engine C6 retained-ingress dependency loop needs the rule | Accepted and implemented ([Dependencies](#dependencies-kept-receipts-and-provenance-facts)); test 2 fails without it. |
| Engine N7 replay readers missing | Accepted: listed with the argument. |
| Engine N8 provenance read between seals | Accepted as a known limit. |
| Engine N9 decision commands sit in a range too | Accepted: rule 2 excludes `decision_commands`; `command_request_id_is_in_use` may answer from either. |
| Engine N10 cap and opt-out | Kept 366 with its reason; the unbounded option is an open decision. |
| Engine N11 linear scans in `evidence_dependencies` and `enqueue_command` | The first fixed (lookup by ID); the second recorded under [E-29](#e-29). |
| Engine N12 verified claims | Noted. |
| Host B1 a new code splits hosts' `IdempotencyConflict` branch | Accepted in part: the branch rule goes into the release entry and the skill, and the two variants cross-reference each other in rustdoc; reusing `IdempotencyConflict` was rejected (it would claim different input the engine no longer knows) ([Alternatives](#alternatives-rejected)). |
| Host B2 guidance and gate ignore hashed IDs | Accepted: the cost per hashed ID, per year, and per century stated; guidance for consecutive IDs (counter or revision-derived); packing single ranges deferred ([Cardinality](#cardinality-and-cost), [What a host must know](#what-a-host-must-know)). |
| Host C1 the window is not about save cadence | Accepted: rewritten in terms of lost-answer recovery, speed, and client delay ([The retention window](#the-retention-window)). |
| Host C2 configurable, fixed at creation | Accepted: kept configurable; fixed at creation and why. |
| Host C3 counter recovery; a status read | Recovery recipe added; the read deferred with its reason ([What a host must know](#what-a-host-must-know)). |
| Host C4 name `RequestRetired` reads as "withdrawn" | Accepted: renamed `RequestOutcomeRetired`; the message names the ID, the retention, and that a new ID is needed; Chinese candidates revised. |
| Host C5 decision-nested path answered `IdempotencyConflict` | Accepted: `enqueue_decision` answers `RequestOutcomeRetired`; the boundary-generated path keeps its generic guard error, stated in the table. |
| Host C6 documentation the host needs | Accepted: [What a host must know](#what-a-host-must-know) and [Documentation delta](#documentation-delta). |
| Host N1 to N5 | Accepted: the restore-from-older-save case, the full-history memory note, and the zero-retention consequence are stated; N4 and N5 confirmed the figures and today's API. |

The implementation was then reviewed by an independent code reviewer
("commit after fixes": one blocking finding, four should-fix, eight nits):

| Finding | Resolution |
| --- | --- |
| Code B1 a rejected zero request ID retires into a range the restore refused | Accepted: zero is a valid range member; test 1 covers it. Separate finding recorded: `enqueue_command` accepts request ID 0 while full-snapshot validation refuses queued command ingress with ID 0. |
| Code S1 redundant closure (pedantic clippy) | Accepted. |
| Code S2 rule 4, the decision path, and the range merge untested | Accepted: tests 2 and 3 extended; dispatch IDs made consecutive. |
| Code S3 archived receipts of retired requests; the plugin provenance rule | Accepted: points 6 and 7 of [What a host must know](#what-a-host-must-know). |
| Code S4 canonical docs not updated | Deferred to the release documentation pass, as for 0.17.0, and tracked as an open item ([Documentation delta](#documentation-delta)). |
| Code N1 the round trip does not canonicalize ranges | Accepted: the restore section corrected. |
| Code N2 "the only place the rule is evaluated" | Accepted: reworded. |
| Code N3 to N6 rustdoc and check order | Accepted: the field, `IdempotencyConflict`, and retention docs reworded; the restore check reads the ranges as a slice after the canonical check. |
| Code N7 the seal copies the ranges | Accepted: in the cost table. |
| Code N8 test helper and a stray edit | Accepted: `Option<ErrorCode>`; the unrelated edit reverted. |
