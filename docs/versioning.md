# Versioning and Persistence

Canwu is pre-1.0. Format 8 is a deliberate clean break: the 0.13 runtime
writes and reads only its current contracts. There is no implicit loader or
runtime migration for pre-8 data. Applications that need old
records must keep the old engine or run an explicit, application-owned export
outside the Canwu runtime.

## Current contract

The workspace version is `0.21.0`. A live `SimulationSnapshot` has:

- snapshot format `9`;
- commitment format `8`;
- checkpoint/evidence-journal format `6`;
- paged-checkpoint envelope format `7` and continuation checkpoint format `4`;
- state revision format `3`;
- admission cursor format `3`;
- exact replay revision format `3`;
- a declared `RunManifest`, a declared or `CompatibilityV1` run
  configuration, and canonical `initial_scenario`;
- a non-zero `authority_root_seed`, derived independently from the simulation
  random streams.

Typed loading and strict JSON loading reject any other engine or contract
version. Strict JSON loading also rejects unknown fields at every nested
object and rejects a wire value whose canonical re-encoding changes shape.

Version 0.21.0 is a pre-1.0 minor release. Its changes come from the
packages that close every downstream gap filed against 0.20.0, each with its
own proposal:

- [decision ticket option bias](proposals/decision-ticket-option-bias.md)
  (`canwu-decision`, `canwu-sim`: downstream gap G-84);
- [movement executions keep only the bookings that bear on their legs](proposals/movement-kept-bookings.md)
  (`canwu-movement`: downstream gap G-83);
- [fiscal archive after a continuation restore](proposals/fiscal-archive-after-continuation-restore.md)
  (`canwu-sim`, `canwu-fiscal`: downstream gap G-91);
- [military engagement revisions](proposals/military-engagement-revisions.md)
  (`canwu-military`: downstream gaps G-85 to G-90).

The release's continuation gate also found engine finding E-55, present since
0.20.0 in the unpublished `canwu-force-supply-reference`, fixed in this release
(below).

What changed, in one list:

- **Ticket option bias.** A `DecisionOption` carries `bias: i64`, written by
  the ticket's drafter at `Open` or `ReplaceOptions`
  (`DecisionOption::with_bias`), zero and omitted when the option has none.
  `WeightedUtilityEvaluator` adds it to the policy's own bias for the option
  (`with_option_bias`) and records the sum, unweighted, as
  `DecisionOptionEvaluation::bias` beside the factor lines;
  `WeightedUtilityPolicy` and `GuardedUtilityPolicy` score through it, in the
  guarded policy before the near-equivalence margin. The ticket's bias is
  ticket content, not policy configuration: the guarded policy's semantic
  hash is unchanged, so one controller serves every ticket of a decision
  maker whatever the tickets' biases. The decision commitment is
  `canwu.commitment.decisions.v4`, `COMMITMENT_FORMAT_VERSION` is 8, and
  `DECISION_ARCHIVE_FORMAT_VERSION` is 2.
- **Kept bookings.** A transport execution keeps only the bookings that
  still bear on its legs: requested or confirmed, consumed by the leg under
  way, or lost (failed, expired, or revoked) for a leg of the active revision
  that has not departed. Every other booking is closed and leaves the
  execution, and its binding the runtime, in the step that closed it (the
  replacing `RequestBooking`, the arrival or failure, the release,
  cancellation, reroute, or close); a consumed booking's quantity moves to
  `MovementState::retired_consumption`, so the pool's `consumed` stays
  counted. `MovementLimitsV1::CURRENT.bookings_per_execution` bounds what an
  execution keeps at once and is raised from 16 to 64; a new
  `bookings_per_runtime` (16,384) bounds the runtime's kept bookings.
  `MOVEMENT_SEMANTIC_HASH` changes; no format number does.
- **A continuation restore is not a seal.** `sealed_evidence_cursor()` (on
  `Simulation`, `CompactedSimulation`, `Canwu`, `CompactedCanwu`, and
  `CanwuRun`) answers where the run's own last seal ended and is `None` until
  a run seals after it is created or restored; in 0.20.0 a run restored from
  a continuation checkpoint answered the checkpoint's settled cut. So
  `prepare_fiscal_archive` no longer refuses, as
  `FiscalArchiveError::UnsealedCitations`, a book citing a version
  established after a continuation restore on a run that has not sealed
  since; a run that has sealed is refused past its own last seal as before.
  `canwu-fiscal` changes no code, only its rustdoc and README.
- **The military report queue.** `canwu-military` queues
  a report item only for an engagement, siege, or crossing whose sides name a
  holder, so a host that names no report holder writes the combat registry
  at most once in a stepping boundary (0.20.0 wrote it twice: the queue
  filled in phase 7 and drained in phase 13).
- **Engagement time limit.**
  `OpenEngagementV1::max_minutes: Option<u32>` opens a fight shorter than its
  kind's rule (`None` takes the kind's; a value must be from 1 to the kind
  rule's, else a recorded `ValueOutOfRange` rejection);
  `EngagementStateV1::max_minutes: u32` holds the duration resolved at the
  opening, and the step reads it. A fight the plugin opens (an assault, a
  sortie, a ferry or bridgehead fight) takes its kind's.
- **Siege join and release.**
  `MilitaryCommandV1::JoinSiege { siege, expected_host_revision, side, forces }`
  adds free forces at the site to a siege's besiegers or defenders, as
  `JoinEngagement` does for a fight; while the siege's assault or sortie is
  in progress the joining forces also enter that fight on the matching side.
  A siege's two sides have different owners: `JoinSiege` refuses a force of
  the opposite side's owner, and `InvestSite` now refuses besiegers and
  defenders of one owner. `ReviseSiegeV1::release: Vec<ForceId>`
  takes forces off either side, free to be placed in the same boundary
  (`PlaceForce` after the release in one admission order); a release that
  would leave no besieger is refused (`LiftSiege` ends a siege), and one that
  leaves no defender lets the next siege day find the place `Abandoned` with
  nobody dispersed. Every change to a siege's sides is a siege command naming
  the siege's `expected_host_revision`.
- **Revised crossing defenders.**
  `MilitaryCommandV1::ReviseCrossing` gains `defenders: Option<Vec<ForceId>>`,
  which replaces the defending list under the checks `OrderCrossing` applies
  (within `forces_per_side`, none named twice, not the crossing force, each
  registered, active, and at the far bank), from a trusted issuer or the
  crossing force's controller, who may name any list, or else from an issuer
  who controls a force on the old or new list and every registered force the
  revision removes; the next landed wave's bridgehead fight takes the free defenders from
  the revised list.
- **The timed step.** A public plugin ingress,
  `military_timed_step_v1` (`MILITARY_TIMED_STEP_INGRESS`, class
  `ScheduledSystem`, payload `Null`), that the host queues on its run handle
  due at a named time with `enqueue_plugin_ingress(timed_step_ingress(due_at))`;
  the boundary at that instant admits it with the host's other ingress, and
  the settle system treats it as a step packet with `up_to` at the boundary's
  time, so a host that steps once a day adds no boundary of its own. The wake
  system schedules nothing in a boundary that admitted a step packet of
  either kind (its contract reads `core_ingress`), so the default `Daily`
  wake cadence may stay. The host may withdraw a pending timed step with
  `cancel_plugin_ingress`. `SCHEMA_VERSION` is 2 and the plugin descriptor
  version `0.21.0`.
- **The force-supply reference keeps the force versions it cites (engine
  finding E-55).** The reference root cited earlier versions of itself in
  its completion state (an acquisition's evidence, a local grant's target,
  an activation certificate's target) but declared none, so the first seal
  after a write superseded such a version dropped it, and a later
  first-time declaration of it by the resource's hot terminal bucket was
  refused at the next seal (`ArchiveNotReady`). The root now declares those
  versions identity-only while a live acquisition, grant, or certificate
  cites them (released, rejected, and expired ones excluded), and an
  acquisition or grant that cites a force version the root has not written
  is a recorded rejection. `canwu-force-supply-reference` is not published;
  its root format moves 2 → 3 (a required
  `canwu_identity_evidence_dependencies`), and its runtime record schema and
  `FORCE_SUPPLY_SEMANTIC_HASH` change.

**Same version only.** Canwu crates depend only on the exact same version of
each other: every first-party crate requires `=0.21.0` of every other
first-party crate it uses, and a host pins `canwu-api = "=0.21.0"` and the
same version of every extension. Nothing written by 0.20.0 is read by
0.21.0. There is no migration and no decoding of an old shape: the exact
engine-version check refuses every 0.20.0 snapshot, checkpoint journal, paged
checkpoint, continuation checkpoint, replay journal, evidence segment, and
scenario that records its engine version; a commitment at format 7, a
decision archive blob or receipt at format 1, and a military payload at
schema version 1 are refused by their format as well; a run with the movement
or military plugin would also be refused by its semantic hash, and a movement
runtime that keeps a closed booking by `MovementState::validate`. A scenario
built with 0.20.0 types is rebuilt with the 0.21.0 types: a `DecisionOption`
struct literal lists `bias`, a `MovementLimitsV1` literal
`bookings_per_runtime`, and the military payloads their new fields. Retain
the 0.20.0 engine to read 0.20.0 data.

Formats and schemas that change, each deliberately versioned:

| Item | 0.20.0 | 0.21.0 | What changed |
| --- | --- | --- | --- |
| Snapshot, commitment, checkpoint/evidence-journal, paged-checkpoint, and continuation-checkpoint formats | 9, 7, 6, 7, 4 | 9, 8, 6, 7, 4 | commitment 8 commits to decision state through `canwu.commitment.decisions.v4`; the other four keep their shape; the engine version in every checkpoint hash changes |
| Decision commitment | `canwu.commitment.decisions.v3` | `canwu.commitment.decisions.v4` | the same inputs (the controllers, the hot-history root, the archive receipt root, the admission batch); the new tag marks that the committed tickets carry option biases; a format 7 snapshot, checkpoint, or replay journal is refused by its format as well as its engine version |
| `DecisionOption` (in `DecisionTicketDraft`, `ReplaceOptions`, and `DecisionTicket`: decision ingress, replay journal, snapshot and checkpoint decision state, continuation checkpoint, hot history, archive blobs) | — | — | `bias: i64`, omitted when zero; strict loading refuses an explicit `"bias": 0` as a non-canonical encoding |
| `DecisionOptionEvaluation::bias` (in `PolicyDecision` and `DecisionTrace`) | the evaluator's bias | the evaluator's bias plus the option's | the same field and encoding, omitted when the sum is zero |
| `DECISION_ARCHIVE_FORMAT_VERSION` (archive blob, receipt, verified archive commit) | 1 | 2 | an archived ticket's options carry their bias; a blob records no engine version, so its format alone names its shape, and a format 1 blob or receipt is refused |
| `GuardedUtilityPolicy` semantic hash; the hot-history leaf tag `canwu.decision.hot-history-leaf.v2`; the decision ingress commitment `canwu.decision.ingress-request.v1`; the decision archive bucket page format (1) | — | — | unchanged: ticket biases are not part of the policy hash, and the two tags hash the canonical encoding of what they cover, so a biased ticket already hashes apart from an unbiased one |
| `MovementState` (runtime record payload) | — | — | shape unchanged; `validate` refuses an execution that keeps a closed booking, a binding's `replaces` naming a kept booking or named by two kept bindings, and more than 64 kept bookings in an execution or 16,384 in the runtime |
| `MovementBookingBindingV1::replaces` | names an earlier booking of its leg still in the runtime | names a booking that has left the runtime | — |
| `MovementState::retired_consumption` | the consumption of retired executions' bookings | the consumption of every booking that has left the runtime | — |
| `TransportExecution::bookings` (in the runtime) | every booking of the execution's life | the kept bookings, in request order | — |
| `MovementReportDetailV1::bookings` (report payload) | every booking of the execution | the kept bookings | payload shape and the `movement_report` schema hash (`33d6f3ab…ed28`) unchanged |
| `MovementLimitsV1` (not persisted) | `bookings_per_execution` 16 | `bookings_per_execution` 64, `bookings_per_runtime` 16,384 | the per-execution bound counts what is kept, not what was booked over the execution's life; the new field is the runtime-wide bound |
| `MOVEMENT_SEMANTIC_HASH` | `b211c92a…abbc` | `e01d702b…826f` | BLAKE3 of the string recorded in the proposal; the snapshot, continuation, and journal formats are unchanged |
| `sealed_evidence_cursor` (a derived read; nothing persisted) | the end of the last seal, or the settled cut of the continuation checkpoint the run was restored from | the end of the run's own last seal; `None` until a run seals after it is created or restored | derived from the segment manifest and the archived cursor the runtime already holds; no fiscal, snapshot, checkpoint, record, or plugin format changes |
| `canwu-military` payloads (`SCHEMA_VERSION`) | 1 | 2 | the engagement record changed shape; the plugin keeps one schema version for all its payloads, so no 0.20.0 military record loads |
| `canwu.military:engagement` | — | `max_minutes: u32` | activation refuses an active engagement whose `max_minutes` is 0 or above its kind rule's, or whose kind has no rule |
| `canwu.military:siege`, `canwu.military:crossing` | — | — | shapes unchanged; activation also checks the side lists and the crossing's defenders against `forces_per_side`, since commands now grow them |
| `canwu.military:registry` | — | — | shape unchanged; the report queue holds items only for subjects with a holder |
| `MilitaryCommandV1` (the `military_command_v1` and `military_apply_v1` payloads in the journals) | 17 variants | 18 variants | adds `JoinSiege { siege, expected_host_revision, side, forces }`; `OpenEngagementV1` gains `max_minutes`, `ReviseSiegeV1` gains `release`, `ReviseCrossing` gains `defenders` |
| `canwu-military` ingress | the command action `military_command_v1`; internal `military_apply_v1` and `military_step_v1` | the same, and the public ingress `military_timed_step_v1` (class `ScheduledSystem`, payload `Null`) | an ordinary host ingress record: the snapshot, the journals, the replay journal, and a continuation checkpoint's pinned ingress carry it while it is pending, and exact replay re-queues it |
| `canwu-military` plugin descriptor | version `0.20.0` | version `0.21.0`; semantic hash BLAKE3 of `canwu-military plugin semantic hash 0.21.0: engagements <resolver name>@<resolver version>` | the public ingress and the wake system's `core_ingress` read are descriptor-recorded; the `engagement_report` and `engagement_outcome` knowledge schemas are unchanged (v1, same bodies and hashes) |

Semantic identities that differ from 0.20.0: `canwu-movement`,
`canwu-military`, and the unpublished `canwu-force-supply-reference`. The other first-party plugins keep their semantic hash
constants; `canwu-transport`'s semantic version, the `movement_report` schema
hash, and every `GuardedUtilityPolicy` hash are unchanged. As with any
version change, the engine version in every checkpoint hash changes and
first-party plugins that take their plugin version from the crate version
report a new one.

New and changed public contracts, by crate:

- `canwu-decision`: `DecisionOption::bias` (new field; a struct literal that
  lists every field sets it, one built with `..DecisionOption::new(..)` needs
  nothing) and `DecisionOption::with_bias` (new builder);
  `DECISION_ARCHIVE_FORMAT_VERSION` is 2; the rustdoc of
  `WeightedUtilityEvaluator`, `with_option_bias`, `GuardedUtilityPolicy`, and
  `DecisionOptionEvaluation::bias` states the sum. Behaviour:
  `WeightedUtilityEvaluator::evaluate` adds the option's bias to its own for
  the option (checked `i64`; an overflow of the sum or of the score is
  `DecisionErrorCode::InvalidDecision` to the caller, as a factor overflow
  is, returned by `prepare_decision` and `drive_decision` before any ingress
  is queued); blocked and guard-excluded options are not scored and record no
  bias. No refusal is new: `Open` and `ReplaceOptions` admit any bias, and a
  `ReplaceOptions` that changes a bias advances the ticket version, so a
  response computed before it is stale.
- `canwu-sim`: `COMMITMENT_FORMAT_VERSION` is 8.
  `Simulation::sealed_evidence_cursor` and
  `CompactedSimulation::sealed_evidence_cursor` keep their signatures and
  answer as above; the rustdoc of `continuation_base` and
  `journal_segment_since` says that a continuation-restored run's retained
  evidence begins at its base until it seals.
- `canwu-api`: no new item (`DecisionOption`, both decision constants, and
  `sealed_evidence_cursor` on `Canwu`, `CompactedCanwu`, and `CanwuRun` were
  already there); no `&mut self` method on `Canwu` is added, so `gated!` is
  unchanged.
- `canwu-movement`: `MovementLimitsV1::bookings_per_runtime` (new field),
  `MovementLimitsV1::CURRENT.bookings_per_execution` 64,
  `MOVEMENT_SEMANTIC_HASH` (new value); the rustdoc of
  `bookings_per_execution`, `MovementBookingRequestV1::replaces`,
  `MovementBookingBindingV1::replaces`, `MovementState::retired_consumption`,
  `MovementState::validate`, and `MovementReportDetailV1::bookings` states
  the new meaning. Behaviour: closed and replaced bookings leave the
  execution, and departure reads every kept booking of its leg (the 0.20.0
  skip of replaced bookings is gone). Refusals of `RequestBooking`, each a
  recorded rejection, in order: a bounded tie-break key and a known pool with
  the booking's resource (`InvalidOperation`, `NotFound`); an identity
  neither kept nor named by a kept binding's `replaces` (`Conflict`); a known
  execution that is not arrival-pending (`NotFound`, `InvalidState`), an
  unstarted leg of the active revision (`InvalidState`), and a window inside
  the leg's re-timed planned window (`InvalidOperation`, `LimitExceeded`); a
  `replaces` naming a kept booking of the same execution and leg
  (`InvalidOperation`), not one that left because a kept booking of that leg
  replaced it (`Conflict`, "already replaced"), in a status that can be
  replaced (`InvalidState`); fewer than 64 kept bookings in the execution and
  16,384 in the runtime (`LimitExceeded`; 0.20.0 refused the 17th booking of
  the execution's life with `InvalidState`). No `canwu-transport`,
  `canwu-api`, or `canwu-sim` change.
- `canwu-fiscal`: no code change; the rustdoc of `prepare_fiscal_archive`
  and `FiscalArchiveError::UnsealedCitations` and the crate README say what
  the read says. `UnsealedCitations` keeps its shape and meaning on a run
  that has sealed, a continuation-restored run no longer meets it until it
  seals, and `Backpressure`, `NothingToArchive`, and `Mismatch` are unchanged.
- `canwu-military`: `MilitaryCommandV1::JoinSiege` (new
  variant), `timed_step_ingress(due_at) -> PluginIngressRequest` and
  `MILITARY_TIMED_STEP_INGRESS` (new), `OpenEngagementV1::max_minutes:
  Option<u32>`, `EngagementStateV1::max_minutes: u32`,
  `ReviseSiegeV1::release: Vec<ForceId>`, and
  `MilitaryCommandV1::ReviseCrossing::defenders: Option<Vec<ForceId>>` (new
  fields), `SCHEMA_VERSION` 2. Behaviour: a subject whose sides name no
  holder queues no report; the wake system skips a boundary that admitted a
  step packet of either kind; `JoinSiege` enters the siege's assault or
  sortie in progress; `InvestSite` refuses besiegers and defenders of one
  owner. Recorded rejections (`canwu.military.command_rejected.v1`), added to
  0.20.0's: a `max_minutes` of 0 or above the kind rule's
  (`ValueOutOfRange`); a joining force that is not free at the site, already
  in the siege, of another owner than its side or of the opposite side's
  owner, or past the siege's or its open fight's side limit, or a join of a
  force the issuer does not control (`InvalidAuthority`); an `InvestSite`
  whose besiegers and defenders have one owner; a release of a force not in
  the siege, in a fight, or not the issuer's (`InvalidAuthority`), or a
  release that would leave no besieger; a revised defender not at the far
  bank, the crossing force itself, or past the side limit, or a revision by
  an issuer who controls neither the crossing force nor a defender, or who
  would remove a defender it does not control (`InvalidAuthority`). A force named twice
  in a join, a release, or a revised defender list is refused at admission
  (`InvalidPayload`). The timed step cannot be refused in a boundary: a
  non-null payload is refused when the host enqueues it, and a timed step
  with no rulebook installed or no work due steps nothing. `canwu-api` and
  the reference crates are unchanged in behaviour (the starter and the
  content only build the new fields).

The source-incompatible changes are the new fields (`DecisionOption::bias`,
`MovementLimitsV1::bookings_per_runtime`, `OpenEngagementV1::max_minutes`,
`EngagementStateV1::max_minutes`, `ReviseSiegeV1::release`, and
`MilitaryCommandV1::ReviseCrossing::defenders`) and the new variant
`MilitaryCommandV1::JoinSiege`. No `&mut self` method on `Canwu` is added.
The release is therefore a minor version.

Version 0.21.0 also changes these behaviours, which are visible to existing
code and runs:

- **A ticket's options carry a bias.** The trace's `bias` is the policy's
  bias for the option plus the bias of the option on the ticket version the
  trace names; a host whose tickets carry no bias scores exactly as in
  0.20.0. A ticket bias that cancels the policy's records zero and is
  omitted. A host that evaluates a policy again after a restore scores the
  same, because the bias is in the ticket it reads.
- **Closed bookings are not in the live runtime.**
  `TransportExecution::bookings` and the owner's report list only the kept
  bookings, so a booking's final status after it closed is read from a
  report published while it was kept or from the record history; a replaced
  booking is gone from the runtime when its replacement is admitted, and a
  settled or cancelled execution keeps no booking. A pool's `consumed` is its
  kept consumed bookings plus `retired_consumption`, which gains a booking's
  quantity when the booking leaves, not only when its execution retires.
- **The booking bound counts what is kept.** A leg may book more than 16 lots
  at once (up to 64 kept per execution, the lots the leg under way consumed
  and the lost lots not yet replaced counted), and a lost lot is re-booked in
  place with `replaces` as often as it is lost, because the replacement takes
  its original's place. `LimitExceeded` on `RequestBooking` means that the
  execution or the runtime keeps as many bookings as it may.
- **A continuation-restored run archives freely until it seals.** A host
  that saves only by continuation checkpoint and never seals archives every
  closed book in every session again, and its ledgers no longer fill after a
  restore. When such a run seals, its segments start at its base, and from
  then on citations past its own last seal are refused as before. Forks
  answer as their source does; a continuation checkpoint written by a run
  that archived books citing versions after its base no longer carries those
  versions and restores.
- **Military.** A host without report holders writes the
  registry once in a stepping boundary when its own fields move and not at
  all otherwise; a host with holders behaves as in 0.20.0. A fight's duration
  is fixed when it opens. A joined or released force changes the siege's
  investment from its next siege day that runs (in the same boundary when one
  is due there); a force that joins during the siege's assault or sortie
  enters that fight too, while revised crossing defenders enter only the next
  landed wave's bridgehead fight. Every command the timed step's boundary
  admits, queued as ingress for its instant or proposed during it, applies in
  the next cut, after the step.

Upgrading from 0.20.0, for hosts and plugin authors:

- **Pin `=0.21.0`** for `canwu-api` and every extension. Nothing earlier
  loads.
- **A decision host** that registered a controller per bias value (a
  per-ticket value folded into `with_option_bias`) registers one controller
  per decision maker and class with the policy's standing biases only, and
  writes each ticket's value on its option with `DecisionOption::with_bias`
  when it drafts the ticket or replaces its options; the trace's `bias` is
  then the sum of the two. Struct literals of `DecisionOption` that list
  every field add `bias: 0`. Decision archive blobs written by 0.20.0
  (format 1) are refused, so keep them with the 0.20.0 engine.
- **A movement host** may book more than 16 lots for one leg and re-book a
  lost lot in place every time it is lost; a leg no longer has to wait for
  lots it cannot book again. A host that read a booking's final status from
  the runtime after it closed reads it from a report published while the
  booking was kept or from the record history, or records it when it closes
  the booking. Treat `LimitExceeded` on `RequestBooking` as the bound (0.20.0
  answered the 17th booking of a life with `InvalidState`). A hand-built
  runtime (`MovementState::into_initial_record`) keeps no closed booking, and
  a binding's `replaces` names a booking not in the runtime; struct literals
  of `MovementLimitsV1` add `bookings_per_runtime` (a host reads `CURRENT`).
- **A fiscal host** that saves only by continuation checkpoint archives its
  closed books in every session again; a "not yet" arm for
  `UnsealedCitations` may stay, it no longer fires there. A host that read
  `sealed_evidence_cursor()` for where a run's retained evidence begins gets
  the 0.20.0 answer with
  `run.sealed_evidence_cursor().or_else(|| run.continuation_base().map(|base| base.cut))`
  on a `CompactedCanwu` (`continuation_base` is not on `CanwuRun`; a base at
  the zero cut answers `Some` of it where 0.20.0 answered `None`);
  `continuation_base()` alone is not that answer, since it stays set after
  the run's first seal. A sealing host changes nothing.
- **A military host** adds `max_minutes: None` to
  `OpenEngagementV1` literals (the kind's duration, as before), `release:
  Vec::new()` to `ReviseSiegeV1`, and `defenders: None` to
  `MilitaryCommandV1::ReviseCrossing`; payloads use `SCHEMA_VERSION` (now 2),
  so a host that writes the constant needs no other change. A host that names
  no report holder pays at most one registry write a stepping boundary, with
  no change on its side. To open a fight shorter than its kind, set
  `max_minutes`. To add a column to an invested place, send `JoinSiege` (not
  a second `InvestSite`); during the siege's assault or sortie the column
  also enters that fight, and an `InvestSite` whose besiegers and defenders
  have one owner is refused. To let a
  besieged force march off, release it with `ReviseSiege { release }` and
  place it in the same admission order; lift a siege whose last besieger
  leaves with `LiftSiege`. To make a force that reached the far bank resist
  the landing, send `ReviseCrossing { defenders: Some(..) }`. To step once a
  day without a boundary of the step's own, queue
  `timed_step_ingress(<the next daily boundary's time>)` on the run handle
  after each daily boundary instead of sending `Step`; the default wake
  cadence may stay. Commands that boundary admits apply in later cuts after
  the step; on a day whose orders must apply before the step, send `Step`
  with them and withdraw that day's timed step with `cancel_plugin_ingress`.
  No 0.20.0 save loads (every military record carries schema version 1), and
  a host record that embeds a 0.20.0 `ReviseSiege` does not decode (`release`
  is required); an embedded `OpenEngagement` or `ReviseCrossing` decodes with
  `max_minutes` or `defenders` as `None`.
- **Saves.** Write a fresh continuation checkpoint, snapshot, or checkpoint
  journal with 0.21.0; nothing from 0.20.0 loads.
- **Force-supply reference.** A host that runs the unpublished
  `canwu-force-supply-reference` rebuilds its initial force root with
  `into_initial_record`; a format-2 root is refused.

Known limits carried by this release, each stated in its proposal:

- Decisions: a trace records one bias per option, and its ticket part is the
  bias of the option on the ticket version the trace names (always the
  current options for a resolving trace, but not for a deferring trace after
  a later `ReplaceOptions`, because hot history keeps only a ticket's latest
  version); only the shipped utility evaluator reads the bias (rule, human,
  external, and LLM policies ignore it, `ExternalDecisionRequest` does not
  carry it, and a host `UtilityEvaluator` reads `option.bias` only if it
  chooses to); a policy built on `WeightedUtilityEvaluator` cannot opt out of
  ticket biases; the bias is unbounded, so a host that wants a clamp applies
  it when it drafts the option
  ([known limits](proposals/decision-ticket-option-bias.md#known-limits)).
- Movement: closed bookings are not in the live runtime (a booking that
  closes in the boundary it was decided appears in no report); a replaced
  booking's identity is free again once its replacement leaves, so a host
  that wants booking identities unique for the run keeps them so; the bound
  counts the leg under way, so a host that books the next leg while the
  current one is under way needs room for both (two legs of 32 lots), and a
  lost lot counts until it is replaced or its leg fails; one runtime-wide
  booking bound without a per-owner quota; each `RequestBooking` scans every
  kept binding of the runtime (at most 16,384) for a `replaces` naming its
  identity; an `OfferPool` window can be narrowed past a consumed booking
  that left, the pool's `consumed` still holding the quantity; a report of an
  execution near 64 kept bookings and 64 legs with labels near 256 bytes can
  pass the 64 KiB knowledge payload limit and is withheld with
  `canwu.movement.report_withheld.v1`
  ([known limits](proposals/movement-kept-bookings.md#known-limits)). The
  0.20.0 limit that replacements count against `bookings_per_execution` (16)
  is lifted.
- Fiscal: a host that saves only by continuation checkpoint and never seals
  keeps no segment, so the versions its archived books cite are in nothing
  it holds once their books leave the run (the archived book names them by
  identity), as was already true for every citation at or below a restore's
  base and for a never-sealed run's first session; a host that seals at each
  save archives, in its first session after a continuation restore and until
  its first seal there, books whose citations its next seal puts in a segment
  rather than one that already holds them, as in a fresh run's first session
  (seal at each save, before the continuation checkpoint, keeps every
  archived citation in a segment); a "not yet" arm for `UnsealedCitations`
  is harmless
  ([known limits](proposals/fiscal-archive-after-continuation-restore.md#known-limits)).
- Military: a day's orders still cost their boundaries
  (a command admitted by the timed step's boundary, queued as ingress for its
  instant or proposed during it, applies in the next cut, after the step, so
  a host whose orders depend on its daily boundary's
  results still pays the command boundary and the cut that applies them, and
  those orders step from the next timed step, a day after a `Step` sent with
  them would); report holders are fixed at the opening, so a fight, siege, or
  crossing opened without them never queues a report; joined and released
  forces change the siege from its next siege day (investment is not
  recomputed at once), and a besieger that joins during a relief stands
  beside it, not in it; a fight's duration is fixed at its opening and a
  fight the plugin opens takes its kind's; revised crossing defenders do not
  enter a fight already in progress; the 0.20.0 military limits (engine
  finding E-51) stand
  ([known limits](proposals/military-engagement-revisions.md#known-limits)).
- Every limit of the 0.20.0 entry stands except the booking bound of 16 for
  an execution's life (downstream gap G-83) and the continuation restore
  counted as a seal (downstream gap G-91), which this release fixes.

The hash algorithms are unchanged. The engine version in every checkpoint
hash changes, first-party plugins that take their plugin version from the
crate version report a new one, and the exact engine-version check rejects
0.20.0 saves; retain the 0.20.0 engine to read them.

Version 0.20.0 is a pre-1.0 minor release. Its changes come from the
packages that close every downstream gap and engine finding filed against
0.19.0, each with its own proposal:

- [correspondence domain-entity parties, withheld reports, and refused seeds](proposals/correspondence-domain-parties.md)
  (`canwu-correspondence`: downstream gaps G-72 and G-78, and the
  correspondence half of engine finding E-41);
- [movement holds before departure, re-timed later legs, and booking revocation](proposals/movement-holds-and-revocation.md)
  (`canwu-movement`, `canwu-transport`: downstream gaps G-73, G-79, and
  G-80, engine finding E-48);
- [decision deadlines, reassignment, and reads](proposals/decision-deadlines-reassignment-and-reads.md)
  (`canwu-decision`, `canwu-sim`, `canwu-api`: downstream gaps G-70, G-81,
  and G-82);
- [military engagements](proposals/military-engagements.md)
  (`canwu-military`, `canwu-military-reference-content`,
  `canwu-military-reference`: downstream gap G-38; it supersedes
  [military domain design](proposals/military-domain-design.md));
- [resource continuation evidence and completion reports](proposals/resource-continuation-and-completion-reports.md)
  (`canwu-resource`, `canwu-production`, `canwu-technology`: downstream
  gaps G-74, G-75, G-76, and G-77);
- [calendar marks and keyed-draw retirement](proposals/calendar-marks-and-draw-retirement.md)
  (`canwu-sim`, `canwu-api`: downstream gaps G-69 and G-71).

What changed, in one list:

- **Correspondence parties.** `canwu-correspondence` names a domain-entity
  sender, recipient, carrier, opportunity candidate, interceptor, or seed
  holder by its domain record, in its operation, opportunity, and seed
  records and in the delivery-attempt and access bindings it hands to
  `canwu-information`; the prepared dispatch's sender is matched in either
  form. Its party roles admit a retired entity, so a correspondence record
  never stops a party's owner from retiring it. A carrier the engine refuses
  as a knowledge holder (retired, deleted, dead, or ineligible) takes no new
  correspondence, retry, or delegation: `InvalidAuthority` at admission, and
  a start whose carrier became so before it settled is skipped with
  `canwu.correspondence.start_skipped.v1` (a retry is recorded without an
  attempt).
- **Withheld reports and refused seeds.** An attempt report to a carrier the
  engine refuses is withheld with
  `canwu.correspondence.attempt_report_withheld.v1`. A knowledge seed whose
  holder the engine refuses, that one publication directive cannot carry, or
  that states a connection no planning network admits publishes nothing and
  records a **seed refusal** (`KnowledgeSeedRefusal`: `HolderUnavailable`,
  `TooLarge`, or `InvalidConnection`) on its `KnowledgeSeedReceipt`, whose
  `published_at` is now `settled_at`. Each of these was a failed boundary.
  The plugin semantic hash changes.
- **Holds before departure and re-timed later legs.**
  `MovementOperation::DelayLeg { execution, leg, until, reason,
  retime_later_legs }` holds the current leg whether it has departed (its
  arrival, as in 0.19.0) or not (its departure, bounded by the departure
  that was pending when the leg was first held), without an itinerary
  revision. `MovementLegHoldV1` is reshaped (`kind`, `scheduled_at`,
  `retime_later_legs`). With `retime_later_legs`, the part of the hold that
  elapsed is added to the order's **schedule shift**
  (`MovementOrderRecordV1::schedule_shift`), which moves the planned
  departures and booking windows of the later legs so they keep their
  planned spacing; a reroute resets it.
- **Booking revocation and replacement.** A pool's custodian, or an
  application incident, revokes a confirmed booking with
  `MovementOperation::RevokeBooking { pool, booking, reason }`
  (`canwu-transport`: `CapacityBookingStatus::Revoked`,
  `CapacityBooking::revoke`, `CapacityBookingRevocationV1`); the capacity
  returns to the pool. The owner replaces a lost booking with a new request
  that names it (`MovementBookingRequestV1::replaces`), and a replaced
  booking no longer counts at departure.
- **Superseded dues are withdrawn.** When an operation or a hold replaces a
  leg due that is not yet due, the movement plugin withdraws the queued
  `movement_leg_due_v1` packet with `CancelPluginIngress`, so superseded
  packets neither accumulate nor wake a boundary (E-48).
  `TRANSPORT_SEMANTIC_VERSION` is `canwu-transport.v6`; the movement
  semantic hash and the `movement_report` schema hash change.
- **Option bias.** `WeightedUtilityEvaluator::with_option_bias(option_id,
  bias)` adds a policy's standing integer preference for one option to its
  weighted score, recorded apart from the factor lines as
  `DecisionOptionEvaluation::bias`; a `GuardedUtilityPolicy` that declares a
  bias has a new semantic hash, one without keeps its 0.19.0 hash.
- **Deadline extension and reassignment.**
  `DecisionMutation::ExtendDeadline { ticket_id, expected_version, deadline }`
  moves an open ticket's deadline later under the same ticket ID;
  `DecisionMutation::Reassign { ticket_id, expected_version, controller_id }`
  moves an open ticket to another registered controller, which alone may then
  resolve it; the decision maker and lineage stay. Both are checked at
  admission against the ticket version, advance it, and are journalled like
  every non-resolving mutation; every refusal is a recorded rejection.
- **Ticket reads and the admission batch.**
  `SimulationView::decision_tickets_opened` (the tickets the current
  boundary's admitted `Open`s created, in admission order),
  `open_decision_tickets_of(&EntityRef)`, and
  `open_decision_tickets_of_controller(&str)`, behind the
  `canwu.core.decisions` read, with the same three on `Simulation`,
  `CompactedSimulation`, `Canwu`, and `CompactedCanwu` for the trusted host.
  The **admission batch** is persisted decision state committed in
  `canwu.commitment.decisions.v3`; `COMMITMENT_FORMAT_VERSION` became 7
  (0.21.0 moves it to 8).
- **The engagement engine.** `canwu-military` is rewritten as an engagement
  engine: typed forces in the host's world, stepped engagements on a
  host-installed `CombatRulebookV1` and a `StepResolver`
  (`ParametricStepResolver` or the host's own type), sieges and contested
  crossings, holder-relative `engagement_report` and `engagement_outcome`
  knowledge, and an optional outcome signal; it owns no doctrine, catalogue,
  control, or store. `canwu-military-reference-content` becomes the
  synthetic rulebook (`synthetic_rulebook_v1`) and `canwu-military-reference`
  the engagement starter (`demo_engagement_scenario`, example
  `military_starter`).
- **Resource continuation evidence.** Each hot terminal bucket declares the
  exact evidence of its records identity-only, and the live root the
  versions its held completion state and observation heads cite
  (`canwu_identity_evidence_dependencies`), so a seal or a continuation
  checkpoint taken while a completion lease's outcomes are hot keeps proof of
  every version they cite, and the run restores, validates, and continues
  exactly (G-77). The production root declares the pending output source of
  every execution that has no output outcomes yet.
  `RESOURCE_STATE_FORMAT_VERSION` 6, `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION`
  2, and `RESOURCE_LIVE_BUCKET_FORMAT_VERSION` 2.
- **Completion reports given up.** A completion's reserved holder report
  whose grant has no observation head, or whose holder the engine refuses,
  is given up when it falls due: the participant archives without it and no
  report wake is queued (G-75). A completion operation that reserves reports
  for a holder the engine refuses is a recorded `invalid_authority`
  rejection, `install_report_grant` refuses a holder kind that can hold no
  knowledge, and a `RecordObservation` head's sources must be current.
- **Declared technology reads and domain-entity technology references.**
  Production declares the `AssetBinding` and `ApplicationSpec` reads and
  refuses a cited record of the wrong kind before reading it (G-74);
  `TechnologyRecordPayload::references` names a domain-entity holder,
  operator, owner, adopter, sponsor, authorizer (`authorized_by`), observer,
  asserter (`asserted_by`), assessor, or site as its domain record (G-76).
  The resource, production, and technology semantic hashes change.
- **Calendar marks.** A host that runs more than one
  calendar marks a calendar boundary with the calendars whose period starts
  there: `schedule_calendar_marks(due_at, marks)` with
  `CalendarMark { cadence, calendar }`. A boundary system registered with a
  calendar key (`BoundarySystemContract::calendar`) is due only in a boundary
  that carries its own mark of its cadence; a system without a key is due in
  every boundary of its cadence, as before. `SimulationView::calendar_marks`
  reads the marks of the boundary being settled, and `BoundaryRecord` and
  `BoundarySummary` carry them. A calendar key is non-empty, free of surrounding whitespace,
  at most `MAX_CALENDAR_KEY_BYTES` (64) bytes, and has no control characters;
  marks on a boundary with no admitted calendar ingress are refused. The core
  still keeps no calendar.
- **Keyed-draw retirement.** A boundary system may
  declare, for a stream it owns, how long its operation-keyed draws stay in
  the keyed-draw ledger (`BoundarySystemContract::keyed_draw_retirement`,
  `KeyedDrawRetirement { stream, after }`). The first boundary at or after a
  draw's boundary time plus `after` drops it before any system runs; a later
  request with the same address draws afresh under a new draw ID (the same
  value for the same bound and purpose) and is no longer checked against the
  retired draw. Draw records stay as evidence. Every restore and exact replay
  retire alike, and a continuation checkpoint carries only the draws not
  retired at its time and refuses one that lists a retired draw:
  `CONTINUATION_CHECKPOINT_FORMAT_VERSION` 4.

**Same version only.** Canwu crates depend only on the exact same version of
each other: every first-party crate requires `=0.20.0` of every other
first-party crate it uses, and a host pins `canwu-api = "=0.20.0"` and the
same version of every extension. Nothing written by 0.19.0 is read by
0.20.0. There is no migration and no decoding of an old shape: the exact
engine-version check refuses every 0.19.0 snapshot, checkpoint journal, paged
checkpoint, continuation checkpoint, replay journal, evidence segment, and
scenario that records its engine version; a commitment at format 6 and a
continuation checkpoint at format 3 are refused by their format as well; a
run with the correspondence, movement, resource, production, technology, or
military plugin would also be refused by its semantic hash, and a resource
root by its state format 6 and bucket formats 2. A scenario built with
0.19.0 types is rebuilt with the 0.20.0 types: a `DelayLeg` carries
`retime_later_legs`, a `KnowledgeSeedReceipt` has `settled_at` and
`refusal`, a resource root installed with `ResourceState::into_records`
encodes at state format 6 with bucket format 2 and its identity-only
declaration, and no 0.19.0 military record loads. Retain the 0.19.0 engine
to read 0.19.0 data.

Formats and schemas that change, each deliberately versioned:

| Item | 0.19.0 | 0.20.0 | What changed |
| --- | --- | --- | --- |
| Snapshot, commitment, checkpoint/evidence-journal, paged-checkpoint, and continuation-checkpoint formats | 9, 6, 6, 7, 3 | 9, 7, 6, 7, 4 | commitment 7 commits to decision state through `canwu.commitment.decisions.v3` (the admission batch); continuation checkpoint 4 lists in `keyed_draws` only the draws not retired at `now` and a restore refuses one that lists a retired draw; the other three keep their shape; the engine version in every checkpoint hash changes |
| Decision commitment | `canwu.commitment.decisions.v2` | `canwu.commitment.decisions.v3` | over the controllers, the hot-history root, the archive receipt root, and the admission batch; every checkpoint hash and `v1:` state commitment of a run with decision state differs |
| `DecisionState` (snapshot, compact and paged checkpoint hot state, continuation checkpoint) | — | — | `opened_in_batch: [DecisionTicketId]`, omitted while empty; the open-ticket indexes by decision maker and by controller are derived and not encoded |
| `DecisionMutation` (decision ingress, replay journal) | 5 variants | 7 variants | adds `extend_deadline { ticket_id, expected_version, deadline }` and `reassign { ticket_id, expected_version, controller_id }` |
| `DecisionOptionEvaluation` (in `PolicyDecision` and `DecisionTrace`, hot history and archive blobs) | — | — | `bias: i64`, omitted when zero; decision archive blob format stays 1 |
| `WeightedUtilityEvaluator` | — | — | `option_biases`, omitted while empty |
| `GuardedUtilityPolicy` semantic hash | — | — | unchanged without biases; a tagged, length-prefixed option-bias section is appended when any is declared, so such a policy is refused by a binding of the same ID and version (`PolicyMismatch`) |
| `DecisionState::validate` | deadline index | every open-ticket index and the batch | also refuses an inconsistent decision-maker or controller index and a batch that repeats a ticket or names one outside hot history; snapshot validation reconstructs the batch per boundary from admitted ingress |
| `IngressPayload::Calendar` (snapshot, journals, replay journal, pinned continuation ingress) | `cadences` | `cadences`, `calendar_marks` | `calendar_marks: Vec<CalendarMark>`, omitted while empty; canonical (sorted, unique, each mark's cadence among `cadences`), checked at load |
| `BoundaryRecord` and its hash material `canwu.boundary-record.v1` | — | — | `calendar_marks`, omitted while empty, so an unmarked run hashes as before; must equal the admitted calendar ingress's marks |
| `BoundarySummary` (boundary ledger, continuation `settled_boundaries`) | — | — | `calendar_marks`, omitted while empty |
| Plugin descriptor `BoundarySystemContract` | — | — | `calendar: Option<String>` (omitted while `None`) and `keyed_draw_retirement: Vec<KeyedDrawRetirement>` (omitted while empty, sorted by stream, each stream once with a positive duration); both part of the descriptor and so of the run's identity; a key on an `EventDriven` system, a malformed key, or a retirement naming an undeclared stream is `InvalidPluginRegistration` and `InvalidSnapshot` at load |
| `MovementOperation` (command and incident payloads) | 11 variants | 12 variants | `delay_leg` adds the required `retime_later_legs`; adds `revoke_booking { pool, booking, reason }`; the incident allow-list gains `RevokeBooking` |
| `MovementLegHoldV1` (in `MovementOrderRecordV1::hold` and `MovementReportDetailV1::hold`) | `leg, scheduled_arrival_at, until, reason, held_at` | `leg, kind, scheduled_at, until, reason, held_at, retime_later_legs` | `kind` is `Departure` or `Arrival`; `scheduled_at` is the due the hold replaced (a departed leg's departure plus its planned duration, or the departure pending when the leg was first held) |
| `MovementOrderRecordV1` (runtime) | — | — | `schedule_shift`, omitted when zero |
| `MovementBookingRequestV1` (command payload) and `MovementBookingBindingV1` (runtime) | — | — | `replaces: Option<CapacityBookingId>`, omitted when `None` |
| `CapacityBooking` (`canwu-transport`, in the movement runtime and reports) | — | — | `revocation: Option<CapacityBookingRevocationV1>` (`at`, `reason`), omitted when `None`, present exactly when the status is `revoked` |
| `CapacityBookingStatus` | 7 values | 8 values | adds `revoked`; `Confirmed → Revoked` returns the quantity from the pool's `booked` |
| `MovementReportDetailV1` (report payload) | — | — | `schedule_shift`, omitted when zero; `hold` carries the reshaped `MovementLegHoldV1` |
| `MovementState::validate` | a booking window inside its leg's planned window; a hold on a departed leg only | a booking window starts no earlier than the leg's planned departure and lasts no longer than the leg; a hold of either kind; replacement and revocation invariants; a non-negative, bounded shift | applied at activation, at each load of the runtime, and to every candidate |
| `TRANSPORT_SEMANTIC_VERSION` | `canwu-transport.v5` | `canwu-transport.v6` | the revocation transition and status |
| `MOVEMENT_SEMANTIC_HASH` | `6cf78f1f…550e` | `b211c92a…abbc` | BLAKE3 of the string recorded in the proposal |
| `movement_report` knowledge schema hash | `81cf8978…4f11` | `33d6f3ab…ed28` | BLAKE3 of the string recorded in the proposal; the schema id stays at version 1 |
| `movement_incident_v1` descriptor description | "…delay, reroute, or handoff" | "…delay, reroute, handoff, or booking revocation" | part of the plugin descriptor |
| `KnowledgeSeedReceipt` (record `canwu.correspondence.knowledge_seed`) | `seed_key`, `canonical_input_hash`, `holder`, `published_at` | `seed_key`, `canonical_input_hash`, `holder`, `settled_at`, `refusal` | `published_at` renamed `settled_at` (the boundary time the seed settled, published or refused); `refusal: Option<KnowledgeSeedRefusal>`, omitted when `None`; `holder` is referenced only when the holder accepts knowledge |
| `KnowledgeSeedRefusal` | — | new | internally tagged (`type`: `holder_unavailable`; `too_large` with `records` and `largest_record_bytes`; `invalid_connection` with `connection` and `reason`), unknown fields refused |
| Correspondence party references and record schemas | `Core` targets; party roles `allow_retired: false`; seed `holder` required | a domain-entity party is a `Domain` target; the party roles of the operation, opportunity, and seed receipt `allow_retired: true`; the seed receipt's `holder` reference role is optional in the schema | 0.19.0 could write no domain target, so no 0.19.0 run held one |
| `correspondence-lifecycle-v1` contract | no `emits` | `emits: ["canwu.correspondence.start_skipped.v1"]` | part of the plugin descriptor |
| `correspondence-knowledge-ingress-v1` contract | no `emits` | `emits: ["canwu.correspondence.attempt_report_withheld.v1"]` | part of the plugin descriptor |
| `canwu-correspondence` semantic hash | `a197a514…5685` | `c26226b9…cfcf` | BLAKE3 of the string recorded in the proposal; the knowledge schema hashes are unchanged |
| `RESOURCE_STATE_FORMAT_VERSION` | 5 | 6 | the live root gains `canwu_identity_evidence_dependencies` (`IdentityEvidenceDependenciesV1`: sorted, unique, no initial-scenario version), the versions its completion lease grants lock, each held, prepared, consumed, or report-waiting external participant's coordinator source and locked external versions, and the source versions of every observation head; counted in `max_state_bytes`; validation refuses a live root whose declaration differs from `computed_identity_evidence`, and a report grant whose holder kind can hold no knowledge |
| `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION` | 1 | 2 | a bucket has `format_version`, `bucket`, `entries`, and `canwu_identity_evidence_dependencies`, the union of its entries' exact evidence (`ResourceTerminalArchivePayloadV1::exact_evidence`); a whole-root read and `ResourceState::from_records` refuse a bucket whose declaration is not the one its entries make; a partial read checks the field's shape and rewrites the declaration whenever it writes the bucket |
| `RESOURCE_LIVE_BUCKET_FORMAT_VERSION` | 1 | 2 | the same field, always empty; any read refuses a live bucket that declares evidence |
| Resource record schemas | `runtime` declares the payload-required continuation | `runtime` declares both reserved properties; `bucket` declares the identity-only one, required on both bucket families | — |
| Production runtime record (`canwu.production:runtime`, scope roots) | — | — | the payload gains `canwu_identity_evidence_dependencies`, the pending output sources of executions without output outcomes yet; its schema declares the property; the canonical-encoding check of restore and activation covers it; the production state formats are otherwise unchanged |
| Resource archive | blob format 1 | blob format 1 | an archived entry carries the same `exact_evidence` as before, now built from `ResourceTerminalArchivePayloadV1::exact_evidence` |
| `RESOURCE_SEMANTIC_HASH` | `3a1e04cb…32ac` | `6fbe0308…159a` | BLAKE3 of the string recorded in the proposal |
| `PRODUCTION_SEMANTIC_HASH` | `7b39f2f5…8ed0` | `eded5044…6d9e` | BLAKE3 of the string recorded in the proposal |
| `canwu-technology` plugin semantic hash | `63f9af9a…23ed` | `8b4703e2…ce3a` | BLAKE3 of the string recorded in the proposal; its record references name a domain entity as a `Domain` target |
| `canwu-military` record kinds | `catalog`, `force`, `operation`, `combat`, `occupation`, `knowledge`, `provider_outcome`, `ledger` | `rulebook` (create-only), `registry`, `force`, `engagement`, `engagement_steps`, `siege`, `crossing` (versioned), `engagement_summary` (create-only) | all schema version 1 of the new shapes; no 0.19.0 payload loads |
| `canwu-military` knowledge schemas | `canwu.military:military_report` v1, payload `Any` | `canwu.military:engagement_report` v1 (`RetireWhenSuperseded`) and `canwu.military:engagement_outcome` v1 (`Keep`) | typed `EngagementReportV1` payloads, new schema hashes |
| `canwu-military` random streams | `military-operation` | `crossing-interception`, `step-contingency` | each draw cites the admitted step packet |
| `canwu-military` ingress | the host command action `military_command_v1`; internal ingress for the command's settlement; the public acknowledgement ingress `military_provider_ack_v1` | the host command action `military_command_v1` (unchanged name, new payload `MilitaryCommandV1`); internal ingress `military_apply_v1` and `military_step_v1` | the acknowledgement ingress is removed; the internal packet types are new |
| `canwu-military` plugin descriptor | version `0.1.0`, semantic hash `f07c44f9…3810` | version `0.20.0`; semantic hash BLAKE3 of `canwu-military plugin semantic hash 0.20.0: engagements <resolver name>@<resolver version>` | a `String` held by the plugin, so a host resolver type gives the plugin its own identity; the wake cadence, the outcome-signal target, and the publication reservation are descriptor-recorded |
| `canwu-military` scenario records | a `MilitaryCatalog` record `root` | a `rulebook` record and `force` records (`record_from`), or by command | — |

Semantic identities that differ from 0.19.0: `canwu-correspondence`,
`canwu-movement` (with `canwu-transport`'s semantic version),
`canwu-resource`, `canwu-production`, `canwu-technology`, and
`canwu-military`. The other first-party plugins keep their semantic hash
constants, and a `GuardedUtilityPolicy` keeps its hash unless it declares an
option bias. As with any version change, the engine version in every
checkpoint hash changes and first-party plugins that take their plugin
version from the crate version report a new one.

New and changed public contracts, by crate:

- `canwu-correspondence`: `KnowledgeSeedRefusal` (new enum, re-exported from
  the crate root), `KnowledgeSeedReceipt::refusal` (new field), and
  `KnowledgeSeedReceipt::settled_at` (renamed from `published_at`);
  `ATTEMPT_REPORT_WITHHELD_EVENT` and `START_SKIPPED_EVENT` (new constants).
  Behaviour: domain-entity parties are admitted everywhere, the dispatch
  sender is matched in either form, a carrier the engine refuses as a
  knowledge holder is refused for a correspondence, a retry, or a
  delegation, and seeds and reports are refused as above. No `canwu-api` or
  `canwu-sim` change.
- `canwu-transport` (re-exported by `canwu-api`):
  `CapacityBookingStatus::Revoked`, `CapacityBookingRevocationV1` (new),
  `CapacityBooking::revocation` (new field), `CapacityBooking::revoke(at,
  reason)` (new method; `transition` refuses `Revoked`),
  `TRANSPORT_SEMANTIC_VERSION` (new value).
- `canwu-movement`: `MovementOperation::DelayLeg::retime_later_legs`,
  `MovementOperation::RevokeBooking` (new variant;
  `MovementOperation::execution` returns `None` for it, like `OfferPool`),
  `MovementBookingRequestV1::replaces`, `MovementBookingBindingV1::replaces`,
  `MovementOrderRecordV1::schedule_shift`,
  `MovementReportDetailV1::schedule_shift`, `MovementLegHoldV1` reshaped
  (`kind`, `scheduled_at`, `retime_later_legs`) with
  `MovementLegHoldV1::extension`, `MOVEMENT_SEMANTIC_HASH`. Behaviour:
  `DelayLeg` on a leg that has not departed is applied (it was an
  `InvalidState` rejection); a superseded due is withdrawn; the runtime
  validation of booking windows is relaxed as above. Refusals, each a
  recorded rejection: `DelayLeg` needs a known execution (`NotFound`) from
  its owner or operator (`Unauthorized`), a bounded reason
  (`InvalidOperation`), the current leg departed or `Planned`, `Booked`, or
  `Waiting` with a pending departure due (`InvalidState`), `until` no earlier
  than the boundary and the due it holds (`InvalidOperation`) and within
  `time_bound_minutes` (`LimitExceeded`), and a valid candidate runtime
  (`InvalidState`); `RevokeBooking` needs a known pool (`NotFound`), the
  pool's custodian or an incident (`Unauthorized`), a bounded reason
  (`InvalidOperation`), a booking bound to that pool (`NotFound`) and
  `Confirmed` (`InvalidState`); `RequestBooking` needs a window inside the
  leg's planned window moved by the booking shift (`InvalidOperation`, or
  `LimitExceeded` past the time bound) and a `replaces` that names a booking
  of the same execution and leg (`InvalidOperation`), not yet replaced
  (`Conflict`), and requested, confirmed, failed, expired, or revoked
  (`InvalidState`).
- `canwu-decision`: `DecisionMutation::ExtendDeadline` and
  `DecisionMutation::Reassign` (new variants);
  `DecisionOptionEvaluation::bias` (new field);
  `WeightedUtilityEvaluator::option_biases` and `with_option_bias`
  (zero removes the entry); `DecisionState::open_tickets_of_decision_maker`,
  `open_tickets_of_controller`, `begin_admission_batch`,
  `tickets_opened_in_batch`, and `ticket_ids_opened_in_batch`. Refusals,
  each a recorded rejection with an existing code: `ExtendDeadline` on a
  ticket that is not hot (`TicketNotFound`), not open or past its deadline
  (`ClosedTicket`), at another version (`VersionConflict`), without a
  deadline, or with a deadline not later than the current one
  (`InvalidDecision`); `Reassign` to a controller whose authority person is
  unavailable (`IssuerUnavailable`), to an unregistered controller or the
  one assigned (`InvalidController`), or breaking the lineage link to a hot
  parent (`InvalidDecision`); a `Resolve` from the old controller
  (`InvalidController`).
- `canwu-sim`: `SimulationView::decision_tickets_opened`,
  `open_decision_tickets_of`, and `open_decision_tickets_of_controller`
  (each behind a declared `StateKey::core_decisions()` read, otherwise
  `UndeclaredStateRead`), and the same three on `Simulation` and
  `CompactedSimulation`, where `decision_tickets_opened` lists the latest
  settled boundary's tickets. `CalendarMark` (with `new`),
  `MAX_CALENDAR_KEY_BYTES` (64), `KeyedDrawRetirement` (with `new`),
  `schedule_calendar_marks(due_at, marks)` on `Simulation` and
  `CompactedSimulation` (`InvalidBoundary` for an empty list, an
  `EventDriven` mark, or a non-canonical or over-long key, before anything
  is queued), `SimulationView::calendar_marks` (no read declaration; empty
  outside a boundary system), `BoundarySystemContract::calendar` and
  `keyed_draw_retirement` (`BoundarySystemContract::new` sets `None` and
  empty), `BoundaryRecord::calendar_marks`, `BoundarySummary::calendar_marks`,
  `IngressPayload::Calendar::calendar_marks`.
- `canwu-api`: `decision_tickets_opened`, `open_decision_tickets_of`, and
  `open_decision_tickets_of_controller` on `Canwu` and `CompactedCanwu`
  (inherent, not on `CanwuRun`; trusted-host reads of ground truth);
  `schedule_calendar_marks` on `Canwu` (through `gated!`) and
  `CompactedCanwu`; `CalendarMark`, `KeyedDrawRetirement`, and
  `MAX_CALENDAR_KEY_BYTES` re-exported. Every decision type was already
  re-exported.
- `canwu-military`: the public surface is new. `MilitaryPlugin<R:
  StepResolver = ParametricStepResolver>` with `new`,
  `with_step_cadence(Option<SystemCadence>)`, and `with_outcome_signal`;
  `StepResolver` (with `const IDENTITY`) and `ParametricStepResolver`;
  `CombatRulebookV1`; the command `MilitaryCommandV1` (`InstallRulebook`,
  `RegisterForce`, `ReviseForce`, `PlaceForce`, `RetireForce`,
  `OpenEngagement`, `JoinEngagement`, `SupplyInput`, `OrderAction`,
  `ReviseStepProfile`, `InvestSite`, `ReviseSiege`, `LiftSiege`,
  `OrderCrossing`, `ReviseCrossing`, `CancelCrossing`, `Step`), each command
  on an existing record naming an `expected_host_revision` that steps never
  advance; the records `EngagementStateV1`, `EngagementStepsV1` (`StepV1`),
  `SiegeStateV1`, `CrossingStateV1`, and the engagement summary;
  `ConditionInputV1`, `ForecastV1`, `EngagementReportV1`; the constants
  `MILITARY_APPLY_INGRESS`, `MILITARY_STEP_INGRESS`,
  `MILITARY_REJECTION_EVENT` (`canwu.military.command_rejected.v1`),
  `ENGAGEMENT_HELD_EVENT` (`canwu.military.engagement_held.v1`),
  `ENGAGEMENT_REPORT_KNOWLEDGE`, and `ENGAGEMENT_OUTCOME_KNOWLEDGE`. The
  0.19.0 types listed under "Upgrading" are gone. Issuers are matched
  against a force's `controllers` (any `Issuer` kind) or the rulebook's
  `trusted_issuers`. Every refusal the proposal lists is a recorded
  rejection; a step the rulebook cannot resolve holds the engagement
  (`engagement_held`) rather than failing the boundary.
- `canwu-military-reference-content`: `synthetic_rulebook_v1()` and
  `SYNTHETIC_RULEBOOK_ID` (`canwu:military:rulebook:synthetic:v1`); the
  `riverine_preindustrial` and `industrial_front` profiles are deleted.
  `canwu-military-reference` (not published): `demo_engagement_scenario()`
  and the `military_starter` example.
- `canwu-resource`: `ResourceState::identity_evidence` (field, serde
  `canwu_identity_evidence_dependencies`) and
  `ResourceState::computed_identity_evidence`;
  `ResourceHotTerminalBucketV1::evidence` and `ResourceLiveBucketV1::evidence`
  (serde `canwu_identity_evidence_dependencies`);
  `ResourceTerminalArchivePayloadV1::exact_evidence`;
  `holder_kind_can_hold_knowledge`; `install_report_grant` refuses a holder
  kind that can hold no knowledge (`InvalidDefinition`);
  `RESOURCE_STATE_FORMAT_VERSION` 6,
  `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION` 2,
  `RESOURCE_LIVE_BUCKET_FORMAT_VERSION` 2, a new `RESOURCE_SEMANTIC_HASH`.
  Refusals, each recorded: an `Acquire`, `SingleStepLease`, or
  `GrantExternalParticipant` that reserves holder reports for a holder the
  engine refuses (`invalid_authority`, which takes its operation key); a
  `RecordObservation` head whose source is not its record's current version
  (`version_conflict`) or of a kind the plugin does not read
  (`invalid_authority`).
- `canwu-production`: a new `PRODUCTION_SEMANTIC_HASH`; the runtime record's
  identity-only declaration; the command handlers' and lifecycle system's
  declared external reads add the technology `AssetBinding` and
  `ApplicationSpec` state keys (part of the plugin descriptor); a production
  evidence binding, technology binding, adoption application, or
  observation attempt of the wrong record kind is a recorded
  `invalid_domain_record` rejection before anything is read; a site report
  to a holder the engine refuses is skipped.
- `canwu-technology`: a new semantic hash; `TechnologyRecordPayload::references`
  names a domain-entity holder, operator, owner, adopter, sponsor,
  `authorized_by`, observer, `asserted_by`, assessor, or site as a `Domain`
  target under the `core` role.

The source-incompatible changes are the new enum variants
(`DecisionMutation::ExtendDeadline` and `Reassign`,
`MovementOperation::RevokeBooking`, `CapacityBookingStatus::Revoked`), the
new and renamed fields (`DecisionOptionEvaluation::bias`,
`MovementOperation::DelayLeg::retime_later_legs`, `MovementLegHoldV1`'s
`kind`, `scheduled_at`, and `retime_later_legs` for `scheduled_arrival_at`,
`MovementOrderRecordV1::schedule_shift`, `MovementReportDetailV1::schedule_shift`,
`MovementBookingRequestV1::replaces`, `MovementBookingBindingV1::replaces`,
`CapacityBooking::revocation`, `KnowledgeSeedReceipt::refusal` and
`settled_at`, `ResourceState::identity_evidence`,
`ResourceHotTerminalBucketV1::evidence`, `ResourceLiveBucketV1::evidence`,
`BoundarySystemContract::calendar` and `keyed_draw_retirement`,
`BoundaryRecord::calendar_marks`, `BoundarySummary::calendar_marks`, and
`IngressPayload::Calendar::calendar_marks`), and the rewritten
`canwu-military` surface. No `&mut self` method on `Canwu` is added outside
`gated!`. The release is therefore a minor version.

Version 0.20.0 also changes these behaviours, which are visible to existing
code and runs:

- **A dead person cannot carry.** A correspondence, retry, or delegation
  naming as carrier a person who is not alive, or a domain entity the engine
  would not publish knowledge to, is rejected (`InvalidAuthority`); 0.19.0
  admitted a dead person. A carrier that becomes so after admission skips the
  start or the retry; an attempt already in flight continues, and only a
  seizure report to it is withheld.
- **A seed receipt no longer means "published".** Every admitted seed key
  settles to one receipt; a refused seed publishes nothing, spends its key,
  and references its holder only when the holder accepts knowledge.
- **Held legs.** A `DelayLeg` on a leg that has not departed holds its
  departure instead of being refused. A repeated hold of one due keeps the
  bound of the first hold. A hold ends when its due settles, when `StartLeg`
  or `CompleteLeg` acts on the leg, when the leg fails, or when a reroute or
  cancellation closes the revision; with `retime_later_legs` the elapsed part
  of the hold (at most its extension) is added to the schedule shift when it
  ends, so later legs depart at `later(at, planned_departure_at +
  schedule_shift)`. Without it the later legs keep their planned times, as
  in 0.19.0.
- **Booking windows.** A booking must lie inside its leg's planned window
  moved by the order's schedule shift plus the pending re-timing hold's
  extension; a held departure moves its own leg's window to open at the
  hold's `until`. A booking requested before a re-timing hold keeps its
  window and expires if the re-timed departure passes it, unless replaced. A
  revoked booking fails its leg at departure (operations settle before dues)
  unless replaced; a replacement cancels a `Requested` original and releases
  a `Confirmed` one.
- **Superseded dues leave the queue.** `pending_ingress()` no longer lists
  the leg-due packet an operation or hold replaced, no boundary wakes for it,
  and each withdrawal is a `PluginCancellation` ingress record of the
  boundary that withdrew it. Each order's final due is scheduled once, and
  not again when the packet queued earlier already carries it (re-stating a
  hold with the same `until` queues no second packet). A due replaced in the
  boundary of its own time is skipped as stale, as before.
- **Option biases apply before the near-equivalence margin**, so a bias can
  make or break a tie-break; blocked and excluded options are not scored and
  carry no bias.
- **A ticket's deadline and controller may move.** An extended ticket
  survives its old deadline and expires at the new one; after a `Reassign`
  only the new controller resolves, and a host response or pending tie-break
  computed for the old version is refused as stale.
- **Decision state is committed differently.** The admission batch is part
  of the decision commitment, so every checkpoint hash and state commitment
  of a run with decision state differs from 0.19.0 even without a new
  mutation; a boundary that opens nothing empties the batch.
- **Completion reports.** A dirty report grant with no observation head, or
  with a holder the engine refuses, leaves the dirty set at once, whatever
  the publication capacity, and every ready lease that reserved it gives its
  report up (0.19.0 re-woke the report system every boundary and rewrote the
  live root). Several completions of one holder that reserved one grant are
  all served, or all given up, by the boundary that takes the grant (0.19.0
  released one and left the others waiting). A lease completed after the
  head's revision waits for a newer head, with nothing waking for it.
- **Resource roots near their byte cap** reach no room (`NoRoom` skips, or
  `limit_exceeded` where a limit is measured) a little earlier, since the
  live root's identity declaration counts toward `max_state_bytes`.
- **A seal or a continuation checkpoint is refused with `ArchiveNotReady`**,
  the run unchanged, when a declared version does not exist at its cut. A
  run's own boundaries never cite such a version (every citation is proven
  when it is first persisted); a scenario that installs hot records or heads
  citing boundary versions the run never established still does, until the
  citing record is archived or replaced. In 0.19.0 the same states sealed
  and then failed on load.
- **Production refuses a wrong record kind before reading it**, as a
  recorded `invalid_domain_record` rejection, where 0.19.0 failed the
  boundary with `UndeclaredStateRead`.
- **Calendar boundaries.** A keyed system is not woken
  by an unmarked boundary of its cadence; calendar ingress due at one
  instant is still merged into one boundary, marks included; knowledge
  reservations count a keyed publisher as due only on its marks. A system
  without a key, and a host that never marks, behave exactly as in 0.19.0.
- **Retired keyed draws.** A request with a retired
  address is a fresh draw: it needs fresh operation evidence, records a new
  draw record under a new ID, returns the same value for the same bound and
  purpose, and is no longer refused as `RandomOperationConflict` for other
  evidence, bound, or purpose. A journal restore drops what retires before
  each draw it rebuilds, so a journal may draw one address again after it
  retired, and drawing it again before is still refused.

Upgrading from 0.19.0, for hosts and plugin authors:

- **Pin `=0.20.0`** for `canwu-api` and every extension. Nothing earlier
  loads.
- **A correspondence host** that worked around downstream gap G-72 by using
  persons as parties may now name a domain entity (holder policy `Allowed`)
  as sender, recipient, carrier, or seed holder; its prepared dispatch names
  a domain-entity sender by its domain record, and its commands for that
  entity use `DecisionOrigin::Institution`. A dead person can no longer
  carry. A host that withheld seeds or seizure reports for dead holders may
  keep doing so; the engine now refuses them itself. Check
  `KnowledgeSeedReceipt::refusal` wherever a receipt meant "seed installed",
  and send a corrected seed under a new key. `published_at` is now
  `settled_at`, and struct literals add `refusal`.
- **A movement host** adds `retime_later_legs` to every `DelayLeg` (a
  `DelayLeg` without it is an unknown shape: refused at admission as a
  command with `InvalidPayload`, ignored as an incident, with no movement
  outcome recorded), passes `retime_later_legs: true` to keep later legs on
  their planned spacing (in 0.19.0 they departed at the held arrival), holds
  a leg that has not departed with `DelayLeg` instead of a `Reroute`, reads
  `MovementLegHoldV1::scheduled_at` and `kind` for `scheduled_arrival_at`,
  adds `RevokeBooking` and `Revoked` arms to exhaustive matches of
  `MovementOperation` and `CapacityBookingStatus`, adds `replaces` (`None`),
  `schedule_shift`, and `revocation` to struct literals, lets a pool's
  custodian or an incident revoke a confirmed booking and the owner replace
  it with a request naming it, and no longer expects superseded leg-due
  packets in `pending_ingress()`.
- **A decision host** holds a ticket back with `ExtendDeadline` instead of
  cancelling and reopening it under a new ID, delegates an open ticket with
  `Reassign` (to a registered controller of the same decision maker, or of
  the same seat where the parent's lineage requires it), finds a boundary's
  new tickets with `decision_tickets_opened` from a boundary system that
  declares `StateKey::core_decisions()` (or the latest settled boundary's
  from `Canwu`) and a desk's with `open_decision_tickets_of_controller`,
  declares a doctrine's preference with `with_option_bias` rather than a
  factor (its `GuardedUtilityPolicy` then has a new semantic hash, so
  re-register the binding), adds `ExtendDeadline` and `Reassign` arms to
  exhaustive matches of `DecisionMutation`, sets `bias` in struct literals
  of `DecisionOptionEvaluation`, and names the public field `option_biases`
  in any struct literal of `WeightedUtilityEvaluator` (or builds it with
  `new` and `with_option_bias`).
- **`canwu-military` is rewritten.** The catalog, operation, combat,
  occupation, knowledge, provider-outcome, and ledger records, the
  `military-operation` stream, the `military_report` schema, the
  acknowledgement ingress, and the commands `OrderMarch`, `PlanOperation`,
  `Recruit`, `TrainAndEquip`, `Recon`, `PrepareAmbush`,
  `ExecuteSpecialOperation`, `EstablishOccupation`, `SetOccupationPolicy`,
  and `MilitaryAdministrationAction` are gone. A host installs a
  `CombatRulebookV1` (by scenario or `InstallRulebook` from an issuer the
  rulebook trusts), constructs `MilitaryPlugin::<R>::new()` with
  `ParametricStepResolver` or its own `StepResolver` type, chooses
  `with_step_cadence(Some(cadence))` or drives stepping with the trusted
  `Step { up_to }` command, registers forces with `RegisterForce`, places
  them on arrival with `PlaceForce`, opens engagements, sieges, and crossings
  by command, supplies condition and environment inputs per engagement-day,
  and applies the `engagement_summary` or the optional outcome signal.
  Commands name an `expected_host_revision` that steps never advance.
  Issuers are matched against a force's `controllers` (any `Issuer` kind) or
  the rulebook's `trusted_issuers`. A host that reads
  `canwu-military-reference-content` takes `synthetic_rulebook_v1()`; the
  two synthetic profiles are gone. No 0.19.0 military record loads.
- **A resource or production host** rebuilds its scenario roots with the
  0.20.0 types (`ResourceState::into_records` writes state format 6 and
  bucket format 2 with their declarations; a hand-built root whose
  declaration is not the computed one is refused at installation), gives
  every report grant a holder that can hold knowledge (a core organization,
  resource, route, or territory, or a person given as an entity, is refused
  at `install_report_grant`), reserves holder reports only for holders the
  engine accepts (a lease, single-step lease, or participant grant for
  another is a recorded `invalid_authority` rejection), records a provider's
  observation head at or after the completion's revision when it wants the
  mandatory report (otherwise the participant archives without one), cites
  only current versions of kinds the resource plugin reads in a
  `RecordObservation` head, may now cite technology asset bindings and
  application specs from production, and installs technology records that
  name a domain-entity holder or site as its record. A continuation
  checkpoint or seal taken mid-lease now loads.
- **A host with several calendars** sets
  `contract.calendar = Some("<calendar>".into())` on a system that should
  wake only on one calendar's periods and schedules that calendar's period
  starts with `schedule_calendar_marks`; it marks every calendar a keyed
  system should see, because an unmarked `schedule_calendar_boundary` wakes
  only unkeyed systems. Code that builds `BoundaryRecord` or
  `BoundarySummary` with a struct literal, or matches
  `IngressPayload::Calendar { cadences }`, adds `calendar_marks` (or `..`);
  `BoundarySystemContract::new` fills the two new contract fields.
- **A plugin that bounds its keyed-draw ledger** adds
  `KeyedDrawRetirement::new(stream, after)` to its contract's
  `keyed_draw_retirement`, with `after` no shorter than the time anything
  retries the address or cites the draw, and expects a retry after that to
  draw afresh. Declaring either feature changes the plugin descriptor, so a
  run is restored and replayed only with the same declaration. Nothing
  changes for a host or plugin that uses neither.
- **Saves.** Write a fresh continuation checkpoint, snapshot, or checkpoint
  journal with 0.20.0; nothing from 0.19.0 loads.

Known limits carried by this release, each stated in its proposal:

- Correspondence: `canwu-information` still pins a party: its dispatch
  `sender` and `intended_recipient`, delivery-attempt `recipient`, and
  access `holder` roles refuse a retired target, so a domain entity that sent
  or received a dispatch cannot be retired while those records exist; only a
  party that appears in correspondence records alone (a carrier, a seed
  holder, an opportunity candidate) is retirable. Every change to a party's
  entity record re-validates each correspondence record that references it,
  and a change whose referencing records pass the runtime's affected-record
  closure (16,384) fails its owner's boundary. Planning-time failures still
  fail the boundary (a carrier ledger whose network does not build, a
  missing recipient address, a ledger past the bounded query, a start with
  no route); the seed check covers only what one fact states. Correspondence
  plans with plain `plan_route` and keeps no `RoutingIndex` across planning
  reads. A refused seed spends its key. A delegating sender still learns of
  a seizure only from the application, and a withheld report is not
  redirected. An attempt in flight keeps its carrier. Many command-handler
  refusals of `initiate_correspondence_v1` and `resolve_correspondence_v1`
  are still raised as `InvalidDomainRecord`, which the engine does not count
  as an expected command rejection, so they fail the command's boundary as
  in 0.19.0 (a malformed key or profile, a zero execution or dispatch
  version, a deadline before admission, a prepared dispatch that is missing,
  stale, not prepared, not single-recipient, or of another channel profile,
  a sender that does not match it; a missing operation, an overlapping
  information operation, a recovery the state does not allow, a non-terminal
  attempt or inactive dispatch): engine finding E-49, pre-existing. Seeds
  that fit one directive but together pass the boundary's publication budget
  still fail the boundary, as do a domain-entity carrier deleted in the
  boundary its start or retry settles, an opportunity naming a missing or
  deleted party, and an interceptor the access `holder` role refuses
  ([known limits](proposals/correspondence-domain-parties.md#known-limits)).
- Movement: one schedule shift per order, so a re-timing hold moves every
  later leg of the revision by the same amount (a host that wants only some
  moved reroutes, and a reroute resets the shift); lateness no re-timing
  hold caused is absorbed as in 0.19.0; repeated holds of one due keep the
  first bound and the latest hold's `retime_later_legs` decides whether the
  whole time since that bound is re-timed; a booking requested before a
  re-timing hold keeps its window; a revocation settled in the boundary of
  the leg's departure due fails the leg (hold the departure in the same
  boundary to make it wait); only a confirmed booking can be revoked; pool
  custodians receive no movement report (E-43), so the host reads the
  runtime to name the bookings it revokes; replacements count against
  `bookings_per_execution` (16); a boundary that replaces a future due reads
  the plugin's cancellable ingress once (a walk of the run's pending ingress
  queue) and each withdrawal adds one cancellation record; holds still count
  against the outcome quotas (1,024 per command holder and 16,384 per
  runtime); `CompleteLeg` still arrives a held leg at once
  ([known limits](proposals/movement-holds-and-revocation.md#known-limits)).
- Decisions: a ticket without a deadline cannot gain one, and no mutation
  shortens or removes a deadline; `Reassign` re-checks lineage only against
  a hot parent and does not compare the two controllers' policies; the
  host's `decision_tickets_opened` shows only the latest settled boundary's
  tickets, so a host that settles several boundaries before reading keeps
  its own list or reads from a boundary system; there is no list by seat,
  definition, or deadline window, and the end-of-boundary sweep of tickets
  whose person became unavailable still scans the hot tickets; option biases
  are keyed by option ID, so a policy shared by ticket definitions that
  reuse an option ID applies the same bias to both
  ([known limits](proposals/decision-deadlines-reassignment-and-reads.md#known-limits)).
- Military: no failure-mode model (a host builds bounded contingency in its
  resolver); two sides per engagement, a third party fighting as a separate
  engagement at the same site; no automatic contact, hostility, detection,
  or surprise; vessels are components, with no boarding, wind, or current
  channel beyond what the host folds into component values; one wave size
  per crossing; reports are bands from `band_cuts`; retired records stay in
  the store and are validated at every load (about 2 KB per closed fight for
  the run's life); its keyed draws stay indexed for the run unless the host
  declares a retirement; an outcome signal whose target
  ingress no plugin registered fails the first closing boundary; a host
  resolver that panics is host code; and no second independent host has
  adopted the crate yet: together engine finding E-51
  ([known limits](proposals/military-engagements.md#known-limits)).
- Resource and production: the bucket declarations are not counted in
  `max_state_bytes` (each repeats evidence its entries already carry), the
  live root's declaration is; a version a hot record cites is kept as
  membership for as long as the record is hot, so a run that never archives
  keeps it until the candidate budget forces a commit; a reserved report
  whose grant has an older head waits for a newer one, the completed
  participant staying in the live root within the participant budget; a
  holder that becomes ineligible after its report was reserved has the
  report given up when it falls due, and one that becomes eligible again
  gets reports only once its grant is dirtied again; a host-supplied version
  of a kind the plugin does not read in a lease's or participant's locked
  target, an acquisition's envelope evidence, or a prepare's targets is
  still a skip with a trace or an admission refusal that leaves the
  operation key free; technology checks other than references (authority
  subjects, knowledge holders) are unchanged, and whether every technology
  operation accepts a domain-entity holder end to end is not covered by a
  test. External completion participants that end `Released`, `Expired`, or
  `Rejected` stay in `ExternalCompletionParticipantBookV1::grants` for the
  run, because each coordinator acknowledges a participant's end by reading
  it from the resource root's current version and the resource plugin learns
  of no acknowledgement; after 1,024 have accumulated
  (`MAX_PENDING_LEASE_ACQUISITIONS_GLOBAL`), `grant` refuses every new
  participant for the run as a recorded `limit_exceeded`. The fix is a
  retirement the coordinator triggers once it has acknowledged, a change to
  the production and force-supply coordinator protocols: engine finding E-50,
  pre-existing; they declare no evidence meanwhile
  ([known limits](proposals/resource-continuation-and-completion-reports.md#known-limits)).
- Calendar marks and keyed-draw retirement: a mark says
  that a period starts, not which one, so a system that needs the month's
  number computes it from the time and its own calendar; the number of marks
  a calendar ingress carries is not capped (the host is trusted); a boundary
  the host settles directly through `BoundaryRequest` has no marks and queues
  them as calendar ingress at the same instant instead. A conflict across a
  retirement is not detected (after `after`, reusing an address with other
  evidence, bound, or purpose draws instead of failing); retirement is by
  simulated time only, not by whether a record still cites the draw; each
  retiring draw adds one small entry to the retirement map while it is live,
  and memory falls only for streams that declare a retirement
  ([known limits](proposals/calendar-marks-and-draw-retirement.md#known-limits)).
- Every limit of the 0.19.0 entry stands except the stale leg-due packets
  of `DelayLeg` (engine finding E-48) and the correspondence half of the
  malformed connection knowledge (engine finding E-41), which this release
  fixes.

The hash algorithms are unchanged. The engine version in every checkpoint
hash changes, first-party plugins that take their plugin version from the
crate version report a new one, and the exact engine-version check rejects
0.19.0 saves; retain the 0.19.0 engine to read them.

Version 0.19.0 is a pre-1.0 minor release. Its changes come from the
packages that close the downstream gaps and engine findings filed against
0.18.0, each with its own proposal:

- [movement leg delay and current incident evidence](proposals/movement-delay-leg.md)
  (`canwu-movement`: downstream gap G-64, engine finding E-44);
- [pending ingress read](proposals/pending-ingress-read.md)
  (`canwu-sim`, `canwu-api`: downstream gap G-62);
- [resource transit, account place, and external inflow](proposals/resource-transit-place-and-inflow.md)
  (`canwu-resource`: downstream gaps G-63, G-65, G-67, and G-68 and engine
  finding E-45; downstream gap G-66 is closed by documentation only).

What changed, in one list:

- Movement gains a **leg hold**: the operation `DelayLeg { execution, leg,
  until, reason }` holds a departed current leg until an absolute instant
  no earlier than the settling boundary or the leg's **scheduled arrival**
  (its departure plus its planned duration), so later delays lengthen,
  shorten, or lift the hold but never make the leg faster. The owner, the
  operator, or an incident with current evidence requests it; only the
  latest hold is kept; owner and operator reports show it, and a delayed
  remote observer learns of it only from the arrival. `Reroute` stays
  refused while a leg is departed.
- Incident evidence and an order's delivery attempt must now be the current
  version of their record (a superseded version is refused even while
  retained), so a sealed run and its replay decide alike; an order whose
  delivery-attempt record is revised before it settles is refused, and the
  host resubmits it with the new version.
- `pending_ingress()` on `Canwu`, `CompactedCanwu`, and `CanwuRun` lists
  every queued `IngressRecord` in queue order, including the records a seal
  or a continuation restore pinned, so a host can list its pending commands
  after a load. A trusted-host read: it holds every source's payloads
  unfiltered. `CanwuRun` gains a required method.
- A resource transfer can lose part of its escrow and stay open (a **partial
  transfer loss**, `RecordTransferLoss`), and acceptance credits the demand
  with what is accepted, so a loss reopens that much of it; an allocation
  pass counts escrow in flight as drawn.
- A transfer carried by a multi-leg execution records each leg and handoff
  in order in its **link history** (`ResourceTransfer::link_history`, at most
  16 entries, 14 in transit), kept through completion and into the archive.
- The host application moves an account to another place with the host-only
  **account rescope** (`RescopeAccount`, queued with
  `enqueue_resource_account_rescope`), citing the current version of the
  record of the move.
- A custodian books value from outside the modelled accounts as a tracked
  **external inflow credit** (`Credit` with `ExternalInflow`), the
  counterpart of `ExternalOutflow`, under a host-issued single-step lease
  bound to exactly that credit: a lease's issuer can bind it to the one
  operation it authorizes (a **lease operation binding**,
  `CompletionOperationBindingV1`), and every redemption checks the binding.
  Every inflow credit's outcome keeps its evidence (E-45).
  `RESOURCE_STATE_FORMAT_VERSION` 5.

**Same version only.** Canwu crates depend only on the exact same version of
each other: every first-party crate requires `=0.19.0` of every other
first-party crate it uses, and a host pins `canwu-api = "=0.19.0"` and the
same version of every extension. Nothing written by 0.18.0 is read by
0.19.0. There is no migration and no decoding of an old shape: the exact
engine-version check refuses every 0.18.0 snapshot, checkpoint journal, paged
checkpoint, continuation checkpoint, replay journal, evidence segment, and
scenario that records its engine version, and a run with the movement or
resource plugin would also be refused by its semantic hash, a resource root by
its state format 5 as well. The engine's persistence formats
(snapshot 9, commitment 6, checkpoint/journal 6, paged checkpoint 7,
continuation checkpoint 3) are unchanged, so an old save is refused by its
engine version alone. A scenario built with 0.18.0 types is rebuilt with the
0.19.0 types: a movement runtime root installed with a departed leg gives
that leg its scheduled arrival as its arrival due, or a hold, and a resource
root installed with `ResourceState::into_records` encodes at state format 5
(an `ExternalInflow` credit source is a record version). Retain the 0.18.0
engine to read 0.18.0 data.

Formats and schemas that change, each deliberately versioned:

| Item | 0.18.0 | 0.19.0 | What changed |
| --- | --- | --- | --- |
| Snapshot, commitment, checkpoint/evidence-journal, paged-checkpoint, and continuation-checkpoint formats | 9, 6, 6, 7, 3 | 9, 6, 6, 7, 3 | shape unchanged; the engine version in every checkpoint hash changes; `pending_ingress` reads current state and changes no persisted field, journal, hash, or replay input |
| `MovementOperation` (command and incident payloads) | 10 variants | 11 variants | adds `delay_leg { execution, leg, until, reason }` |
| `MovementOrderRecordV1` (in the runtime record `canwu.movement:runtime`) | — | — | `hold: Option<MovementLegHoldV1>` (`leg`, `scheduled_arrival_at`, `until`, `reason`, `held_at`), omitted when `None`, so a runtime with no hold encodes as before |
| `MovementState::validate` | a departed leg has an arrival due | the arrival due time is checked | a departed leg's arrival due equals its scheduled arrival, or the hold's `until` when a hold exists; a hold names the departed leg, keeps the derived scheduled arrival, ends no earlier than it, was set between the departure and `until`, and has a bounded reason; no hold exists without a pending arrival. Applied at activation, at each load of the runtime, and to every candidate |
| `MovementReportDetailV1` (report payload) | — | — | `hold: Option<MovementLegHoldV1>`, omitted when `None`; a report without a hold, its digest included, is byte-identical |
| `MOVEMENT_SEMANTIC_HASH` | `2d3c7d8e…a383` | `6cf78f1f…550e` | BLAKE3 of the string recorded in the proposal |
| `movement_report` knowledge schema hash | `31cf5c80…1e3a` | `81cf8978…4f11` | BLAKE3 of the string recorded in the proposal; the schema id stays at version 1 |
| `movement_incident_v1` descriptor description | "…leg failure, reroute, or handoff" | "…leg failure, delay, reroute, or handoff" | part of the plugin descriptor |
| `RESOURCE_STATE_FORMAT_VERSION` | 4 | 5 | `ResourceTransfer` gains `link_history: Vec<ResourceTransferLinkV1>` (`progress`, `transport`, `evidence`), omitted while empty; an open transfer keeps its transport evidence in `link_history` only, its `exact_evidence` empty until its terminal disposition, and the payload-required continuation declares the history's evidence; `ResourceOperationRequestV1` gains `record_transfer_loss` and `rescope_account`, `ResourceOperationKind` gains `rescope_account`, and `ResourceCreditSourceV1::ExternalInflow` encodes a record version (was an evidence reference). Validation refuses a link history its advances could not have recorded (more than 16 entries or 14 in transit, a transition `advance_transfer` does not admit, a `transport` that is not the last link, an open transfer in a state its history does not leave or with `exact_evidence`, an accepted transfer accepted neither after an arrival nor locally before any link) and an applied `Credit` or `RescopeAccount` outcome without exactly one piece of evidence. An ended demand whose consumed leg an open transfer draws on stays in the live root, with its reservation and leg, until that transfer's terminal disposition; validation refuses an ended demand that is neither an archive candidate nor held back by the same rule the closure waits on (an active reservation, a reserved leg, or an open transfer drawn on its legs): a leaked closure |
| `EligibilityEnvelopeV1` (completion lease envelopes) | — | — | gains `operation_binding: Option<CompletionOperationBindingV1>` (`kind`, `quantity`, and an optional `disposition`, omitted while `None`), omitted while `None`, so an unbound envelope, its digest, and every 0.18.0 lease record encode as before; a bound envelope digests its binding. Lease book validation (restore and `validate`) also requires each certificate's `eligibility_envelope_digest` to equal its acquisition's envelope digest |
| Resource archive | blob format 1 | blob format 1 | an archived transfer carries its `link_history`; archive validation also refuses a terminal transfer whose accepted, lost, returned, and let-out quantities do not add up to its quantity, or whose link history is not valid. A loss of an open transfer is a hot terminal record from the moment it is recorded and may be archived before its transfer |
| Resource ingress | — | — | new internal ingress `resource_account_rescope_v1` (`RESOURCE_ACCOUNT_RESCOPE_INGRESS`); tracked commands admit an external-inflow `Credit` and `RecordTransferLoss`; adapter ingress admits `RecordTransferLoss` |
| `RESOURCE_SEMANTIC_HASH` | `56b1daef…6778` | `3a1e04cb…32ac` | BLAKE3 of the string recorded in the proposal |

Semantic identities that differ from 0.18.0: `canwu-movement` and
`canwu-resource`.
The other first-party plugins keep their semantic hash constants. As with any
version change, the engine version in every checkpoint hash changes and
first-party plugins that take their plugin version from the crate version
report a new one.

New and changed public contracts, by crate:

- `canwu-movement`:
  - `MovementOperation::DelayLeg { execution, leg, until, reason }`, a new
    variant (`MovementOperation::execution` returns its execution). A
    command from the order's owner or operator, or a `movement_incident_v1`
    incident (the incident allow-list gains it). Its checks, each a recorded
    rejection on the operation's outcome, in order: for an incident, the
    operation is on the allow-list (`Unauthorized`) and its evidence is the
    current version of its record (`Evidence`); the execution is known
    (`NotFound`) and the command holder is its owner or operator
    (`Unauthorized`); the reason is a non-empty, trimmed label of at most 256
    bytes (`InvalidOperation`); `leg` is the current leg and is `Departed`
    (`InvalidState`); `until` is no earlier than the settling boundary and no
    earlier than the scheduled arrival (`InvalidOperation`); `until` is
    within `MovementLimitsV1::time_bound_minutes` (`LimitExceeded`); the
    candidate runtime validates (`InvalidState`).
  - `MovementLegHoldV1 { leg, scheduled_arrival_at, until, reason, held_at
    }`, re-exported from the crate root, kept as `MovementOrderRecordV1::hold`
    (one per order) and shown as `MovementReportDetailV1::hold`. Each
    accepted delay replaces it; lifting a hold is `until ==
    scheduled_arrival_at`, and the lifted hold stays, with its reason and
    time, until the leg arrives. It clears when the leg arrives (by its due
    or by `CompleteLeg`), fails, or the execution closes. Re-stating a hold
    with the same `until` is accepted and updates the reason and time.
  - `MOVEMENT_SEMANTIC_HASH` has a new value.
  - `MovementIncidentV1::evidence` and `MovementOrderRequestV1::delivery_attempt`
    must be the current version of their record when the operation settles,
    refused otherwise as a recorded `Evidence` rejection ("incident evidence
    must be the current version of its record", "the delivery attempt must
    be the current version of its record").
  - New variant and new public fields: source-incompatible for exhaustive
    matches of `MovementOperation` and for struct literals of
    `MovementOrderRecordV1` and `MovementReportDetailV1`.
- `canwu-sim`: `Simulation::pending_ingress` and
  `CompactedSimulation::pending_ingress`, each `impl Iterator<Item =
  &IngressRecord>` over the scheduler's queue in queue order (due time,
  ingress class, priority from the highest, issue time, and ID), each key
  resolved to the retained record or the pinned one. A record leaves when a
  boundary admits it or a cancellation withdraws it; a record still queued
  at or before `now` (zero-delay work for the next boundary at the same
  instant) is listed; a `PluginCancellation` record is never queued, so it is
  never listed. Additive.
- `canwu-api`:
  - `Canwu::pending_ingress` and `CompactedCanwu::pending_ingress` (`impl
    Iterator<Item = &IngressRecord>`) and the required trait method
    `CanwuRun::pending_ingress` (`Box<dyn Iterator<Item = &IngressRecord> +
    '_>`, so the trait stays usable as `dyn CanwuRun`). Never on
    `SimulationView` or `CanwuViewer`; there is no lookup by ID. A type
    outside the workspace that implements `CanwuRun` adds it.
    Source-incompatible for such implementations only.
  - `CompactedCanwu::ingress_log` still returns the retained tail only; its
    documentation points to `pending_ingress`.
- `canwu-resource`:
  - `ResourceOperationRequestV1::RecordTransferLoss(ResourceTransferLossRequestV1
    { operation_key, loss_id, transfer, expected_transfer_revision, quantity,
    cause, at, completion_certificate })`: in every state that is not
    terminal, any number of times, the transfer keeps its state, `escrow`
    falls and `lost` grows by `quantity`, its revision advances, and a
    `ResourceLoss` with `account: None` and `transfer: Some(..)` counts as
    admitted loss; the outcome is of kind `Loss` (shared with `RecordLoss`).
    The quantity is above zero (`invalid_definition`) and below the escrow
    (`invalid_lifecycle`). The certificate locks the transfer at the expected
    revision and, for a domain-record cause, that record version, and the
    loss completes only its own lease. A tracked command comes from the
    transfer's controller (the source custodian, or the grantee of a
    transfer started under an access grant); adapter ingress carries it with
    the cause as provider source.
  - `AdvanceTransfer` admits `InTransit → InTransit` when the new
    `TransportExecutionLink` names the execution of the current link and a
    new leg not lower than the current one or a handoff the transfer has not
    recorded (`invalid_lifecycle` otherwise); each advance appends a
    `ResourceTransferLinkV1 { progress, transport, evidence }` to
    `ResourceTransfer::link_history`, at most `MAX_RESOURCE_TRANSFER_LINKS`
    (16) entries, 14 in transit, so an intermediate advance past that is a
    recorded `limit_exceeded` rejection, while an arrival, a return, and a
    terminal disposition are always admitted. `transport` stays the last
    link.
  - `ResourceOperationRequestV1::RescopeAccount(ResourceRescopeAccountRequestV1
    { operation_key, account, expected_account_revision, place_scope,
    evidence })` and `ResourceOperationKind::RescopeAccount`, queued only by
    the host with `enqueue_resource_account_rescope(run, due_at, &request)` on
    `RESOURCE_ACCOUNT_RESCOPE_INGRESS` (`resource_account_rescope_v1`). It
    needs no completion lease, advances the account revision, and keeps
    `evidence` in its outcome. Refusals, each recorded: the account is missing
    (`not_found`), the revision is stale (`version_conflict`), a transfer that
    is not terminal names the account (`invalid_lifecycle`), a local
    completion lease or an external participant grant locks its current
    revision (`invalid_lifecycle`), the evidence is not its record's current
    version (`version_conflict`; the helper also refuses it when queuing) or
    not of one of the plugin's adapter evidence kinds (`invalid_authority`). A
    tracked command carrying it is refused at admission (`InvalidAuthority`),
    adapter ingress refuses it, and a packet another plugin schedules on the
    ingress is skipped with `failed_settlement_checks`.
  - `ResourceCreditSourceV1::ExternalInflow` holds a `DomainRecordVersionRef`
    (was an `EvidenceRef`). A `Credit` with it is admitted as a tracked command
    from the account's custodian under a single-step lease bound to exactly
    that credit; adapter ingress keeps its 0.18.0 path. Every applied credit
    outcome keeps exactly one piece of evidence (the inflow's record, a
    production credit's source). A tracked production `Credit` is refused with
    a message naming the production output batch ingress.
  - `CompletionOperationBindingV1 { kind, quantity, disposition }` with `new`,
    `with_disposition`, and `admits`; `ResourceTransferDispositionKindV1`
    (`Accept`, `AcceptLocal`, `Lose`, `Return`, `ExternalOutflow`) and
    `ResourceTransferDispositionV1::kind`;
    `EligibilityEnvelopeV1::operation_binding` and
    `EligibilityEnvelopeV1::bound_to`. The issuer (the host through the
    completion ingress, or a coordinating plugin) binds a lease; a holder's
    tracked `Acquire` whose envelope carries a binding is a recorded
    `invalid_authority` rejection. Every redemption checks the binding: the
    same kind, the same disposition when the binding names one, and the
    quantity the operation moves (the credit, outflow, or loss quantity; the
    allocation leg's quantity for `Consume` and `BeginTransfer`, each leg's
    own for `BeginExchange`; the items' legs together for `ConsumeBatch`;
    the escrow a `CompleteTransfer` disposition settles); a mismatch is a
    recorded `invalid_authority` rejection.
  - `ResourceOperationRequestV1::completion_certificates` and
    `ResourceOperationRequestV1::loss_identity`. Additive.
  - New variants of the exhaustive enums `ResourceOperationRequestV1` and
    `ResourceOperationKind`, and new fields of `ResourceTransfer` and
    `EligibilityEnvelopeV1`: source-incompatible for exhaustive matches and
    struct literals.
- `canwu-economy-reference` (not published) builds its harvest credit's
  `ExternalInflow` from the record version directly, and that credit's
  outcome now keeps the harvest record as its evidence.

The source-incompatible changes are the new enum variant
(`MovementOperation::DelayLeg`), the new struct fields
(`MovementOrderRecordV1::hold`, `MovementReportDetailV1::hold`), and the
required `CanwuRun` method, the new resource variants
(`ResourceOperationRequestV1::RecordTransferLoss` and `RescopeAccount`,
`ResourceOperationKind::RescopeAccount`), the new resource fields
(`ResourceTransfer::link_history`, `EligibilityEnvelopeV1::operation_binding`),
and `ResourceCreditSourceV1::ExternalInflow` holding a
`DomainRecordVersionRef`. The release is therefore a minor version.

Version 0.19.0 also changes these behaviours, which are visible to existing
code and runs:

- **Current evidence in movement (E-44).** An incident's `evidence` and an
  order's `delivery_attempt` must be the current version of their record
  when they settle. 0.18.0 checked both with
  `domain_record_version_evidence_exists`, whose answer for a superseded
  version depends on retained evidence: an unsealed run and exact replay
  found it, a sealed run did not, so a boundary could decide differently in
  a sealed run and its replay. Now a superseded version is refused in every
  run, even while it is retained. An application that revises its evidence
  record between reporting an incident and its settlement cites the new
  version, or reports after the revision; an order whose delivery-attempt
  record is revised between the order's submission and its settlement is
  refused, and the host resubmits it with the new version. The order's
  authority basis already had to be current, and a reroute's
  `ExternalCondition` reads only the record's current version number, so no
  movement decision reads retained evidence.
- **A departed leg's arrival due is checked.** A movement runtime whose
  departed leg's arrival due is not its departure plus its planned duration,
  and that holds no `MovementLegHoldV1`, is now invalid: a scenario-installed
  runtime root (`MovementState::into_initial_record`) with such a leg is
  refused at load, and so is any restored runtime. Every runtime the plugin
  writes satisfies the rule, since a departure always schedules its arrival
  at the departure plus the planned duration.
- **Held legs.** A delay replaces the order's `pending_due`. Operations
  settle before due packets in one boundary, so a delay settled in the
  boundary of the arrival it replaces wins and the replaced arrival packet
  is skipped as stale; the stale packet of an earlier hold is skipped the
  same way at its time. `CompleteLeg` still arrives a held leg at once and
  `FailLeg` still fails it; both clear the hold. A later leg whose booking
  expires while an earlier leg is held fails with `capacity_unavailable`
  when it is due to depart, through the existing recorded leg failure. A
  delay creates no itinerary revision and no leg, so a resource transfer's
  `TransportExecutionLink` to the execution and leg stays valid.
- **Movement reports.** Owner and operator reports carry the hold, and its
  `held_at` is an event time of detailed reports only, so a delayed remote
  observer's report never changes for a hold. A report without a hold is
  byte-identical to 0.18.0's, so nothing is republished because of the
  change.
- **Pending ingress.** `pending_ingress` reads only current state; a sealed
  run, a continuation-restored run, and their unsealed twin list the same
  records in the same order.
- **Resource demands are credited with what is accepted.** Acceptance credits
  the transfer's demand with the escrow accepted, not with the allocation
  leg; without a partial loss the two are equal. A transfer lost whole,
  returned, or let out credits its demand nothing, as before.
- **Escrow in flight counts as drawn.** An allocation pass counts the escrow
  of the open transfers drawn on a demand beside its active reservations, in
  the quantity it still needs, in what remains of its minimum useful
  quantity, and in the guard that rejects it for its minimum, so it never
  allocates escrow in flight again (0.18.0 did, a transfer start having
  consumed the reservation) and never rejects a demand for its minimum while
  a transfer for it is in flight. A returning transfer (`ReturnPending`) is
  not in flight for its demand: it is never accepted, so its escrow is needed
  again as soon as it turns back. A loss or a return gives back exactly what
  it takes, which the next pass may supply.
- **A demand that ends in flight.** A demand cancelled, expired, or rejected
  for its minimum while a transfer drawn on it is open keeps its terminal
  status when the transfer is accepted: `fulfilled` counts what was delivered,
  a fulfillment record names it, and the demand rejoins no due, expiry, or
  dirty index. Its closure (the demand, its reservations, and its consumed
  legs) is held in the live root until no open transfer draws on its legs and
  becomes an archive candidate at that transfer's terminal disposition, so an
  acceptance never finds its allocation leg archived.
- **Amendments cannot undercut what is committed.** An `AmendDemand` that
  asks for less than the demand's fulfilled, actively reserved, and in-flight
  quantities together (returning escrow excluded) is a recorded
  `invalid_lifecycle` rejection. 0.18.0 compared `requested` with `fulfilled`
  only, so a later acceptance could fulfil the demand while a reservation was
  still active, or exceed it.
- **`Lose` adds to `lost`.** A `CompleteTransfer` with `Lose` adds the escrow
  left to `lost` (0.18.0 set it), so a transfer's partial losses and its
  final loss add up.
- **Tracked commands and leases.** A tracked command is refused at admission
  (`InvalidAuthority`), so it settles nothing and takes no operation key,
  when it redeems any lease whose expected participants or participant
  grants name another plugin (not only for an inflow), when its lease's
  binding does not admit it, and, for an external-inflow `Credit`, when its
  lease is unbound; the operation the lease was issued for then settles
  under its key. Another plugin's lease that appears only at settlement is a
  `failed_settlement_checks` skip, which takes no key. A holder's tracked
  `Acquire` cannot write a binding. A bound lease is redeemed only by its
  operation, on every path.
- **Held closures need room.** A terminal transfer disposition whose demand
  closure does not fit the archive candidates is a no-room skip, sent again
  when room returns, not a recorded `limit_exceeded` rejection.
- **Loss identities across the archive.** An account loss, a partial loss,
  or a `Lose` disposition whose loss identity an archived loss used is a
  recorded `idempotency_conflict` rejection, found by an archive lookup as
  for a lease identity.
- **Open transfer evidence.** While a transfer is open its transport
  evidence is in `link_history` only and `exact_evidence` is empty; a
  terminal disposition writes the evidence it cites to `exact_evidence`, as
  in 0.18.0, so readers of the acceptance evidence are unchanged.

Upgrading from 0.18.0, for hosts and plugin authors:

- **Pin `=0.19.0`** for `canwu-api` and every extension. Nothing earlier
  loads.
- **A movement host** adds a `DelayLeg` arm to exhaustive matches of
  `MovementOperation` and `hold` (`None`) to struct literals of
  `MovementOrderRecordV1` and `MovementReportDetailV1`; holds a leg under
  way with `DelayLeg` instead of failing and rerouting it; cites the current
  version of every incident evidence record and delivery-attempt record,
  and resubmits with the new version an order refused because its
  delivery-attempt record was revised before it settled; and gives a
  departed leg in a scenario-installed runtime root its scheduled arrival
  as its arrival due.
- **A host that shows its own pending commands** (a "sent, not yet
  applied" line) or deduplicates its adapters by payload reads
  `pending_ingress()` after a seal or a continuation restore, where
  `ingress_log` holds only the retained tail, and keeps only its own items
  (for example those with `cause == None`, which the boundary ledger counts
  as host ingress, or its own request IDs) before it shows anything to a
  player or an agent. A type of the host's own that implements `CanwuRun`
  adds `pending_ingress`.
- **A resource host that settles both legs of an exchange locally**
  (downstream gap G-66): two `AcceptLocal` completions that share one
  handover record cannot hold its lock at once, since a completion lease
  holds it from activation to completion. Settle the second in a further
  boundary at the same simulated instant (a host may settle several
  boundaries at one instant; take the second leg's lease once the first
  acceptance has settled, as each holder's lease cooldown allows), or give
  each leg its own handover record, since the lock is keyed by the exact
  record version. The engine is unchanged, by owner decision.
- **A resource host that lets a custodian book an inflow** queues the single
  step (`enqueue_resource_completion_operation` with
  `ResourceCompletionOperationV1::SingleStepLease`, `expected_participants`
  exactly `{"canwu-resource"}`) whose targets lock the account revision and
  the inflow's evidence record version and whose eligibility envelope is
  bound to that credit:
  `envelope.bound_to(CompletionOperationBindingV1::new(ResourceOperationKind::Credit,
  quantity))`. The custodian then sends the tracked `Credit` with
  `ResourceCreditSourceV1::ExternalInflow(record_version)` and that quantity;
  an unbound lease, or one bound to another kind or quantity, is refused at
  admission. A host that issues leases whose targets could serve another
  operation (an outflow, an inflow, and an account loss with a record cause
  lock the same targets) binds them too, with `with_disposition` for a
  `CompleteTransfer` lease.
- **A resource host that moves an account** finishes or returns the
  transfers that name the account and releases the leases that lock its
  current revision, then calls `enqueue_resource_account_rescope(&mut run,
  due_at, &ResourceRescopeAccountRequestV1 { operation_key, account,
  expected_account_revision, place_scope, evidence })` with the new scope
  (or `None`) and the current version of the record of the move, of a kind
  passed to `ResourcePlugin::new` as adapter evidence; the call refuses
  evidence that is not current. Reservations, demands, and access grants that
  name the account carry over.
- **A resource host or provider that follows a transfer** reads its legs
  and handoffs from `ResourceTransfer::link_history` (in order; `transport`
  is the last link), not from `exact_evidence`, which is empty while the
  transfer is open; an archived transfer carries its history. A provider
  sends an intermediate `AdvanceTransfer` for each later leg or handoff, and
  a part lost on the way as `RecordTransferLoss` under its own lease instead
  of booking the cargo as several transfers; a lease that locks the
  transfer's current revision for a terminal disposition blocks the loss's
  lease, so the host releases it and takes a new one for the revision the
  loss leaves.
- **Resource code to update.** `ResourceCreditSourceV1::ExternalInflow` takes
  a `DomainRecordVersionRef`; exhaustive matches of
  `ResourceOperationRequestV1` and `ResourceOperationKind` add the new
  variants; struct literals of `ResourceTransfer` and
  `EligibilityEnvelopeV1` name the new fields; code that assumed a demand is
  credited with its whole allocation leg reads the fulfillment; a requester
  that lowers a demand asks for no less than its fulfilled, actively
  reserved, and in-flight quantities together.
- **Saves.** Write a fresh continuation checkpoint, snapshot, or checkpoint
  journal with 0.19.0; nothing from 0.18.0 loads.

Known limits carried by this release, each stated in its proposal:

- Movement: `CompleteLeg` arrives a held leg at once, before its scheduled
  arrival if the owner or operator says so, as in 0.18.0. Each `DelayLeg`
  schedules one due packet and leaves the packet it supersedes queued until
  that packet's own due time, which can be as late as the time bound
  (`MovementLimitsV1::time_bound_minutes`); until then the stale packet is
  pending ingress like any other (a seal and a continuation checkpoint pin
  it, and `pending_ingress()` lists it), and at its time it wakes a
  boundary, one that would otherwise be empty, where it is skipped without
  a trace. Stale packets are bounded per live execution by its operation
  outcomes (1,024 per command holder and 16,384 per runtime, which also
  bound "any number" of delays), but they accumulate over a long run with
  many holds. Only the latest hold is kept; the reports already published
  keep each earlier one in the owner's and operator's knowledge. A booking
  expiry republishes a detailed report without moving `observed_as_of`. An
  incident whose evidence record, or an order whose delivery-attempt
  record, is revised before it settles is refused; the application cites
  the revision, and the host resubmits the order with it. A movement runtime
  root whose departed leg's arrival due is not its departure plus its
  planned duration, and that holds no hold, is refused at load, so a
  scenario installs a late leg only with its hold
  ([known limits](proposals/movement-delay-leg.md#known-limits)).
- Pending ingress: a trusted-host read of every source (host commands and
  packets, decision ingress, other plugins' internal packets,
  boundary-generated work, calendar and maintenance requests) with its
  payloads, unfiltered, so it must not reach an actor as it is; the host
  selects its own items. There is no lookup by ID or filter by source, so a
  host that needs one record filters the iterator, `O(queue)`, at one
  lookup per record in the retained journal or the pinned map. `CanwuRun`
  gains a required method, so a host type that implements it adds
  `pending_ingress`
  ([known limits](proposals/pending-ingress-read.md#known-limits)).
- Resource: every open transfer lives in the live root, which each boundary
  that settles a resource packet re-encodes. Measured once (JSON encoding of
  `ResourceTransfer`, identities of 20 to 30 characters; not a committed
  benchmark), an open transfer takes 506 bytes with no link, 895 with one,
  and 5,219 with the full sixteen, about 286 bytes a link, so 8,192 open
  transfers (the canonical `max_transfers`) with full histories take about
  43 MB of the live root (4.1 MB without links), inside the 256 MiB state
  cap but re-encoded at every settling boundary; longer identities grow it in
  proportion, and each open transfer also declares up to sixteen
  payload-required versions, up to 131,072 for the root. An allocation pass
  reads every open transfer to count escrow in flight, once a pass. An
  operation key can be taken before its lease exists: a tracked command that
  names a key no lease in the root carries yet is admitted, and its refusal
  at settlement takes the key, so an issuer that expects front-runs chooses
  keys a holder cannot predict or takes the lease before announcing its key.
  A redemption refused when it settles (an adapter packet, a disposition whose
  escrow differs from its binding, a tracked command whose lease changed
  after admission) takes its operation key, and the lease stays active until
  the host releases it. Binding is the issuer's choice except for a tracked
  inflow: an unbound lease for any other operation authorizes whatever its
  targets allow, as in 0.18.0. A transfer does not list its losses; each
  loss names its transfer and `lost` totals them. Partial losses per
  transfer are not capped beyond the escrow, and each takes its own lease
  and leaves five hot terminal records. A demand's closure (each leg, its
  reservation, and the demand: `2 × legs + 1` records) becomes archive
  candidates in one step, so a demand with more than
  `(max_archive_candidates − 1) / 2` legs (127 at the canonical 256, 8,191 at
  the 16,384 maximum) can never close: the operation that ends it is a
  permanent no-room skip (an acceptance, loss, return, or outflow of its last
  transfer, or a consumption in a batch) or a recorded `limit_exceeded` (a
  single consumption, a cancellation, or an expiry); nothing bounds a
  demand's legs below that, and each allocation pass adds at most one per
  source account.
  The link history keeps fourteen in-transit links, so a longer execution's
  later intermediate links are refused and stay in the movement root only;
  the engine checks only "not lower" leg order and does not compare an
  arrival's or a return's link with the current one. A rescope checks its
  evidence's currency and kind, not that the record shows the stock at the
  new place, and a detached `ResourceState::apply_operation` does not read
  rescope evidence. The engine sets no cap on inflow, and conservation
  totals stay global across resources and units (engine finding E-46). Two
  local acceptances on one handover record still cannot settle in one
  boundary (downstream gap G-66, declined)
  ([known limits](proposals/resource-transit-place-and-inflow.md#known-limits)).
- Every limit of the 0.18.0 entry stands; this release fixes none of them.

The hash algorithms are unchanged. The engine version in every checkpoint
hash changes, first-party plugins that take their plugin version from the
crate version report a new one, and the exact engine-version check rejects
0.18.0 saves; retain the 0.18.0 engine to read them.

Version 0.18.0 is a pre-1.0 minor release. Its changes come from the
packages that close the remaining downstream gaps and engine findings of the
0.17.0 runs, each with its own proposal:

- [recurring traversal, edge conditions, and the compiled routing index](proposals/recurring-traversal.md)
  (`canwu-routing`, `canwu-correspondence`);
- [society intergroup relations and settlement cost](proposals/society-intergroup-relations.md)
  (`canwu-society`);
- [law: aggregate caseload, plan records at scale, and bounded bookkeeping](proposals/law-aggregate-caseload.md)
  (`canwu-law`, `canwu-information`);
- [request outcome retention](proposals/request-outcome-retention.md)
  (`canwu-sim`, `canwu-api`);
- [fiscal archive and due-instant cost](proposals/fiscal-archive-and-due-instant-cost.md)
  (`canwu-fiscal`, one read on `canwu-sim` and `canwu-api`);
- [engine load and budget guards](proposals/engine-load-and-budget-guards.md)
  (`canwu-sim`: initial-scenario declarations checked at load, boundary
  ingress without an early refresh, knowledge publication reservations,
  request ID 0 refused at enqueue);
- [resource series scale](proposals/resource-series-scale.md)
  (`canwu-resource`: consumption batches, live buckets, cheaper archive
  commits);
- [reference archive receipts and operation room](proposals/reference-archive-receipts.md)
  (`canwu-force-supply-reference`, `canwu-economy-reference`);
- [production at series scale](proposals/production-series-scale.md)
  (`canwu-production`: scope roots, the batch command, per-root limits,
  residue archives, folded incident receipts);
- [runtime state copies](proposals/runtime-state-copies.md)
  (`canwu-sim`: persistent current state and scheduler, shared journals, a
  constant-time `fork`, a cheaper restore; no byte, hash, or API change);
- [paged restore budget and seal cost](proposals/paged-restore-budget-and-seal-cost.md)
  (`canwu-sim`, `canwu-api`: a host-supplied budget on every paged
  restore, the knowledge ledger paged, the compact body and hot decision
  state chunked, and a seal that hashes each sealed record once; paged
  format 7).

What changed, in one list:

- A route can follow a **recurring departure** pattern
  (`TraversalModel::Recurring`), a host applies **edge conditions**
  (closures and adjustments by departure instant) without rebuilding the
  network, and a compiled **routing index** (`RoutingIndex`) plans a query in
  about a hundredth of the 0.17.0 time. `ROUTING_ALGORITHM_VERSION` is
  `canwu-routing.v2`.
- Society gains **intergroup relations**: directed, typed relations between
  two **cohort classes** in a region, in their own record, revised by a
  public packet; organization relation kinds can be bounded by a vocabulary;
  a settlement writes the society root once; an opt-in **society outcome
  retention** bounds the cohort exchange ledger.
- Law settles **aggregate caseload** reports per jurisdiction and period into
  bounded records with a **clearance rate**; the plan record is written once;
  the stale-proposal check is budgeted; `max_provider_packets` defaults to
  256; an information **delegation ledger** per grant drops expired
  delegations.
- A tracked command request's exact outcome is kept for the run's **request
  outcome retention** (default 30 simulated days); a later request with a
  retired ID is refused as `RequestOutcomeRetired`. Continuation checkpoint
  format 3.
- Fiscal archive helpers take the run and read only the records a commit
  needs; the sealed cut comes from the run (`sealed_evidence_cursor`); the
  due instant allocates a third less and publishes a third of the payload
  (`fiscal_report` schema 6; plugin `0.5.0-experimental`).
- The engine refuses at load an initial-scenario declaration no seal can
  accept, computes the checkpoint hash once per boundary, lets a plugin hold
  a **publication reservation** of every boundary's knowledge budget, and
  refuses request ID 0 at enqueue.
- Resource settles up to 256 consumptions under one lease as a **consumption
  batch**, persists accounts, grants, definitions, and units in **live
  buckets** with a **custodian index**, retires finished leases, halves the
  journal of an archive commit, and refuses a host `Expire` at enqueue.
  `RESOURCE_STATE_FORMAT_VERSION` 4.
- The reference package archive folds acknowledged receipts into a digest
  chain and skips an operation without room instead of failing the boundary;
  both reference roots are format 2.
- Production splits its state into **scope roots** (one `ProductionState`
  per host-chosen scope beside the primary root), settles many operations
  of one holder against one root as a **batch command**
  (`apply_production_batch_v1`), bounds every collection per root with
  `ProductionLimitsV1` fixed by the scenario (canonical 16,384 sites, work
  orders, and executions), and lets nothing a root settles stay for life:
  residues archive, incident receipts fold, rolled-out observation heads
  prune. `PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION` 3.
- The runtime holds its current state, scheduler, and retained journals in
  persistent, shared structures, so a `fork` and every rollback copy are
  constant-time (53 ms to 0.03 ms at 65,536 records) and a large unsealed
  restore is about a third faster, with every byte, hash, and public
  signature unchanged.
- Every paged restore takes a **restore budget** (`PagedRestoreBudget`)
  that bounds the bytes it assembles from pages, and refuses past it with
  `RestoreBudgetExceeded` before reading a value's chunks; a descriptor
  states its **assembled bytes**. The knowledge ledger is paged as two
  content-addressed maps and the compact body and hot decision state are
  chunked, so no part of a paged checkpoint is limited to one 4 MiB page and
  a world of 4,000 report holders now saves as a paged checkpoint (16,347
  pages, prepared in 89 ms, restored in 244 ms). A seal hashes each sealed
  record once: about 900,000 index entries in 3.3 s instead of 30 to 59 s,
  byte-identical. Paged-checkpoint format 7.

**Same version only.** Canwu crates depend only on the exact same version of
each other: every first-party crate requires `=0.18.0` of every other
first-party crate it uses, and a host pins `canwu-api = "=0.18.0"` and the
same version of every extension. Nothing written by 0.17.0 is read by
0.18.0. There is no migration and no decoding of an old shape: the exact
engine-version check refuses every 0.17.0 snapshot, checkpoint journal, paged
checkpoint, continuation checkpoint, replay journal, evidence segment, and
scenario that records its engine version, and several would also be refused
by their format or schema versions (continuation format 3, resource state
format 4 and its `bucket` record kind, legal shard payload format 2,
`fiscal_report` schema 6, the information delegation ledger, the reference
roots' format 2, paged-checkpoint format 7). The formats of a flat full save
(snapshot 9, commitment 6, checkpoint/journal 6) are unchanged, so an old
snapshot or checkpoint journal is refused by its engine version alone. A scenario built with 0.17.0 types is
rebuilt with the 0.18.0 types: a resource root with opening accounts is
installed with `ResourceState::into_records`, a society scenario may author
an intergroup relations record, and a law plan may declare a caseload
profile. Retain the 0.17.0 engine to read 0.17.0 data.

Formats and schemas that change, each deliberately versioned:

| Item | 0.17.0 | 0.18.0 | What changed |
| --- | --- | --- | --- |
| Snapshot, commitment, checkpoint/evidence-journal formats | 9, 6, 6 | 9, 6, 6 | shape unchanged; the engine version in every checkpoint hash changes |
| `PAGED_CHECKPOINT_FORMAT_VERSION` (descriptor, envelope, decision manifest and directory pages) | 6 | 7 | the descriptor gains `assembled_bytes`, the exact charge of an honest restore; the envelope is `{ format_version, body: PagedBytes, domain_records, knowledge: { bodies?, holders? }, decision_manifest_page_id }`: the compact checkpoint body is no longer embedded but named as 1 MiB chunk pages (`PagedBytes { bytes, chunks }`), the knowledge ledger leaves the envelope and is paged as two Patricia maps under `canwu.paged-checkpoint.knowledge.bodies.v1` (key the canonical JSON of the body ID, value the body) and `canwu.paged-checkpoint.knowledge.holders.v1` (key the holder, value `{ holder, next_local, entries }`), one leaf per body and per holder ledger, with the record store's codec (`raw-canonical-v2`), chunking, and load checks; the decision manifest's `hot_state` is `PagedBytes` in place of `hot_page_id`; no part of a paged checkpoint is limited to one page. The evidence segment, its archive index, every receipt, the continuation checkpoint, and every hash are unchanged |
| `CONTINUATION_CHECKPOINT_FORMAT_VERSION` | 2 | 3 | `RequestIndex` gains `retired_commands: Vec<RetiredRequestRange>` (canonical, sorted, disjoint, non-adjacent inclusive ranges of retired request IDs); `commands` holds only outcomes not retired at the checkpoint's `now`, and `queued_commands` no receipt of a retired request; the restore refuses non-canonical ranges, a kept outcome, receipt, or unadmitted attempt inside a range, an outcome outside the retention, and a pending command request whose receipt is missing or whose ID is retired; the hash domain stays `canwu.continuation-checkpoint.v1` |
| `RunConfiguration` | — | — | `request_outcome_retention: SimDuration`, default 30 days, 0 to 366 days, omitted from the wire at the default, so a default configuration's semantic hash is unchanged; `RUN_CONFIGURATION_FORMAT_VERSION` stays 2 |
| `PluginDescriptor` | — | — | `knowledge_publication_reservation: Option<KnowledgePublicationReservation>`, omitted when absent, so every existing descriptor's bytes and hash are unchanged; a run restores and replays only with a plugin that declares the same reservation |
| `ROUTING_ALGORITHM_VERSION` | `canwu-routing.v1` | `canwu-routing.v2` | routes, costs, and refusals are those of v1 for every valid network whose arrival times are representable; `RoutePlan` gains `edge_conditions_digest: Option<String>` (skipped in JSON when `None`); `PlanningSnapshot::digest` is composed from the cut, the versions, and the new `RoutingNetwork::digest`, so snapshot and plan digests differ from 0.17.0; `TraversalModel` gains `Recurring { period, offsets, duration }` |
| `canwu-correspondence` plugin | — | — | `SEMANTIC_HASH` and the `routing_connection` knowledge schema hash change (its records hold v2 plans and composed snapshot digests; the payload accepts `{"type": "recurring", ...}`); the schema id stays at version 1 |
| Society root `canwu.society:state` | schema 2 | schema 2 | gains the optional `relation_vocabulary`, omitted while empty, so a root without one encodes byte for byte as before |
| Society relations record | none | `canwu.society:intergroup-relations`, schema 1 | `IntergroupRelations { schema_version, kinds, edges }`, at most `MAX_INTERGROUP_RELATIONS` (4,096) edges; exists only when a scenario authors it or a revision creates it; binds no record references |
| `canwu-society` plugin descriptor | — | — | loses the systems `evaluate-mobilization-candidates`, `aggregate-social-state`, and `materialize-society-projections`; gains `intake-intergroup-relations` (phase 7, event-driven), the public ingress `intergroup_relation_revision_v1`, the event `society_intergroup_relation_rejected_v1`, and the relations record schema; `intake-cohort-transfers` reads the ledger; the four pairs of transfer settlement mode and outcome retention have their own semantic hashes, all new |
| `LEGAL_SHARD_PAYLOAD_FORMAT_VERSION` | 1 | 2 | the plan record holds only the compiled plan, its hash, and its budgets, declares no evidence, and is written once; the ledger counters move to the coordinator shard and the identity-evidence declaration to the directory; order and jurisdiction shards and their archive heads are created when they first hold something; `LegalRuntime` serializes without `plan` |
| Legal caseload records | none | `LEGAL_CASELOAD_FORMAT_VERSION` 1 | `canwu.law` `caseload_head` and `caseload_period` (`law_record_schemas` returns six schemas; the two new ones declare no evidence); a plan without a caseload profile keeps its encoding and hash |
| `LawBudgets::max_provider_packets` | default 4,096 | default 256 (`DEFAULT_PROVIDER_PACKETS`) | the default changes every compiled plan's content hash; `LAW_SEMANTIC_HASH` changes |
| Information `delegated_authority` record | create-only, one per delegation command | `Versioned`, one per command grant | payload `DelegatedAuthorityLedgerPayload { format_version, authority_grant, delegations, expiring }` (`DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION` 1) at `delegated_authority_reference(grant)`; the plugin semantic hash changes |
| `fiscal_report` knowledge schema | 5 | 6 | a part lists its rows' institutions once in an `institutions` table and encodes each row and fact as an array; version 5 is no longer registered, so a scenario whose knowledge carries one is refused; the summary, the lines, and the part digests are unchanged |
| `canwu-fiscal` plugin | `0.4.0-experimental` | `0.5.0-experimental` | a new semantic hash; `FISCAL_RUNTIME_SCHEMA_VERSION` stays 6 and every fiscal record format is unchanged |
| `RESOURCE_STATE_FORMAT_VERSION` | 3 | 4 | the state gains `live` (a `ResourceLiveLedgerV1` counting what the live root no longer holds) and `custodian_accounts`; `ResourceConsumption` gains `fulfillment`; the live root `resource:runtime:v1` holds no account, access grant, definition, unit revision, or custodian index; the encoded limits name `max_batch_items_per_boundary` instead of `max_mutations_per_boundary`; the adapter ingress accepts `ConsumeBatch`, the skip reason `boundary_budget` is new, and the deferred packet may wrap an adapter packet; `RESOURCE_SEMANTIC_HASH` changes |
| Resource bucket record | `canwu.resource` `hot-terminal` (`ResourceHotTerminalBucketRecord`) | `canwu.resource` `bucket` (`ResourceBucketRecord`) | one record kind for both bucket families: `ResourceHotTerminalBucketV1` under `resource:hot-terminal:{n}` and the new `ResourceLiveBucketV1` (`RESOURCE_LIVE_BUCKET_FORMAT_VERSION` 1, entries in ascending key order) under `resource:live:{n}`; a hot bucket holds about two records (`RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET`), so the bucket count is half the candidate budget rounded up to a power of two and records land in other buckets than in 0.17.0; the byte cap counts the live entries through the ledger |
| `ForceSupplyStateV1`, `EconomyReferenceStateV1` | 1 (economy), unversioned (force) | 2 (`FORCE_FORMAT_VERSION`, new; `ECONOMY_FORMAT_VERSION`) | each root gains the required `archive_receipt_chain: PackageArchiveReceiptChainV1 { issued, folded, digest }`; `FORCE_SUPPLY_SEMANTIC_HASH` and `ECONOMY_SEMANTIC_HASH` change, and each apply system declares an `operation_skipped` event; archive objects, lookup pages, and the commit and acknowledgement packets keep their shape |
| Production roots | one root `canwu.production:runtime` | the primary root and one scope root `canwu.production:runtime:scope:{scope}` per scope | `ProductionState` gains `scope` (absent on the primary root, whose encoding is otherwise unchanged), `limits` (`ProductionLimitsV1`, `Default` is `canonical()`, validated against `maximum()`), and `incident_history` (`ProductionIncidentHistoryV1 { folded_receipts, digest }`, a hash chain of folded incident receipts); `ProductionLimitsV1` replaces `max_mutations_per_boundary` with `max_batch_items` and `max_operations_per_boundary`, adds `max_observation_rollover`, and drops `max_incident_receipts`; the new record kind `canwu.production` `boundary-activity` (`ProductionBoundaryActivityV1`, written by the lifecycle system and trusted only as the settling boundary wrote it); every packet carries a `scope` member; a scope root's lease and output keys begin with `{scope}/` and the primary root's hold no `/` |
| `PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION` | 2 | 3 | blobs gain `residue_records` (`ProductionResidueArchiveRecordV1`: ended work orders, ended leases that bound no execution, unclaimed outcomes); the archive key gains `WorkOrder`, `CompletionLease`, and `OperationOutcome`; archived executions gain `decision_receipts`; directories and the head gain `archived_residue_count`; maintenance receipts gain `archived_residues` and `duplicate_settlements` (one receipt per handle for repeated stale settlements); prepared batches and commits gain `scope` and `selected_residues`; the archive source root domain is `canwu.production.archive-source-root.v2` and commits to the scope; `ProductionArchiveIngressReceiptV1` gains `scope`; the incident operation key names the root record |
| `canwu-production` plugin | — | — | `PRODUCTION_SEMANTIC_HASH` changes (BLAKE3 of the string recorded in the proposal); the new packet `production_batch_v1`, the new event `canwu.production.batch_settled.v1`, and the activity record schema are registered; `MAX_HOT_RECEIPTS` is removed |
| Engine state copies | — | — | no snapshot, checkpoint, journal, hash, or replay input changes: the world-scale snapshot is byte-identical before and after ([runtime state copies](proposals/runtime-state-copies.md)) |

Semantic identities that differ from 0.17.0: `canwu-society` (four hashes,
one per pair of transfer settlement mode and society outcome retention),
`canwu-law`, `canwu-information`, `canwu-correspondence`, `canwu-fiscal`
(version and hash), `canwu-resource`, the `canwu-force-supply-reference` and
`canwu-economy-reference` plugins, and `canwu-production`. `canwu-military`,
`canwu-movement`, `canwu-technology`,
and `canwu-culture` keep their semantic hash constants. As with any version
change, the engine version in every checkpoint hash changes and first-party
plugins that take their plugin version from the crate version report a new
one.

New and changed public contracts, by crate:

- `canwu-routing`, re-exported by `canwu-api`:
  - `TraversalModel::Recurring { period, offsets, duration }`: departures at
    `EPOCH + k × period + offset` for every integer `k` and every offset,
    each of `duration`; `period > 0`, `duration > 0`, 1 to
    `MAX_RECURRING_OFFSETS` (4,096) strictly increasing offsets in `[0,
    period)`, refused as `InvalidNetwork`. Source-incompatible for
    exhaustive matches. `TraversalModel::next_departure(at)` is public: the
    earliest departure at or after `at` and its duration, `O(log n)` for
    every scheduled model.
  - `EdgeConditionSet::new(overlays)` over `EdgeConditionOverlay { id:
    EdgeConditionRef, connection, window_from, window_until, effect }` with
    `EdgeConditionEffect::Closed` or `Adjusted { duration_per_mille,
    resource_cost_per_mille, risk_per_mille }`; at most `MAX_EDGE_CONDITIONS`
    (65,536) overlays and `MAX_EDGE_CONDITIONS_PER_CONNECTION` (1,024) per
    connection, unique ids, no overlapping adjustments on one connection,
    `duration_per_mille` 1 to `MAX_CONDITION_PER_MILLE` (1,000,000); refused
    as `RoutingError::InvalidConditions`, a new variant. An overlay naming a
    connection the queried network lacks has no effect.
  - `RoutingIndex`: `new(snapshot)`, `reobserve(topology_version,
    timetable_version, observer, observed_at, valid_until, knowledge_cut)`
    (another revision than the index's is refused as `InvalidSnapshot`),
    `plan_route`, `plan_route_with_conditions`, `unmatched_conditions`,
    `digest`, `network_digest`, `network`, the cut accessors, and
    `to_snapshot`; `Send + Sync`, cheap to clone. `RoutingNetwork::digest`
    and `RoutingCache::index_key(index, conditions, request)` are new.
    Additive.
  - `plan_route` keeps its signature; it compiles a borrowed index per call
    and now refuses a hand-assembled network that `RoutingNetwork::new`
    refuses (`InvalidNetwork`). `RoutePlan.edge_conditions_digest` is new
    (struct literals name it).
- `canwu-correspondence`: no public type changes; plans with plain
  `plan_route`; `SEMANTIC_HASH` and the `routing_connection` schema hash are
  BLAKE3 of the strings recorded in the proposal.
- `canwu-society`:
  - `CohortClass { dimension, value }` (authored text of at most
    `MAX_COHORT_CLASS_TEXT_BYTES`, 4,096 bytes); `IntergroupRelationEdge {
    id, kind, source_class, target_class, region, acceptance_per_mille,
    tension_per_mille, trust_per_mille, evidence, revision, revised_by,
    revised_at }`; `IntergroupRelations { kinds, edges }` with
    `into_record`; `IntergroupRelationsRecord`;
    `intergroup_relations_reference()`; `load_intergroup_relations(&canwu)`;
    `IntergroupRelationRevisionV1 { changes }` with
    `IntergroupRelationChange` (the edge in full, with `expected_revision`:
    zero to create it, otherwise its current revision) for the public
    ingress `intergroup_relation_revision_v1` (1 to
    `MAX_INTERGROUP_RELATION_CHANGES`, 1,024, changes in ascending ID order);
    `MAX_INTERGROUP_RELATIONS` (4,096), `MAX_INTERGROUP_RELATION_EVIDENCE`
    (16), and the other `INTERGROUP_RELATION_*` constants;
    `SocietyState::cohort_classes()`. Additive.
  - `SocietyState::relation_vocabulary: BTreeSet<String>` (struct literals
    name it); while empty any canonical identifier is admitted as an
    organization relation kind.
  - `OutcomeRetention::{Keep, LatestSettlement}` (default `Keep`),
    `SocietyPlugin::with_outcome_retention` and `outcome_retention()`;
    `SocietyPlugin` gains a private field, so it is built through its
    constructors. The retention is plugin identity, like the cadence and the
    transfer settlement mode.
- `canwu-law`:
  - `LegalDefinition::caseload: Option<CaseloadProfileDefinition>` and
    `CompiledLawPlan::caseload` (struct literals add `caseload: None`;
    `CompiledLawPlan` implements `Default`). `CaseloadProfileDefinition {
    case_classes, review_levels, retained_periods, max_reports_per_boundary
    }` with `new`, `max_period_record_bytes`, and `max_state_bytes`; the
    profile compiles only if its records' widest encoding fits
    `MAX_CASELOAD_STATE_BYTES` (256 MiB). `CaseloadIntakeReport`,
    `CaseloadPeriod`, `CaseloadStock`, `CaseloadReviewFlow`,
    `CaseloadReviewQueue`, `AggregateCaseloadRecord` (with `clearance_bp:
    Option<u32>`), `LegalCaseloadHead`, `LegalCaseloadState`,
    `CaseloadRefusalReason` (fourteen stable codes), the record wrappers
    `LegalCaseloadHeadRecord` and `LegalCaseloadPeriodRecord`,
    `caseload_head_record_id`, `caseload_period_record_id`,
    `legal_caseload_head_reference`, `legal_caseload_period_reference`,
    `enqueue_caseload_intake(run, reports)` (`LAW_CASELOAD_INGRESS`,
    `caseload_intake_v1`), `load_legal_caseload(run, plan, jurisdiction)`,
    `LAW_CASELOAD_HEAD`, `LAW_CASELOAD_PERIOD`,
    `LAW_CASELOAD_REJECTED_EVENT` (`canwu.law.caseload_rejected.v1`),
    `LEGAL_CASELOAD_FORMAT_VERSION`, `MAX_CASELOAD_CLASSES` (64),
    `MAX_CASELOAD_REVIEW_LEVELS` (16), `MAX_CASELOAD_RETAINED_PERIODS`
    (120), `MAX_CASELOAD_REPORTS_PER_BOUNDARY` (16,384),
    `MAX_CASELOAD_EVIDENCE_PER_REPORT` (16), `MAX_CASELOAD_EVIDENCE_BYTES`
    (2,048). Additive.
  - `LawBudgets::max_provider_packets` defaults to `DEFAULT_PROVIDER_PACKETS`
    (256). `LegalRuntime`'s serialized form omits `plan`, and
    `LegalRuntime::from_state` sets it from the plan it is given.
    Source-incompatible for code that read the plan from the runtime's JSON.
- `canwu-information`: `DelegatedAuthorityLedgerPayload { format_version,
  authority_grant, delegations, expiring }`, `DelegationExpiry {
  expires_at, command }`, `DELEGATED_AUTHORITY_LEDGER_FORMAT_VERSION`;
  `delegated_authority_reference(authority_grant)` takes only the grant, and
  `DelegatedAuthority`'s payload is the ledger. Source-incompatible.
- `canwu-sim`, re-exported by `canwu-api`:
  - `RunConfiguration::request_outcome_retention`,
    `RunConfiguration::with_request_outcome_retention(SimDuration)`,
    `RunConfigurationSnapshot::request_outcome_retention()`,
    `DEFAULT_REQUEST_OUTCOME_RETENTION` (30 days),
    `MAX_REQUEST_OUTCOME_RETENTION` (366 days); validation refuses a longer
    retention. `ErrorCode::RequestOutcomeRetired`
    (`"request_outcome_retired"`): `process_command` returns it as a
    `CommandOutcome::Rejected` with `attempt_id: None`, `enqueue_command` and
    `enqueue_decision` as `Err`. `RetiredRequestRange { first, last }` and
    `RequestIndex::retired_commands`. `CONTINUATION_CHECKPOINT_FORMAT_VERSION`
    3. Additive, except for `ErrorCode` matches (the enum is
    `#[non_exhaustive]`).
  - `Simulation::sealed_evidence_cursor()` and
    `CompactedSimulation::sealed_evidence_cursor()` return the cut through
    the evidence the run no longer retains: the end of its last sealed
    segment, or the settled cut of the continuation checkpoint it was
    restored from; `None` while the run retains all its evidence (it never
    sealed, or it was restored with its whole journal). Additive.
  - `KnowledgePublicationReservation { records, holder_entries,
    payload_bytes }`, `PluginRegistrar::reserve_knowledge_publication`, and
    `KnowledgeReservedPlugin::new(&plugin, reservation)`, which registers a
    plugin and reserves for it; `PluginDescriptor::knowledge_publication_reservation`
    (struct literals name it). Registration refuses, with
    `InvalidPluginRegistration`, an empty reservation, a second one, a
    reservation of a plugin with no knowledge-writing boundary system, and
    reservations that together exceed `KnowledgeLimitsV1::CURRENT`'s
    per-boundary bounds; restore refuses such descriptors with
    `InvalidSnapshot` and another reservation with `PluginManifestMismatch`.
    `knowledge_publication_budget()` keeps its signature and now excludes
    every other held reservation's unspent share. Additive, except for the
    descriptor literal.
  - `register_plugin` refuses with `InvalidDomainRecord` an initial-scenario
    record of the plugin whose payload-required continuation or identity
    dependencies name an initial-scenario version, its own version, or are
    malformed, and every restore refuses such a record with
    `InvalidSnapshot`; the seal's per-record checks and the load share one
    function.
  - `enqueue_command` refuses request ID 0 with `InvalidPayload`, as
    admission does; it queues nothing.
  - Boundary-generated ingress (`ScheduleIngress`, `SchedulePluginIngress`,
    `CancelPluginIngress`, boundary-generated decisions) no longer refreshes
    the checkpoint hash mid-boundary. No public change; the checkpoint
    hash, state hash, and boundary hash after the boundary are unchanged.
  - Runtime state copies: no public type, signature, or accessor changes.
    `SimulationView::person` and `army` still return `Option<&Person>` and
    `Option<&Army>`, `person_availabilities` still yields references, and the
    journal accessors (`events()`, `boundaries()`, and the rest) still
    return slices. Internally the current state and the scheduler are
    persistent `im` collections with rewritten heap values in an `Arc`, the
    retained journals are `SharedJournal<T>` (an `Arc<Vec<T>>` copied on
    first change), the command-history index is persistent, and the bound
    initial scenario is shared; every rollback copy and `fork` share them.
- `canwu-api`:
  - `Canwu::sealed_evidence_cursor()`, `CompactedCanwu::sealed_evidence_cursor()`,
    and the required method `CanwuRun::sealed_evidence_cursor`; a type
    outside the workspace that implements `CanwuRun` adds it.
    Source-incompatible for such implementations only.
  - `Canwu::from_paged_checkpoint_and_journal` and
    `from_paged_checkpoint_and_journal_with_plugins`, and the same two on
    `CompactedCanwu`, take a new last parameter `budget: PagedRestoreBudget`
    (on `canwu-sim`: `Simulation::from_paged_checkpoint`,
    `from_paged_checkpoint_and_journal(_with_plugins)`,
    `from_portable_paged_checkpoint`,
    `CompactedSimulation::from_paged_checkpoint_and_journal(_with_plugins)`,
    and `PersistentDomainRecordStore::from_state_pages`).
    `PagedRestoreBudget { max_assembled_bytes: u64 }` with `new`,
    `DEFAULT` (2 GiB, also `Default`), and
    `PagedSimulationCheckpoint::assembled_bytes` are new;
    `ErrorCode::RestoreBudgetExceeded` (`restore_budget_exceeded`) is a
    new code. Source-incompatible for every paged restore call.
  - Re-exports the new routing items, `PagedRestoreBudget`, `RetiredRequestRange`, the two
    retention constants, `KnowledgePublicationReservation`, and
    `KnowledgeReservedPlugin`.
- `canwu-fiscal`:
  - `prepare_fiscal_archive(run, book_limit, store)` is a free function
    taking any `CanwuRun`; `FiscalState::prepare_fiscal_archive` is gone.
    `enqueue_fiscal_archive(run, prepared, store)` loses its `sealed_cut`
    argument and re-derives the batch from the run;
    `finalize_fiscal_archive_retention(run, store, receipt)` keeps its
    signature and decodes the archive head alone. The refusal of a book
    citing a version established after the run's sealed cut
    (`FiscalArchiveError::UnsealedCitations`) now reads the cut from
    `sealed_evidence_cursor`. Source-incompatible.
  - `FiscalReportPart` keeps its fields and decodes the version-6 form;
    `FiscalReportRow` on its own keeps its object form; `assemble_fiscal_reports`
    fails to decode a part whose institution table lacks an institution a
    row names, in a host read only.
  - The plugin is `0.5.0-experimental`.
- `canwu-resource`:
  - `ResourceConsumptionBatchRequestV1` (1 to 256 items beside one consumer
    evidence, one instant, and one completion certificate;
    `ResourceOperationRequestV1::ConsumeBatch`,
    `ResourceOperationKind::ConsumeBatch`) with `item_operation_key` and
    `locked_targets`; `ResourceConsumptionBatchItemV1`;
    `MAX_RESOURCE_CONSUMPTION_BATCH_ITEMS` (256);
    `resource_consumption_batch_item_key(batch key, index)`
    (`RESOURCE_CONSUMPTION_BATCH_ITEM_KEY_PREFIX`);
    `RESOURCE_CONSUMPTION_BATCH_ITEM_RECORDS` (6),
    `RESOURCE_CONSUMPTION_BATCH_RECORDS` (2);
    `ResourceState::consumption_batch_can_fit(items)`;
    `ResourceIngressSkipReasonV1::BoundaryBudget`. New variants of
    exhaustive enums: source-incompatible for exhaustive matches.
  - `ResourceLiveRecordKeyV1`, `ResourceCustodianAccountsV1`,
    `ResourceLivePayloadV1`, `ResourceLiveEntryV1`, `ResourceLiveBucketV1`,
    `ResourceLiveLedgerV1`, `RESOURCE_LIVE_BUCKET_FORMAT_VERSION`,
    `RESOURCE_HOT_TERMINAL_RECORDS_PER_BUCKET` (2),
    `RESOURCE_LIVE_RECORDS_PER_BUCKET` (4), `resource_live_bucket_reference`,
    `resource_live_bucket_count`, `resource_live_bucket`;
    `ResourceBucketRecord` replaces `ResourceHotTerminalBucketRecord`;
    `ResourceRootReader::attach_live(keys)` and
    `attach_custodian_accounts(holder)`; `ResourceState::{live,
    custodian_accounts}` (fields) and `ResourceState::{live_records_attached,
    account_count, access_grant_count, definition_count, unit_count,
    total_account_balance, refresh_custodian_index}`;
    `ResourceConsumption::fulfillment`. Struct literals of `ResourceState`
    and `ResourceConsumption` break; a fixture that inserts accounts into
    `ResourceState::accounts` directly calls `refresh_custodian_index`.
  - `ResourceLimitsV1`: `max_mutations_per_boundary` is
    `max_batch_items_per_boundary` and is enforced (canonical
    `CANONICAL_BATCH_ITEMS_PER_BOUNDARY`, 32; hard
    `MAX_BATCH_ITEMS_PER_BOUNDARY`, 2,048); `max_definitions` and
    `max_unit_revisions` may be 16,384 (`MAX_DEFINITIONS`);
    `MAX_RESOURCE_ACCESS_GRANTS` is 16,384; `max_allocation_candidates` and
    `max_dirty_demands` keep their canonical 2,048 with a hard maximum of
    16,384 (`MAX_ALLOCATION_CANDIDATES`); `max_accounts` keeps 8,192
    canonical and 262,144 hard. Source-incompatible for struct literals and
    for code that read the renamed field.
  - `ResourceState::into_record` refuses a root holding accounts, grants,
    definitions, units, or a custodian index, as it refused hot terminal
    records; `into_records` is the authoring path. A live root decoded
    directly (`ResourceRuntimeRecord`) holds no accounts, so a reader
    attaches what it reads. Validation refuses an account missing from its
    custodian's index and a consumption naming another fulfillment.
  - `enqueue_resource_completion_operation` refuses `Expire` and
    `ExpireExternalParticipants`; `enqueue_resource_completion_expiry_tick`
    is the host's way to expire grants.
  - `resource_completion_status`, `resource_completion_certificate`, and
    `resource_completion_grant` no longer find a finished lease.
- `canwu-force-supply-reference` and `canwu-economy-reference` (not
  published): `PackageArchiveReceiptChainV1` (with `fold`),
  `validate_package_archive_maintenance`,
  `settle_package_archive_acknowledgement`,
  `package_archive_finalization`, `PackageRootShortageV1`
  (`archive_backlog`, `root_limit`), `PackageOperationSkipV1`,
  `PACKAGE_MAINTENANCE_STATE_BYTE_RESERVE` (128 KiB),
  `MIN_PACKAGE_STATE_BYTES` (512 KiB), `package_operation_state_bytes`,
  `FORCE_FORMAT_VERSION`, `FORCE_OPERATION_SKIPPED_EVENT`
  (`canwu.force-supply.operation_skipped.v1`),
  `ECONOMY_OPERATION_SKIPPED_EVENT`
  (`canwu.economy-reference.operation_skipped.v1`), `force_operation_skip`,
  `economy_operation_skip`, `force_operation_outcome`,
  `economy_operation_outcome` (a settled outcome, hot or archived),
  `encoded_len` and `outcome_room_shortage` on both states,
  `EconomyReferenceStateV1::exceeded_record_cap`.
  `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS` is 64 (was 8,192) and bounds
  unacknowledged receipts; both limits validators refuse `max_state_bytes`
  below `MIN_PACKAGE_STATE_BYTES`; `prepare_economy_archive` takes `force:
  Option<&ForceSupplyStateV1>`; `finalize_force_archive_retention` and
  `finalize_economy_archive_retention` return `Option<IngressReceipt>`
  (`None` once the acknowledgement settled). The force-supply settlement of
  a resource outcome attaches the outcome, the consumption (whose closure
  brings its fulfillment and account), the transfer, the participant, and
  the destination account by identity instead of reading the whole resource
  root.
- `canwu-production`:
  - New: `ProductionScopeId`, `production_scope_runtime_reference(scope)`,
    `production_root_reference`, `production_root_scope`,
    `ProductionState::runtime_reference`, `production_root_state`,
    `ProductionBatchEnvelopeV1 { holder, scope, expected_runtime_revision:
    Option<u64>, items }` with `ProductionBatchItemV1 { operation_id,
    operation }`, `PRODUCTION_BATCH_COMMAND` (`apply_production_batch_v1`)
    and `production_batch_command_descriptor`,
    `ProductionBoundaryActivityRecord`, `ProductionBoundaryActivityV1`,
    `ProductionRootActivityV1`, `production_boundary_activity_reference`,
    `PRODUCTION_BOUNDARY_ACTIVITY_ID`, `ProductionIncidentHistoryV1`,
    `ProductionResidueArchiveRecordV1`, `ProductionResidueBodyV1`,
    `ProductionCompletionLeaseArchiveV1`, `ProductionLimitsV1::maximum` and
    `validate`, `ProductionState::residue_archive_candidates`,
    `production_root_names_key`, and the `_in_scope` / `_in_root` variants
    of the completion and continuation enqueue helpers,
    `production_report`, `production_observation_witness`, and
    `degraded_facility_decision_ticket`. Additive.
  - Changed: `ProductionState` gains `scope`, `limits`, and
    `incident_history`; `ProductionLimitsV1` replaces
    `max_mutations_per_boundary` with `max_batch_items` (items one batch
    carries; canonical 1,024, maximum 4,096) and
    `max_operations_per_boundary` (operations and batch items a root records
    outcomes for in one boundary; canonical 4,096, maximum 65,536;
    `max_batch_items` above it is refused), adds `max_observation_rollover`
    (canonical 8,192), and drops `max_incident_receipts`; the canonical
    values rise to 16,384 sites, facilities, work orders, executions, and
    capacity allocations (maxima 65,536), 32,768 operation outcomes
    (maximum 131,072), and 16,384 observation heads, dirty entries, and
    archive-due entries (maxima 262,144 heads, 65,536 dirty and archive-due
    entries); `MAX_HOT_RECEIPTS` is removed;
    `acknowledge_production_archive_retention` returns whether it changed
    the root; `ProductionCommandEnvelope` gains `scope`; the archive key,
    blob, directory, head, receipt, batch, and commit shapes change as the
    table above says. Source-incompatible for struct literals and for code
    that read the removed limits.
  - Behaviour: every bound is a recorded rejection or a refusal with a
    trace; a command naming no installed root is a recorded command
    rejection (`InvalidPayload`), as is malformed content, a non-canonical
    payload (an explicit `null` or unknown member), and a degraded-facility
    choice without its persisted decision; a lease or output key without its
    root's scope prefix is a recorded rejection.

The source-incompatible changes are the new enum variants (`TraversalModel`,
`RoutingError`, `ResourceOperationRequestV1`, `ResourceOperationKind`,
`ResourceIngressSkipReasonV1`, `ErrorCode`), the new struct fields
(`RoutePlan`, `SocietyState`, `LegalDefinition`, `PluginDescriptor`,
`ResourceState`, `ResourceConsumption`, `ResourceLimitsV1`), the renamed
resource limit and bucket record, the fiscal archive helpers, the required
`CanwuRun` method, the information delegation payload, and the reference
helpers. The release is therefore a minor version.

Version 0.18.0 also changes these behaviours, which are visible to existing
code and runs:

- **Routing.** A departure slot whose arrival overflows the time range makes
  the connection unusable from that time instead of falling through to the
  next slot. `plan_route` and `RoutingIndex::new` refuse a hand-assembled
  network that `RoutingNetwork::new` refuses, where 0.17.0 planned over it.
  Conditions apply by departure instant: a traveller waits out a closure
  for the first departure at or after its end, and an adjustment scales the
  duration and resource cost of a departure inside its window, rounded up,
  and replaces its risk when given. Plans, costs, and refusals are otherwise
  those of 0.17.0 (checked on 40,000 random networks, and by the review on
  200,000). A correspondence carrier's `routing_connection` knowledge now
  accepts a recurring traversal; a malformed one still fails the boundary as
  `InvalidDomainRecord`.
- **Society settles in one write.** The phase-7 settlement materializes the
  derived views its changes cleared (mobilization candidates, aggregates,
  and the projections of observers whose estimates changed) before it writes,
  and the phase-10, -12, and -13 systems are gone, so a settlement journals
  one root record change, summarized "Settled society", where it journaled
  up to four. The committed root is byte for byte the root the four systems
  committed; only record versions and the number of record changes differ. A
  system of another plugin that reads the root in phases 8 to 13 of a
  settling boundary reads the derived views materialized, where it read
  them cleared. Rebases and transfers touch only their cohorts'
  distributions and the root is validated once before the write; the
  pending transfer record no longer holds transfers settled before the
  boundary that queues the next one.
- **Intergroup relations.** `intergroup_relation_revision_v1` applies in
  phase 7 of the boundary that admits it, each packet as a whole or not at
  all; every refusal (`malformed_payload`, `stale_revision`,
  `identity_conflict`, `unknown_kind`, `unknown_class`, `unknown_region`,
  `relations_full`) is one `society_intergroup_relation_rejected_v1` event
  and the boundary commits. A region is an identifier checked when an edge
  is created, not a record reference, so its owner can still retire or
  delete it; an edge is never removed. The settlement neither reads nor
  writes the relations. Run creation and restore check every class an edge
  names against the root's cohorts.
- **Society outcome retention.** Under `LatestSettlement` a settlement that
  records rebase or lifecycle delta outcomes first drops those recorded at
  an earlier time; transfer outcomes and pressure revision outcomes are kept
  under both retentions. The default `Keep` is the 0.17.0 behaviour.
- **Aggregate caseload.** A `caseload_intake_v1` packet's reports settle in
  phase 7 into one head and a fixed ring of `retained_periods` period
  records per jurisdiction (the period with ordinal `n` in slot `(n − 1) mod
  retained_periods`). Periods are contiguous, stocks are conserved, an exact
  repeat of the newest period is already settled and decided before its
  evidence is checked, every refused report changes nothing and is one
  `canwu.law.caseload_rejected.v1` event with a stable code, and a packet
  that does not decode, that the plan has no profile for, or that carries
  more reports than `max_reports_per_boundary` is refused whole with
  `canwu.law.ingress_rejected.v1`. The records declare no evidence, never
  load the legal runtime, and do not count toward the legal state budget.
- **Law plan record and provider index.** The plan record is written once
  and a law boundary decodes the plan once, by reference. The stale-proposal
  pass checks at most `max_mutations_per_boundary` dirty procedures in
  procedure-ID order and settlement defers the rest to a continuation
  boundary at the same time, so no procedure is advanced at a boundary that
  did not check it. The provider index keeps the newest 256 packets by
  default and forgets the lowest ingress ID first: a citation of a forgotten
  packet is refused with `canwu.law.ingress_rejected.v1`, also within one
  boundary that admits more provider packets than the index keeps.
- **Delegation expiry.** The lifecycle system removes, at every boundary it
  already runs, every delegation whose `expires_at` is at or before the
  boundary time, and does not record a claim already expired at admission.
  A delegation whose `expires_at` has come authorizes nothing even before it
  is removed: an interpretation admitted at or after `expires_at` is
  refused with `invalid_authority` even when it is dated inside the
  delegation window. A claim without `expires_at` stays for the run.
- **Late retries.** An exact retry of a tracked command request is answered
  with its original outcome or receipt while its attempt is inside the
  run's request outcome retention (default 30 simulated days after the
  attempt); after that it, and any other request with the ID, is refused
  with `RequestOutcomeRetired`, without an attempt, a revision, or an
  event. A retired ID stays used for the life of the run. A request with no
  attempt (a queued command still pending) is never retired. The rule is
  pure simulated time, so an unsealed run, a sealed run, a
  continuation-restored run, and an exact replay answer alike and write
  byte-identical continuation checkpoints; a seal only moves retired
  outcomes into the canonical ID ranges. A retired request cites no
  evidence dependency, so the dependency list, the kept archive receipts,
  and the provenance facts stay at one window's size. No authoritative
  state, hash, or journal changes.
- **Fiscal archive helpers.** `prepare_fiscal_archive` reads the root and
  the archive head, walks every ledger only far enough to see whether it
  holds a closed book, and decodes only the ledgers that hold the first
  `book_limit` closed books in (scope, cycle) order and their offices; it
  refuses `Backpressure` before it reads any ledger. The selection is the
  one the whole state would make. The sealed cut is the run's own, so a
  host cannot pass a wrong one; a run that retains all its evidence (every
  `Canwu`, a `CompactedCanwu` that never sealed, a run restored with its
  whole journal until its next seal) refuses nothing, so a host that wants
  every archived citation inside a sealed segment **seals at each save**,
  before it writes a continuation checkpoint.
- **Fiscal due instant.** The report system reads a book's figures without
  decoding the whole ledger, reads the ledger version a scope row cites for
  that row alone, copies each observer once, and encodes a part compactly.
  Report contents, lines, and part digests are unchanged, so nothing is
  republished because of the change; the publication budget charges less
  per part.
- **Initial declarations checked at load.** A scenario or save whose
  initial records declare evidence no seal can accept is refused at
  creation (`InvalidDomainRecord`) or restore (`InvalidSnapshot`) instead of
  starting a run that can never seal or write a continuation checkpoint. An
  initial record that names evidence which does not exist yet is still
  accepted.
- **Boundary ingress without an early refresh.** Settlement computes the
  checkpoint hash once at its end; a queued command admitted before the
  first phase keeps its per-admission refresh. A scheduled packet's ID,
  issue and due times, eligibility cut, queue key, and receipt are claimed
  exactly as before.
- **Publication reservations.** A boundary holds the reservations of the
  plugins with a knowledge-writing boundary system due in it, decided from
  the boundary's cadences and admitted events before the first phase; a
  system of another plugin is not shown, and cannot take, a held
  reservation's unspent share, whatever its dispatch order, while the
  reserving plugin reads its share through the unchanged
  `knowledge_publication_budget()`. A plugin publishing in phases 4 and 13
  has one share for both. Restore validation charges each knowledge change
  by the same rule. Overshooting what a system was shown still fails the
  boundary with `KnowledgeLimitExceeded`. Without reservations the figures
  are the 0.17.0 ones.
- **Consumption batches.** A batch enters only through its provider's
  adapter ingress, with a typed consumption intent per item under the
  item's reserved key. Items settle in order; each is refused alone
  (a rejected `Consume` outcome under its key, the leg still reserved) or
  consumed with its own outcome; several items may draw on one account
  (an item that expects the revision the batch found draws on the account as
  the batch left it); the batch outcome totals the consumed quantity with
  the refused quantity as `remainder`; the lease completes once and leaves
  the root. A batch with no items, more than 256, or a certificate that does
  not bind exactly its targets is one rejected batch outcome with the lease
  left active. Room is measured on the whole result: a batch that does not
  fit is skipped (`archive_backlog`, or `reserved_capacity` when no commit
  or release can make room) and records nothing. A batch that fits alone
  but not beside the batches the boundary already applied is deferred, with
  the `boundary_budget` skip event, to the next boundary at the same
  instant.
- **Resource live buckets.** A boundary attaches, before it settles a
  packet, every live record the packet names and every record those name;
  a pooled demand attaches every account; settling one operation writes
  the same records whatever the number of accounts, and creating an account
  writes its own bucket and its custodian's index. A release build refuses a
  boundary that would write over a live record it did not attach
  (`invalid_boundary`); a debug build also settles every boundary on the
  whole root and requires the same proposal.
- **Resource leases and expiries.** A finished lease (released or expired,
  its report reservation published) leaves the root at once; its receipts
  stay hot terminal records until an archive commit takes them, and the
  outcomes under its keys answer a resend. An acquisition identity still
  starts one lease for the run: a reuse is `idempotency_conflict`, at one
  more archive lookup per lease start. A host `Expire` or
  `ExpireExternalParticipants` is refused at enqueue; a plugin expiry that
  names another boundary than the one settling it is skipped with
  `failed_settlement_checks` and expires nothing early; an expiry that does
  not fit the root is skipped, never deferred.
- **Archive commits.** The resource plugin applies an archive commit by
  grouping the selected keys by bucket and rewriting or deleting each
  bucket once, without decoding the records it does not read; a hot bucket
  holds about two records.
- **Reference package archive.** The acknowledgement of a retention handle
  folds every receipt recorded under it, oldest first, into the root's
  digest chain; a repeated acknowledgement is a no-op; a finalization
  retried after its acknowledgement settled returns `None`. An operation
  whose outcome finds no hot slot, or whose result passes the share of the
  byte cap operations may fill (the cap less a 128 KiB maintenance reserve),
  is skipped with its own `operation_skipped` event (`archive_backlog` or
  `root_limit`), records nothing, keeps its ID free, and settles under the
  same ID once an archive commit made room and the host rebuilt the command
  against the current revision. An economy result past another bounded
  collection is a recorded `query_budget_exceeded` rejection. The economy's
  externality materialization waits for a hot slot and for the operation
  share; `prepare_economy_archive` leaves out an externality outcome the
  force-supply root still holds hot; `CloseMonth` reads its force-supply
  operation hot or archived.
- **Production scope roots.** A host installs one `ProductionState` per
  scope with the scenario, `scope` set, under
  `production_scope_runtime_reference(scope)`; the primary root (`scope`
  absent) is unchanged, so a small world needs nothing new. Every packet
  names its root (`ProductionCommandEnvelope.scope`, the batch envelope, a
  `scope` member on completion steps, witnesses, acknowledgements, and
  wakes, `scope` on archive batches and commits); an output acknowledgement
  is routed by the production source its execution pinned, and the resource
  plugin's participant acknowledgement by the participant's coordinator
  source. A packet naming no installed root is refused with
  `canwu.production.ingress_refused.v1`; a command naming none is a recorded
  command rejection. The lifecycle system loads each root a boundary's
  packets name once, settles them (archive commits first), writes only the
  roots that changed, and writes the boundary activity record naming each
  written root and the first packet that changed its production lifecycle;
  the later systems read that record alone, so a boundary with no production
  work reads one small record, and a boundary that touches many roots may
  settle, publish, and draw for each of them, since every per-root budget
  adds up across roots (only `max_observation_records` and
  `max_observation_records_per_holder` count across the world). Roots share
  no identity: a scope root's completion acquisition IDs and lease and
  output operation keys begin with `{scope}/`, the primary root's hold no
  `/`, and a scope ID holds no `/` (`production_root_names_key`), so no two
  roots can name one resource-side key; a key that breaks the rule is a
  recorded rejection, and every root's validation checks its hot leases and
  executions.
- **Production batch command.** `apply_production_batch_v1` carries many
  operations of one holder against one root. Admission checks the issuer's
  authority once, the root, at most `max_batch_items` items with distinct
  operation IDs, no `ResolveDegradedFacility`, and outcome room for every
  unsettled item; an item whose ID has another operation's outcome refuses
  the command (`IdempotencyConflict`). A batch whose
  `expected_runtime_revision` is set and is not the root's, or whose items
  would take the root past `max_operations_per_boundary`, is refused whole
  with a trace and records nothing, so it can be sent again; a batch without
  an expected revision has each item checked by its own rules against the
  root the items before it left, so a monthly settlement need not predict
  the revisions of packets other plugins land in the same boundary. Each
  item's outcome records the batch envelope as it applied, so a single
  command reusing an item's operation ID meets `IdempotencyConflict`; one
  `canwu.production.batch_settled.v1` event counts the applied, rejected,
  already settled, and refused items.
- **Nothing a production root settles stays for life.** Lease outcomes
  leave with the execution or project that bound the lease; a work order
  that ended without an execution, a lease released or expired without one
  (once every participant grant it recorded has ended), and any other
  outcome archive as residues, candidates after receipts, executions, and
  projects; incident receipts of earlier boundaries fold into the root's
  incident history at its first incident pass; a rolled-out observation head
  is pruned once a newer head of the same holder and site is eligible, on
  every pair the report system visits, and a materialization that would
  roll a head into a full rollover waits as dirty work. Reports are kept for
  life and outlive their heads; restore checks a report whose head left the
  root by its content.
- **Production refusals and duplicates.** A completion step, continuation
  witness, archive commit, or retention acknowledgement that production's
  checks refuse is refused with a trace, whoever queued or scheduled it;
  only a failed store read (`InvalidArchive`) and engine invariants fail the
  boundary. A stale commit under a handle that already has a stale receipt
  counts on that receipt's `duplicate_settlements`. A wake no longer runs
  incident evaluation; a recorded rejection, a continuation witness, and
  archive maintenance drive no incidents, dispatch, or reports; incident
  draws cite the trigger packet, and the incident operation key names the
  root record, so draws of two roots never share an address. Settlement
  checks each operation's outcome room before it applies it, so two
  commands admitted at one free slot no longer fail the boundary. The report
  system reads `knowledge_publication_budget()`, publishes what fits, and
  retries what did not on a wake a minute later; production reserves no
  share itself (register it through `KnowledgeReservedPlugin` to hold one).
- **Paged restores are budgeted.** Before a paged restore allocates any
  value it assembles from pages (every value of the four domain-record
  indexes, every knowledge body and holder ledger, the compact body, the hot
  decision state) it charges the value's canonical length to the host's
  budget; a value that would pass it fails with `RestoreBudgetExceeded`
  before any chunk of it is read, and nothing is restored. A descriptor that
  states more than the budget is refused before any page is read, and one
  whose `assembled_bytes` differs from what its pages assemble is refused as
  `invalid_snapshot`. The budget bounds assembled bytes, not heap: a restore
  holds the assembled values, the state decoded from them, and validation
  copies, and decoded JSON can take many times its text, so hostile pages
  inside a 2 GiB budget can peak at tens of GiB. `DEFAULT` is therefore a
  ceiling for a host's own checkpoints (four times the record ceiling,
  about ten times the world-scale fiscal state), not a bound for untrusted
  pages. The page loader refuses branch counts past the addressable range
  instead of overflowing, and a page graph naming one node page in two
  places is refused.
- **The knowledge ledger is paged.** A paged checkpoint builds the two
  knowledge maps from the live ledger when it is prepared (nothing is kept
  between checkpoints, so a boundary pays nothing); an unchanged body or
  holder keeps its leaf page, and a holder whose entries grow at the end
  keeps its leading chunks. The restore decodes the maps element by element
  through the ledger's own decoder, checks every value sits under its own
  canonical key, rebuilds both maps and requires the envelope's roots (so
  one ledger has one paged form), and then runs the full snapshot
  validation and commitment as before. A world of 4,000 report holders
  (a 9.8 MB ledger), refused in 0.17.0 as "state page bytes exceed the
  bounded page limit", now saves as 16,347 pages and restores in 244 ms;
  a one-day delta is 2,328 pages.
- **The seal hashes each sealed record once.** `seal_evidence` encodes each
  record once, streaming it into both its item commitment and the journal
  root, builds Merkle levels once, materializes a receipt (with its path)
  only for the dependencies the run keeps, no longer rebuilds the index to
  compare it with itself, and moves the index into the segment instead of
  cloning it. Segments, indexes, receipts, and hashes are byte-identical to
  0.17.0's; `verify_archived_segment`, used for every segment a host
  supplies, keeps its full rebuild and comparison. A seal of about 900,000
  index entries falls from 30.8 to 59.2 s to 3.3 to 3.4 s.
- **Runtime state copies.** `fork` is constant-time: it copies no state and
  no journal. The first append on either side while both still hold the
  shared journals copies them (about 53 ms at 65,536 records), so the pair
  costs what the fork alone cost before; a save fork that outlives the
  original's next boundary moves that copy onto the original's boundary,
  and a sealed run's tail keeps the copy small. A plugin registration's
  rollback copy costs nothing. The restore decodes by reference, checks
  unknown fields with a serializer walk that falls back to the full
  comparison whenever it cannot decide, so every refusal and its message
  are unchanged.

Upgrading from 0.17.0, for hosts and plugin authors:

- **Pin `=0.18.0`** for `canwu-api` and every extension.
- **A host that reads `IdempotencyConflict`** as "this request ID is already
  used, take another" accepts `RequestOutcomeRetired` the same way, from
  `process_command`, `enqueue_command`, and `enqueue_decision`. Choose the
  retention from the highest simulation speed and the longest client retry
  or reconnect delay (`RunConfiguration::with_request_outcome_retention`,
  up to 366 days); it is fixed when the run is created and cannot be raised
  later. Allocate request IDs consecutively (a counter the run's state
  restores, or the revision a batch is built against plus an offset), so
  retired IDs collapse into a few ranges; IDs hashed from keys cost about 59
  bytes each for the life of the run. A client that may be away longer than
  the window is answered from the outcomes the host received. After a
  restore, the next free ID is one more than the largest ID in the
  checkpoint's `retired_commands`, `commands`, `queued_commands`,
  `decision_commands`, unadmitted attempts, and pending ingress. After the
  next seal, `archived_evidence_receipt` and `load_archived_evidence_segment`
  no longer find a retired request's attempt, command, events, or ingress
  through the kept receipts.
- **A plugin that reads the provenance of a command ingress record**
  (`plugin_ingress_matches`) declares it as an identity-only dependency, as
  the evidence rule already requires; an undeclared read answers `false`
  while the record is retained and `EvidenceContentUnavailable` once a seal
  removed it. No first-party plugin reads one.
- **A fiscal host** calls `prepare_fiscal_archive(&run, book_limit, &store)`
  and `enqueue_fiscal_archive(&mut run, prepared, &store)` without a
  `sealed_cut`; a type of the host's own that implements `CanwuRun` adds
  `sealed_evidence_cursor`. A sealing host seals at each save. A host that
  decoded report parts itself reads the version-6 form through
  `FiscalReportPart` or `assemble_fiscal_reports`.
- **A resource host** installs opening accounts with
  `ResourceState::into_records`, never `into_record`; replaces
  `ResourceHotTerminalBucketRecord` with `ResourceBucketRecord`; names
  `max_batch_items_per_boundary` instead of `max_mutations_per_boundary`
  (and raises `max_archive_candidates` with it for full batches: a batch of
  256 items needs 1,539 candidates); sets `ResourceConsumption::fulfillment`
  in any consumption it builds; calls `refresh_custodian_index` after
  inserting accounts into a state directly; expires grants with
  `enqueue_resource_completion_expiry_tick`; and no longer reads a finished
  lease through `resource_completion_status`. A plugin that reads accounts,
  grants, definitions, or units from the root names them through
  `ResourceRootReader::attach_live` or `attach_custodian_accounts`, or reads
  the whole root; a reader of a consumption's fulfillment follows
  `ResourceConsumption::fulfillment` through `resource_root_in_view`
  instead of searching every hot fulfillment. A provider that settles many
  small consumptions authorizes one `ConsumeBatch` with a consumption intent
  per item, checks `consumption_batch_can_fit` before it takes the lease,
  and treats a `boundary_budget` skip as a deferral.
- **A routing host** keeps one `RoutingIndex` per network revision, moves it
  across read cuts with `reobserve`, plans with `plan_route_with_conditions`
  when conditions apply, and keys its cache with `RoutingCache::index_key`;
  `plan_route` still works and is faster. A host that stored
  `canwu-routing.v1` plans or snapshot digests treats them as another
  version's. A correspondence host validates a carrier's `routing_connection`
  payloads before publishing them, as before.
- **A society host** that read mobilization candidates, aggregates, or
  projections between phases 8 and 13 of a settling boundary now sees them
  materialized; a host that counted record-change events per settlement sees
  one. A scenario may author `IntergroupRelations::into_record` beside the
  root; a host revises relations with `intergroup_relation_revision_v1`,
  many per packet, and reads them with `load_intergroup_relations`. The
  society outcome retention, if chosen, is fixed before the first boundary; a
  struct literal of `SocietyState` names `relation_vocabulary`.
- **A law host** adds `caseload: None` to `LegalDefinition` literals, or a
  `CaseloadProfileDefinition` to report routine justice in aggregate; a
  host that cites provider packets long after admission raises
  `max_provider_packets` knowing each kept packet costs about 9 µs of every
  law boundary; a host that read the plan from `LegalRuntime`'s JSON reads
  the plan record. An information host that reads
  `delegated_authority_reference` passes only the grant code and decodes
  the ledger payload.
- **A plugin author** who needs a guaranteed share of the publication
  budget registers the plugin through `KnowledgeReservedPlugin::new(&plugin,
  reservation)` or calls `reserve_knowledge_publication` in `register`;
  the reservation is plugin identity, held at every boundary where the
  plugin's publisher is due. A plugin that declared an initial-scenario
  version as payload-required is refused at registration now, not at the
  first seal.
- **A reference-integration host** acknowledges each package archive commit
  it queued before it queues many more (64 unacknowledged commits fit), reads
  a skipped operation with `force_operation_skip` or `economy_operation_skip`
  and rebuilds the command against the current revision before sending it
  again, reads an archived outcome with `force_operation_outcome` or
  `economy_operation_outcome`, passes the run's force-supply root to
  `prepare_economy_archive`, and treats `None` from a finalize helper as
  done.
- **A production host** that needs more than one root installs one
  `ProductionState` per scope with `scope` set (roots are installed with the
  scenario; a run cannot add a scope), sets `limits` (or keeps
  `canonical()`), names the scope in every `ProductionCommandEnvelope`, in
  the batch envelope, and through the `_in_scope` enqueue helpers, prefixes a
  scope root's lease and output keys with `{scope}/` and leaves the primary
  root's keys without `/`, sends a monthly settlement as one
  `apply_production_batch_v1` per holder and root (`max_batch_items` at most
  `max_operations_per_boundary`), and keeps each root's archive lookup pages
  for the heads a restore, replay, or fork passes. Struct literals of
  `ProductionState` and `ProductionLimitsV1` name the new fields; code that
  read `max_mutations_per_boundary`, `max_incident_receipts`, or
  `MAX_HOT_RECEIPTS` reads the per-root bounds; a host that archives reads
  residues through the three new archive key variants. A fixture that drove
  incidents with wakes drives them with applied operations.
- **Every paged restore call passes a budget**, including a downstream
  host's own entry points (a page-store adapter or save-system wrapper that
  calls `from_paged_checkpoint_and_journal`, its `_with_plugins` form, or
  `from_state_pages`). For a checkpoint the host wrote, pass
  `PagedRestoreBudget::new(checkpoint.assembled_bytes)`, the exact charge
  of an honest restore; `PagedRestoreBudget::DEFAULT` (2 GiB) is a ceiling
  for the host's own larger checkpoints. For pages from a source the host
  does not trust, pass a budget near the size it expects (the
  `assembled_bytes` of the checkpoints it writes, with headroom) and
  authenticate the root page ID against one it recorded: the budget bounds
  assembled bytes, not heap. A host that kept its own checkpoint-size
  guard keeps the root check; the budget replaces the guard. Write fresh
  paged checkpoints with 0.18.0 (format 7); a page collector reads the new
  envelope's body chunks and knowledge roots and the manifest's hot-state
  chunks through `StatePageRetentionLedger` as before.
- **A world-scale host may save as a paged checkpoint again**: the knowledge
  ledger no longer needs to fit one page. The continuation checkpoint stays
  the save for a run whose evidence prefix is too long to verify on restore.
- **A host that saves from a fork** on a worker thread keeps doing so;
  `fork` is now constant-time, but if the save fork is still alive at the
  original's next boundary, that boundary pays the journal copy the fork
  used to pay (about 53 ms at 65,536 records on a full-history run; a
  sealed run copies only its tail).
- **Saves.** Write a fresh continuation checkpoint, snapshot, or checkpoint
  journal with 0.18.0; nothing from 0.17.0 loads.

Known limits carried by this release, each stated in its proposal:

- Routes depart as soon as possible at each node; there is no voluntary
  waiting, and an adjustment can make an edge non-FIFO, the `Piecewise`
  class. Conditions act on departures, not on a traversal under way; one
  duration per recurring pattern; a calendar month is not a fixed period;
  `reobserve` trusts the revision the caller names; the default expansion
  budget (10,000 pops, stale pops counted) and the unbounded `RoutingCache`
  are unchanged; `Departures` has no slot cap. Correspondence plans with
  plain `plan_route` (no conditions or index), hashes the holder snapshot
  per plan, and still fails the boundary as `InvalidDomainRecord` on a
  malformed `recurring` payload in a carrier's knowledge
  ([known limits](proposals/recurring-traversal.md#known-limits)).
- The society root is still one record rewritten whole by a settlement that
  changes it (10 MB and about 20 MB of evidence a day at 4,000 cohorts and
  two targets, half of it the materialized views); under the default `Keep`
  the ledger still grows with every rebase and lifecycle delta outcome, and
  transfer outcomes grow under both retentions (about 300 bytes each); the
  relations record is rewritten whole by a revision (about 30 to 40 ms and
  2 MB of evidence at 4,096 edges); society does not follow a region's
  lifecycle; edges are never removed, and observers get no estimate of a
  relation
  ([known limits](proposals/society-intergroup-relations.md#known-limits)).
- A law boundary on a plan of 1,555 jurisdictions still decodes the plan
  once (about 8 ms); the directory's object routes grow with hot legal
  objects; `max_provider_packets` (default now 256, stricter than 0.17.0's
  4,096) bounds how long after admission a provider packet can be cited,
  and the provider index costs about 9 µs per kept packet on every law
  boundary; a delegation claim without `expires_at` stays for the run,
  and a ledger is decoded whole when it changes or is cited; caseload
  history beyond `retained_periods` is the host's to keep
  ([known limits](proposals/law-aggregate-caseload.md#known-limits)).
- Request outcome retention: hashed or random request IDs stay unbounded at
  about 59 bytes per retired request; `RequestIndex::decisions` and
  `decision_commands` keep every decision request for the run;
  `evidence_times` grows with distinct instants (about 2.5 KB a month at
  smoke scale); between seals the pins of the last seal may still hold the
  provenance fact of a retired request's ingress, so an undeclared
  provenance read can answer `false` in a sealed run and fail in a run
  restored later; there is no read-only request status call
  ([non-goals and known limits](proposals/request-outcome-retention.md#non-goals-deferred-work-and-known-limits)).
- `enqueue_command` scans every retained and pinned ingress record for an
  exact re-enqueue (`O(retained ingress)` per call), recorded as a finding
  for a later release
  ([E-29](proposals/request-outcome-retention.md#e-29)); the seal's own
  cost is now about 2.6 µs an index entry (below).
- Paged restore and seal: the knowledge maps are rebuilt from the ledger at
  every paged checkpoint, `O(ledger)` encoding and hashing (about 90 ms for
  the 4,030-body run), though only changed pages are stored, and a restore
  builds them once more to check the roots; the budget bounds assembled
  bytes, not the restore's heap, and the restore still holds a decoded copy
  of the records beside the loaded store while the snapshot path validates
  them; a paged restore still needs, and verifies, the whole evidence prefix
  from the zero cut, so the continuation checkpoint remains the world-scale
  save; the seal still copies the sealed tail and encodes every sealed
  record once, about 2.6 µs an entry
  ([known limits](proposals/paged-restore-budget-and-seal-cost.md#known-limits)).
- The fiscal books are still summed per boundary from every scope of a
  court's partition (50,000 ledger reads at the due instant); the view's
  `current_domain_record_version`, called once per scope row, costs about
  75 ms there; the engine's own share of the due instant and of its heap
  peak (the record writes of 4,000 observers and 2,561 queue pages, the
  journal and commitments) is not reduced; a fiscal host that saves only by
  continuation checkpoint and never seals archives with no refusal in its
  first session and, after the restore, sees such citations refused until
  it seals; external-operation uniqueness is checked within a scope's hot
  books and resolved auditor proposals are capped at 64 per policy, each
  with its reason in the proposal
  ([deviations and limits left](proposals/fiscal-archive-and-due-instant-cost.md#deviations-and-limits-left),
  [the sealed cut](proposals/fiscal-archive-and-due-instant-cost.md#the-sealed-cut-comes-from-the-run)).
- A publisher that does not read the budget, or reads it and overshoots,
  still fails its boundary with `KnowledgeLimitExceeded`; a reservation is
  held at every boundary where its plugin's publisher is due, whether or not
  it publishes; two refreshes of the checkpoint hash remain inside a
  boundary before its first phase (a queued command's admission and a
  command handler's `EnqueuePluginIngress`), where they hash little
  ([engine load and budget guards](proposals/engine-load-and-budget-guards.md)).
- Resource: open demands with their reservations and legs, active transfers,
  leases, report grants, and observation heads stay in the live root, so
  `max_demands`, `max_transfers`, leases, and report grants bound what every
  changing boundary costs (about 790 KB of live root for 500 open household
  demands; an allocation pass over 8,192 due demands leaves a 12.6 MB live
  root); a pooled demand's allocation pass reads every account, about 30 µs
  each (2.0 s at 65,536), so the canonical `max_accounts` stays 8,192 and a
  host that raises it draws on exact accounts or grants; a holder's
  custodian index is one entry rewritten each time the holder gains an
  account (some 250 KB, journaled twice, at 10,000 accounts); one lease per
  holder per simulated minute bounds one holder's batches to one a minute; a
  live bucket holds about four records and is rewritten with each; an
  operation rewrites its outcome's hot bucket with the record already there;
  a batch item costs its boundary about 0.5 ms and 13 KB of journal, so the
  hard bound on batch items per boundary is 2,048; `resource_state`, archive
  preparation, and restore validation read every bucket
  ([known limits](proposals/resource-series-scale.md#known-limits)).
- Reference integrations: the force-supply plugin's acknowledgements of
  resource and externality outcomes, which other plugins schedule, are
  checked against the full byte cap, can fill the maintenance reserve, and
  one that passes the cap still fails its boundary; an externality outcome
  stays in the economy root while its force-supply saga is open or its
  terminal receipt hot; one commit still carries one page of at most 512
  records; superseded lookup pages are reclaimed only from each batch's list
  ([known limits](proposals/reference-archive-receipts.md#known-limits)).
- Production: an item of a batch, like an operation, costs one copy and one
  validation of its root (about 3.5 ms at 1 MB, 170 ms at 13.6 MB), so a
  root of several megabytes settles items slowly and roots are sized by
  scope; a key a host-queued resource operation takes after an execution
  started still strands the execution; roots are installed with the
  scenario and a run cannot add a scope; production reports keep
  `KnowledgeRetention::Keep`, so a holder keeps at most
  `max_observation_records_per_holder` reports for life and later reports
  wait; an activated lease that never starts an execution never ends; a
  re-sent operation whose outcome was archived is applied again, and once a
  cancelled order's residue is archived a re-sent `CreateWorkOrder`
  recreates the order; a forged folded incident receipt or chain is caught
  only by exact replay, and a forged activity record is ignored after the
  domain commit; each observation prune scans the whole rollover table
  twice, negligible at the canonical limits (64 reports × 8,192 heads) and
  heavy at the maxima (about 1,024 keys × 65,536 heads × 6 passes in one
  boundary), until the rollover is indexed by key; a host keeps each root's
  lookup pages for the heads a restore, replay, or fork passes
  ([known limits](proposals/production-series-scale.md#known-limits)).
- Runtime state copies: a fork, or a rollback copy, still costs one copy of
  the retained journals when both sides keep them and one appends, paid at
  the first append (about 53 ms at 65,536 records); a save fork alive at the
  original's next boundary moves that copy onto the original, and a sealed
  run copies only its tail. What is left of a 662 MB restore, about 29 s, is
  mostly validation of the evidence itself (about 14 s), the JSON parse and
  the drop of its tree (about 5 s), the decode (3 s), and the plugins'
  activation checks (the fiscal plugin's about 3.5 s); a sealed run's
  continuation checkpoint remains the save for long campaigns. A commitment
  root of the world, plugin-component, or scheduler domain still collects
  the domain's values into vectors when the domain is dirty
  ([what remains](proposals/runtime-state-copies.md#what-remains)).
- Every limit of the 0.17.0 entry that this release does not touch stands:
  plugin-scheduled resource work waiting on an archive backlog or on
  reserved room is retried, at most about once a day once its delay has
  stopped growing, until the host commits or releases, and the run cannot
  seal while a deferral waits; an activated lease never expires; a
  `Requested` acquisition holds its reservation until it is granted or
  aborted; a production output batch skipped for room, for a taken key, or
  for an output account that changed revision leaves its execution
  `CompletedPendingOutputSettlement`; a continuation runtime
  cannot produce a full snapshot, replay journal, checkpoint journal, or
  paged checkpoint, and its keyed-draw ledger grows for the life of the run;
  what a continuation restore verifies without history is covered only by
  the stored `continuation_hash`; R4 is a plugin obligation; a part of
  institution rows cites the office version and the number of books it
  summed; fiscal report retention clears a closed cycle's figures by
  default, the fiscal knobs are run-bound, an `Interval` report shows the
  state as of its last due boundary, calendar cadences are not expressible,
  and domain-entity report holders stay deferred; several dirty cycles owed
  at once take more than one boundary through the drain packet; the
  continuation gate proves state equality with the run's own registry; the
  Ming-validator item; and the `Send` runtime's `Sync` reliance. Dropped
  from the 0.17.0 and 0.16.0 lists, because this release fixes them: the
  knowledge ledger inside the 4 MiB envelope page of a paged checkpoint, and
  the envelope page bounding the compact body (both paged or chunked now); a
  paged restore allocating what the pages describe with no bound a host can
  set (the restore budget); the world-scale seal's 30 s of double hashing;
  the mid-boundary refresh; the plugin that spends the whole budget before
  information; the engine accepting an unsealable initial declaration; the
  request index's growth with every command ever settled and the monthly
  creep it caused; the due instant's uninvestigated heap peak and the
  fiscal side above its expectation; the whole-state fiscal archive helpers
  and the host-supplied sealed cut; the reference archive's 8,192-receipt
  cap and the economy root that stopped there; the never-retired
  `DelegatedAuthority` records, the quadratic stale-proposal pass, and the
  4,096-packet provider index; the resource items (the archive commit's
  per-record bucket rewrite, the whole-root decode per operation boundary,
  finished leases in the root, the force-supply settlement reading the whole
  root, and the host `Expire` refusal); the society ledger item (bounded by
  the opt-in retention); the production items (a re-sent operation whose
  outcome was archived is still applied again, but a forged plugin-scheduled
  packet is now a refusal with a trace, refused packets drive no incident or
  report, and a commit costs its selected records plus one pass over the hot
  outcomes); and the stable copy of plugin components and core maps for
  phases 1 to 9, `fork` copying every journal, and the per-command
  transaction checkpoint.

Headline measurements, each from its proposal and, where one exists, from
the reports under `benchmarks/baselines/` (release builds, one Windows
x86-64 machine shared with other builds throughout, so elapsed times show
the order of magnitude and byte and allocation counts are exact; the `world`
preset is 200 polities, 50,000 scopes, 4,200 institutions, and 4,000
observers, courts at `Institution` granularity, monthly `Interval` cadence):

| Measure | Workload | 0.17.0 | 0.18.0 |
| --- | --- | ---: | ---: |
| Routing, cold `plan_route` | `routing_scale` grid, 1,296 nodes and 5,040 connections, 72 ferries as 240 slots each | 30.0 ms | 3.5 ms |
| Routing, plan through a compiled index | the same grid | — | about 0.3 ms (compile about 4 ms, once per network revision) |
| Routing, recurring ferries and 568 conditions | the same grid; build the set, plan | — | 0.32 ms, 0.57 ms |
| Routing, rail chain | 512 nodes, 1,021 connections, 10 cold plans | 271.4 ms | 5.8 ms |
| Routing, snapshot JSON with recurring ferries | the same grid | 1.66 MB (slots) | 1.03 MB (recurring) |
| Society `Daily` settlement that moves people | 4,000 cohorts, two targets | 0.85 to 0.91 s, 3 to 4 root writes, 53 MB of evidence | 0.23 to 0.31 s, 1 write, 14 to 20 MB |
| Society queued rebases | 4,000 in one `Monthly` settlement | 32.5 s (66.5 s in month 2) | 0.46 to 0.92 s |
| Society ledger under `LatestSettlement` | 4,000 rebases a month | 1.4 MB a month for good | 1.40 to 1.42 MB every month |
| Intergroup revision | 4,096 edges, 1 to 1,024 changed | — | 29 to 41 ms, about 2.1 MB journaled |
| Law boundary settling one mutation | 1,555 jurisdictions, 1,400 institutions | 100.7 ms | 9.4 ms |
| Law records at activation | the same plan | 3,118 (plan 917 KB rewritten every boundary; directory 612 KB) | 6 (plan written once; directory 0.9 KB) |
| Caseload reports | 1,400 reports a monthly boundary, 12 kept periods | — | 60 to 100 ms, flat to month 14; 730 B a period record, 12.3 MB in all |
| Law boundary per kept provider packet | small plan, 256 / 1,024 / 4,096 packets | 2.8 / 10.0 / 40.0 ms | the same per packet; the default is now 256 |
| Continuation checkpoint at world scale | after months 1, 5, and 24 (institution courts, interval cadence) | 172 MB, about 221 MB, 293 MB | 171.6 MB, 205.45 MB, 206.30 MB (+0.85 MB from month 5 to 24; request index: one month of outcomes and about two retired ranges a month) |
| Compact checkpoint dependencies and kept receipts | smoke preset, months 1 to 12 | +2,520 a month | flat at 8,039 |
| Fiscal due instant, allocation-counting build | 4,000 observers due, 9,603 bodies | 6.52 M allocator calls, 472 MB heap peak, 15.7 MB of payload | 4.26 M, 306 MB, 5.5 MB |
| Fiscal due instant, elapsed in a fresh process | `--rows interval`, three alternating rounds | 1,827 to 4,054 ms (quiet: 1,259 ms) | 1,259 to 1,858 ms |
| Fiscal archive commit, host side | 256 and 4,096 books | 6.15 M and 6.90 M allocator calls; 1,402 and 1,813 ms | 98 K and 1.19 M; 166 and 501 ms |
| Held knowledge after 24 months | world preset | 14,138 bodies, 15.13 MB | 14,138 bodies, 5.67 MB |
| Mid-boundary ingress over many components | 20,000 components, 64 `SetComponent` + `ScheduleIngress` pairs | 1.8 to 2.1 s | 0.10 to 0.11 s |
| Resource operation boundary | 16,384 accounts, no backlog | 100 ms, 9.49 MB of journal | 1.27 ms, 8.3 KB |
| Resource operation boundary | 65,536 accounts, 16,000 hot records | — | 0.72 ms, 10.9 KB |
| Resource archive commit | 16,054 records beside 16,384 accounts | 713 ms, 30.3 MB | 370 ms, 17.1 MB |
| Consumption batch | 32 and 256 items, adapter ingress | — | 14 to 28 ms, 432 KB; 105 to 151 ms, 3.26 MB |
| Household consumptions, live root after | 500 households, one lease each / batches of 256 | 1.85 MB, 500 finished leases left | 262 KB, none left / 2.9 KB |
| Economy commit chain | settled commits, root size, operation boundary | refused at 8,192; 3.52 MB; 231 ms | 25,600 and flat; 37.3 KB; 3.7 ms |
| Reference roots after ten simulated years | monthly maintenance | force-supply 89.8 KB, economy 92.8 KB | 53.8 KB, 43.1 KB, flat from year 1 |
| Production, 64 single commands | `production-archive-profile --preset scale`, 0.99 MB root | 2.39 s apply (956 ms admission) | 188 ms (526 ms) |
| Production, one `CreateWorkOrder` | the same root: admission / apply | 211 ms / 288 ms | 12 ms / 57 ms |
| Production, scope batch of 64 items | a 400-scope world of 25,600 sites (13.4 MB of roots): admission / apply / journal | — (a root held at most 2,048 sites) | 1.6 ms / 24 ms / 140 KB; idle boundary 0.14 ms |
| Production, scope batch of 64 items | 400 scopes of 160 sites (64,000 sites, 31.9 MB) | — | 2.4 ms / 52 ms / 233 KB |
| Production archive commit, prepare | 1,023 executions beside 8,000 unrelated outcomes | 279 to 384 ms | 108 ms |
| `fork` | a run of 65,536 records / the unsealed world after a burst | 53.3 ms / 0.77 to 0.93 s | 0.025 ms / 0.1 to 0.2 ms (786,706 → 156 allocations; the first appending boundary after the fork pays 53.5 ms) |
| World-scale admission boundary, every scope | 4,200 commands | 1.7 to 2.4 s | 0.9 to 1.0 s |
| Restore of the 662 MB world snapshot | three runs, byte-identical snapshot | 47.7 to 71.4 s | 28.7 to 44.8 s |
| Continuation restore | month 14 of the continuation profile | 31 to 48 ms | 19 to 23 ms |
| Seal, about 900,000 index entries | `continuation-profile --seal-months 3 --commands 280 --churn 15000`, whole `seal_evidence` | 30.8 to 59.2 s (index 7 to 15 s, verify 20 to 47 s, receipts 3 to 11 s) | 3.3 to 3.4 s (index 2.3 s, verify 0.24 s, receipts 0.02 s), byte-identical segments |
| Paged checkpoint with 4,000 report holders | `continuation-profile --report-holders 4000 --report-bytes 2000 --paged`, a 9.8 MB ledger | refused (the envelope page) | 16,347 pages, 12.3 MB, prepared in 89 ms; restored with its prefix at `assembled_bytes` (10.2 MB) in 244 ms; one-day delta 2,328 pages, 1.0 MB, 44 ms |

The hash algorithms are unchanged. The engine version in every checkpoint
hash changes, first-party plugins that take their plugin version from the
crate version report a new one, and the exact engine-version check rejects
0.17.0 saves; retain the 0.17.0 engine to read them.

Version 0.17.0 is a pre-1.0 minor release. Its changes come from one design,
[fiscal observers at scale](proposals/fiscal-observers-at-scale.md), Phase 3
of [world-scale fiscal state](proposals/world-scale-fiscal.md) and the rest
of downstream gap G-15/G-16, which changes the generic knowledge ledger,
every plugin that publishes knowledge, and the fiscal extension; and from the
continuation gate (E-27), which this release commits as a release test. What
changed, in one list:

- A boundary has one **publication budget** of record bodies, holder entries,
  and payload bytes, shared by every publishing system in dispatch order and
  read with `SimulationView::knowledge_publication_budget()`. The per-system
  count of 64 publication batches is gone.
- A `PublishKnowledge` directive carries **groups** of holders and drafts.
  Each record body is stored **once**, and each holder holds a **holder
  entry** with its own stored local ID and its own supersession, so one body
  can serve many holders, and a body is encoded, hashed, and kept once per
  boundary for the overlay, the ledger, and the journal.
- A knowledge schema declares a **retention** policy. Under
  `RetireWhenSuperseded` an entry a new entry of the same holder supersedes
  is cleared at publication, in the same commit, live and in replay alike;
  `Keep`, the default, is the 0.16.0 behaviour.
- One `KnowledgePublished` event per directive, naming every holder.
- Evidence that cites an initial-scenario record version resolves through an
  index built once per scenario (E0), not a scan of every initial record.
- The knowledge commitment is chunked (commitment format 6) and a snapshot
  stores bodies and holder entries (snapshot format 9).
- Fiscal reports gain a **report cadence** (`OnChange` or `Interval`), an
  **institution granularity**, a report schedule record, a bounded parked
  rotation, and cheaper part encoding, digests, and queue reads; the report
  share of the engine's budget and the retention of report records are host
  knobs, fixed for a run.

**Same version only.** Canwu crates depend only on the exact same version of
each other: every first-party crate requires `=0.17.0` of every other
first-party crate it uses, and a host pins `canwu-api = "=0.17.0"` and the
same version of every extension. Nothing written by 0.16.0 is read by
0.17.0. There is no migration and no decoding of an old shape: the exact
engine-version check refuses every 0.16.0 snapshot, checkpoint journal, paged
checkpoint, continuation checkpoint, replay journal, evidence segment, and
scenario that records its engine version, and most would also be refused by
their format or schema versions (snapshot format 9, commitment format 6,
checkpoint/journal format 6, paged envelope format 6, continuation format 2,
fiscal runtime schema 6, `fiscal_report` schema 5). A scenario built with
0.16.0 types is rebuilt with the 0.17.0 types; its `knowledge` ledgers are
encoded as bodies and holder entries. Retain the 0.16.0 engine to read
0.16.0 data.

Formats and schemas that change, each deliberately versioned:

| Item | 0.16.0 | 0.17.0 | What changed |
| --- | --- | --- | --- |
| Snapshot format | 8 | 9 | `KnowledgeSnapshot::records` (and a scenario's `knowledge`) encodes `{ bodies: [...], holders: [{ holder, next_local, entries: [{ record, local, supersedes, contradicts }] }] }`, bodies in ID order, holders in holder order, entries in local order; decoding refuses a body no holder holds, an entry naming a missing body, a local ID that is zero, out of order, or at or above `next_local`, an empty holder whose `next_local` is 1, a relation that does not resolve within its holder, and a duplicate or unordered body or holder; a holder emptied by retention stays encoded with its `next_local` |
| `COMMITMENT_FORMAT_VERSION` | 5 | 6 | the knowledge root is `canwu.commitment.knowledge.v3` over the actor sub-root and a chunked ledger root: a 32-byte leaf hash per holder entry, cached and recomputed only where an entry changed, chunk roots over `KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES` (256) local IDs, a holder root over its chunks, and the ledger root over the holder roots; the append-only records accumulator in global ID order is gone, so an entry can leave; the boundary record hash covers the new shape of `knowledge_changes` (bodies by digest, per-holder entries, the supersession mode); every checkpoint hash, `v1:` boundary state commitment, boundary hash, and `boundary_chain` root therefore differs from 0.16.0 |
| Checkpoint/evidence-journal format | 5 | 6 | `BoundaryKnowledgeChange` carries `supersession`, the bodies it stored (`records: Vec<Arc<KnowledgeRecordBody>>`, shared with the ledger) and each holder's entries (`holders: Vec<HolderPublication>`); one `KnowledgePublished { holders, record_count, entry_count }` event per directive; segments keep their shape, item encoding, and IDs, and their `format_version` field reads 6 |
| `PAGED_CHECKPOINT_FORMAT_VERSION` | 5 | 6 | the envelope embeds the format-6 checkpoint and the format-9 snapshot; the knowledge ledger still lives inside the 4 MiB envelope page (paging it, E4, is later work) |
| `CONTINUATION_CHECKPOINT_FORMAT_VERSION` | 1 | 2 | embeds snapshot format 9; the hash domain stays `canwu.continuation-checkpoint.v1` |
| `PluginDescriptor` | — | — | every registered `PluginKnowledgeSchema` carries its `retention`, so the persisted descriptor of a plugin that publishes knowledge records the policy and a run restores and replays only with a plugin that declares the same one |
| `fiscal_report` knowledge schema | 4 | 5 | the summary carries the binding's `granularity` and `cadence`; a part of institution rows carries `books_summed`; versions 1 to 4 are no longer registered, so a scenario whose knowledge carries one is refused |
| `FISCAL_RUNTIME_SCHEMA_VERSION` | 5 | 6 | `FiscalObserverBinding::granularity` and `cadence`; `FiscalReportQueueEntry::due_at` is the one due field, with the `Interval` meaning; the new record kind `report-schedule` (`FiscalReportSchedule { page_due, parked_pages }`, identity `primary`); `FiscalRoot::earliest_due`, `parked`, and `parked_cursor` replace `queue_pages`; `FiscalOffice::mechanisms` and `observers` (a map of `FiscalOfficeObserver { first_row, granularity, interval }`) |
| `canwu-fiscal` plugin | `0.3.0-experimental` | `0.4.0-experimental` | a new semantic hash; the three knobs are in the descriptor: `with_report_cadence` as the wake cadence of `wake-fiscal-reports-on-cadence-v1`, `with_report_budget` in the report system's name (`materialize-fiscal-reports-v1/share/{bodies}/{entries}/{bytes}` for a custom share; the default keeps the plain name), and `with_report_retention` in the registered report schema |

The `canwu-fiscal` semantic identity differs from 0.16.0 (version and hash).
The other first-party plugins that register a knowledge schema (military,
movement, information, correspondence, resource, production, technology)
declare `KnowledgeRetention::Keep` and keep their semantic hash constants;
their persisted descriptors gain the field, which no cross-version
comparison ever sees, since no 0.16.0 save loads. As with any version
change, the engine version in every checkpoint
hash changes and first-party plugins that take their plugin version from the
crate version report a new one.

New and changed public contracts, by crate:

- `canwu-knowledge`, re-exported by `canwu-sim` and `canwu-api`:
  - A record is stored as a `KnowledgeRecordBody` (`id`, `schema`,
    `subjects`, `payload`, `as_of`, `learned_at`, `confidence_per_mille`,
    `origin`) held once, and a `HolderEntry { record, local, supersedes,
    contradicts }` per holder. `KnowledgeRecord` keeps its fields and is the
    assembled form a reader gets (`KnowledgeRecord::assemble(holder, entry,
    body)` and `into_parts` convert); `HolderLedger::get`, `iter`, and
    `values` return it by value, and `HolderLedger::entry`, `body`, and
    `entries` read the stored forms. A holder's local IDs are stored, come
    from the holder's own counter, never move when an entry leaves, and are
    read with `HolderLedger::local_of` and `id_of`.
  - `HolderLedgers` gains `insert_body`, `insert_publication` (bodies and
    entries, atomically, with every relation resolved), `remove_entry`,
    `remove_body`, `retire_superseded_by`, `body`, `body_digest`,
    `body_holder_count`, `body_count`, `bodies` (`BodiesById`),
    `last_body_id`, `commitment_root`, and `refresh_commitment`;
    `insert(KnowledgeRecord)` stays as the authoring path. Removed: `record`,
    `records`, `records_after`, `record_count`, `last_record_id`, `remove`,
    indexing by record ID, and `RecordsById`. `KnowledgeBodyDigest`,
    `KnowledgeEncodingError`, `KnowledgeLedgerError`, `HolderEntriesIter`,
    and `KNOWLEDGE_COMMITMENT_CHUNK_ENTRIES` (256) are new and re-exported
    by `canwu-api`; the runtime's overlay type `StagedKnowledgeEntries` is
    public in `canwu-knowledge` only. Each holder's current-record index is
    keyed by SipHash keys that store the exact kind or subject beside the
    key and fall back to exact comparison after any collision, so a lookup
    never depends on key values and neither the commitment nor a restore
    uses them. Iteration stays forward only. Source-incompatible.
  - `KnowledgeRetention::{Keep, RetireWhenSuperseded}` (default `Keep`).
  - `KnowledgeRecordDraft::payload_bytes()` and
    `KnowledgeRecordBody::payload_bytes()` return `Result<usize,
    KnowledgeEncodingError>`: the compact JSON length of the payload, which
    is exactly what the budget charges. Additive.
  - `KnowledgeQueryResult` gains `retention: BTreeMap<KnowledgeSchemaId,
    KnowledgeRetention>` (serde-default): the policy of every schema the
    query named, or of the returned records' schemas when it named none, so
    a research host can tell when `FullHistory` shows what `CurrentHeads`
    shows. `with_retention` fills it. Additive, except for struct literals.
  - The encoded ledger is the format-9 shape above. Source-incompatible for
    JSON a host wrote by hand.
- `canwu-sim`, re-exported by `canwu-api`:
  - `BoundaryDirective::PublishKnowledge { visibility, producer_correlation,
    supersession, groups, summary }` replaces `{ holder, visibility,
    producer_correlation, records, summary }`. A
    `KnowledgePublicationGroup { holders, records }` publishes every draft
    to every holder of the group (`holders` sorted and unique).
    `KnowledgeSupersession::Explicit` applies each draft's `supersedes` and
    `contradicts` to the one holder of its group, so a group of one holder
    with `Explicit` is the 0.16.0 directive; `CurrentOnSubjects` lets the
    engine compute, per holder, the holder's current records of the draft's
    kind that carry every subject of the draft (the `knowledge_heads` rule)
    and name them in that holder's entry, and requires empty relation lists
    on the drafts. A group with a duplicate holder, an `Explicit` group of
    more than one holder, a `CurrentOnSubjects` draft with relations, or one
    whose match is truncated fails the boundary with
    `InvalidKnowledgeRecord`. Source-incompatible.
  - `KnowledgeLimitsV1` loses `batches_per_system_boundary` and gains
    `holder_entries_per_batch` (4,096), `holder_entries_per_boundary`
    (32,768), and `payload_bytes_per_boundary` (64 MiB);
    `records_per_boundary` rises from 10,000 to 32,768;
    `records_per_batch` (1,000) now bounds the bodies of one directive over
    all its groups. Source-incompatible for struct literals and for code
    that read the removed field.
  - `SimulationView::knowledge_publication_budget()` returns a
    `KnowledgePublicationBudget { records, holder_entries, payload_bytes,
    records_per_batch, holder_entries_per_batch }`: what the boundary may
    still publish at this system's cut, after every proposal of earlier
    phases and of earlier systems of this phase, with the per-directive
    bounds a publisher splits its groups against. `admits(records,
    holder_entries, payload_bytes)` and `take(...)` (which also says whether
    it took; both `#[must_use]`) keep a publisher inside it. It needs no
    read declaration. Additive.
  - `PluginKnowledgeSchema::retention: KnowledgeRetention` is a new field
    (struct literals must name it). `Simulation::knowledge_retention(&schema)`
    and `CompactedSimulation::knowledge_retention` read a registered
    schema's policy. Every version of a knowledge kind must declare the
    same retention, because a kind's entries supersede across its versions:
    plugin registration refuses a plugin whose versions differ, and a
    snapshot whose descriptors do is refused at load.
  - `BoundaryKnowledgeChange { plugin, system, phase, producer_correlation,
    visibility, summary, supersession, records, holders }` with
    `HolderPublication { holder, entries }`, `entry_count()`, `body(id)`,
    and `records_of(&holder)`, which assembles the records one holder
    received. The bodies are the ledger's own `Arc`s. Source-incompatible.
  - `BoundaryReceipt` gains `knowledge_entry_count`; `knowledge_record_count`
    counts bodies and `knowledge_batch_count` directives.
    `SimulationView::knowledge_record_count_in_namespace` counts bodies.
  - `KnowledgePublished { holders: Vec<KnowledgeHolderRef>, record_count,
    entry_count }` replaces `{ holder, record_count }`; its audience is
    `EventAudience::KnowledgeHolders(holders)`, a new variant in
    `canwu-event` (`KnowledgeHolder(_)` stays for other uses), so each
    holder's `visible_changes_since` sees the event. Source-incompatible
    for exhaustive matches and for code that read `holder`.
  - A proposal that publishes past the remaining budget fails the boundary
    with `KnowledgeLimitExceeded`, as a directive past its per-directive
    bounds does. `KnowledgeEncodingError` converts into `CanwuError`
    (`InvalidKnowledgeRecord`); a staging misalignment inside the kernel is
    `InvalidBoundary`. A restored run keeps the ledger it rebuilt from the
    journal, its bodies shared with the journal and held once.
  - `SNAPSHOT_FORMAT_VERSION` 9, `COMMITMENT_FORMAT_VERSION` 6,
    `CHECKPOINT_JOURNAL_FORMAT_VERSION` 6, `PAGED_CHECKPOINT_FORMAT_VERSION`
    6, `CONTINUATION_CHECKPOINT_FORMAT_VERSION` 2.
  - `knowledge_heads`, `knowledge_records`,
    `knowledge_holder_accepts_publication`,
    `knowledge_changes_by_correlation*`, `KnowledgeQuery`, `KnowledgeCursor`,
    `KnowledgeRecordView`, `KnowledgeWriteGrant`, and
    `MAX_KNOWLEDGE_PAGE_SIZE` keep their signatures and meaning. E0 adds no
    public type.
- `canwu-api`:
  - `CanwuViewer::query_knowledge`, `query_holder_knowledge`,
    `CanwuRun::admin_query_knowledge`, and `audit_knowledge_record` keep
    their signatures; the query results carry `retention`, and the audit
    read resolves a stored holder-local ID, so a record keeps its local ID
    after another entry of the holder is cleared.
  - The Cargo feature `continuation-gate` (off by default) is a release
    test, never enabled by CI or by an application: with it on, every
    `Canwu` keeps a sealed shadow run restored from its own continuation
    checkpoints, runs every mutation on both, and panics with `continuation
    gate:` on any divergence of results, checkpoint hash, or state hash, or
    when an exact replay differs. Its hooks are `#[doc(hidden)]`. A new
    `&mut self` method on `Canwu` goes through `gated!` or drops the gate's
    shadow (`AGENTS.md`). [Releasing](releasing.md) and the
    `canwu-contributor-release` skill run it before a release is tagged.
- `canwu-fiscal`:
  - `FiscalObserverBinding` gains `granularity: FiscalReportGranularity`
    (`Scope`, the default, or `Institution`) and `cadence:
    FiscalReportCadence` (`OnChange`, the default, or `Interval { every,
    offset }`, aligned to `SimTime::EPOCH`, both whole minutes with `offset
    < every`), both serde-defaulted so a 0.16.0 scenario that names neither
    behaves as before; neither changes with `RebindObserver`.
    `FiscalReportCadence::staggered(every, index, stride)` spreads offsets,
    `is_interval` reads the kind, and
    `FiscalObserverBinding::scheduled_cadence` gives the cadence a binding
    is served by (`OnChange` once removed). `FiscalState::add_observer`
    queues the binding's first report at its first due time, the first
    aligned time at or after the state's `historical_context.updated_at`.
  - `FiscalReportQueueEntry { due_at, cycles, whole, republish, parked }`
    keeps its fields; `due_at` is the one due time and, for an `Interval`
    binding, the next aligned time. `FiscalReportSchedule { page_due,
    parked_pages }` is the payload of the new record
    `canwu.fiscal:report-schedule` (`FiscalReportScheduleRecord`,
    `FiscalRecordKind::ReportSchedule`, `fiscal_report_schedule_reference`).
    `FiscalRoot` gains `earliest_due`, `parked`, and `parked_cursor` and
    loses `queue_pages`. `FiscalOffice` gains `mechanisms` and lists its
    `observers` as `FiscalOfficeObserver { first_row, granularity, interval
    }` with `row(rank, mechanism, mechanisms)`; `FiscalObserverRows::span`
    takes the granularity; `FiscalState` gains the public field
    `report_schedule`, which `into_records` derives. Source-incompatible,
    struct literals of `FiscalState`, `FiscalReportSummary`,
    `FiscalReportPart`, and `FiscalObserverRows` included.
  - `FiscalReportSummary` gains `granularity` and `cadence`;
    `FiscalReportPart` gains `books_summed: Option<u32>`; an institution row
    of `FiscalReportRow` carries the institution and mechanism and an empty
    `scope_binding_id`. `MAX_FISCAL_OBSERVER_ROWS` (16,384) also caps
    institution rows; sums saturate at `u64::MAX`.
  - `FISCAL_PARKED_CHECKS_PER_BOUNDARY` (16) is new.
  - `FiscalPlugin::with_report_cadence(SystemCadence)` (the wake cadence,
    distinct from a binding's report cadence) and
    `report_cadence()` add the phase-13 system
    `wake-fiscal-reports-on-cadence-v1`, which writes nothing;
    `EventDriven` is refused at registration (`InvalidPluginRegistration`).
  - `FiscalReportBudget { bodies, entries, payload_bytes }` is fiscal's
    share of a boundary's publication budget: `FiscalReportBudget::DEFAULT`
    is half of `KnowledgeLimitsV1::CURRENT` (16,384 bodies, 16,384 entries,
    32 MiB) and `MINIMUM` two bodies, two entries, and 128 KiB (a summary
    and one part of the largest payload); `FiscalPlugin::with_report_budget`
    sets it and `report_budget()` reads it, and registration refuses a
    share outside that range (`InvalidPluginRegistration`). A custom share
    is carried in the report system's name
    (`materialize-fiscal-reports-v1/share/{bodies}/{entries}/{bytes}`; the
    default keeps the plain name), so it is in the plugin descriptor. The
    report system publishes the least of its share and what the engine has
    left at its cut; each holder's records are one `Explicit` group, the
    groups are packed into directives up to the per-directive bounds (ten
    directives at the world due instant), and parts are cut so that 64 KiB
    always remains for the summary. `FiscalPlugin::with_report_retention`
    (default `RetireWhenSuperseded`; `report_retention()` reads it) sets
    the `fiscal_report` schema's `retention`.
    `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` is deleted.
  - `compute_projection`, `fiscal_reports`, `fiscal_reports_of_holder`,
    `assemble_fiscal_reports`, the pending bitmap, `is_stale`, and the line
    scheme (`1 + slot × 1,024 + part`) are unchanged.
- `canwu-military`, `canwu-movement`, `canwu-information`,
  `canwu-correspondence`, `canwu-resource`, `canwu-production`,
  `canwu-technology`: no public type changes; each builds the new directive
  (one `Explicit` group of one holder). Military, movement, information, and
  technology read the budget: military publishes commanders in ID order
  while the next commander's whole report fits; movement keeps
  `MovementLimitsV1::report_holders_per_boundary` (64) and also stops at the
  budget; information keeps its chunking and a chunk also stops at the
  budget: a chunk whose first record does not fit is deferred whole, the
  operation returns to awaiting publication, and the next boundary at the
  same instant retries, so information never fails a boundary for the
  budget; technology keeps `max_publications_per_boundary` (32) and records
  a publication past the budget as a rejection with its existing event,
  whose summary says when the engine's shared budget was the limit.

The source-incompatible changes are the directive, the knowledge ledger
types and their removed reads, `KnowledgeLimitsV1`,
`PluginKnowledgeSchema::retention`, `BoundaryKnowledgeChange`,
`KnowledgePublished`, the `EventAudience` variant, the fiscal binding,
root, office, queue, state, summary, part, and observer-rows fields, and the
removed fiscal cap. The release is therefore a minor version.

Version 0.17.0 also changes these behaviours, which are visible to existing
code and runs:

- **One budget, read before publishing.** The budget is spent in dispatch
  order: phase 4 before phase 13, and within a phase in the registry's
  `(phase, plugin, system)` order. A system sees exactly what earlier
  phases and earlier systems of its own phase have taken, so a publisher
  that reads the budget and defers what does not fit never fails a
  boundary. Overshooting it is the undeclared-write class, not an expected
  refusal: the boundary fails with `KnowledgeLimitExceeded`, because a
  publication the engine cut would leave the plugin's own state writes
  believing the records were published. Fairness between plugins is the
  host's composition problem.
- **One copy per body.** A body is validated once, encoded once (streamed
  into its digest), and wrapped in one `Arc` shared by the same-boundary
  overlay, the ledger, and the journal; the proposal's duplicate check keeps
  digests, not bytes. The same `(holder, body)` twice in one proposal is
  still refused.
- **Per-holder supersession.** A holder's current records are the entries
  no entry of the same holder names in `supersedes`; `knowledge_heads` keeps
  returning global IDs. A holder sees local IDs, its own entry's relations,
  and the body; it cannot tell a shared body from a private one, and its
  local IDs come from its own counter, so the 0.16.0 residual (global IDs
  showing allocation gaps) ends.
- **Retention at publication.** Under `RetireWhenSuperseded`, when a new
  entry of holder *H* supersedes *H*'s entry *S*, *S* leaves *H* in the
  commit of the publishing boundary: it leaves the entries, the local-ID
  map, and the current-record index; every entry of *H* that named *S* in
  `supersedes` or `contradicts` drops that ID, so "every relation of an
  entry resolves within its holder" holds at all times; and when no holder
  holds *S*'s body, the body leaves the ledger and its namespace count.
  Retention follows the superseding entry's schema, runs in holder order
  then ID order, never fails a boundary, and is not tied to a seal, so a
  sealed run and its unsealed twin keep the same state hash and checkpoint
  hash in lockstep (the plan's "after a seal" was changed for that reason,
  recorded in the proposal). `FullHistory` of a retiring schema shows only
  current entries, and the query result says so;
  `knowledge_changes_by_correlation*` still return what the boundary
  published, retired or not, from retained evidence; `visible_changes_since`
  is events, unchanged; saves hold only current entries. An entry of a dead
  person is never cleared (nothing can supersede it) and stays while the
  holder does.
- **Restore validation rebuilds the ledger.** A restore builds the ledger
  boundary by boundary from the journal, recomputes every `CurrentOnSubjects`
  list, applies retention, checks each directive's bounds and the
  per-boundary budget, and compares the result with the saved ledger, so a
  tampered relation list, a retained entry the policy says was cleared, a
  `next_local` below an entry's local ID, or a chunk or holder root that
  does not match is refused at load.
- **One event per directive.** `KnowledgePublished` names the directive's
  holders (sorted, at most `holder_entries_per_batch`) and its total body
  and entry counts; a host that wants one holder's own count reads its
  ledger (`fiscal_reports` for fiscal). 0.16.0's one event per holder would
  have been 4,000 events at the fiscal gate, each admitted as ingress by the
  next boundary.
- **Initial-version evidence is indexed.** A publication citing a record at
  its initial-scenario version resolves through an index of `(reference,
  version)` built once per scenario and shared by the runtime, its forks,
  and snapshot validation; the scan of every initial record (about 320 µs
  per citation at 68,000 records) is gone. No format changes.
- **Fiscal report cadence.** An `Interval` report is published by the first
  boundary at or after each due time that runs the report system, with what
  changed since its last publication and its summary, and never between due
  times, so between due times it shows the records as of its `as_of`
  boundary. An active `Interval` binding always has exactly one queue entry,
  due at one of its due times: created when the binding is added, merged
  into by settlement without moving `due_at` (settlement reads the office's
  observer list, not the observer records), and queued again at the next due
  time by the report system whether or not it published. At a due time at
  which nothing changed, the summary alone is republished once the report
  has been published at least once, so `as_of` advances and silence never
  means "no report". A rebound `Interval` binding keeps its due time (the
  new holder gets the whole report then; the old keeps its report until
  then); a removed binding's report is withdrawn at once. `OnChange` is the
  0.16.0 behaviour. The report schedule keeps each queue page's earliest
  due time and the pages with parked entries; the root keeps the earliest
  of all, so a boundary before it, with nothing parked, reads the root
  alone, and one at or after it decodes only the pages whose earliest due
  time has come. `audit_fiscal_records` checks the chain from the pages to
  the root and that every active `Interval` binding has its one entry at
  one of its due times; an entry of a removed binding that names no work is
  refused. No future-dated packet is scheduled.
- **Waking the report system.** The report system stays event-driven. A
  host whose boundaries can pass without any event registers
  `with_report_cadence(cadence)`: every boundary of that category with no
  event reads the root and, when `earliest_due` has come and the run can
  schedule ingress, schedules the drain packet due at once, so the next
  boundary at the same instant runs the report system (the engine refuses
  two phase-13 writers of the same records, so the report system itself
  cannot also be registered on the cadence). Parked entries alone wake
  nothing.
- **Institution rows.** An `Institution` observer has one row per visible
  institution and mechanism (the office lists its scopes' mechanisms), its
  facts the sums of the institution's hot books of the cycle of that
  mechanism per unit and payment form, rendered at the observer's
  confidence; `compute_projection` sums the same way. A part of institution
  rows cites the current version of each office its rows belong to and
  counts the books it summed (`books_summed`, excluded from the digest), so
  a change of the count alone does not republish it; that is weaker
  provenance than a scope part's ledger citations.
- **Parked rotation.** Each run of the report system checks the holders of
  the next `FISCAL_PARKED_CHECKS_PER_BOUNDARY` (16) parked entries after the
  root's cursor (the cursor's page after the cursor, later pages, earlier
  pages, then the cursor's page up to the cursor), and publishes what a
  revived holder is owed, so a revived holder is published to within
  `ceil(parked / 16)` runs rather than by the reviving boundary; the cursor
  is cleared when a run checked every parked entry or nothing is parked.
- **Fiscal cost cuts.** Part and summary payloads are built directly (no
  `to_value` round trip); part digests are over a compact binary encoding of
  the rows (`canwu.fiscal.report-part.v2`); each index page is decoded once per page per
  boundary; settlement reads each office's observer list once.
- **Expected refusals, unchanged.** No fiscal packet fails a boundary; owed
  work past the budget waits for the drain; an observer with an unreachable
  holder is parked; a report with no hot cycle is not published.
- **Drains only after progress.** The report system schedules the drain
  packet only when its boundary did some of the owed work (published a part
  or a withdrawal, or finished a queue entry). When earlier publishers left
  no room, the owed work waits for the next boundary that runs the report
  system (one with an event, or the wake cadence) instead of filling the
  instant with drain boundaries that publish nothing.

Upgrading from 0.16.0, for hosts and plugin authors:

- **A plugin that published knowledge** wraps its 0.16.0 directive: where it
  wrote `PublishKnowledge { holder, visibility, producer_correlation,
  records, summary }`, it writes `supersession:
  KnowledgeSupersession::Explicit` and `groups: vec![KnowledgePublicationGroup
  { holders: vec![holder], records }]`. Nothing else about grants,
  `visibility` (which applies to every group), schemas, or drafts changes.
  A plugin that publishes the same bodies to several holders puts them in
  one group, and uses `CurrentOnSubjects` when each holder should supersede
  its own current record on the same line.
- **A plugin that paced itself** by `batches_per_system_boundary` reads
  `view.knowledge_publication_budget()` at the start of its system, sums
  each directive's bodies, holder entries (holders × drafts per group), and
  `draft.payload_bytes()?`, calls `budget.take(bodies, entries, bytes)`
  before building each directive, and defers or records as a rejection what
  does not fit, as the first-party publishers do. Never publish what the
  read did not admit.
- **Every registered knowledge schema names `retention`.** `Keep` keeps the
  0.16.0 behaviour. Choose `RetireWhenSuperseded` for a schema whose
  records are lines a newer record replaces, such as reports; the holder
  then keeps one entry per line. The policy is plugin identity: changing it
  needs a new run.
- **A host that read the journal or events** matches
  `KnowledgePublished { holders, record_count, entry_count }` and
  `EventAudience::KnowledgeHolders`, and reads a boundary's publications
  through `BoundaryKnowledgeChange::records_of(&holder)` or its `records`
  and `holders` fields. Struct literals of `KnowledgeLimitsV1`,
  `PluginKnowledgeSchema`, `KnowledgeQueryResult`, and
  `BoundaryKnowledgeChange` name the new fields.
- **A host that queries knowledge** sees `KnowledgeQueryResult::retention`;
  under `RetireWhenSuperseded`, `FullHistory` is `CurrentHeads`. A host that
  needs a superseded record's history keeps `Keep` on that schema or its own
  archive of the records it received.
- **A fiscal scenario** that sets no `granularity` or `cadence` is
  unchanged. For many observers, give each `Interval` with
  `FiscalReportCadence::staggered(every, index, stride)` and courts
  `Institution`, set `historical_context.updated_at` to the run's start
  before `add_observer`, and register `with_report_cadence` (the wake
  cadence: the `SystemCadence` on which `wake-fiscal-reports-on-cadence-v1`
  runs, distinct from a binding's report cadence) when the host's
  boundaries can pass without events. The knobs `with_report_cadence`,
  `with_report_budget`, and `with_report_retention` are recorded in the
  plugin descriptor (the share in the report system's name, the retention
  in the registered schema): they are chosen before a run's first boundary,
  and a restore, an exact replay, or a fork with a plugin of another setting
  is refused as a plugin-identity mismatch.
- **A fiscal host that read closed cycles' figures from old report records**
  sets `with_report_retention(KnowledgeRetention::Keep)`: under the default,
  a closed cycle's parts leave the holder's ledger once the empty parts
  supersede them, and a superseded record stays only in the boundary
  evidence that published it, until a seal takes the evidence.
- **Saves.** Write a fresh continuation checkpoint, snapshot, or checkpoint
  journal with 0.17.0; nothing from 0.16.0 loads.

Known limits carried by this release, each stated in its proposal:

- The knowledge ledger still lives inside the 4 MiB envelope page of a paged
  checkpoint, so a world-scale run cannot prepare one and the continuation
  checkpoint is its save; paging the ledger (E4) is later work
  ([paging](proposals/fiscal-observers-at-scale.md#persistence-hashing-replay-and-formats)).
- A mid-boundary `ScheduleIngress` (the fiscal drain packet's
  `append_ingress`) refreshes every dirty commitment root early, about
  195 ms after a boundary of 56,891 record writes and 0.6 ms in a normal
  drain; deferring the refresh to the boundary's own hashing phase is a
  follow-up
  ([engine findings](proposals/fiscal-observers-at-scale.md#engine-findings-outside-this-package)).
- A publisher that does not read the budget, or reads it and overshoots,
  fails its boundary with `KnowledgeLimitExceeded`; the engine does not cut
  a publication to fit
  ([what can still fail a boundary](proposals/fiscal-observers-at-scale.md#what-can-still-fail-a-boundary)).
- The engine still accepts at load a plugin's declaration of an
  initial-scenario record version as payload-required; only the seal refuses
  it (E-27's remaining item, [releasing](releasing.md)).
- At world scale the continuation checkpoint still grows about 3.8 MB a
  simulated month (172 MB after month 1, 293 MB after month 24, restored in
  58 s): not knowledge or fiscal state, which stay flat, but the engine's
  request index (`RequestIndex`: commands, queued commands, ingress
  provenance), which keeps every accepted command's outcome at about 440
  bytes per command; and the wall time of a simulated month creeps about
  1 s a month after month 5, not yet located (a guess is the seal rebuilding
  the request maps). Both are engine follow-ups.
- The due instant's heap peak, 472 MB for 15.7 MB of payload in the
  allocation-counting build, is not investigated, and the fiscal side of
  that boundary (1.8 s) is above the design's expectation of about 0.6 s.
- A plugin that runs before `canwu-information` in dispatch order and spends
  the whole budget at every same-instant boundary keeps information
  deferring its chunk; no first-party plugin does.
- A part of institution rows cites the office version and the number of
  books it summed, not each ledger version; summing a 6,000-scope office
  costs about 40 ms at its court's due time
  ([coarser rows](proposals/fiscal-observers-at-scale.md#coarser-rows-for-upper-observers)).
- Fiscal report retention clears a closed cycle's figures from the holder
  by default, and the three fiscal knobs are run-bound; an `Interval`
  report shows the state as of its last due boundary; calendar cadences are
  not expressible (`every` is a fixed duration); an ex-holder's withdrawn
  parts and a dead holder's entries never clear, bounded by rebindings and
  deaths; domain-entity report holders stay deferred
  ([risks](proposals/fiscal-observers-at-scale.md#risks-and-open-questions),
  [parked holders](proposals/fiscal-observers-at-scale.md#parked-holders-domain-entity-holders-and-the-report-system-cadence)).
- Several dirty cycles owed at once (about 5,600 parts a cycle on the world
  preset) take more than one boundary by design, through the drain packet
  ([the gate](proposals/fiscal-observers-at-scale.md#the-problem-and-the-gate)).
- The continuation gate proves state equality of the live run and its sealed
  shadow with the run's own plugin registry, not that the public restore and
  replay paths accept the checkpoint or the journal
  ([releasing](releasing.md)).
- Every limit of the 0.16.0 entry that this release does not touch stands:
  a paged restore allocates what the pages describe; a boundary still takes
  a stable copy of plugin components and core maps for phases 1 to 9, and
  `fork` copies every journal; a continuation runtime cannot produce a full
  snapshot, replay journal, checkpoint journal, or paged checkpoint, and its
  keyed-draw ledger and request index grow for the life of the run; what a
  continuation restore verifies without history is covered only by the
  stored `continuation_hash`; R4 is a plugin obligation; the resource,
  production, reference-archive, law, society, and Ming-validator items; the
  per-command transaction checkpoint; and the `Send` runtime's `Sync`
  reliance. The 0.16.0 knowledge limit (superseded records stay in every
  ledger and save; queries cost the holder's whole ledger) is replaced by
  retention and the per-holder entry walk above.

Headline measurements, from the design's profile, the engine package's
probes, and the `fiscal-world-profile` baselines
`benchmarks/baselines/2026-10-16-fiscal-world-world-institution-interval-{elapsed,allocations}.json`
(release builds, one Windows x86-64 machine shared with other builds;
elapsed times show the order of magnitude; the `world` preset is 200
polities, 50,000 scopes, 4,200 institutions, and 4,000 observers, courts at
`Institution` granularity, monthly `Interval` cadence staggered over 30
days):

| Measure | Workload | 0.16.0 | 0.17.0 |
| --- | --- | ---: | ---: |
| Publishing boundary, engine time | synthetic publisher, 64 holders × 16 records × 512 B | 13.7 ms | 9.7 ms |
| Publishing boundary, engine time | 64 holders × 64 records × 512 B | 55.2 ms | 37.9 ms |
| Publishing boundary, engine time | 64 holders × 16 records × 8 KiB | 39.2 ms | 15.0 ms |
| Engine cost per KiB of payload | the same probe | 3.3 µs | 0.7 µs |
| Engine cost per holder entry | 4,000 holders in one boundary | — | 7.8 to 15.4 µs |
| Journal of 1,024 bodies | the same probe | 1.6 ms | 0.1 ms |
| Hashing of 1,024 bodies | the same probe | 4.3 ms | 0.4 ms |
| Initial-version evidence citation | 68,000 initial records (E0) | about 320 µs each | index lookup |
| `knowledge_heads` boundary | `knowledge-ledger-profile`, 2,048 and 16,384 held records | 131 to 152 µs | 120 to 149 µs |
| `publish_boundary` | the same profile | 56 and 58 µs | 49 and 54 µs |
| Snapshot restore | the probe's snapshot | — | about equal (+2 to 9 %, within noise) |
| **Gate: fiscal due instant** | every one of the 4,000 observers due at one instant, owing one dirty cycle: 9,603 bodies and 9,603 entries, 15.67 MB of payload, 10 directives | 62 same-instant drain boundaries, about 3.1 to 4.2 s | **one boundary, no drain**: 2,504 ms settlement (fiscal 1,821 ms, engine 676 ms); heap peak 472 MB and 6.52 M allocator calls in the allocation-counting build |
| **Gate: held knowledge over simulated years** | 24 monthly cycles (open, assess, close, archive) with monthly `Interval` observers | +12,416 records and +65 MB per cycle | flat at 14,138 bodies and entries, 15.12 MB of payload, from month 2 to month 24 |
| Fiscal spread due day | 134 observers due, 304 report records | 56 ms admission, 82 ms settlement, 3 drains (137 assessments) | 43.2 ms settlement, no drain; heap peak 33 MB |
| Fiscal idle boundary | nothing due | — | 0.63 ms |
| Fiscal assess 64 scopes | one batch | — | 8.97 ms |
| Fiscal burst, assess every scope | 50,000 assessments in one boundary | 2,139 ms admission, 4,790 ms settlement, 62 drains | 1,510 ms admission, 4,568 ms settlement (fiscal 3,091 ms, engine 1,457 ms), no report due, no drain |
| Fiscal receipts | 1,024 and 16,384 receipts in one boundary | — | 90.6 ms; 1,693 ms |
| Fiscal close every book | 50,000 books | — | 1,626 ms admission, 5,052 ms settlement |
| Fiscal archive | 256 and 4,096 books | — | 26.6 ms; 228 ms |
| Continuation checkpoint at world scale | after month 1 and month 24 | — | 172 MB and 293 MB (the request index, not knowledge), restored in 58 s |

The hash algorithms are unchanged. The engine version in every checkpoint
hash changes, first-party plugins that take their plugin version from the
crate version report a new one, and the exact engine-version check rejects
0.16.0 saves; retain the 0.16.0 engine to read them.

Version 0.16.0 is a pre-1.0 minor release. Each group of changes has a
proposal that gives its design, invariants, and measurements:

- [knowledge ledger cost and authentication](proposals/knowledge-ledger-cost-and-authentication.md)
  (held knowledge no longer costs a boundary; commitment format 5);
- [fiscal and society limits](proposals/fiscal-and-society-limits.md)
  (fiscal byte budget and owed report parts, the society ingress queue, the
  society transfer settlement knob, auditor appointment);
- [resource refusals without stalls](proposals/resource-refusals-without-stalls.md);
- [production archive volume and lookup](proposals/production-archive-volume.md)
  (also the production refusals, output acknowledgement, and output keys);
- [law, information, and military correctness](proposals/law-information-military-correctness.md);
- [world-scale fiscal state](proposals/world-scale-fiscal.md), phases 1 and 2
  (downstream gaps G-15 and G-16);
- [record store ceiling and cost](proposals/record-store-ceiling-and-cost.md)
  (paged-checkpoint envelope format 5);
- [continuation checkpoint](proposals/continuation-checkpoint.md) (G-61, with
  the keyed-draw ledger, seal-stable evidence identity, sealing with work in
  flight, and compact host parity; checkpoint/evidence-journal format 5);
- [reference archive lookup, seal-stable reference validation, and replay
  profile](proposals/reference-archive-and-replay-profile.md);
- [command admission history index](proposals/command-admission-history-index.md)
  (admission no longer slows with run history);
- [resource root write cost](proposals/resource-root-write-cost.md) (the
  resource root persists as a live root and hot terminal buckets, so a
  boundary reads and writes what it touches);
- [a `Send` runtime](proposals/send-runtime.md) (G-59: the runtime handles
  and their forks are `Send`, so a host saves a fork on a worker thread).

**Same version only.** The owner's policy of 2026-10-14 is that Canwu crates
depend only on the exact same version of each other: every first-party crate
requires `=0.16.0` of every other first-party crate it uses, and a host pins
`canwu-api = "=0.16.0"` and the same version of every extension. Nothing
written by 0.15.0 is read by 0.16.0. There is no migration and no decoding of
an old shape: the exact engine-version check refuses every 0.15.0 snapshot,
checkpoint journal, paged checkpoint, replay journal, evidence segment, and
scenario that records its engine version, and several would also be refused
by their format or schema versions (commitment format 5, checkpoint/journal
format 5, paged envelope format 5, fiscal runtime schema 5, the society queue
head schema 2, production archive blob format 2, and archive directories that
now require `lookup_root`). A scenario built with 0.15.0 types is rebuilt
with the 0.16.0 authoring helpers (`FiscalState::into_records`,
`ProductionState::into_initial_record`). Retain the 0.15.0 engine to read
0.15.0 data.

Formats and schemas that change, each deliberately versioned:

| Item | 0.15.0 | 0.16.0 | What changed |
| --- | --- | --- | --- |
| Snapshot format | 8 | 8 | shape unchanged |
| `COMMITMENT_FORMAT_VERSION` | 4 | 5 | the knowledge root is two sub-roots (`canwu.commitment.knowledge.v2` over an actor sub-root and a records sub-root hashed incrementally in global ID order); the boundary record hash covers every field of the boundary's `knowledge_changes` when there are any, and hashes each domain-record change's `previous` and `current` by their `canwu.domain-record.encoding.v1` digests; the domain-record roots are the page IDs of the new binary pages; every checkpoint hash, `v1:` boundary state commitment, boundary hash, and `boundary_chain` root therefore differs from 0.15.0 |
| Checkpoint/evidence-journal format | 4 | 5 | `SimulationCheckpoint` loses `keyed_draw_reservations` and `keyed_reservation_root`; its dependency and receipt lists no longer name keyed draws; an evidence segment's per-journal cursors may end at different cuts (events, commands, and attempts at the admission cursors, the other journals at their end); segments keep their shape, item encoding, and IDs, and their `format_version` field reads 5 |
| `PAGED_CHECKPOINT_FORMAT_VERSION` | 4 | 5 | the envelope embeds the format-5 checkpoint and names `STATE_PAGE_CODEC` `raw-canonical-v2`: domain-record Patricia nodes and record-value chunk pages are binary pages; a record longer than `STATE_PAGE_VALUE_CHUNK_BYTES` (1 MiB) is cut into content-addressed 1 MiB chunk pages that its leaf names, up to `MAX_PAGED_RECORD_BYTES` (512 MiB); `MAX_STATE_PAGE_BYTES` (4 MiB) still bounds every page; a record-page defect found on load is `invalid_snapshot` (was `invalid_archive`), a missing page stays `state_page_unavailable` |
| Compact checkpoint hash domain | `canwu.compact-checkpoint.v1` | `.v2` | the material loses the keyed reservation root |
| Seal-token hash domain | `canwu.evidence.seal-token.v1` | `.v2` | bumped with the compact checkpoint identity it binds, which the two-phase seal now computes from the runtime's fields |
| Continuation checkpoint | none | format 1 | `ContinuationCheckpoint`, hash domain `canwu.continuation-checkpoint.v1` |
| `authoritative_state_hash` | hash of the retained journals | the commitment-format state hash (`v1:` over the canonical roots) | a value change, not a type change: equal for sealed, unsealed, and restored runs |
| Journal commitments | `blake3::Hasher` | resumable BLAKE3 stream hash | the command, attempt, event, ingress, and draw roots are unchanged values; the workspace requires `blake3 = "1.8"` |
| `FISCAL_RUNTIME_SCHEMA_VERSION` | 2 | 5 | the state is partitioned into the record kinds of `FiscalRecordKind` (`root`, `archive`, `office`, `ledger`, `jurisdiction`, `observer`, `directory`, `index`, `report-queue`) beside the create-only `catalog`, all in the `canwu.fiscal` namespace; `canwu.fiscal:state` is gone |
| `fiscal_report` knowledge schema | 2 | 4 | lines on each binding's observer record, parts of 16 scope rows per cycle slot, cycles and `withdrawn` in the summary; versions 1 to 3 are no longer registered |
| Fiscal archive objects | none | blob format 1 | `canwu.fiscal.archive.*` namespaces: blob, membership page, temporal page, index directory with `lookup_root`, and lookup page in the resource page format |
| Society `SocietyIngressQueue` | schema 1 | schema 2 | `entries` becomes `queued` and `batches`; the new record kind `canwu.society:ingress-batch` (schema 1, `SocietyIngressBatch`) holds the entries; the cohort exchange ledger keeps schema 1 and gains the outcome codes `queue_full` and `insufficient_source` |
| `RESOURCE_STATE_FORMAT_VERSION` | 2 | 3 | the root persists as its live root `resource:runtime:v1` (payload `ResourceLiveRootV1`, the state without its hot terminal records, which the new field `hot_terminal`, a `ResourceHotTerminalLedgerV1`, counts) and one record per hot terminal bucket; the byte cap is measured on the live root plus the hot terminal entries; the retention acknowledgement payload gains a `handle` form beside `receipt`; the internal ingress `resource_deferred_packet_v1` is new; `production_output_ack_v1` gains `settled_at`; an archive commit packet may select 16,384 keys |
| Resource hot terminal bucket | none | format 1 | the new record kind `canwu.resource` `hot-terminal` (`resource:hot-terminal:{bucket}`, payload `ResourceHotTerminalBucketV1`, `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION` 1, entries in ascending key order); a run holds up to one record per bucket, as many buckets as the candidate budget rounded up to a power of two |
| Resource archive source root | `canwu.resource.archive-source-root.v1` | `.v2` | the digest a commit names covers the state revision, the admitted sequence, the number of hot terminal records, and the archive head instead of the candidate index, so blob, retention handle, and directory ids differ from 0.15.0 while their shapes are unchanged; a directory may list 64 blobs |
| Production root and archive | — | — | `ProductionArchiveHeadStateV1` gains `archived_receipt_count` and `receipt_sequence`, the maintenance receipt gains `retention_handle_id`, `archived_receipts`, and `settled_at`, `ProductionExecution` gains `output_settled_at`, the archive directory gains `lookup_root` and `archived_receipt_count`, blobs gain `receipts` and `PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION` 2, the commit packet gains `selected_receipts` and `lookup_root`, the output acknowledgement gains `settled_at`, and `canwu.production.archive.lookup-page` is a new object kind |
| Legal plan and coordinator shard | — | — | `LawBudgets::max_provider_packets` (default 4,096) changes every compiled plan's content hash; the coordinator shard gains `provider_packets` |
| Information records | 12 schemas | 13 | the create-only `DelegatedAuthority` record (`canwu.information/delegated_authority`); the finalization packet names its chunk and becomes internal |
| Package archive (force-supply and economy reference) | — | — | the directory gains `lookup_root`, so directory IDs change; lookup pages are a new object kind under `canwu.force-supply-reference.archive.lookup-page` and `canwu.economy-reference.archive.lookup-page` |

Nine plugin semantic identities differ from 0.15.0: `canwu-society` (whose
hash now also depends on its transfer settlement mode), `canwu-fiscal`,
`canwu-resource`, `canwu-production`, `canwu-law`, `canwu-information`,
`canwu-military`, and the `canwu-force-supply-reference` and
`canwu-economy-reference` plugins. The proposals name no change to the other
first-party plugins' descriptors. As with any version change, the engine
version in every checkpoint hash changes and first-party plugins that take
their plugin version from the crate version report a new one.

New and changed public contracts, by crate:

- `canwu-sim`, re-exported by `canwu-api`:
  - `DomainRecordChange::previous` is `Option<Arc<DomainRecord>>` and
    `current` is `Arc<DomainRecord>`; the serde form is unchanged. Field reads
    work through `Deref`; code that builds a change wraps the records, and
    code that edits one uses `Arc::make_mut`. Source-incompatible.
  - In `canwu-knowledge`, re-exported by `canwu-sim` and `canwu-api`:
    `KnowledgeSnapshot::records` and `GenericKnowledgeLedger::records` are
    `HolderLedgers` (a persistent map of `HolderLedger`s with a derived index
    of current records by kind and subject), and `for_holder` returns
    `Option<&HolderLedger>`. The read methods keep their names and shapes,
    but no iterator of either implements `DoubleEndedIterator`: `.rev()`,
    `next_back`, and `rfind` do not compile; read the newest record with
    `last_key_value` or `last_record_id`. Build ledgers with
    `HolderLedgers::new`, `insert`, `extend`, or `collect`; edit a record by
    cloning, changing, and inserting it again. New: `HolderLedgers`,
    `HolderLedger`, `HolderLedgersIter`, `HolderRecordsIter`, `RecordsById`,
    `CurrentRecords`, `HolderLedger::current_records`, `is_current`,
    `is_superseded`, `HolderLedgers::record`, `records`, `records_after`,
    `record_count`, `last_record_id`, and `ptr_eq`. The encoded ledger is
    byte for byte the 0.15.0 entry list; decoding refuses a holder entry
    without records. Source-incompatible.
  - In `canwu-decision`, `PersistentDecisionLog::iter` and `ordinals` return
    `impl ExactSizeIterator` and `PersistentDecisionLogIter` no longer
    implements `DoubleEndedIterator`. Source-incompatible for code that
    walked a decision log backwards; no code in the workspace did.
  - `ErrorCode` is `#[non_exhaustive]` and gains `HistoryUnavailable`, which
    `checkpoint`, `prepare_paged_checkpoint`, `snapshot_with_segments`, and
    `replay_journal_with_segments` return on a continuation-restored run.
    Source-incompatible for exhaustive matches, once.
  - `KeyedDrawReservation` and the two `SimulationCheckpoint` fields are
    removed; `canwu-api` never re-exported the type.
  - `ContinuationCheckpoint` (`CONTINUATION_CHECKPOINT_FORMAT_VERSION` 1),
    `Canwu::continuation_checkpoint()` and
    `CompactedCanwu::continuation_checkpoint()`, which return the checkpoint
    with its `continuation_hash`, and
    `CompactedCanwu::from_continuation_checkpoint`,
    `from_continuation_checkpoint_with_plugins`,
    `from_continuation_checkpoint_verified(checkpoint, expected_hash,
    plugins)`, and the JSON forms, with the carried types
    (`JournalFrontiers`, `UnadmittedEvidence`, `PinnedEvidenceRecords`,
    `PluginIngressProvenanceFact`, `CurrentRecordVersion`, `KeyedDrawEntry`,
    `RequestIndex`, `HistoryFlags`, `EvidenceTimes`, `EvidenceTimeRun`).
    `CompactedCanwu::continuation_base()` returns the base cut and hashes of a
    continuation-restored run, or `None`. Additive.
  - `CompactedCanwu` and `CompactedSimulation` gain the host reads and entry
    points of `Canwu` whose meaning does not need full history:
    `set_plugin_archive_object_provider`, `plugin_archive_object`, `fork`
    (keeping the committed seal tokens), `viewer`, `viewer_for_actor`,
    `viewer_context`, `admin_query_knowledge`, `explain`,
    `authoritative_state_hash`, `run_manifest_hash`, `plugin_descriptors`,
    `evidence_exists`, `evidence_time`, `domain_record_version`,
    `domain_record_version_evidence_exists`,
    `current_domain_record_version`, `domain_records`, `domain_record_page`,
    `evidence_cursor`, and the retained-tail `boundaries`, `events`,
    `commands`, `command_attempts`, `ingress_log`, and `random_draws`.
    Additive.
  - `replay_stable_domain_record_version` has a host form on `Simulation`,
    `CompactedSimulation`, `Canwu`, and `CompactedCanwu`: the body and
    establishment time of a record's committed current version or of an
    initial-scenario version, `None` for any other. Additive.
  - `SimulationView` gains the read-only `plugin_ingress_payload_hash` (the
    canonical hash of a packet the boundary admits, under a caller-supplied
    domain), `knowledge_holder_accepts_publication` (the exact holder rule
    the engine applies to a `PublishKnowledge` directive, at the same state),
    `ingress_is_declarable` (whether an ingress record is queued or admitted
    by the boundary being settled, alike in every run), and
    `can_schedule_ingress` (false in a run that holds direct command history,
    where a scheduled packet would fail the boundary). Additive.
  - `Simulation::replay_from_journal_with_boundary_profiles(plugins, journal,
    archive_provider, on_profile)` and the same function on `Canwu` replay
    with profiling on from the first recorded boundary and hand each
    `BoundaryProfile` to `on_profile` as the boundary settles; the returned
    runtime has profiling off. `BoundaryProfile` gains `engine`, one
    `BoundaryEngineTiming { phase, elapsed_micros }` per `BoundaryEnginePhase`
    (`Admission`, `SystemDispatch`, `Staging`, `Commit`, `Knowledge`,
    `Journal`, `Hashing`; `BoundaryEnginePhase::ALL` is a slice and the enum
    is non-exhaustive) and `engine_micros(phase)`; the handlers and the engine
    phases partition `total_micros` to within one microsecond per listed
    value. Additive, except that a struct literal of `BoundaryProfile` must
    name `engine`.
  - `MAX_PAGED_RECORD_BYTES` (512 MiB) and `STATE_PAGE_VALUE_CHUNK_BYTES`
    (1 MiB) are new; `STATE_PAGE_CODEC` is `raw-canonical-v2`;
    `PAGED_CHECKPOINT_FORMAT_VERSION` is 5; a live record above the ceiling
    makes `state_pages`, `missing_state_pages`, and
    `prepare_paged_checkpoint` fail with `value_out_of_range`, which is not a
    corrupt page.
  - `validate_directives` refuses with `InvalidPayload` a
    `SystemDirective::Schedule` that wraps `EnqueuePluginIngress` at any
    depth, and the snapshot and continuation loaders refuse scheduled work
    holding one. `EnqueuePluginIngress` takes its own delay, which gives the
    same due time. No first-party extension scheduled one.
- `canwu-api`:
  - `pub trait CanwuRun`, implemented by `Canwu` and `CompactedCanwu`: `time`,
    `revision`, `checkpoint_hash`, `entity_exists`, `domain_record`,
    `domain_records` (a boxed iterator), `current_domain_record_version`,
    `domain_record_version_evidence_exists`,
    `replay_stable_domain_record_version`, `evidence_cursor`,
    `boundaries_between`, `boundary_schedule_horizon`,
    `enqueue_plugin_ingress`, `enqueue_permitted_plugin_ingress`, and
    `admin_query_knowledge`. It is a read-and-ingress contract that settles
    nothing and requires neither `Send` nor `Sync`.
  - `Canwu`, `CompactedCanwu`, `Simulation`, `CompactedSimulation`, and every
    fork of them are `Send` ([a `Send` runtime](proposals/send-runtime.md));
    `Sync` is not promised, although nothing the handles hold today has
    interior mutability. The run holds its plugin archive provider as
    `Arc<dyn PluginArchiveObjectProvider + Send + Sync>`, and a fork shares
    it, so `set_plugin_archive_object_provider` on all four handles,
    `replay_from_journal_with_archive_provider`, and
    `replay_from_journal_with_boundary_profiles` take that type instead of
    `Rc<dyn PluginArchiveObjectProvider>`; so do the three
    `canwu_resource::*_with_archive_store` restores (`Arc<dyn
    ResourceArchiveStore + Send + Sync>`), the three
    `canwu_production::*_with_archives` restores (both stores), and the three
    force-supply restore helpers. The bound is on the holder, not a
    supertrait: a provider or store a run keeps is `Send + Sync`, one lent
    for a call (`&dyn Trait`, a `SimulationView` passed as a provider) is
    not constrained. `GrainHarness` keeps its store behind mutexes and is
    `Send`; its `archive_store()` returns an `Arc`. A host passes
    `Arc::new(store)` where it passed `Rc::new(store)` and moves `RefCell`
    maps in a held store to `Mutex` or `RwLock`. No format, hash, or replay
    input changes. Source-incompatible.
    `canwu_api::typed_domain_record(run, reference)` is the typed record read
    over it. Additive.
  - Every public extension function whose first parameter was `&Canwu` or
    `&mut Canwu` and that uses only those reads and canonical ingress now
    takes `&H` or `&mut H` with `H: CanwuRun + ?Sized`; callers that pass a
    `Canwu` compile unchanged. The inventory is in the continuation proposal
    (Part F): the resource, fiscal, law, military, production, movement,
    economy-reference, force-supply-reference, and Ming fiscal reference
    enqueue, finalize, state, report, and witness helpers. Functions that read
    history or exact version bodies keep `Canwu`: the restore and replay
    wrappers, the technology, production, society, and historical research
    validators, the force-supply restore audits, the Ming reference sample
    cycle and trace, and the reference world's `snapshot`.
- `canwu-society`:
  - `TransferSettlement` and `SocietyPlugin::with_transfer_settlement(mode)`
    choose how a settlement checks the cohort transfers due at it. `Batch`,
    the default for `SocietyPlugin::DAILY`, `SocietyPlugin::default()`, and
    `with_settlement_cadence`, checks every due transfer against the state at
    the start of the settlement and applies the ones that still fit in
    admission order, each to the state the earlier ones left; a transfer
    whose cohorts an earlier settlement changed after it was admitted settles
    as `stale_source`, and one whose quantity no longer fits its source
    cohort as `insufficient_source`
    (`COHORT_TRANSFER_INSUFFICIENT_SOURCE`). `OneByOne` is the 0.15.0 rule:
    each due transfer is checked against the state the earlier transfers of
    the same settlement left, in operation-ID order, so of the due transfers
    that share a cohort one completes and the others settle as
    `stale_source`. The mode is part of the plugin identity, so a run is
    restored and replayed only with a plugin of the same mode
    (`PluginManifestMismatch` otherwise). `SocietyPlugin::transfer_settlement`
    reads it. `SocietyPlugin` gains a private field and stays `Clone`.
  - The owner-side ingress queue is a head record (`SocietyIngressQueue`
    with `queued` and `batches` in place of `entries`, which is
    source-incompatible) and one `SocietyIngressBatch` record per admitting
    boundary (`SocietyIngressBatchRecord`, `society_ingress_batch_reference`);
    `society_ingress_queue_entries` reads the waiting packets in admission
    order, and `SocietyIngressQueue::empty` builds an empty head. A packet
    that finds the queue full is refused by the boundary that admits it, with
    a terminal `queue_full` outcome in the cohort exchange ledger
    (`SOCIETY_INGRESS_QUEUE_FULL_REJECTION`) and one event; the
    `revise_society_pressure` command is refused at admission while the
    queue is full, as a recorded `ValueOutOfRange` rejection.
- `canwu-resource`:
  - `RESOURCE_INGRESS_SKIPPED_EVENT` (`canwu.resource.ingress_skipped.v1`),
    `ResourceIngressSkipReasonV1` (`operation_key_taken`,
    `reserved_operation_key`, `failed_settlement_checks`, `archive_backlog`,
    `reserved_capacity`, `root_limit`; `is_capacity` is true for the last
    three), `ResourceIngressSkipV1` with `from_event` and `deferred_until`,
    `RESOURCE_DEFERRED_INGRESS` (`resource_deferred_packet_v1`, internal),
    `ResourceLimitsV1::MAINTENANCE_STATE_BYTE_RESERVE` (64 KiB) and
    `operation_state_bytes`, and `enqueue_resource_completion_expiry_tick`.
    Additive; `RoomShortage` stays internal.
  - The lookup-index functions take key digests and are public, so another
    archive can keep an index in the same format:
    `prepare_resource_archive_lookup_entries`,
    `find_resource_archive_lookup_entry`,
    `verify_resource_archive_lookup_entries`,
    `collect_resource_archive_lookup_pages`, and
    `audit_resource_archive_lookup_index` with `ResourceArchiveAuditEntry`.
    `MAX_PRODUCTION_OUTPUT_BATCH_CREDITS` (64) and
    `ResourceState::production_output_batch_can_fit` are new. Additive.
  - `ResourceState::apply_operation` measures room on the result, so it
    settles some operations 0.15.0 refused for room, refusals in particular;
    its `LimitExceeded` messages for room changed. `enqueue_resource_allocation`
    refuses a requester that is not an entity of the run (`EntityNotFound`).
    `finalize_resource_archive_retention` reads the disposition from the
    state-side retention handle and succeeds after a later commit archived
    the receipt.
  - The root persists in parts ([resource root write
    cost](proposals/resource-root-write-cost.md)). New: `ResourceLiveRootV1`,
    `ResourceHotTerminalLedgerV1`, `ResourceHotTerminalEntryV1`,
    `ResourceHotTerminalBucketV1`, `ResourceHotTerminalBucketRecord`,
    `RESOURCE_HOT_TERMINAL_BUCKET_FORMAT_VERSION`,
    `resource_hot_terminal_bucket_reference`,
    `resource_hot_terminal_bucket_count`, `resource_hot_terminal_bucket`,
    `resource_hot_terminal_state_key`, the readers `ResourceRootReader`,
    `resource_root_in_view(view, keys)` (the live root with the hot terminal
    records the keys name and their closures), and
    `whole_resource_root_in_view` (every bucket), `ResourceState::{
    into_records, from_records, hot_terminal_records, hot_operation_outcomes,
    hot_terminal_records_attached, refresh_hot_terminal_ledger}`, the field
    `ResourceState::hot_terminal`, and `ResourceLimitsV1::MAX_OPERATION_OUTCOMES`.
    Breaking: `ResourceRuntimeRecord::Payload` is `ResourceLiveRootV1`, so
    code that decoded the root and read a hot terminal record from it no
    longer compiles; a boundary system or command handler of another plugin
    reads the root through the readers and declares the read of
    `resource_hot_terminal_state_key()` beside the live root's;
    `ResourceState` gains a field (struct literals break);
    `ResourceState::into_record` refuses a root that holds hot terminal
    records (use `into_records`); `prepare_resource_archive` refuses a
    partial root; `archive_source_root` has a new definition;
    `ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES` rises from 4,096 to 16,384,
    `MAX_RESOURCE_ARCHIVE_COMMIT_PAGES` from 16 to 64
    (`MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS` 16,384), and the hard maximum of
    `max_operation_outcomes` from 4,096 to 16,384 (the canonical values stay
    256 and 8,192). `resource_state` reads the whole root from `Canwu` and
    `CompactedCanwu`. A fixture that places candidates in the maps directly
    calls `refresh_hot_terminal_ledger`. In `canwu-force-supply-reference`,
    `from_force_supply_snapshot_json`, `from_force_supply_checkpoint_journal`,
    and `replay_force_supply_from_journal` take the archive provider and
    attach it before they validate, so they compare every resource outcome a
    force runtime cites, hot or archived; `validate_force_supply_runtime`
    compares the hot ones only and
    `validate_force_supply_runtime_with_archive_store` reads through the
    attached provider. The provider is an
    `Arc<dyn PluginArchiveObjectProvider + Send + Sync>`. Source-incompatible.
- `canwu-production`:
  - `ProductionState::prepare_production_archive(candidate_limit, store)`
    takes the archive store (`()` is the absent store). `MAX_ARCHIVE_PAGE_ENTRIES`
    falls from 512 to 64, `MAX_ARCHIVE_PREPARE_CANDIDATES` from 2,048 to
    1,024 (a commit can now carry that many), `MAX_ARCHIVE_OBJECTS_PER_BATCH`
    rises from 1,024 to 8,192 and `MAX_ARCHIVE_BYTES_PER_BATCH` from 16 MiB
    to 64 MiB; `MAX_ARCHIVE_COMMIT_PAGES` (16) is new.
    `PreparedProductionArchiveBatchV1` gains `selected_receipts`,
    `additional_pages`, `lookup_pages`, `superseded_lookup_pages`,
    `validate`, and `selected_count`; `VerifiedProductionArchiveCommitV1`
    gains `selected_receipts`, `lookup_root`, `validate`, and
    `selected_count`. New: `PreparedProductionArchivePageV1`,
    `ProductionArchivedRecordV1`, `PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE`,
    `PRODUCTION_ARCHIVE_BLOB_FORMAT_VERSION`,
    `production_archive_lookup_key_digest`,
    `archived_production_terminal_record`, and
    `live_production_archive_lookup_pages`;
    `ProductionTerminalArchiveKeyV1::ArchiveMaintenanceReceipt` is a new key
    kind. Source-incompatible.
  - `ProductionOutputAcknowledgement` carries the required `settled_at`;
    `acknowledge_output` requires the execution's pinned `output_source` to be
    set and equal to the acknowledgement's source, and every outcome's
    request digest to commit to `settled_at`. `enqueue_production_archive`
    refuses a batch that differs from what the current root and the store
    derive; `acknowledge_production_archive_retention` is a no-op for a
    handle the root does not hold. A process may declare at most 64 outputs
    (was 128). The events `canwu.production.ingress_refused.v1` and
    `canwu.production.output_ack_refused.v1` are new.
- `canwu-fiscal` (the world-scale fiscal package on top of the fiscal and
  society package):
  - `FiscalState` is the logical and authoring model: `root` (`FiscalRoot`),
    `archive` (`FiscalArchiveHeadV1`), `offices` (`FiscalOffice`, by
    `fiscal_office_key`), `ledgers` (`FiscalScopeLedger`, one per scope
    binding, each with one `FiscalCycleBook` per hot accounting cycle),
    `jurisdictions` (`FiscalJurisdiction`), `observers` (`FiscalObserver`),
    and `report_queue`, with the `add_*` and `remove_observer` authoring
    helpers, `validate`, `into_records(&catalog)` (which derives every
    derived field and partitions the state into records), and
    `from_records`. `audit_fiscal_records(&run)` and
    `audit_fiscal_record_set(records)` check that a run's records are exactly
    the partition of a valid state; activation runs the audit on every
    scenario and restored snapshot. `load_fiscal_state` assembles the state
    at a cost proportional to the fiscal records. `fiscal_root_reference`,
    `fiscal_office_reference`, `fiscal_ledger_reference`,
    `fiscal_jurisdiction_reference`, `fiscal_observer_reference`, and
    `fiscal_archive_reference` name single records. `FiscalStateRecord`,
    `fiscal_state_reference`, `FiscalState::into_record`,
    `compute_aggregates`, `recompute_derived`, `compute_projections`,
    `FiscalState::report_plan`, `FiscalReportPlan`, `FiscalActionOutcome`,
    `MAX_FISCAL_ACTION_OUTCOMES`, `MAX_FISCAL_STATE_JSON_BYTES`,
    `MAX_FISCAL_REPORT_PLAN_BYTES`, `MAX_FISCAL_RUNTIME_BINDINGS`,
    `MAX_FISCAL_ASSESSMENTS`, `MAX_FISCAL_EXECUTION_REQUESTS`,
    `MAX_FISCAL_EXECUTION_RECEIPTS`, `MAX_FISCAL_REPORT_PARTS`,
    `FISCAL_REPORT_FACTS_PER_PART`, `FiscalStrategicAggregate` as state, and
    the two derived boundary systems are removed; `compute_projection(state,
    observer_id, at)` derives one observer's projection. The flat maps of
    assessments, remissions, requests, receipts, audits, adoptions, scope
    bindings, and outcomes are read through `FiscalState::assessments()` and
    the other iterators, `ledger`, `scope_binding(s)`, and `books`.
  - The command payload is a `FiscalActionBatch { authority_binding_id,
    entries }` of 1 to `MAX_FISCAL_BATCH_ENTRIES` (4,096) `FiscalActionEntry
    { action_id, expected_revision, action }` values
    (`MAX_FISCAL_BATCH_JSON_BYTES`, 4 MiB; `FiscalActionBatch::single` makes a
    batch of one); `FiscalActionRequest` is no longer public (it is the
    plugin's internal per-entry form) and `expected_procedure_revision` is
    gone. Every action has a `FiscalActionTarget` (`Ledger`, `Office`,
    `Jurisdiction`, or `Observer`) whose own revision it names;
    `fiscal_action_target`, `fiscal_target_revision`, and
    `fiscal_recent_outcome` are the host reads. `GrantRemission`,
    `AuthorizeExecution`, and `RecordAudit` gain `scope_binding_id`; the new
    variants are `CloseAccountingCycle`, `RebindObserver`, `RemoveObserver`,
    `NominateAuditor`, `PetitionAuditorRemoval`, and
    `DecideAuditAppointment` (`FiscalAppointmentDecision`);
    `FiscalIssuerRole` gains `ListedActor`. Book objects gain
    `source_action_id`. `FiscalSettledAction::from_summary` and `from_events`
    read outcomes back from `canwu.fiscal.action_settled.v1`
    (`FISCAL_ACTION_SETTLED_EVENT`). Source-incompatible.
  - Admission refuses an action ID with an outcome in its target's ring of
    `FISCAL_RECENT_OUTCOMES` (8) (`IdempotencyConflict`), a revision above
    the target's (`FISCAL_REVISION_UNKNOWN`), and one at or below the
    target's outcome floor (`FISCAL_REVISION_EXPIRED`), each as
    `ValueOutOfRange`; it refuses a whole batch for an inadmissible entry,
    naming its index, and for one action ID named twice for one target.
    The limits, each a recorded rejection: `MAX_FISCAL_ACTIONS_PER_BOUNDARY`
    rises from 64 to 65,536 (`FISCAL_BOUNDARY_ACTION_LIMIT`);
    `MAX_FISCAL_LEDGER_WRITES_PER_BOUNDARY` (65,536) and
    `MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY` (64,
    `FISCAL_RECORD_ACTION_LIMIT`), `MAX_FISCAL_PARTITION_KEYS` (4,
    `FISCAL_PARTITION_KEYS_EXHAUSTED`), `MAX_FISCAL_HOT_BOOKS_PER_LEDGER`
    and `MAX_FISCAL_HOT_CYCLES_PER_OFFICE` (64, `FISCAL_HOT_BOOKS_EXHAUSTED`),
    and the record byte budgets `MAX_FISCAL_LEDGER_RECORD_BYTES` (256 KiB)
    and `MAX_FISCAL_RECORD_BYTES` (1 MiB for an office or jurisdiction), each
    held at settlement less `FISCAL_RECORD_RING_RESERVE_BYTES` (4 KiB)
    (`FISCAL_RECORD_BUDGET_EXHAUSTED`, which replaces
    `FISCAL_STATE_BUDGET_EXHAUSTED`). Other new constants:
    `MAX_FISCAL_SCOPE_BINDINGS` (1,048,576),
    `MAX_FISCAL_AUTHORITY_BINDINGS_PER_OFFICE` and
    `MAX_FISCAL_AUDIT_BINDINGS_PER_OFFICE` (256), `MAX_FISCAL_OBSERVERS`
    (65,536, was 64), `MAX_FISCAL_VISIBLE_INSTITUTIONS` and
    `MAX_FISCAL_OBSERVERS_PER_INSTITUTION` (4,096), `MAX_FISCAL_OBSERVER_ROWS`
    (16,384), per-book caps (`MAX_FISCAL_REMISSIONS_PER_BOOK`,
    `MAX_FISCAL_EXECUTION_REQUESTS_PER_BOOK`, `MAX_FISCAL_AUDITS_PER_BOOK`
    256, `MAX_FISCAL_EXECUTION_RECEIPTS_PER_BOOK` 1,024,
    `MAX_FISCAL_ADOPTIONS_PER_LEDGER` 64), `FISCAL_REPORT_ROWS_PER_PART` (16),
    `MAX_FISCAL_REPORT_PARTS_PER_CYCLE` (1,024),
    `MAX_FISCAL_REPORT_CYCLE_SLOTS` (64),
    `FISCAL_REPORT_QUEUE_PAGES` (4,096), `FISCAL_REPORT_QUEUE_CYCLES` (16),
    `FISCAL_REPORT_QUEUE_PARTS` (64), `FISCAL_DIRECTORY_BUCKETS` (4,096),
    `FISCAL_INDEX_PAGE_SCOPES` (1,024), `MAX_FISCAL_CATALOG_REGIONS` (65,536,
    was 1,024), and `MAX_FISCAL_CATALOG_COVERAGE_CELLS` (1,048,576, was
    65,536). The stable reasons `FISCAL_CYCLE_CLOSED`,
    `FISCAL_CYCLE_ARCHIVED`, and `FISCAL_OBSERVER_REMOVED` are new.
  - Receipts come as `fiscal_execution_receipts_v1` packets of up to
    `MAX_FISCAL_RECEIPTS_PER_PACKET` (4,096) through
    `enqueue_execution_receipts(run, due_at, &[packet])`, which replaces
    `enqueue_execution_receipt`; `FiscalExecutionReceiptPacket` and
    `FiscalExecutionEvidence` gain `scope_binding_id`. Every refusal of a
    receipt or a historical-context change is one
    `canwu.fiscal.ingress_rejected.v1` event
    (`FISCAL_INGRESS_REJECTED_EVENT`) whose summary ends with a stable
    reason: `FISCAL_INGRESS_MALFORMED_REJECTION` (`malformed_payload`),
    `FISCAL_INGRESS_BUDGET_REJECTION` (`state_budget_exhausted`),
    `FISCAL_INGRESS_INVALID_REJECTION` (`invalid_packet`),
    `FISCAL_INGRESS_ORIGIN_REJECTION` (`unauthorized_origin`),
    `FISCAL_RECEIPT_REQUEST_UNKNOWN`, `FISCAL_RECEIPT_CYCLE_CLOSED`,
    `FISCAL_RECEIPT_EVIDENCE_COUNT`, `FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED`,
    `FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT`, `FISCAL_RECEIPT_CLAIM_MISMATCH`,
    `FISCAL_RECEIPT_QUANTITY_EXCEEDED`, `FISCAL_RECEIPT_OPERATION_REUSED`,
    `FISCAL_RECEIPT_EVIDENCE_REUSED`, `FISCAL_RECEIPT_ID_CONFLICT`,
    `FISCAL_RECEIPT_BOUNDARY_LIMIT`, and `FISCAL_CONTEXT_OUTSIDE_PACK`. The
    action packet `fiscal_action_v1` is internal ingress, so
    `enqueue_plugin_ingress` refuses it with `InvalidAuthority`.
  - Auditor appointment: `FiscalAppointmentPolicy` (`appointer`,
    `nominators`, `petitioners`, `appoints_without_nomination`,
    `removes_without_petition`), `FiscalAppointmentParty`,
    `FiscalAuditProposal`, `FiscalAuditProposalKind`,
    `FiscalProposalResolution`, `FiscalProposalOutcome`, `FiscalProposalRef`,
    `FiscalState::add_appointment_policy` and `add_audit_proposal`,
    `MAX_FISCAL_APPOINTMENT_PARTIES` (32),
    `MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY` (4),
    `MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY` (64), and the reasons
    `FISCAL_APPOINTMENT_POLICY_MISSING`, `FISCAL_ISSUER_NOT_APPOINTER`,
    `FISCAL_ISSUER_NOT_LISTED`, `FISCAL_NOMINATION_REQUIRED`,
    `FISCAL_PETITION_REQUIRED`, and `FISCAL_PROPOSAL_NOT_PENDING`.
    `FiscalAuditBinding` gains `nomination`; `FiscalAuditFinding` gains
    `appointed_by` and `nomination`. `RebindAudit` keeps its 0.15.0 shape and
    behaviour only where the policy makes the audited institution its own
    appointer and lets it act without a nomination, a petition, or both.
  - Observers and reports: `FiscalObserverBinding::actor` is optional (a
    government holds a report with no actor), and the binding gains
    `controlled_by`, `rebound_by`, and `removed`. `fiscal_reports(&viewer)`
    returns one `FiscalReport` per binding whose report the viewer's holder
    holds; `fiscal_reports_of_holder(run, holder)` reads any holder as a
    trusted host; `FiscalReport` keeps `projection` and gains `pending`
    (the parts not current, by cycle) and `omitted_cycles`, the summary
    record lists the cycles it shows and marks a `withdrawn` report,
    `FiscalProjection::actor` is optional, and a `FiscalReportFact` names
    its `scope_binding_id` and `accounting_cycle_id` and is labelled
    `cycle:row:fact`. `FISCAL_REPORT_DRAIN_INGRESS`
    (`fiscal_report_drain_v1`, internal) is the drain packet.
  - The fiscal archive: `FiscalState::prepare_fiscal_archive(book_limit,
    store, sealed_cut)`, `PreparedFiscalArchiveBatchV1` with
    `store_and_verify` and `validate`, `VerifiedFiscalArchiveCommitV1`,
    `enqueue_fiscal_archive(run, prepared, store, sealed_cut)`,
    `finalize_fiscal_archive_retention(run, store, receipt)`,
    `FiscalArchiveIngressReceiptV1`, `archived_fiscal_record(state, store,
    key)`, `audit_fiscal_archive(store, state)`,
    `live_fiscal_archive_lookup_pages`, `fiscal_archive_lookup_key_digest`,
    `FiscalArchiveStore` (`()` holds nothing), `FiscalArchiveError` (with
    `UnsealedCitations` and `Backpressure`), `FiscalArchiveKeyV1` (`Book`,
    `InstitutionTotals`, `ArchiveMaintenanceReceipt`), the archived record
    and object types, the ingress types `fiscal_archive_commit_v1`
    and `fiscal_archive_retention_v1`, the event
    `canwu.fiscal.archive_maintenance.v1`, the `FISCAL_ARCHIVE_*` namespaces,
    `FISCAL_ARCHIVE_BLOB_FORMAT_VERSION` (1),
    `MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB` (64),
    `MAX_FISCAL_ARCHIVE_BLOBS_PER_COMMIT` (64),
    `MAX_FISCAL_ARCHIVE_COMMIT_RECORDS` (4,096), and
    `MAX_FISCAL_ARCHIVE_PENDING_HANDLES` (64). `canwu-fiscal` depends on
    `canwu-resource` for the lookup-index functions only; no resource type
    appears in the fiscal API. Every host helper takes any `CanwuRun`.
- `canwu-law`: `LAW_INGRESS_REJECTED_EVENT`, `LAW_PROPOSAL_REJECTED_EVENT`,
  `LegalProposalRejection`, `LegalProviderPacket`,
  `LEGAL_PROVIDER_PAYLOAD_DOMAIN`, `LawBudgets::max_provider_packets`,
  `LegalRuntime::provider_packets`, and `LegalBoundaryResult::{
  rejected_proposals, deferred }` are new; `LegalRuntime::settle_boundary` no
  longer fails on a per-boundary budget or on a passed final stage whose
  adoption guard fails (it settles a slice, reports `deferred`, and the
  caller settles again); `LegalRuntime::record_publicity` marks the
  proposal's procedure for the next settlement.
- `canwu-information`: `INFORMATION_REJECTION_EVENT`, `DelegatedAuthority`,
  `DelegatedAuthorityPayload`, and `delegated_authority_reference` are new;
  `information_record_schemas` returns 13 schemas; the finalization ingress
  is internal.
- `canwu-military`: no API change; the semantic hash changes because every
  operation-keyed draw now cites the admitted packet that carries its
  command or tick.
- `canwu-force-supply-reference` and `canwu-economy-reference` (not
  published): `PackageArchiveDomainV1` gains `lookup_namespace`,
  `PackageArchiveIndexDirectoryV1` gains `lookup_root`,
  `PreparedPackageArchiveBatchV1` gains `lookup_pages` and
  `superseded_lookup_pages`; `prepare_package_archive`,
  `prepare_force_archive`, and `prepare_economy_archive` take the archive
  store; `extend_archive_reachability` takes the head and the pending roots
  separately; `FORCE_ARCHIVE_LOOKUP_PAGE_NAMESPACE`,
  `ECONOMY_ARCHIVE_LOOKUP_PAGE_NAMESPACE`, `package_archive_lookup_key_digest`,
  `package_archive_commit_room`, and `impl PackageArchiveStore for ()` are
  new; `enqueue_force_archive` and `enqueue_economy_archive` refuse, before
  anything is queued, a commit the root and the queued commits leave no
  receipt or handle room for (`archive_backpressure`).
- `canwu-ming-fiscal-reference` (not published):
  `validate_ming_fiscal_reference<R: CanwuRun + ?Sized>(&R)` takes either
  handle and reads only seal-stable state; `close_ming_fiscal_sample_cycle`
  and `archive_ming_fiscal_books` are new; `enqueue_reference_execution_result`
  and `reference_execution_evidence_version` take any `CanwuRun`.

The source-incompatible changes are the `DomainRecordChange` fields, the
knowledge ledger types and their forward-only iterators, the decision-log
iterators, the non-exhaustive `ErrorCode`, the removed keyed reservations, the
`SocietyIngressQueue` fields and the new `SocietyPlugin` field, the production
archive signatures, constants, and record fields, the whole fiscal state and
action API, and the reference package archive signatures. The release is
therefore a minor version.

Version 0.16.0 also changes these behaviours, which are visible to existing
code and runs:

- **An expected refusal never fails a boundary.** Across the extensions a
  refusal of host input is now a recorded rejection, a skipped packet with an
  event, or a deferral, never a failed boundary that stops the run:
  - `canwu-resource`: a packet whose result does not fit the root's archive
    budgets, whose operation key a different request holds, that names a
    reserved key, or that does not decode or fails its settlement checks is
    skipped with `canwu.resource.ingress_skipped.v1`, changes nothing, and
    leaves its key free; room is measured on the result, so a refused
    operation needs room for its outcome only. A production output batch, or
    a completion step a plugin scheduled that can settle later as the same
    step, that does not fit while a commit or a lease release can make room
    is deferred to the plugin itself, one minute after the first deferral and
    doubling up to 1,024 minutes; work no wait can fit is skipped at once.
    A resend whose outcome is recorded settles nothing on every ingress and
    no longer rewrites the root. An expiry tick skipped for room schedules no
    successor; the next applied archive commit resumes it. An expiry tick
    whose settlement key a host completion operation holds is a resend of
    the same expiry, or is skipped with `operation_key_taken` and scheduled
    again at a boundary whose key is free, where 0.15.0 failed the boundary
    with an idempotency conflict. A tracked
    `Acquire` must be eligible at the instant it is admitted and cite current
    evidence. What still fails a resource boundary is an unreadable or forged
    archive object during a key lookup, a forged archive packet, a root that
    does not validate for a reason other than a limit, maintenance writes
    past the byte reserve, and arithmetic or encoding failure.
  - `canwu-production`: a packet that does not decode, an operation packet
    not queued by a tracked command's admission, and a completion step or
    continuation witness that production's checks refuse are refused with
    `canwu.production.ingress_refused.v1`; an acknowledgement that does not
    settle its execution exactly with `canwu.production.output_ack_refused.v1`;
    a stale or duplicate archive commit and a repeated acknowledgement settle
    as no-ops; a commit never fails for capacity. `StartExecution` refuses an
    output key under the reserved prefix, one another unsettled execution
    names, or one a resource outcome holds in the root or its archive, more
    than 64 outputs, and an output set the resource root can never hold.
  - `canwu-fiscal`: no receipt, historical-context change, action packet, or
    archive packet fails a boundary; each is refused with
    `canwu.fiscal.ingress_rejected.v1`, and every limit is a rejected outcome.
    Owed report parts are published by the next boundary at the same instant
    through the internal drain packet, or by the boundary that revives a dead
    observer, without a fiscal change. An authored state whose first
    derivation would not fit is refused when the run is created.
  - `canwu-society`: a packet that finds the queue full is a recorded
    `queue_full` ledger outcome at the boundary that admits it, not a drop.
  - `canwu-law`: each admitted law packet is applied or refused on its own
    with `canwu.law.ingress_rejected.v1`; per-boundary budgets defer work to
    a zero-delay continuation wake at the same time; a stale host
    compare-and-set, a retired culture dependency, an adoption guard that can
    never hold, and an adoption past a lifetime budget reject the proposal
    with `canwu.law.proposal_rejected.v1`; a refused archive commit records a
    `RejectedStale` terminal; a packet that names an unheld shard is refused
    before load; a signal must target a deliberating proposal; wakes no
    longer count against the ingress budget.
  - `canwu-information`: an undecodable or conflicting operation packet is
    refused with `canwu.information.ingress_rejected.v1`, the input matching
    a persisted operation is kept, operations are planned oldest first and
    one whose plan touches a record already changed in the pass waits a
    boundary, a holder the engine does not accept at publication is left
    unpublished, and a record naming a holder kind that holds no knowledge is
    refused at creation.
  - `canwu-economy-reference` and `canwu-force-supply-reference`: a second
    command reusing a settled operation ID with other input is skipped with
    an `operation_rejected` event; the economy root ends a boundary one
    revision past its record version however many packets changed it.
- **Boundary decisions no longer read retained evidence.** `canwu-law` checks
  host `expected_versions` against `current_domain_record_version`, checks
  provider citations against the bounded legal provider index it records at
  admission (at most `max_provider_packets`, lowest ingress ID forgotten
  first), and refuses a cited record version that is not replay-stable or a
  cited packet that is neither declarable now nor already declared by law.
  `canwu-information` records a delegation when its command is admitted and
  reads the `DelegatedAuthority` record afterwards, finalizes publications
  through `knowledge_heads`, and publishes each holder, kind, and subject set
  at most once per boundary. `canwu-military` cites the admitted packet that
  carries the command or tick as the operation evidence of every keyed draw,
  so reconnaissance now succeeds. `canwu-history-research` resolves related
  assessments through `replay_stable_domain_record_version`. The Ming fiscal
  validator reads only current records, replay-stable versions, and journal
  counters.
- **Sealing with work in flight.** A seal moves events, commands, and
  attempts up to the admission cursors and the other journals to their end;
  pending ingress and the unadmitted tail no longer block it. The
  continuation closure below the cut (pending ingress, the causes of pending
  work, payload-required declarations, and identity facts) is pinned in
  memory, a boundary that replaces a version whose establishing boundary is
  sealed pins the version it replaces, and every pending or declared ingress
  record keeps a provenance fact, so `plugin_ingress_matches`,
  `plugin_ingress_payload_matches`, and cancellation answer as an unsealed
  run does. The one-phase seal no longer refuses a run with
  `PayloadRequired` dependencies. A declared dependency that no longer exists
  or can no longer be read, including an ingress record first declared after
  its seal and no longer held, fails the seal, the two-phase commit, or a
  continuation checkpoint with `ArchiveNotReady` and changes nothing.
- **Seal-stable evidence identity.** Existence of sealed ordinal evidence
  comes from the journal counters and its time from a run-length index, so
  `evidence_exists` and `evidence_time` answer alike in a live, sealed,
  restored, continuation-restored, and replayed run; a knowledge origin or an
  identity-only declaration may cite evidence sealed earlier. A fresh
  operation-keyed draw accepts sealed ordinal evidence and refuses a
  non-current, non-initial record version in every run
  (`InvalidRandomOperationEvidence`; 0.15.0 accepted it unsealed). A sealed
  payload read (`command`, `event`, `ingress`, `domain_record_version` of a
  version whose existence is known) reports `EvidenceContentUnavailable`
  instead of `Ok(None)`.
- **The keyed-draw ledger.** The keyed-draw index holds every drawn address,
  retained or sealed, for the life of the run; keyed draws are no longer
  evidence dependencies, and a checkpoint, a paged checkpoint, a seal, and a
  fork do no work in proportion to them. Every restore builds the ledger
  once.
- **A command cannot schedule plugin ingress.** A command or event handler
  whose directives wrap `EnqueuePluginIngress` in `SystemDirective::Schedule`
  at any depth is refused with `InvalidPayload` before anything changes, and
  loaders refuse scheduled work holding one: the packet such a directive
  fires carries a cause but no issue time, which the full-save loader could
  not check exactly.
- **Plugins must not declare initial-scenario record versions as
  payload-required.** The engine refuses such a declaration at the seal, so a
  run whose root held one could not seal or write a continuation checkpoint
  from its first boundary on. An initial-scenario version stays readable
  after any seal, so it needs no declaration. The production and resource
  roots no longer declare one; a plugin that does cannot seal.
- **Held knowledge no longer costs a boundary.** A boundary, command, or
  scheduled batch that changes no knowledge does nothing in proportion to the
  records a run holds; a publishing boundary pays for its own records;
  `knowledge_heads` reads an index of current records and
  `knowledge_record_count_in_namespace` one count. Superseded records stay in
  the ledger, in `FullHistory` views, and in every snapshot, as before.
- **An idle boundary no longer costs the records a run holds**, and a record
  write costs the record: the current-version index is a persistent map,
  phase 7 applies its same-boundary writes once and the phase-9 commit takes
  the prepared result, records are shared rather than copied, and Patricia
  node hashes are computed once per boundary.
- **Fiscal.** The state is partitioned, so a boundary decodes and writes only
  the records its packets touch and an idle boundary reads the root only.
  Each ledger, office, jurisdiction, and observer has its own revision; an
  action is stale only when its own target changed since its issuer looked
  (a historical-context change makes nothing stale; a transition advances
  every ledger whose adoptions it changes). A re-sent action is refused at
  admission from hot state alone: its outcome is in the target's ring of 8,
  or its expected revision is at or below the target's floor. Accounting
  cycles close per scope by `CloseAccountingCycle` (a closed book takes only
  audit findings) and archive into the fiscal archive; no new book is opened
  at or below the scope's or its office's watermark, so an accounting
  partition is assessed at most once for the life of a run without reading
  the archive; no boundary reads the archive store. Observers can be
  re-pointed and removed, a government can hold a report, report lines live
  on each binding's observer record, parts are republished only when their
  64-bit digest changed, a leaving cycle publishes its parts empty, and a
  report with no hot cycle is not published. Who appoints and removes an
  institution's auditors is its scenario-declared appointment policy; an
  institution with no policy keeps only the audit bindings installed with
  the state, and `RebindAudit` is refused unless the policy makes the
  institution its own appointer. Settlement measures every record it writes
  exactly against its budget. The ledgers declare the commutation quotes and
  receipt evidence their hot books cite as payload-required; archiving a book
  ends the declaration, and `prepare_fiscal_archive` refuses a book citing a
  version newer than the sealed cut the host passes.
- **Society.** Due cohort transfers settle as a batch by default
  (`TransferSettlement::Batch`); a run that needs the 0.15.0 rule asks for
  `OneByOne`. The queue is stored as a head and one batch record per
  admitting boundary, so its evidence follows what each boundary admits.
- **Production and resource archives.** The production archive has the
  cumulative lookup index of the resource archive, commits of up to 16 blobs
  of 64 records, and receipts that leave the root, so a run may make any
  number of commits; restore validation resolves each compacted resource
  input through the resource archive's lookup index and audits that archive
  once whenever it has a head. Every hot execution and project declares the
  versions restore reads until it is archived, and a settled output's pinned
  source is no longer read back. The package archive of the two reference
  plugins has the same index, so a lookup inside a boundary reads the head
  directory, one page per level, and one blob on a hit. A host application
  that archives must keep every archive's lookup pages for the heads a
  restore, replay, or fork passes.
- **Profiling and validation.** An exact replay can be profiled boundary by
  boundary, and every profile splits the simulation core's time into named
  engine phases. The view's lookup of a record version proposed earlier in
  the boundary is a range query instead of a scan of the boundary's
  evidence.
- **Command admission no longer slows with run history.** The four admission
  checks that rescanned the retained command, attempt, and ingress journals
  (the unqueued-history check before every canonical ingress append, the
  legacy-command check, the cached-outcome lookup, and the decision
  request-in-use checks) read a derived `CommandHistoryIndex` that the
  retained journals keep in step. The retained journals are
  `RetainedJournal` values, readable anywhere as a slice but changed only
  through the runtime's push, truncate, seal, and restore methods. The index
  is never persisted, hashed, or committed, every answer equals the scan it
  replaces (debug builds assert so on every call), and no format, hash, or
  public API changes: the change is replay-neutral and seal-independent.
- **A resource boundary reads and writes what it touches.** The resource
  root persists as its live root and one record per hot terminal bucket,
  chosen by a hash of each record's identity among as many buckets as the
  candidate budget rounded up to a power of two, and a ledger in the live
  root counts the hot terminal records. Before it settles each packet the
  resource plugin attaches the outcome under every key the packet settles
  under, every hot terminal record the payload names, and the closure of
  those (a demand's hot reservations and legs, what a leg or receipt belongs
  to); every rule that counts hot terminal records reads the ledger, so a
  partial root settles every operation as the whole root would, and every
  debug build settles each boundary again on the whole root and asserts the
  same proposal. The boundary then writes the live root and only the buckets
  whose entries changed, and a boundary with nothing for the resource plugin
  reads nothing of the root. In every build a boundary that would write over
  a hot terminal record it did not attach fails with `invalid_boundary`, an
  engine invariant, instead of overwriting a stored outcome. An archive
  commit attaches the records it selects only when it applies. Other
  plugins (production, the economy and force-supply references) read the
  root through `resource_root_in_view`, `ResourceRootReader`, or
  `whole_resource_root_in_view`; the host's `resource_state` reads the whole
  root.

Known limits carried by this release, each stated in its proposal:

- A paged restore allocates what the pages describe, up to 512 MiB per
  record, even when the provider holds far fewer distinct bytes, because a
  leaf may name one chunk page many times. A host application that restores
  pages from an untrusted source authenticates the checkpoint first, for
  example by its root page ID; a restore-wide byte budget is a follow-up
  ([hostile pages](proposals/record-store-ceiling-and-cost.md#a-paged-checkpoint-holds-records-of-any-legal-size)).
- The paged-checkpoint envelope page still holds the compact checkpoint body,
  knowledge ledger and plugin components included, in one 4 MiB page; a run
  whose body exceeds it cannot prepare a paged checkpoint, and a continuation
  checkpoint is the save that works
  ([not done](proposals/record-store-ceiling-and-cost.md#not-done)).
- A boundary still takes a stable copy of the plugin components, the core
  entity set, persons, letters, and armies for phases 1 to 9, and `fork`
  still copies every journal and those maps, so a run that keeps much state
  in plugin components pays for it on every boundary
  ([not done](proposals/record-store-ceiling-and-cost.md#not-done)).
- A continuation runtime cannot produce a full snapshot, a replay journal, a
  checkpoint journal, or a paged checkpoint (`HistoryUnavailable`), holds no
  archived receipts, and its retained-evidence reads see only evidence after
  its base; a replay journal assembled from a continuation base and later
  segments is follow-up work. The keyed-draw ledger and the request index
  grow for the life of the run (about 520 bytes per tracked request and per
  drawn address). Host validators that read history or exact version bodies
  (technology, production, society, historical research, force-supply
  audits, the Ming sample cycle) have no continuation form and stay
  seal-dependent, as does any undeclared non-current record version
  ([non-goals](proposals/continuation-checkpoint.md#non-goals),
  [what a restored run can and cannot do](proposals/continuation-checkpoint.md#what-a-restored-run-can-and-cannot-do)).
- What a continuation restore verifies without history is the state and its
  commitments; the carried evidence records, the pending set, the keyed
  entries' evidence, and the settled cut are covered only by the
  `continuation_hash` the host stored, so a host restores with
  `from_continuation_checkpoint_verified` and never gives the checkpoint to
  player, agent, or remote clients
  ([identity and trust](proposals/continuation-checkpoint.md#identity-and-trust)).
- R4 (a dependency is added only while its evidence is still readable) stays
  a plugin obligation; the engine enforces it only by pinning at commit, and
  a violation is `ArchiveNotReady` at the next seal
  ([the rule plugins obey](proposals/continuation-checkpoint.md#the-rule-plugins-obey)).
- Superseded knowledge records stay in every holder's ledger and every save
  for the life of the run; the host and viewer knowledge queries and
  `audit_knowledge_record` still cost the holder's whole ledger per call
  ([what each operation costs now](proposals/knowledge-ledger-cost-and-authentication.md#what-each-operation-costs-now)).
- Plugin-scheduled resource work waiting on an archive backlog or on reserved
  room is retried, at most about once a day once its delay has stopped
  growing, until the host application commits or releases, and the run cannot
  seal while a deferral waits; an activated lease never expires; a
  `Requested` acquisition holds its reservation until it is granted or
  aborted; a production output batch skipped for room, for a taken key, or
  for an output account that changed revision leaves its execution
  `CompletedPendingOutputSettlement`
  ([open decisions](proposals/resource-refusals-without-stalls.md#open-decisions)).
- Production: a re-sent production operation whose outcome was archived is
  applied again; a well-formed plugin-scheduled packet that is forged, a
  failed store read, counter overflow, and the root's 256 MiB byte cap still
  fail a production boundary; any admitted production packet, a refused one
  included, triggers the incident and report systems; preparing, enqueueing,
  and settling an archive commit cost the selected records times the hot
  outcomes, so a host commits small batches often
  ([open decisions and follow-up](proposals/production-archive-volume.md#open-decisions-and-follow-up)).
- The reference package archive keeps every maintenance receipt in its root,
  so a run makes at most `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS` (8,192)
  archive commits per reference plugin, after which the enqueue helpers
  refuse; the economy root then fills to its 8,192 hot operation outcomes
  (the default `max_operation_outcomes`; the type cap is 32,768)
  and the run stops there. Its superseded lookup pages are reclaimed only
  from each batch's list
  ([known limits](proposals/reference-archive-and-replay-profile.md#known-limits)).
- Law: `max_provider_packets` bounds how long after admission a provider
  packet can be cited; `DelegatedAuthority` records are never retired and
  grow with the delegation commands a run admits; a pile-up of `D` dirty
  procedures under budget `B` makes about `D^2 / B` provenance reads over
  its continuation chain
  ([open decisions](proposals/law-information-military-correctness.md#open-decisions)).
- Fiscal: observers are scope-granularity and `OnChange` only, so a world
  with thousands of observers whose institutions change daily drains for
  many same-instant boundaries (4,000 observers whose institutions all
  change need 63 boundaries at one instant; institution granularity,
  `Interval` cadence, and domain-entity holders are phase 3); an action ID
  reused for a different decision is detected only while the earlier outcome
  is in the ring; external-operation uniqueness is checked within a scope's
  hot books; the host archive helpers assemble the whole committed state
  (about 1 s at 68,000 records); a receipt or context change refused for the
  byte budget leaves no record in the state; a stale commit refused for lack
  of head room has no state handle and the host abandons its store copy
  ([what remains](proposals/world-scale-fiscal.md#phase-2-as-implemented),
  [open decisions](proposals/fiscal-and-society-limits.md#open-decisions)).
- Restoring a large unsealed run is slow: after the fiscal burst its
  snapshot is 1.09 GB, almost all retained evidence and published knowledge,
  and restoring it takes 231 s; sealing and the continuation checkpoint are
  the answer
  ([what remains](proposals/world-scale-fiscal.md#phase-2-as-implemented)).
- The engine's share of admitting a command no longer grows with history,
  but about half of what is left, about 0.15 ms per command in a
  50,000-scope world, is the per-command transaction checkpoint, which
  clones the people, letter, army, plugin component, and scheduled action
  maps so that a failed command can be rolled back; it grows with the
  world's size. Persistent maps for those collections, or a capture of only
  what a command can change, are a follow-up
  ([the per-command transaction checkpoint](proposals/command-admission-history-index.md#follow-up-the-per-command-transaction-checkpoint)).
- The Ming fiscal validator no longer checks that a superseded quote or
  record-version audit citation names the exact change index of its
  establishing boundary; a forged change index within the version's limits is
  accepted alike in every mode
  ([invariants](proposals/reference-archive-and-replay-profile.md#seal-stable-reference-validation)).
- Society: a boundary that refuses a packet for a full queue rewrites the
  cohort exchange ledger, which keeps every outcome for the life of the run
  ([open decisions](proposals/fiscal-and-society-limits.md#open-decisions)).
- Resource: the boundary that applies an archive commit rewrites one bucket
  per archived record, about 25 µs, 210 allocations, and 1.3 KB of journal
  each (103 ms at 4,054 records, 524 ms at 16,054), so a host with a large
  backlog commits it in batches and acknowledges each before it prepares the
  next; a 64-page commit that goes stale records a retention handle of about
  13 KB in the live root, so a root near its byte cap has room for about five
  unacknowledged stale commits of that size; the live root (accounts, open
  demands, active transfers, leases, report grants, observation heads) is
  still decoded, validated, and re-encoded whole once per operation
  boundary, and finished leases stay in it until their last receipt is
  archived; `resource_state`, archive preparation, restore validation, and
  the force-supply reference's settlement and acknowledgement of a resource
  outcome still read the whole root
  ([known limits](proposals/resource-root-write-cost.md#known-limits)).
- `Send` runtime: a provider or store a run holds must be `Send + Sync`, and
  the handles promise `Send` only, so a host that wraps one run as another
  run's provider relies on the handles being `Sync` today, which a later
  release may withdraw; a provider that blocks on a lock a host thread holds
  delays the boundary that loads from it, and a poisoned lock in a host store
  is the host's to recover; while a fork is live on a worker the host keeps
  the lookup pages and package archive objects that fork still reads
  ([host guidance](proposals/send-runtime.md#host-guidance)).

Headline measurements, each from its proposal and, where one exists, from the
reports under `benchmarks/baselines/` (release builds, one Windows x86-64 machine shared
with other builds; allocation counts are exact, elapsed times show the order
of magnitude):

| Measure | Workload | 0.15.0 | 0.16.0 |
| --- | --- | ---: | ---: |
| Idle boundary with knowledge held | 65,536 held report records, no system due (`knowledge-ledger-profile`) | 182 ms, 1,459,736 allocations | 0.026 ms, 215 allocations |
| Idle boundary with records held | 65,536 domain records, nothing written (`record-write-profile`) | 51 to 67 ms, 404,364 allocations | 0.08 to 0.10 ms, 184 allocations |
| One record write | 2.45 KiB structured record into a store of 65,536, 10,000 writes per boundary | 526 µs, 2,048 allocations | 24.3 µs, 91 allocations |
| Paged checkpoint of a large record | one 1.2 MB and one 32 MB record (`paged-record-profile`) | refused | 33.4 ms to prepare, 472 ms to restore |
| 64 `knowledge_heads` calls | 65,536 held records | 832 ms | 0.190 ms |
| Checkpoint, paged checkpoint, seal, fork of a sealed compact run | 100,000 sealed keyed draws (`keyed-draw-profile`) | 4,730,434 allocations for `checkpoint()`, paged checkpoint refused | 185 allocations; 248; 1,112; 291 |
| Continuation checkpoint against full save | month 14 of `continuation-profile`, 10 keyed draws per packet | full save 159.1 MB, written in 436 ms, restored in 6,244 ms | 5.72 MB, 38.8 ms, 207 ms |
| World-scale fiscal, spread day | 137 assessments over 50,000 scopes, 4,200 institutions, 4,000 observers (`fiscal-world-profile`, `world`) | not reachable (caps) | 56 ms admission, 82 ms settlement, 3 drain boundaries |
| World-scale fiscal, burst | 50,000 assessments in one boundary | not reachable | 2,139 ms admission, 4,790 ms settlement, 62 drain boundaries (about 11.5 s at the instant) |
| Fiscal settling boundary of 40 actions | 48 institutions, four observers (`fiscal-profile`, allocations) | 6,476,053 | 231,492 |
| Production archive commit chain | 30,000 one-record commits through the plugin (`production-archive-profile`) | stops at 8,192 commits, root 3.37 MB | root 4.8 KB, 1.0 ms per boundary, miss lookup 46 µs |
| Production restore of one compacted input | resource archive of 100,010 records in 3,705 commits | 2.09 s, 37,050 objects | 123 µs, 14 objects |
| Package archive lookup, miss | 102,400 archived records in 200 commits | 488 ms, 1,000 objects | 32 µs, 5 objects |
| Law refusal | 500 duplicate submissions in one boundary among 1,000 open proposals (debug build) | 181 s with a reload per refusal (first fix) | 0.42 to 0.48 s |
| Profiled boundary | 4,000 daily boundaries, four boundary systems | not profiled in replay | 27.0 µs profiled against 25.5 µs unprofiled |
| Command admission after a long run | 4,200 fiscal batch commands admitted after a 50,000-scope burst (`fiscal-world-profile`, `world`), engine share per command | 0.71 ms (0.40 ms in a fresh world) | 0.31 to 0.34 ms (0.29 to 0.31 ms fresh); the host's enqueue of the 4,200 commands 1.6 to 1.8 s → 0.41 to 0.46 s |
| Resource operation boundary over a backlog | one refused allocation pass with 4,000 hot terminal records (`resource-root-profile`) | 397 ms, 4.43 MB of journal, 2,388,771 allocations (64.6 ms and 694,822 with the record store package alone) | 0.24 ms, 8.1 KB, 1,070, the same as with no backlog; 0.42 ms and 10.7 KB at 16,000 |
| Idle boundary beside a resource backlog | another plugin's packet only, 4,000 hot terminal records | 29.1 ms, 198,154 allocations (15.3 ms with the record store package) | 0.037 ms, 294 allocations |
| Resource archive commit | every hot record, 4,054 and 16,054 records committed | 102 ms, 2.24 MB (4,054) | 103 ms, 5.21 MB, 849,342 allocations (4,054); 524 ms, 20.8 MB (16,054): about 25 µs and 1.3 KB per archived record, paid once |
| `Send` runtime | `keyed-draw-profile`, live boundary on a fork and `compact_fork` at 10,000 retained draws, before and after the `Arc` provider | 104.4 µs; 13.1 µs | 103.5 µs; 9.3 µs: the same allocations and bytes in every case, every elapsed difference inside one build's run-to-run spread |

Snapshot format 8 and the hash algorithms are unchanged. The engine version in
every checkpoint hash changes, first-party plugins that take their plugin
version from the crate version report a new one, and the exact
engine-version check rejects 0.15.0 saves; retain the 0.15.0 engine to read
them.

Version 0.15.0 is a pre-1.0 minor release. Each group of changes has a
proposal that gives its design, invariants, and measurements:

- [society settlement cadence and runtime revision](proposals/society-cadence-and-runtime-revision.md)
  (downstream gaps G-40 and G-45);
- [same-instant supersession and unplaced persons](proposals/same-instant-supersession-and-unplaced-persons.md)
  (G-58 and G-49);
- [keyed-draw index, draw ID, and boundary profile](proposals/keyed-draw-index-and-settlement-profile.md)
  (G-50, G-51, and G-47);
- [resource archive lookup and volume](proposals/resource-archive-lookup-and-volume.md)
  (G-52 and G-54);
- [single-step completion lease and runtime protected-floor install](proposals/resource-single-step-lease-and-floor-install.md)
  (G-53 and G-56);
- [fiscal throughput and authority](proposals/fiscal-throughput-and-authority.md)
  (G-41, G-44, G-48, and G-57, and fiscal reports).

New and changed public contracts, by crate:

- `canwu-sim`, re-exported by `canwu-api`:
  - `SimulationView::knowledge_heads(holder, kind, subjects)` returns
    `KnowledgeHeads { records, truncated }`: the global IDs of a holder's
    current records of one schema kind (those no record of the holder
    supersedes) that carry every listed subject. A publishing system uses it
    to name the records its next record supersedes. It needs the
    `canwu.core.knowledge` read, returns at most 64 IDs (the newest when more
    match, with `truncated` set, which is not an error), and reads committed
    state and the boundary being settled, so live, sealed, restored, and
    replayed runs return the same IDs. Additive.
  - `PersonDraft::placement: Option<PersonPlacement>` replaces
    `PersonDraft::government` and `PersonDraft::current_location`, so a draft
    is placed or it is not. A draft without a placement creates an unplaced
    person: an identity in the entity registry with person availability and a
    created-person receipt, and no `Person` record in the `WorldSnapshot`
    projection. The creation evidence `BoundaryPersonCreation::person` is the
    new flat `BoundaryCreatedPerson`, with `placement()`, `is_placed()`, and
    `person_record()`. Both changes are source-incompatible.
  - `SimulationView::random_draw_for_operation` takes the arguments of
    `random_sample_for_operation` and returns
    `RandomOperationDraw { draw_id, sample }`: the sample with the
    `RandomDrawId` of its draw evidence, so a record can cite the draw it was
    computed from. `RandomSample`, `random_sample_for_operation`, and
    `random_range_for_operation` are unchanged. Additive.
  - An opt-in wall-time profile of settled boundaries:
    `set_boundary_profiling`, `boundary_profiling`, and
    `take_boundary_profiles` on `Canwu`, `CompactedCanwu`, `Simulation`, and
    `CompactedSimulation`, with `BoundaryProfileBatch { profiles, dropped }`,
    `BoundaryProfile { boundary_id, settled_at, total_micros, handlers }`,
    `BoundaryHandlerTiming { kind, phase, plugin, name, elapsed_micros }`,
    `BoundaryHandlerKind` (`BoundarySystem`, `CommandHandler`, `EventSystem`),
    and `MAX_RETAINED_BOUNDARY_PROFILES` (1,024). Profiling is off in a new,
    restored, replayed, and forked runtime. It is a host diagnostic outside
    authoritative state: a profiled run produces the same receipts, evidence,
    hashes, snapshots, and replay journal. Additive.
- `canwu-society`:
  - `SocietyPlugin::with_settlement_cadence(cadence)` runs the society's four
    settlement systems on a host-chosen `SystemCadence` category;
    `SocietyPlugin::DAILY` and `SocietyPlugin::default()` keep `Daily`, and
    `EventDriven` is refused at registration. A transition rate applies once
    per settlement, whatever the cadence. The cadence is recorded in the
    persisted plugin descriptor, so a run is restored and replayed only with
    a plugin of the same cadence (`PluginManifestMismatch` otherwise);
    `from_society_snapshot_json_with_plugin` restores a run on another
    cadence. `SocietyPlugin` is now a struct with a private field, `Clone`
    but not `Copy`, which is source-incompatible: code that wrote
    `SocietyPlugin` as a value writes `SocietyPlugin::DAILY`.
  - On a cadence coarser than `Daily`, everything the society's phase-7
    system writes waits for a settlement: due transfers, rebases, lifecycle
    deltas, pressure revisions, institutional decisions, and transitions.
    Of the due transfers that share a cohort, one completes per settlement
    and the others settle as `stale_source`, as transfers due on one day
    of a `Daily` society already do. The owner-side queue record is
    rewritten whole by every boundary that admits a packet, so the
    evidence of a queue that waits a period grows with the square of its
    packets, and a packet that arrives while `MAX_SOCIETY_INGRESS_QUEUE`
    (4,096) wait is dropped with only a rejection event.
  - The `revise_society_pressure` command (`PressureRevisionDecision`, built
    into a ticket by `pressure_revision_ticket`) lets the institution behind
    an alignment revise, within the alignment's affected cohorts, policy
    pressure rows it issued and the activity and reach of its target's
    influence edges and organization nodes; an organization row needs an
    alignment that covers every cohort. A decision revises at most
    `MAX_PRESSURE_REVISION_ROWS` (256) rows, applies as a whole or not at
    all, and takes effect at the first society settlement after the boundary
    that admits its internal `society_pressure_revision_v1` packet.
    `SocietyCohortExchangeLedger::pressure_revisions`, a new public field
    that is omitted while empty, keeps the latest applied and the latest
    rejected decision of each alignment. The new field is
    source-incompatible for struct literals.
  - Also new: `SocietyPlugin::settlement_cadence`,
    `PolicyPressureRevision`, `InfluenceEdgeRevision`,
    `OrganizationRevision`, `PressureRevisionChoice`,
    `PressureRevisionOutcome`, `PressureRevisionOutcomes`,
    `SocietyPressureRevisionV1`, the events `society_pressure_revised_v1`
    and `society_pressure_revision_rejected_v1`, and constants for the
    command, ingress, event, and rejection-reason names.
- `canwu-resource`:
  - Each archive commit directory commits to a cumulative lookup index
    through the new required field
    `ResourceArchiveIndexDirectoryV1::lookup_root`: a content-addressed
    16-way trie over key digests, made of `ResourceArchiveLookupPageV1`
    objects of at most 4 KiB in the namespace
    `canwu.resource.archive.lookup-page`.
    `archived_resource_terminal_record` and
    `archived_resource_operation_outcome` read the head directory and one
    lookup page per index level, plus one blob on a hit, instead of walking
    and authenticating every commit. The cost of a cold lookup therefore
    follows the logarithm of the archived records and does not depend on the
    number of commits, and a lookup no longer returns
    `ResourceError::LimitExceeded`. The 2,048-commit limit
    `MAX_RESOURCE_ARCHIVE_LOOKUP_DIRECTORIES` is removed, so a run may make
    any number of archive commits. A missing or malformed archive object is
    an error, never a miss.
  - `ResourceState::prepare_resource_archive(candidate_limit, store)` takes
    the archive store, which it reads to extend the index; `()` implements
    `ResourceArchiveStore` for a root that has no archive head yet.
    `PreparedResourceArchiveBatchV1` gains `additional_pages`,
    `lookup_pages`, and `superseded_lookup_pages`. All of these are
    source-incompatible.
  - One commit carries up to `MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS` (4,096)
    records as up to `MAX_RESOURCE_ARCHIVE_COMMIT_PAGES` (16) blobs of 256.
    `max_archive_candidates` may be set up to
    `ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES` (4,096); the canonical value
    stays 256. `ResourceLimitsV1::validate` now requires
    `max_archive_maintenance_receipts` to be at least
    `max_archive_candidates`, and its hard maximum rises from 1,024 to 4,096.
    One rule is unchanged and matters when sizing the budget: an operation
    that does not fit the candidate budget fails its boundary and stays
    queued until an archive commit makes room; it is not a recorded
    rejection. The budget must cover the backlog plus every terminal
    record the operations due at one boundary can produce. Every boundary
    that changes the resource root also costs time and journal bytes in
    proportion to the hot terminal records: in the proposal's
    measurements, about 50 to 95 microseconds and 1.1 KB of journal for
    each hot record.
  - Also new: `ResourceArchiveLookupEntryV1`,
    `PreparedResourceArchiveLookupV1`, `PreparedResourceArchivePageV1`,
    `prepare_resource_archive_lookup`, `resource_archive_lookup_key_digest`,
    `live_resource_archive_lookup_pages`, and the lookup constants
    `MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES`,
    `MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH`, and
    `MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES`.
  - `ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1)`
    takes a completion lease whose only participant is the resource plugin
    from nothing to active in one operation. "Only participant" means that
    the lease's `expected_participants` is exactly `{"canwu-resource"}`. The
    operation runs the acquire, grant, prepare, and activate steps with the
    checks each has on its own, and its activation certificate equals the one
    the four steps produce for the same acquisition, grant, and targets at
    the same boundary, so the operation it authorizes cannot tell the paths
    apart. It leaves one `CompletionLease` outcome, keyed
    `resource:completion:lease:{acquisition}`, and one receipt, where the
    four steps leave four of each: in the committed fixture a lease with its
    completion receipt goes from 9 terminal records and 4 archive lookups to
    3 and 1, and a same-place transfer from 27 and 15 to 15 and 9. A tracked
    command cannot carry it; it enters through
    `enqueue_resource_completion_operation` or a plugin that declares the
    completion ingress. A domain refusal is the operation's recorded
    rejection and leaves no part of a lease behind. The four-step path stays
    for a lease with any other participant. The new variant is
    source-incompatible for exhaustive matches.
  - `ResourceProtectedFloorRequestV1::install` lets an account's custodian
    set a floor of any quantity at runtime: `SetProtectedFloor` carries a
    content-addressed revision
    (`ProtectedFloorPolicyRevision::content_addressed`) and installs it and
    points the account at it together or not at all. A content-addressed
    revision is always named with its body. A root holds at most
    `MAX_PROTECTED_FLOOR_POLICY_REVISIONS` (4,096) revisions, authored and
    content-addressed together, as one quota shared by every custodian;
    `ResourceState::install_protected_floor_policy` and
    `ResourceState::validate` enforce it, and a runtime install at the bound
    is a recorded `limit_exceeded` rejection. Every applied
    `SetProtectedFloor` retires the content-addressed revisions that no
    account and no demand in the root refers to, whoever installed them; an
    authored revision is never retired. The new field is source-incompatible
    for struct literals, which add `install: None`, and it is omitted from
    the canonical encoding while `None`.
  - Also new: `RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`,
    `PROTECTED_FLOOR_POLICY_CONTENT_ID_PREFIX`,
    `MAX_PROTECTED_FLOOR_OVERRIDE_CLASSES`,
    `ProtectedFloorPolicyRevision::is_content_addressed`,
    `ResourceCompletionOperationV1::has_canonical_operation_key`, and
    `ResourceOperationRequestV1::request_digest`.
- `canwu-fiscal`:
  - Several fiscal actions settle in one boundary, in admission order. Each
    is compared with the procedure revision the state had when the boundary
    began and is validated against the state the previous action left. One
    boundary applies at most `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (64) actions;
    only an action that is current and authorized counts, and a further one
    is `Rejected` with `FISCAL_BOUNDARY_ACTION_LIMIT`. Admission bounds a
    request (`MAX_FISCAL_ACTION_JSON_BYTES`, 32 KiB), its free text
    (`MAX_FISCAL_ACTION_TEXT_BYTES`, 2,048 bytes), and its units and
    referenced IDs (`MAX_FISCAL_IDENTIFIER_BYTES`, 160 bytes). An action
    whose result would not fit the state byte budget, less a 16 KiB reserve,
    is rejected as its own outcome with `FISCAL_STATE_BUDGET_EXHAUSTED`.
  - `FiscalAction::RebindAuthority` replaces a binding's authorized actor,
    acting actor, and authority basis in one commit. It is issued under the
    binding it replaces, by its authorized actor or by the institution
    deciding as a body (a decision whose origin is the institution with no
    responsible actor), never by an acting actor or a third person. Every
    person it names must exist, and a new basis must be the current version
    of its record at admission and in the state the settling boundary began
    with. The binding records the action and its issuer in the new public
    field `FiscalAuthorityBinding::rebound_by` (`FiscalRebinding`).
  - `FiscalAuditBinding`, held in the new `FiscalState::audit_bindings`,
    admits an auditor from outside an institution for `RecordAudit` on that
    institution's scopes and for nothing else, while its own authority basis
    is current. `FiscalAction::RebindAudit` installs, replaces, or removes
    one, under the rules of an authority rebind. Every finding settled from
    0.15.0 names who entered it in the new public field
    `FiscalAuditFinding::entered_by` (`FiscalActionIssuer`,
    `FiscalIssuerRole`). The audited institution installs and removes its
    own auditors, so an audit binding establishes who entered a finding, not
    that the auditor is independent.
  - An observer's fiscal report is one summary record and parts of
    `FISCAL_REPORT_FACTS_PER_PART` (16) facts: `FiscalReportRecord`, version
    2 of the `fiscal_report` knowledge schema. Aggregate IDs are stable, a
    boundary republishes only the parts whose facts changed, and each record
    supersedes the current record of its own line, which the report system
    finds with `SimulationView::knowledge_heads`. A reader calls
    `fiscal_reports(&viewer)` or `assemble_fiscal_reports` and gets one
    `FiscalReport` for each observer binding. One boundary publishes at most
    `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024) report records; the
    parts that are left are named in `pending_parts` and published by the
    next boundary that changes the fiscal state.
    `FiscalState::report_plan` (`FiscalReportPlan`) is a new public field.
  - The two new `FiscalAction` variants are source-incompatible for
    exhaustive matches, and the new public fields on
    `FiscalAuthorityBinding`, `FiscalAuditFinding`, and `FiscalState` for
    struct literals. Also new: `FiscalReport`, `FiscalReportSummary`,
    `FiscalReportPart`, `MAX_FISCAL_REPORT_PARTS`, and the stable reasons
    `FISCAL_AUTHORITY_BASIS_NOT_CURRENT` and `FISCAL_ISSUER_NOT_BOUND`.

The source-incompatible changes are the `PersonDraft` and
`BoundaryPersonCreation` shapes; the `SocietyPlugin` type; the new variants
of `ResourceCompletionOperationV1` and `FiscalAction`; the new public fields
on `SocietyCohortExchangeLedger`, `ResourceArchiveIndexDirectoryV1`,
`PreparedResourceArchiveBatchV1`, `ResourceProtectedFloorRequestV1`,
`FiscalAuthorityBinding`, `FiscalAuditFinding`, and `FiscalState`; the
`prepare_resource_archive` signature; and the removed constant. The release
is therefore a minor version.
`PersonDraft` also rejects unknown fields when decoded: a draft written as
JSON in the 0.14.0 shape fails to decode instead of becoming unplaced. That
protection does not reach a draft an application flattened into its own
struct with `#[serde(flatten)]`, where the old shape decodes as unplaced, so
an application must not flatten a `PersonDraft`.

Version 0.15.0 also changes these behaviors, which are visible to existing
code and runs:

- Settlement and snapshot validation now apply one rule to the records a
  published knowledge record may supersede or contradict. A target must be a
  record of the same holder and schema kind that an earlier boundary
  committed, whatever its learned time, or that the same boundary published
  with `SameBoundary` visibility in an earlier phase. Under 0.14.0 snapshot
  validation compared learned times instead. It refused a target learned at
  the same instant in an earlier boundary, which settlement admits, so a
  plugin that superseded a holder's record in two boundaries of one instant
  wrote a save that no load accepted. It also accepted a phase-4 record
  naming an earlier phase-4 record of its own boundary, which settlement
  refuses. What settlement admits is unchanged.
- An operation-keyed request for an address the run has already drawn no
  longer depends on evidence retention. It returns the recorded draw when
  the evidence reference, bound, and purpose match, and
  `RandomOperationConflict` when they do not; 0.14.0 returned
  `InvalidRandomOperationEvidence` once a seal had removed the evidence, so a
  sealed run refused an exact retry that its unsealed replay answered. In a
  view with a random session, an undeclared stream or a malformed address is
  now reported before the evidence is looked at. A fresh draw still needs
  operation evidence that exists at the reader's cut.
- The runtime keeps its index of operation-keyed draws as derived state,
  extended when a boundary commits and rebuilt on restore, instead of
  rebuilding it at every boundary and copying it for each due system. A
  boundary no longer does work in proportion to the keyed draws the run has
  kept. A checkpoint, a paged checkpoint, a seal, a fork, and a restore
  still cost time per keyed draw ever made, and the index keeps one entry in
  memory for each drawn address for as long as the run lives (522 bytes each
  in the proposal's measurement).
- A society settlement writes the society root only when it changes more
  than a timestamp. `last_transition_at` is the time of the last settlement
  that changed a distribution or a carried remainder. `last_mobilization_at`,
  `last_aggregation_at`, and `last_projection_at` are the times those views
  were last materialized, and a set timestamp means the view matches the
  authoritative state. A projection's `observed_at` is the time that
  observer's own estimate last changed; it is recorded history that
  validation only bounds from above. A host that read these fields as the
  time of the latest settlement uses the time of the latest boundary that
  carried the cadence. `settle_transitions` returns `false` and leaves the
  state untouched when it changes nothing. An idle society no longer emits
  record-change events, so it no longer wakes other plugins' event-driven
  systems at the next boundary.
- A refused society command is a recorded rejection, and the run continues.
  `set_institutional_policy` refuses bad fields as `InvalidPayload`, an
  unknown alignment or a missing society root as `InvalidAuthority`, and a
  version that is not newer than the applied or pending one, including a
  resend, as `IdempotencyConflict`. `transfer_cohort_population` refuses the
  same cohort as source and destination as `InvalidPayload`, a missing
  society root as `InvalidAuthority`, an unknown cohort as `EntityNotFound`,
  the operation ID of a pending transfer with other data as
  `IdempotencyConflict`, a due time in the past as `InvalidDuration`, and a
  stale expected source version as `ValueOutOfRange`. A defective
  cohort-transfer packet is skipped with a `society_ingress_rejected_v1`
  event (`malformed_payload`, `unauthorized_origin`, `authority_unavailable`,
  `unknown_cohort`, or `operation_id_conflict`), and its boundary commits.
  Under 0.14.0 each of these failed the boundary and every later canonical
  step. A host that matched the old codes, or that detected a refusal by a
  failed boundary, reads the rejected command attempt or the event instead.
- An archive commit in `canwu-resource` no longer fails its boundary for
  capacity. The maintenance receipt of a stale commit is not an archive
  candidate: it stays in the root, outside the budgets, until its retention
  is acknowledged, and is then removed. Under 0.14.0 a stale commit against
  a full candidate budget failed every later boundary. A retention
  acknowledgement that arrives twice, or after a later commit archived the
  receipt it names, settles instead of failing its boundary. Within one
  boundary the resource plugin now settles archive commits and retention
  acknowledgements before its other packets, so a commit prepared from the
  root the boundary started from applies even when an operation was admitted
  ahead of it.
- A host application that archives resource records must now keep the
  archive's lookup pages: a restore, an exact replay, or a fork reads the
  lookup index of each archive head it passes. The engine never deletes one.
  `superseded_lookup_pages` names the pages a host application may delete if
  it gives up restoring, replaying, and forking from points before a commit:
  only after that commit settled as `Applied`, and only once a durable
  snapshot or checkpoint taken after it exists.
  `live_resource_archive_lookup_pages` recomputes the pages a live run reads.
- A tracked resource command that reuses an operation key for a different
  request no longer fails a boundary. When the key's outcome is in the root
  or the archive, the command is refused at admission with
  `IdempotencyConflict`; when it was admitted before the first request
  settled, its packet settles nothing and the outcome under the key names
  the request that owns it. Under 0.14.0 the boundary that settled a reused
  key still in the root failed, and so did every later one.
- An operation key under `resource:completion:` is reserved for completion
  operations. Any other request that names one is refused: at admission for
  a tracked command, by `resource_adapter_ingress` and
  `enqueue_resource_allocation`, and as an error from
  `ResourceState::apply_operation`. A holder can still take a completion key
  with the two completion operations it may send as tracked commands: an
  `Acquire`, and an `Abort` of a lease it holds.
- A packet on the completion ingress whose operation key a different request
  holds settles nothing, and its boundary commits. Under 0.14.0 that boundary
  failed, and so did every later one.
- A tracked `Abort` is authorized by the holder of the acquisition in the
  root, not by the holder the request names. An abort by another holder, or
  for an acquisition the root does not hold, is rejected at admission with
  `InvalidAuthority` and leaves nothing under the abort key.
- A single-step lease whose later `Release` key would not be a valid
  operation key is refused with `invalid_identifier`.
- An `Acquire` that names an acquisition the root already holds is a recorded
  `idempotency_conflict` rejection. A completion operation whose identities
  are too long to form its operation key is refused at admission or by
  `enqueue_resource_completion_operation`, and one that reaches settlement
  is a recorded `invalid_identifier` rejection instead of a panic.
- A scenario that installs more than 4,096 protected-floor policy revisions
  is refused, and so is a scenario revision whose identity lies under
  `resource:protected-floor:` without being derived from its content.
- Fiscal actions admitted to one boundary now settle instead of all but the
  first being rejected as stale. A later action of the same boundary is
  stale only when it would overwrite an adoption, authority binding, or
  audit binding that an earlier action of that boundary created or changed.
  An execution receipt or a historical-context change that settles in the
  same boundary no longer makes an action stale; one that settles in the
  boundary that admits the action's command still does.
- A fiscal action never fails a boundary for the others. An oversized
  request, over-long text or references, and a non-canonical ID of a new
  entry are rejected at admission as `InvalidPayload`, and so is a rebind
  that names a person who does not exist, as `EntityNotFound`. An action
  that settlement can give no outcome is dropped: it changes nothing, its
  action ID is not consumed, and one `canwu.fiscal.action_dropped.v1` event
  names it and the institution it was issued for. An accepted command whose
  action has no outcome after the next boundary was dropped. Under 0.14.0
  such a boundary failed on every retry.
- Settlement re-checks the role an issuer held at admission against the
  state the action settles on. An action of an authorized actor or auditor
  who has since been replaced is rejected with `FISCAL_ISSUER_NOT_BOUND`,
  and the basis a rebind or an audit binding relies on must still be current
  (`FISCAL_AUTHORITY_BASIS_NOT_CURRENT`). Authority bases are read from the
  current-version provenance index at admission and at settlement, which a
  seal cannot change.
- An invalid fiscal catalog or state is refused when the run is created or
  restored, because plugin activation now validates both. Under 0.14.0 it
  was accepted and failed the first boundary that admitted anything. A state
  is also refused when an observer binding names a person who does not
  exist (the state record now references each observer's person under the
  role `observer`, so encode it with `FiscalState::into_record`), when an
  assessment's unit or accounting cycle, or the namespace, name, and ID of a
  scope's domain-record institution together, exceeds 160 bytes, and when
  the scenario's knowledge carries a `fiscal_report` record of schema
  version 1. A binding may now cite a record of an external evidence kind as
  its authority basis.
- The four fiscal boundary systems return before they decode the fiscal
  records on a boundary that has nothing for them; a state still at its
  initial-scenario version is derived as before. Activation does not
  recompute derived fields, so a restored state whose transition candidates
  or aggregates were altered outside the engine keeps them until its next
  fiscal write.
- A reader that decoded one `fiscal_report` record as a `FiscalProjection`
  calls `fiscal_reports` instead. A boundary that changes nothing an
  observer can see publishes nothing to that observer, so a report's `as_of`
  is the time it last changed. A holder with several observer bindings gets
  one report for each. A report larger than one knowledge record is
  published instead of failing its boundary, which under 0.14.0 happened
  past about 196 aggregates. A sealed run supersedes the reports it
  published before the seal. Nothing is published to an observer whose actor
  is dead: the changed parts are deferred and go out with the first fiscal
  change after the person is alive again. A report plan that comes with a
  scenario is ignored.

Known limits carried by this release, each stated in its proposal:

- An operation that does not fit the resource archive-candidate budget
  fails its boundary and stays queued until an archive commit makes room,
  and a commit can free at most the budget minus one
  ([choosing the budget](proposals/resource-archive-lookup-and-volume.md#choosing-the-budget)).
- Every boundary that changes the resource root still decodes, validates,
  and journals all of its hot terminal records
  ([choosing the budget](proposals/resource-archive-lookup-and-volume.md#choosing-the-budget)).
- The restore audit of the resource archive is linear in the archive, the
  production archive still has a lifetime limit of 8,192 commits, and the
  force-supply reference archive still walks its commit chain
  ([measurements](proposals/resource-archive-lookup-and-volume.md#measurements),
  [other archives](proposals/resource-archive-lookup-and-volume.md#other-archives)).
- A single-step lease that finds no room in the archive-candidate or hot
  outcome budget, a failed archive lookup, or the state byte cap still fails
  the boundary that settles it, so a host application checks headroom first
  ([single-step completion lease](proposals/resource-single-step-lease-and-floor-install.md#single-step-completion-lease-downstream-gap-g-53)).
- An active completion lease never expires, and a stray `Requested`
  acquisition blocks every single step that sorts after it
  ([what a host application has to watch](proposals/resource-single-step-lease-and-floor-install.md#what-a-host-application-has-to-watch)).
- The 4,096 protected-floor revisions are one quota shared by every
  custodian, below the canonical account limit, and an archived demand keeps
  only the identity of the floor it was admitted under
  ([runtime protected-floor install](proposals/resource-single-step-lease-and-floor-install.md#runtime-protected-floor-install-downstream-gap-g-56)).
- A first operation-keyed draw that cites evidence a seal archived is
  refused where an unsealed run would draw
  ([an exact retry after a seal](proposals/keyed-draw-index-and-settlement-profile.md#an-exact-retry-after-a-seal)).
- A checkpoint, a paged checkpoint, a seal, a fork, and a restore still do
  work for every keyed draw the run has made, and the keyed-draw index keeps
  one entry in memory for each drawn address
  ([what it costs, and what still grows](proposals/keyed-draw-index-and-settlement-profile.md#what-it-costs-and-what-still-grows)).
- Under a coarse society cadence one due transfer per cohort completes per
  settlement, and the owner-side queue record is rewritten whole by every
  boundary that admits a packet
  ([open decisions and follow-up](proposals/society-cadence-and-runtime-revision.md#open-decisions-and-follow-up)).
- A fiscal execution receipt or historical-context change that does not fit
  the state byte budget still fails its boundary, and the fiscal caps are
  lifetime caps because the crate has no archive
  ([several actions per boundary](proposals/fiscal-throughput-and-authority.md#several-actions-per-boundary-g-44)).
- Deferred fiscal report parts are published only by the next boundary that
  changes the fiscal state, and the engine's cost of carrying published
  knowledge grows with every publication
  ([reports](proposals/fiscal-throughput-and-authority.md#reports)).
- An audit binding establishes attribution, not independence, and who
  appoints an auditor is left open
  ([audit bindings](proposals/fiscal-throughput-and-authority.md#audit-bindings-g-57)).
- The record versions that `rebound_by`, an audit binding, and a finding's
  `entered_by` name are not declared as required evidence, so a superseded
  one may no longer be provable after `seal_evidence`
  ([authority rebinding](proposals/fiscal-throughput-and-authority.md#authority-rebinding-g-48)).

Snapshot format 8, commitment format 4, checkpoint/evidence-journal format 4,
paged-checkpoint envelope format 4, and the hash algorithms are unchanged, and
the simulation core adds no persisted field: the keyed-draw index and the
boundary profile are never persisted or committed, and a placed person
creation encodes as it did in 0.14.0. The society root keeps schema version 2
and its shape, and the cohort exchange ledger keeps schema version 1: a
ledger without pressure revisions encodes to the same bytes. The resource
plugin state shape and `RESOURCE_STATE_FORMAT_VERSION` are unchanged.
`FISCAL_RUNTIME_SCHEMA_VERSION` stays 2: every new fiscal field is omitted
while unused, so the payload of a fiscal state written by 0.14.0 decodes and
encodes back to the same bytes. The state record around it is not the same:
its references now carry an `observer` entry for each observer's person, so
a record that 0.14.0 encoded with observers is refused when the run is
created and has to be built again with `FiscalState::into_record`. The
`fiscal_report` knowledge schema moves to version 2, and version 1 is no
longer registered.

Three plugin semantic identities differ from 0.14.0: `canwu-society`,
`canwu-resource`, and `canwu-fiscal`. The society descriptor gains the new
command, the internal ingress type, and its events. The fiscal descriptor
gains one event type on the settlement system, the person-availability read
on the aggregate and report systems, the `observer` reference role on the
state record schema, and the new report schema version. The semantic
identities of the other first-party plugins are unchanged.
Resource archive directories written by 0.14.0 do not decode, which affects
no run, because no 0.15.0 run can reach one. As with any version change, the
engine version in every checkpoint hash changes, first-party plugins that
take their plugin version from the crate version report a new one, and the
exact engine-version check rejects 0.14.0 saves; retain the 0.14.0 engine to
read them.

Version 0.14.0 is a pre-1.0 minor release. It replaces 0.13.3's day read with
a read of any window of simulated time (see the
[boundary window proposal](proposals/boundary-window.md)):
`SimulationView::boundaries_between(from, to)` returns `BoundaryWindow`, and
the host forms are `Simulation::boundaries_between`,
`CompactedSimulation::boundaries_between`, `Canwu::boundaries_between`, and
`CompactedCanwu::boundaries_between`. A boundary system declares how far back
it reads settled boundaries as the new public field
`BoundarySystemContract::boundary_schedule_horizon` (zero by default, at most
`MAX_BOUNDARY_SCHEDULE_HORIZON`, and only with the `canwu.core.boundary_schedule`
read); each system may look back only as far as its own horizon, the run keeps
settled boundaries for the longest declared one, and
`boundary_schedule_horizon` on `Canwu`, `CompactedCanwu`, `Simulation`, and
`CompactedSimulation` reports it. `boundaries_on_day` and
`DayBoundaries` are removed. The new contract field and the removal are
source-incompatible, so the release is a minor version. The field is omitted
from a serialized contract while it is zero, so plugin descriptors and semantic
hashes of systems that do not declare a horizon are unchanged. Snapshot format
8, commitment format 4, checkpoint/evidence-journal format 4, paged-checkpoint
envelope format 4, and the hash algorithms are unchanged; the ledger adds
nothing to saves or commitments. As with any version change, the engine
version in every checkpoint hash changes, first-party plugins that take their
plugin version from the crate version report a new one, and the exact
engine-version check rejects 0.13.3 saves; retain the 0.13.3 engine to read
them.

Version 0.13.3 is a patch release. It adds a read of the boundaries of one
simulated day, so a boundary system can count the boundaries its day has
settled, is settling, and still has queued, including those other sources
open, before it carries work into another boundary:
`SimulationView::boundaries_on_day` behind the new
`StateKey::core_boundary_schedule` read; the host forms
`Simulation::boundaries_on_day`, `CompactedSimulation::boundaries_on_day`,
`Canwu::boundaries_on_day`, and `CompactedCanwu::boundaries_on_day`; the types
`DayBoundaries`, `BoundarySummary`, `BoundaryIngressCount`,
`BoundaryIngressKind`, and `BoundaryIngressOrigin`; and in `canwu-time`,
`SimTime::day` and `SimTime::MINUTES_PER_DAY`. Only phased boundary systems
may declare the read: like `canwu.core.ingress`, it is refused for plugin
commands and synchronous event-driven systems, because it names other
plugins' queued work. The settled ledger is a runtime index derived from
the boundary and ingress journals and rebuilt on restore, so snapshot format 8,
commitment format 4, checkpoint/evidence-journal format 4, paged-checkpoint
envelope format 4, public type shapes, and the hash algorithms are unchanged,
and every change is additive and source-compatible. As with any version
change, the engine version in every checkpoint hash changes, first-party
plugins that take their plugin version from the crate version report a new
one, and the exact engine-version check rejects 0.13.2 saves; retain the
0.13.2 engine to read them.

Version 0.13.2 is a patch release. It adds public API calls that write and
restore the paged checkpoint, which `canwu-sim` already implemented:
`Canwu::prepare_paged_checkpoint`, `Canwu::from_paged_checkpoint_and_journal`,
and `Canwu::from_paged_checkpoint_and_journal_with_plugins`; the same three
calls and `journal_segment_since` on `CompactedCanwu`; and in `canwu-sim`,
`CompactedSimulation::from_paged_checkpoint_and_journal`, its `_with_plugins`
form, and `Simulation::from_paged_checkpoint_and_journal_with_plugins`.
[Format 8 state pages and decision history](#format-8-state-pages-and-decision-history)
describes them. Every change is additive and source-compatible. Snapshot
format 8, commitment format 4, checkpoint/evidence-journal format 4,
paged-checkpoint envelope format 4, public type shapes, and the hash
algorithms are unchanged. The engine version is part of every checkpoint hash,
so every run's checkpoint hashes, and the paged roots that record them, differ
from 0.13.1 exactly as they do across any version change, and first-party
plugins that take their plugin version from the crate version report a new
one. The exact engine-version check rejects 0.13.1 saves; retain the 0.13.1
engine to read them.

Version 0.13.1 is a patch release. Snapshot format 8, commitment format 4,
checkpoint/evidence-journal format 4, and public type shapes are unchanged. It
adds `SimulationView::replay_stable_domain_record_version`,
`canwu_military::MILITARY_REJECTION_EVENT`, and `OperationState::validate`,
and fixes these behaviors, which are visible to existing runs:

- A `Deferred` resolution no longer removes its ticket from the decision
  deadline index, so a deferred ticket expires at the first boundary after its
  deadline in live, restored, and replayed runs alike. Under 0.13.0 a live run
  kept such a ticket `Open` indefinitely, yet every later mutation was rejected
  as `ClosedTicket`; a person-availability sweep that tried to cancel it failed
  the boundary; and a run restored from a snapshot expired it and so diverged
  from the live run.
- A record's current version keeps its exact-version evidence after
  `seal_evidence`: `domain_record_version_evidence_exists`,
  `domain_record_version_is_current`, `domain_record_version`, and
  `evidence_time` resolve it in a sealed run as they do in full exact replay.
  Under 0.13.0 a sealed run lost that evidence with its establishing boundary,
  so it could reject a continuation that its exact replay accepted. A sealed
  non-current version still has no body or time.
- `canwu-production` admits an exact domain-record version as evidence only
  when it is the current version of its record: the realization evidence of a
  `CompleteExecution`, and every version a `StartExecution` or
  `CreateFacilityProject` cites or reads through a cited record.
  `AdvanceFacilityProject` no longer rereads a project's provider and
  technology evidence, which its creation validated, and still rechecks its
  resource evidence. Under 0.13.0 an earlier version was accepted while its
  evidence was retained, but the command was rejected, or its boundary failed,
  once `seal_evidence` had removed that evidence, so a compact run could
  diverge from its exact replay. Such an earlier version is now rejected in
  every run.
- `canwu-history-research` and `canwu-fiscal` resolve the exact versions they
  cite through `SimulationView::replay_stable_domain_record_version`, which
  returns a version's body and establishment time only when no seal can change
  them: for the committed current version, a version proposed earlier in the
  boundary and not superseded since, or an initial-scenario version. Every
  exact domain-record version a historical assessment command names (its
  subject, contradictions, supersessions, and record-version citations) must
  resolve through it when the command is admitted. Settlement no longer
  re-checks that evidence, so a later update to the subject cannot fail the
  boundary, and an identical resubmission of a recorded assessment stays a
  no-op. Each external evidence version a fiscal execution receipt cites must
  resolve the same way when the receipt settles, and
  `enqueue_execution_receipt` rejects one that would not; an identical resent
  receipt settles unchanged before its evidence is checked. If a cited
  evidence record changes after the receipt is enqueued and before it
  settles, the settling boundary fails and the due receipt cannot be
  cancelled, so receipts should cite create-only evidence kinds. As with
  production, an earlier version that 0.13.0 accepted only while its evidence
  was retained is now rejected in every run, so an assessment can no longer
  name a superseded version that the scenario did not provide. Generic
  history-research citations, such as events and boundaries, are still
  checked against retained evidence and remain a known way for a sealed run
  to differ from its exact replay.
- `canwu-military` no longer panics or jams its queue. A successful special
  operation completes and returns the force to its start as `Ready`, instead
  of panicking. Apart from host setup errors, admission returns only errors
  the engine records as rejected attempts. A command that fails in phase 7,
  including a stale `expected_force_revision`, a duplicate ID, a missing
  opposing force, or an over-long operation ID, becomes a `Rejected` ledger
  outcome and a `canwu.military.ingress_rejected.v1` event
  (`MILITARY_REJECTION_EVENT`); a bad provider acknowledgement only emits that
  event. Two provider acknowledgements due in one boundary now compose,
  reports skip dead commanders and send one knowledge batch per commander (at
  most 64 commanders per boundary), and combat losses come off subunits in ID
  order, so a force that has fought still validates. Under 0.13.0 each of
  these failed the boundary, and every later boundary failed the same way. A
  failure in a tick the plugin scheduled for itself still fails the boundary.
  Authority is tightened: `CreateForce` must name the issuer as commander,
  force and occupation commands require the current commander,
  `AssignCommander` rejects a missing or dead commander, `AdvanceTick` sent as
  a command is rejected, and hosts can no longer enqueue the internal
  `military_command_v1` packet. `MAX_RECORDS` (4,096 records per kind over a
  run), `MAX_COMPOSITION_ENTRIES` (at most 64 branch profiles, 64 tactics, and
  64 terrain modifiers in a ruleset), and `MAX_OPERATION_PARTICIPANTS`
  (distinct forces) are now enforced. Admission validates the installed
  ruleset, so a ruleset over these limits rejects every military command as
  `InvalidPayload`.
- `canwu-fiscal` admission rejects a bad command instead of failing the
  boundary. Under 0.13.0 a stale procedure revision, a missing quote, a quote
  of an undeclared kind, a remission naming an unknown assessment, an unknown
  scope, or a malformed action ID failed the boundary, and every later
  boundary failed the same way with time and revision frozen. Admission no
  longer checks the revision; settlement records a stale action as a
  `Rejected` outcome, which consumes its action ID and advances the procedure
  revision like any settled action, so a client retries with a new action ID.
  A missing assessment, scope, or audit target is rejected as
  `EntityNotFound`, and so is a commutation quote of an undeclared kind or one
  that does not resolve through `replay_stable_domain_record_version`, such as
  a superseded version that the scenario did not provide. A malformed action
  or binding ID is rejected as `InvalidPayload`. A missing fiscal catalog or
  state still fails the boundary, as a host setup error.
- `canwu-resource` checks the exact versions it cites against the
  current-version provenance index rather than retained evidence, so a sealed
  run admits exactly what its replay admits. The authority evidence of
  `IssueAccessGrant` (0.13.0 accepted any available exact version), the
  eligibility-envelope evidence of an `Acquire`, and adapter packets that name
  a force-supply record must cite the record's current version; an `Acquire`
  whose eligibility evidence is no longer current when it settles fails that
  boundary. A production output batch's pinned provider source, a
  certificate's locked targets, and a force `Consume` may cite an earlier
  version, which is checked only for consistency with the current version (an
  earlier version number and establishing source); a force `Consume` must
  keep as its consumer evidence the exact force version its certificate
  locked. Under 0.13.0 output settlement read the retained bodies of those
  earlier versions, so a sealed production run stalled when its output
  settled. `SimulationView::current_domain_record_version` also accepts the
  administrative domain-record read.
- The economy and force-supply reference integrations no longer read
  historical record bodies that a seal can remove. Requisition coordinator
  commands, force supply observations, and route and price observations cite
  the current version of the record they name; other observation source
  versions are checked only for consistency with the current version. The
  economy plugin reads live state, or the resource package archive for an
  archived record, instead of a cited body, and a second intent holding the
  same lock can no longer change a local economy the first one already
  changed. A force-supply resource outcome or externality packet whose cited
  version is no longer current when it applies fails its boundary, so a host
  cites the version current when it enqueues a packet that is due at once.
  Under 0.13.0 a sealed run of the requisition saga could stall or diverge
  from its exact replay. The grain loop also applies each month's resilience
  and force postures in the month they were chosen rather than a month late,
  and its requisition branch applies its local cost.

The production, history-research, fiscal, military, resource, economy
reference (its plugin and its typed-evidence adapter), and force-supply
reference plugin semantic identities change. The engine version is part of
every checkpoint hash, so every run's checkpoint hashes differ from 0.13.0; a
run in which a deferred ticket was still open when a boundary passed its
deadline also records the ticket as `Expired`, which changes its decision
commitment. The exact engine-version check rejects 0.13.0 saves; retain the
0.13.0 engine to read them.

Version 0.13.0 shipped the second and final release group of the
[downstream grand-strategy gap set](proposals/downstream-grand-strategy-gap-set.md);
only the §29 series items remain future work. Every contract is additive:

- `canwu-core`, re-exported by `canwu-api`: `EvaluationTraceRecord` and
  `EvaluationTerm`, the application-neutral explanation of how one rule
  produced one integer result for one subject at one boundary.
- `canwu-sim`, re-exported by `canwu-api`: rule-evaluation traces recorded by
  the `RecordEvaluationTrace` boundary directive from phase 7 or phase 12 into
  `BoundaryRecord::evaluation_traces` (`BoundaryEvaluationTrace`), bounded per
  boundary by the run configuration's `EvaluationLimitsV1`
  (`RunConfiguration::with_evaluation_limits`) and failing the boundary with
  `EvaluationTraceLimitExceeded` when exceeded. Transition manifests add the
  `RegisterTransitionManifest` and `StageTransitionWrite` directives, the
  `canwu.core.transitions` state key, `TransitionManifest`,
  `TransitionParticipant`, `TransitionRecordVersion`, `TransitionManifestId`,
  `PendingTransitionManifest`, and a phase-11 audit recorded as
  `TransitionAuditRecord` (`Committed` or `Expired`) on the boundary record and
  receipt; a partially staged manifest fails the boundary with
  `TransitionParticipantMissing` and a wrong expected version with
  `TransitionVersionMismatch`. Pending manifests persist in
  `SimulationSnapshot::pending_transition_manifests`.
- `canwu-api`: `CanwuViewer::evaluation_traces` with the evidence-free
  `EvaluationTraceView` and `EvaluationTermView`, and
  `pending_transition_manifests` on `Canwu` and `CompactedCanwu`.
- `canwu-decision`: a ticket's `parent_ticket` may also name a terminal ticket
  whose assigned controller is bound to the same non-empty seat as the new
  ticket's controller, so a successor continues a predecessor's decision.
- `canwu-resource`: delegated access grants (`ResourceAccessGrantV1`, the
  `IssueAccessGrant` and `RevokeAccessGrant` requests, the
  `ResourceDemandSourcePolicyV1::Granted` source policy, grant-backed
  transfers, and the holder-bound `resource_access_grant_status` read). A
  granted debit settles only in a boundary whose time equals its certified
  lease time, and the grant must be current then.
- `canwu-transport`: capacity pools and deterministic booking allocation
  (`TransportCapacityPoolV1`, `CapacityBookingRequestV1`,
  `allocate_capacity_bookings`, `BookingAllocationV1`,
  `CapacityBookingAllocationEvidenceV1`, `CapacityAllocationFailureV1`),
  booking requests on an execution, booking confirmation or cancellation
  before the booking window opens, execution cancellation, arrival settlement
  for an execution without a delivery attempt, and a terminal seizure after
  which an execution can only be closed. `TRANSPORT_SEMANTIC_VERSION` is `canwu-transport.v5`.
- `canwu-movement` (new crate): `MovementPlugin`, the movement lifecycle
  extension that owns transport executions and capacity pools in one
  `MovementState` record, settles legs at their due times, allocates pools in
  phase 7, and publishes holder-relative movement reports. It never draws
  randomness.
- `canwu-correspondence`: delegated carriers (`delegate_carrier_v1`,
  `CarrierDelegationRequest`, `InitiateCorrespondenceRequest::carrier_delegation`,
  `CorrespondenceIntent::carrier_authority`), recorded by the plugin as the carrier's current delegation for each sender
  (citable from the next boundary, replaced by a newer delegation); the terminal
  `CorrespondenceIncidentKind::CarrierSeized` incident; and the carrier's
  `attempt_report` knowledge schema.
- `canwu-information`: `AuthenticityFinding` on `InterpretationPayload`.
- `canwu-law`: weighted and unit-block procedure stages (`seat_weights`,
  `block_of_seat`, `block_threshold`, and the `status-quo` and
  `casting-seat:<seat>` tie-breaks) and the advisory
  `ProcedureStageKind::Consultation`.
- `canwu-society`: policy-pressure provenance (`PolicyPressure::issuer`,
  `decision_version`), the `cohort_headcount_rebase_v1` ingress, the internal
  `society_lifecycle_delta_v1` ingress, and the queued phase-12 society intake.
- `canwu-culture`: the new `CultureBoundaryPlugin`, which settles the culture
  lifecycle in a Monthly phase-7 system, admits `culture_exposure_v1` batches,
  and hands society deltas to the society plugin. `CulturePlugin` and its
  host-driven flow are unchanged.
- `canwu-technology`: an optional `external_source` on practice transmission
  opportunities (`ExternalTransmissionSourceV1`).

New enum variants (including `ErrorCode`, `BoundaryDirective`,
`ResourceOperationRequestV1`, `ResourceOperationKind`,
`ResourceDemandSourcePolicyV1`, `CorrespondenceIncidentKind`,
`ProcedureStageKind`, and `TransportError`) and new public struct fields (for
example on `RunConfiguration`, `BoundaryRecord`, `BoundaryReceipt`,
`SimulationSnapshot`, `ResourceState`, `ResourceTransfer`,
`InitiateCorrespondenceRequest`, `CorrespondenceIntent`,
`InterpretationPayload`, `ProcedureStageDefinition`, `PolicyPressure`, and
`TransmissionOpportunityPayload`) are source-incompatible: exhaustive matches
and struct literals must be updated, so the release is a pre-1.0 minor version.
New serialized fields are omitted while empty or at their default, and runs
that never register a manifest or record a trace hash exactly as before.
Snapshot format 8, commitment format 4, and checkpoint/evidence-journal format 4
are retained.

The resource, correspondence, information, society, technology, and law plugin
semantic identities change. `MovementPlugin` and `CultureBoundaryPlugin` are new
plugin identities. The `CulturePlugin`, production, fiscal, military,
history-research, and reference-world plugin semantic identities are unchanged. Existing law plans keep their canonical encoding and
content hash.

Version 0.13.0 also fixed these behaviors, which are visible to existing runs:

- A live production completion now settles its output. The resource side of an
  execution's completion lease locked the production runtime at the version
  current when the lease was granted, so a live completion could not settle its
  credit on 0.11 or 0.12; the credit now cites the version production pinned
  as the execution's output source.
- `AmendDemand` can no longer change a demand's lifecycle status, rejection
  reason, or requester; such an amendment is a recorded rejection instead of a change that
  jammed later boundaries.
- A law procedure stage that stops accepting ballots expires its pending seat
  work; a late seat response is recorded as a rejected outcome instead of
  failing the plugin boundary; and the pre-settlement budget check counts
  ticket work emitted at the exact deadline minute.
- The society Daily boundary after an applied cohort transfer no longer fails:
  applying a transfer invalidates the derived aggregates, mobilization
  candidates, and projections it affects, and a transfer's digest binds only
  the state the transfer depends on. An initial scenario cannot seed queued
  society ingress, and a restored snapshot's queue is validated like admitted
  packets.
- Snapshot restore accepts initial domain records that reference entities
  listed only in `Scenario::entities`, as the live state check already did.

Exact engine-version and plugin-descriptor checks still apply: a 0.12.0
snapshot must not be relabeled or loaded into 0.13.0; retain the previous
engine or use an explicit application-owned export. `canwu-movement` joins
publish group 6, so all 24 publishable library crates move to 0.13.0 together.

Version 0.12.0 shipped the first release of the
[downstream grand-strategy gap set](proposals/downstream-grand-strategy-gap-set.md).
Every contract is additive:

- `canwu-sim`, re-exported by `canwu-api`: core person availability
  (`PersonAvailability`, `LifeState`, `CustodyState`) changed only by the
  `SetPersonAvailability` boundary directive. Commands from an unavailable
  issuer are rejected with `IssuerUnavailable`; decision tickets whose person
  decision maker or controller authority person is unavailable cannot be
  opened, prepared, or resolved (`DecisionMakerUnavailable`,
  `IssuerUnavailable`), and open ones are cancelled at the end of the boundary
  that made the person unavailable (`decision_maker_unavailable`,
  `controller_authority_unavailable`). Runtime person creation adds
  `CreatePerson`, `PersonDraft`, and correlation-keyed `created_persons`
  receipts. Queued plugin ingress can be withdrawn before its due time through
  `cancel_plugin_ingress`, `cancel_permitted_plugin_ingress`, or the
  `CancelPluginIngress` directive, journaled as
  `IngressPayload::PluginCancellation`.
- `canwu-decision`: `GuardedUtilityPolicy`, `RuleChoice::Exclude`,
  `DecisionStage`, `DecisionOutcome::PendingRandom`, the controller opt-in
  `random_tie_break`, `DecisionPolicyIdentity::semantic_hash`, and ticket
  `parent_ticket` lineage.
- `canwu-resource`: account-level `RecordLoss`, atomic `BeginExchange`,
  same-place `AcceptLocal` gated by the host-declared
  `ResourceAccount::place_scope`, and loss observations in holder reports.
- `canwu-production`: realized output at `CompleteExecution`, bounded by
  `ProcessRevision::max_realized_per_mille` and the process's declared
  realization evidence kinds.
- `canwu-fiscal`: an acting actor admitted while its exact authority basis
  record version is current (`FiscalPlugin::with_authority_basis_kinds`).
- `canwu-transport`: the `PersonsGroup` subject role, `HandoffKind::Seizure`,
  and the `ExternalCondition` itinerary revision reason.
- `canwu-correspondence`: `planning_snapshot_from_holder_knowledge`, its pure
  form `planning_snapshot_from_knowledge_result`, and `KnowledgeReadCutDigest`.

New enum variants (including `ErrorCode`, `DecisionAttemptErrorCode`,
`BoundaryDirective`, `IngressPayload`, `RuleChoice`, `DecisionOutcome`,
`ResourceOperationRequestV1`, `ResourceOperationKind`,
`ResourceTransferDispositionV1`, `MovementSubjectRole`, and
`ItineraryRevisionReason`) and new public struct fields (for example on
`BoundaryRecord`, `BoundaryReceipt`, `SimulationSnapshot`, `DecisionTicket`,
`DecisionTrace`, `DecisionControllerBinding`, `ResourceAccount`,
`ProcessRevision`, `FiscalAuthorityBinding`, `Handoff`, and the
`CompleteExecution` operation) are source-incompatible: exhaustive matches and
struct literals must be updated, so the release is a pre-1.0 minor version. New
serialized fields are omitted while empty or at their default, so records that
do not use a contract keep their canonical encoding and hashes. Snapshot format
8, commitment format 4, and checkpoint/evidence-journal format 4 are retained.
The resource, production, and fiscal plugin semantic identities change, as does
the resource holder-report knowledge schema; `TRANSPORT_SEMANTIC_VERSION` is
`canwu-transport.v4`; the correspondence plugin semantic hash is unchanged.
Host-authored decision ingress that carries random draw evidence is now
rejected, both live and on load: only a boundary `ResolveDecisionRandomly`
resolution may produce draw evidence, for random-policy controllers and
guarded-utility tie-breaks alike. Exact engine-version and plugin-descriptor
checks still apply: a 0.11.1 snapshot must not be relabeled or loaded into
0.12.0; retain the previous engine or use an explicit application-owned export.
The documented publish order now lists every publishable crate, including
`canwu-law` and the military crates `canwu-military` and
`canwu-military-reference-content` (last published at 0.10.0), so all 23
publishable library crates move to 0.12.0 together.

Version 0.11.0 added a persisted source policy to resource demands: `Pooled`
retains shared-pool behavior, while bounded `ExactAccounts` names lawful
requester-custodied sources. Admission, amendment, allocation, restore and
replay validate that policy before reservation; there is no silent fallback to an
unlisted account. This public Rust field is an additive semantic capability but
struct literals are source-incompatible before 1.0, so the release is a minor
version and retains snapshot format 8. The resource plugin semantic identity
changes. A 0.10.1 snapshot must not be relabeled or loaded into 0.11.0; retain
the previous engine or use an explicit application-owned export. A downstream
application's pinned registry version does not change with this local checkout.

Version 0.10 adds optional resource, production, reference economy-content,
force-supply, and runnable economy-reference packages. They are built above
`canwu-api`; the simulation core gains no production, market, resource, or
military-doctrine semantics. The release retains format 8. Exact plugin
descriptors and semantic hashes make a 0.9 save intentionally incompatible
with a 0.10 run that enables the new packages, and no implicit migration is
provided.

Version 0.9 added the public Random decision policy, random option-weight and
draw-evidence fields, a decision-ticket random-operation target, and a
boundary-generated random resolution directive. These additions change public
Rust exhaustive matches and struct literals, so they require a pre-1.0 minor
version bump. The snapshot format number remains 8, while the exact
`engine_version` continues to make 0.8 and 0.9 snapshots intentionally
non-interchangeable; applications must use an explicit export if they need to
carry state across that engine boundary.

Format 8 keeps the exact decision-ingress commitment and adds explicit plugin
descriptor versioning, typed decision-history locations, canonical domain-record
primary/reverse/successor commitment roots, and content-addressed state-page
prepare/store/verify contracts. Format-7 snapshots, replay journals,
checkpoints, evidence segments, and plugin descriptors therefore require the
0.7 engine or an explicit application-owned export; the 0.8 runtime rejects
them before constructing mutable state.

The runtime no longer contains `migration.rs` or `legacy_v4.rs`. An older
snapshot must not be silently relabeled as format 8, and a legacy replay
journal cannot be promoted to exact replay.

Random streams persist their reduction algorithm. Existing
`SplitMix64V1` streams retain their historical modulo reduction, while new
streams use `SplitMix64V2`, which applies unbiased rejection sampling. This
keeps old format-7 journals replayable while making the current behavior
explicit in the persisted state.

## Format 8 state pages and decision history

`StatePageBlob` is the canonical uncompressed page envelope. Its `page_id` is
the domain-separated BLAKE3 hash of the bytes; hosts may store it in any
backend, but `StatePageProvider` must return the exact bytes or the runtime
returns `state_page_unavailable`. `prepare_state_delta` and
`verify_state_delta` are deterministic and idempotent: they do not mutate the
authoritative simulation revision or replay journal.

The paged-checkpoint envelope format is `4`. It stores a compact
`SimulationCheckpoint`, not an unaudited state-only snapshot. Restoring a
checkpoint whose journal cut is non-empty requires the exact contiguous
evidence prefix through `from_paged_checkpoint_and_journal`; the empty-journal
shorthand is valid only for a zero cut.

Hosts write and restore paged checkpoints through the public API.
`Canwu::prepare_paged_checkpoint(source, provider)` returns a
`PreparedPagedSimulationCheckpoint` whose delta leaves out the pages the
provider already holds. A runtime restored from a paged checkpoint also
leaves out the archived decision pages it never loaded, so its provider must
already hold every page of the checkpoint it was restored from.
`store_and_verify` stores the delta and checks only its new pages and the
root; `StatePageRetentionLedger::prepare` and `verify` check a checkpoint's
complete page set.
`Canwu::from_paged_checkpoint_and_journal(checkpoint, provider, segments)`
and `from_paged_checkpoint_and_journal_with_plugins` restore it from the
provider and the evidence prefix, which a host can collect as it goes with
`journal_segment_since`. Every segment must advance at least one journal, so
a host skips a segment whose start equals its end. Preparing changes nothing:
the authoritative revision, the checkpoint hash, and every journal are
unchanged. The checkpoint carries no evidence records; apart from compact
continuation metadata, such as one archived-segment header per seal, its size
follows current state rather than the length of the run, and each delta holds
only the changed pages. Preparing can still read retained evidence, and a
restore still validates the whole evidence prefix.

`CompactedCanwu` offers the same calls. Its evidence prefix is the host's
sealed segments in cursor order followed by `journal_segment_since` the end of
the last one. A compact restore needs the `_with_plugins` form whenever the
checkpoint names a plugin, and keeps the whole supplied prefix as retained
evidence. Its next seal therefore returns one segment from the zero cut that
replaces every earlier sealed segment: prefixes for later checkpoints start
from that segment, and only checkpoints taken before it still need the
earlier ones. The flat `CompactedCanwu::from_checkpoint_and_journal` behaves
the same way. A paged root also commits to which evidence is sealed, so it is
not a state identity: any restore from sealed segments, through `Canwu` or
`CompactedCanwu`, keeps the original checkpoint hash but gets a different
paged root, which a later seal does not bring back. A runtime that has never
sealed evidence restores to the same paged root.

Decision history is queried by `DecisionHistoryKey` (`Ticket`, `Attempt`, or
`Trace`) and reports `Hot`, `Archived`, `Unresolved`, or `Absent` through
`DecisionHistoryLocation`. Terminal tickets, attempts, and traces can move to
verified archive blobs through canonical maintenance ingress. The compact
checkpoint keeps a bounded hot decision page plus a two-level locator
directory: 4,096 stable hash-prefix buckets, each split into 16 deterministic
subsegments. A segment is capped at 64 entries and 1 MiB; unchanged pages are
reused by content address. The locator page table is itself chunked into
authenticated directory pages of at most 1,024 page references, so the root
manifest remains below the 4 MiB state-page limit at one million keys. An exact
lookup loads exactly one locator segment from a provider. The hot-history root is maintained incrementally from count, XOR, and
modular-sum digests and is rebuilt and verified on restore. Missing pages return
`DecisionHistoryUnavailable`, never `Absent` or an empty success. Verification
produces a provider-authenticated full replacement for every touched locator
page, bound to its previous page ID. Boundary commit therefore needs no provider
read, is replay-safe, and can append into an existing segment after a root-only
restart without discarding earlier receipts.
Before hot-state validation, paged restore derives the exact archived ticket or
trace references still named by hot traces, resolved tickets, and accepted
attempts, then authenticates only those locator pages against the committed
directory. Partially archived dependency graphs therefore restart correctly
without loading unrelated history.
Restore rejects noncanonical locator bucket/segment placement and any archived
locator or terminal payload duplicated in hot metadata. Repeat delta
preparation reads only directory pages for changed locator segments rather than
walking all 65,536 possible pages.

Offline GC starts from `ArchiveReachabilityManifest`. The kernel contributes
retained state pages, evidence segments, decision locator pages, pending
maintenance locators, and decision blobs. Every frozen plugin archive
participant extends the same manifest automatically; callers do not append law
marks manually. The law plugin contributes committed or leased content, blob,
membership, effective-time, and recorded-time page IDs. The retention ledger
derives and verifies the exact transitive page closure from the provider,
persists `Prepared`, `Verified`, `DurableIngress`, and terminal root handles
across restart, interlocks each ownership handoff with the active GC epoch, and
keeps a pending handle as a new-object delta plus the proposed current-page
closure while the previous committed root protects older data. Commit moves
the prior object closure into the new root, retires the superseded root, and
clears the terminal handle payload. Root-only restore and overlapping-root
tests prove that shared pages and plugin marks remain protected after restart. Internal
plugin ingress may also carry authenticated archive-retention roots. A pending
verified legal transition carries one directory root rather than a
history-sized list. The law reachability participant authenticates and expands
that root into blob, membership, and temporal objects before mark/sweep.
Snapshot/restart preserves the root; applying or terminally rejecting a stale
item transfers or releases it deterministically.
The legal commit itself binds the exact directory root, object count, shard,
token, source root, and expected record versions; neither a caller-provided
summary nor a different authenticated tree can be substituted.
The synchronous helper authenticates the store retention handle and complete
root closure before mutation, applies to a detached clone, finalizes retention,
and only then swaps the live runtime. Canonical ingress records terminal
outcomes by retention handle until the host finalizes the store from the
authoritative reloaded runtime and a private acknowledgement retires that
recovery metadata. Multiple archive commits in one boundary therefore remain
independently recoverable after restart.

The legal archive object remains format `1`; its authenticated membership and
bitemporal index is format `2`. Format `2` uses the full 16-bit deterministic
bucket space and rejects any membership or temporal page above 64 entries or
1 MiB before storage or root admission. A dense effective-time or recorded-time
cell is represented by an ordered vector of authenticated page segments, so 65
or more versions in one cell remain exact instead of overflowing one page.
Provider-backed temporal queries stream those segments and enforce candidate,
intersection, provider-call, segment, and decoded-byte budgets during I/O;
one query-wide meter covers directory, membership, temporal, and blob reads
across every shard. Pathological overlap fails before an unbudgeted next
segment is read.

Generic domain records use four canonical Patricia roots: primary records,
reverse references, successor edges, and predecessor edges. Forks and rollback
captures share these roots. Incremental checkpoints stop at provider-known
subtree pages, so changing one record emits only its missing Patricia paths and
the bounded manifest path rather than reserializing every record. The paged
checkpoint builder constructs its compact checkpoint body without first
cloning the full domain-record and decision payloads. Boundary proposal
validation likewise filters empty stages first, overlays only changed records
onto a structurally shared root, and validates the affected reference closure.

Plugin descriptors also persist owner-authorized maintenance dependency
resolver declarations. A cross-plugin retirement commit must include the exact
target schema owner and every declared dependent owner. Each proposal is bound
to the plugin semantic hash and may mutate only schemas owned by that plugin;
the complete proposal set is validated against one source domain root and
applied atomically through replay-visible maintenance ingress.
Plugin-owned maintenance packets use registrar-issued opaque internal-ingress
permits; ordinary host-authored plugin ingress cannot forge a verified archive
commit. Applied and post-admission stale maintenance both produce chained
terminal records. A stale item is a successful terminal no-op, not a boundary
rollback that can retry forever. Plugin descriptors persist the exact internal
ingress names so this authority and pending-retention ownership survive restart.

## Self-contained exact replay

`ReplayJournal` is the complete replay boundary. It carries the canonical
initial scenario, declared run identity, run configuration, plugin descriptors,
authority root, commands, attempts, ingress, boundaries, random draws, and
final revision/checkpoint commitments. `replay_from_journal(plugins, journal)`
does not accept a second scenario supplied by the caller; the scenario in the
journal is authoritative and is validated against the manifest.
`replay_from_journal_json` is the strict JSON counterpart and rejects unknown
fields recursively before replay begins.

Executable policy implementations are not replay inputs. Decisions, outcomes,
and evidence already admitted to the journal are replayed as records.
`DecisionPolicyKind::Random` therefore does not execute an RNG during replay.
Its format-8 evidence consists of an operation-keyed draw targeting the exact
ticket version, a `DecisionSelection` outcome, canonical option weights in the
decision trace, and boundary-generated decision ingress. New optional random
evidence fields remain absent on historical non-random decision traces. Since
0.12.0, a guarded-utility random tie-break uses the same evidence, with weights
naming only the pending near-equivalent candidates and the trace recording the
`Random` stage; the stage, fired-guard, and parent-ticket fields are absent on
traces that do not use them.

Since 0.13.0, rule-evaluation traces, transition-manifest registrations, and
transition audits are hash-chained boundary evidence. Exact replay regenerates
and compares them with the rest of the boundary, snapshot validation rebuilds
the pending manifests from that evidence, and sealing archives traces with
their boundary records. A trace is never read back as state, so replay needs no
separate trace input.

## Durable outbox

Boundary emissions are exposed through `Canwu::outbox_entries()`. Each entry
has a stable `delivery_id` derived from the run manifest, boundary, event, and
emission index. The host application must deliver entries at least once and
deduplicate by `delivery_id`. Exact replay regenerates the same outbox
identity; it does not re-send external effects.

In compact mode, `CompactedCanwu::outbox_entries()` returns entries from the
retained evidence tail. Once evidence is sealed, the caller owns the returned
`EvidenceJournalSegment` and must keep its boundary emissions with the host's
delivery/acknowledgement state; compaction does not acknowledge or deliver an
external effect on the host's behalf.

## Granularity boundary

The engine exposes the domain-neutral `SimulationGranularity` enum:

| Value | Meaning |
| --- | --- |
| `aggregate` | A coarse aggregate or population-scale state. |
| `group` | A bounded social, institutional, military, or organizational group. |
| `actor` | A person or other principal with its own knowledge and authority. |

These are engine simulation levels, not a fixed historical ontology. A host
game may map them to its own terms. In Celestial Mandate, for example,
`aggregate` can map to Population, `group` to Special Group, and `actor` to
Character. That mapping belongs in a CM reference integration or host adapter,
not in Canwu core.

## Reference integrations

Southern Ming and WWII are content and ruleset integrations for Celestial
Mandate. They are not Canwu adapters and are intentionally absent from this
repository. Canwu provides the generic state, authority, boundary, evidence,
granularity, persistence, replay, and outbox contracts that those integrations
consume.

## Source compatibility

The 0.8 change removes caller-supplied replay wrappers from the public facade
and adds schema-declared identity-only evidence dependencies plus compact,
Merkle-bound plugin-ingress provenance.
Use the constructor APIs for new runs, `snapshot`/`checkpoint` for persistence,
`replay_from_journal` for exact replay, and `outbox_entries` for host delivery.
Downstream crates must update their code to these contracts; no deprecated
alias is retained before 1.0.


Version 0.11.1 is a patch release. It fixes completion-lease abort and release cleanup so every released held or prepared grant is removed from the bounded expiry index before validation; snapshot format and public type shapes remain unchanged.
