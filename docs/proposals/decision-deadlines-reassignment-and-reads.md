# Decision Deadlines, Reassignment, and Reads

Status: implemented in 0.20.0. It closes downstream gaps G-70 (a per-option
bias on the utility stage), G-81 (no mutation moves an open ticket's deadline
or controller), and G-82 (no read lists the tickets a boundary opened, or a
decision maker's or a controller's open tickets). It changes `canwu-decision`,
the decision paths of `canwu-sim` (ingress, availability, validation and
reconstruction, settlement, the view), and `canwu-api`. Same version only: no
compatibility work with 0.19.0 bytes or hashes.

## Problem

- **G-70.** `WeightedUtilityEvaluator::evaluate` scores an option as
  `Σ weight × utility_inputs[factor]`. An application whose AI holds a standing
  preference for one option (a doctrine's) can only fold it into a factor, so
  the trace shows a preference as a factor line and a policy cannot say "a
  bias, never another factor".
- **G-81.** `DecisionMutation` had `RegisterController`, `Open`,
  `ReplaceOptions`, `Resolve`, and `Cancel`. The deadline is indexed once at
  `Open`, and the assigned controller is fixed there. A host that holds a
  ticket back (an attention cap), grants its holder more time, or delegates an
  open ticket can only cancel and reopen it under a new ID, which breaks every
  owner record keyed on the ticket ID.
- **G-82.** The view reads one ticket, controller, or attempt by ID; the
  boundary window counts admitted ingress without payloads; a controller
  binding lists no tickets. A boundary system that must find the tickets a
  boundary opened, or the host listing a desk's open tickets, reads every
  owner's request records at every boundary that admitted decision ingress, a
  read that grows with the campaign.

## Invariants

- A policy still selects only an existing option; a bias changes a score, never
  the option set or the authority envelope, and it is evidence apart from the
  factors.
- Every ticket mutation goes through decision ingress, is checked at admission
  against the ticket version it names, and is journalled; an expected refusal
  is a recorded rejection, never a failed boundary.
- A ticket keeps its ID for its life. Its deadline moves only later; only its
  assigned controller resolves it; its decision maker and lineage never change.
- What a boundary system reads about decisions is current decision state, so a
  sealed run, its unsealed fork, a restore, and an exact replay decide alike.

## Design

### G-70: option bias on the utility stage

`WeightedUtilityEvaluator` gains `option_biases: BTreeMap<String, i64>` (option
ID to bias) and the builder `with_option_bias(option_id, bias)`, which removes
the option's entry when `bias` is zero, so the map (and the hash below) holds
only biases that change a score. An available
option's score is the weighted factor sum plus its bias (checked `i64`
arithmetic; overflow is a policy error, as for a factor). The evaluation
records it in the new field `DecisionOptionEvaluation::bias` (zero and
omitted from the encoding when there is none), so `score == Σ
factors.contribution + bias` and the trace keeps the bias apart from the
factor lines. Blocked and excluded options are not scored and carry no bias.
In `GuardedUtilityPolicy` biases apply before the near-equivalence margin, so a
bias can make or break a tie-break, and the semantic hash covers them as a
tagged, length-prefixed section appended only when at least one is declared:
a policy without biases keeps its 0.19.0 hash, and one that gains a bias is
refused by a binding of the same ID and version (`PolicyMismatch`).

The row recommended a map on `UtilityProfile` or a field on `DecisionOption`.
A field on the option would make the preference ticket content rather than the
policy's, so two controllers with different doctrines could not read the same
ticket differently. A map on `UtilityProfile` is the policy-side shape, but
`UtilityProfile` is built by struct literal across the workspace and
downstream (`UtilityProfile { weights }`), and a new field breaks every such
literal; `WeightedUtilityEvaluator` is built only through `new`. The map
therefore sits on the evaluator, beside the profile: the same policy-side
shape at the utility stage, with no literal broken.

### G-81: `ExtendDeadline` and `Reassign`

Two new variants:

- `ExtendDeadline { ticket_id, expected_version, deadline }`. Checks, in
  order: the ticket is hot (`TicketNotFound`); it is open and not past its
  deadline (`ClosedTicket`) at `expected_version` (`VersionConflict`); it has
  a deadline (`InvalidDecision`: a ticket without one never expires, so there
  is nothing to extend); `deadline` is later than the current one
  (`InvalidDecision`), and so no earlier than the admission time, because the
  open check already refused a ticket past its deadline. The ticket's deadline
  index entry moves, `updated_at` becomes the admission time, and the version
  advances.
- `Reassign { ticket_id, expected_version, controller_id }`. Checks, in
  order: the new controller's authority person is available
  (`IssuerUnavailable`, the rule `Open` applies to its controller, so a ticket
  never moves to a controller that could not resolve it); the ticket is hot,
  open, and at `expected_version` as above; `controller_id` is a registered
  controller (`InvalidController`) other than the one assigned
  (`InvalidController`); and, when the ticket's parent is in hot decision
  history, the new controller keeps the lineage link, the same decision maker
  or the parent's controller and the new one bound to the same seat
  (`InvalidDecision`). The controller index entry moves, `updated_at` becomes
  the admission time, and the version advances. The new controller alone may
  resolve the ticket from then on (`Resolve` from the old one is refused as
  `InvalidController`); host preparation already follows the assigned
  controller.

"The same decision maker" in the row is kept literally: `Reassign` never
changes the decision maker. The engine has no relation from a controller to a
decision maker beyond the lineage rule, so it adds none; who may answer for
whom stays application policy, and the selected command is still issued under
the new controller's authority and checked by ordinary command admission.

Both are journalled as every non-resolving mutation is: the admitted decision
ingress record carries the mutation and the `DecisionAttemptRecord` records
`Accepted { trace_id: None }` or `Rejected { code, message }`; the ticket's new
version replaces the old in hot history. They record no `DecisionTrace`: a
trace is the evidence of a policy's resolution, and a resolved ticket names
exactly one.

### G-82: tickets opened, open tickets by decision maker and controller

Two open-ticket indexes join the deadline index in `DecisionState`:
`OrdMap<EntityRef, OrdSet<DecisionTicketId>>` by decision maker and
`OrdMap<String, OrdSet<DecisionTicketId>>` by assigned controller. Every
mutation that opens, closes, or moves a ticket keeps them (`Open`, a selecting
`Resolve`, `Cancel`, expiry, `Reassign`); a deferred resolution keeps the
ticket in them. They are derived: never persisted, rebuilt from the open
tickets on every load, and recomputed and compared by `DecisionState::validate`
(as the deadline index is).

Which tickets a boundary opened is not derivable from the tickets (two
boundaries can settle at the same instant), so `DecisionState` persists it as
the **admission batch**: `opened_in_batch`, the ticket IDs in admission order.
`begin_admission_batch` clears it, and each accepted `Open` appends its ticket.
Settlement begins a batch at the start of every boundary, before it admits
anything; snapshot reconstruction does the same per boundary and compares the
reconstructed batch with the persisted one. The batch is part of the decision
commitment (`canwu.commitment.decisions.v3` over the controllers, the
hot-history root, the archive receipt root, and the batch) but not of the
hot-history root, which a prepared archive commit names as its source: a new
batch must not make every queued archive commit stale. Validation requires
each batch entry to be unique and a hot ticket; an archive commit drops
archived IDs from the batch (in a runtime it never holds one, because a batch
begins before an archive commit is admitted).

Reads, each behind a declared `StateKey::core_decisions()` read on the view:

- `SimulationView::decision_tickets_opened()`: the batch's tickets at their
  current version, in admission order. A boundary system reads its own
  boundary's; a plugin command handler admitted in the boundary sees the
  `Open`s admitted before it.
- `SimulationView::open_decision_tickets_of(&EntityRef)`: one decision
  maker's open tickets in ticket-ID order.
- `SimulationView::open_decision_tickets_of_controller(&str)`: one
  controller's open tickets in ticket-ID order.

The host has the same three on `Simulation`, `CompactedSimulation`, `Canwu`,
and `CompactedCanwu` (inherent, not on `CanwuRun`), where
`decision_tickets_opened` lists the latest settled boundary's tickets. They
are trusted-host reads of ground truth; the rustdoc says so.

Cost: each index entry is one ordered-set insert or remove per mutation
(`O(log n)` in open tickets), plus one key per distinct decision maker and
controller; a load rebuilds both in `O(open tickets × log)`, beside the
deadline index it already rebuilt. A read is one map lookup plus the tickets it
lists. The batch costs one ID per `Open` of the latest boundary in every
snapshot and in the decision commitment; it is an `im::Vector`, so the
per-request copy of decision state stays shared.

## Persistence and replay

| Item | 0.19.0 | 0.20.0 | What changed |
| --- | --- | --- | --- |
| `DecisionState` (snapshot, compact and paged checkpoint hot state, continuation checkpoint) | — | — | gains `opened_in_batch: [DecisionTicketId]`, omitted while empty; the two open-ticket indexes are derived and not encoded |
| Decision commitment | `canwu.commitment.decisions.v2` | `canwu.commitment.decisions.v3` | adds the admission batch; every checkpoint hash and `v1:` state commitment of a run with decision state differs. `COMMITMENT_FORMAT_VERSION` moves from 6 to 7 for the new root, as earlier root changes did, so a format 6 snapshot, checkpoint, or replay journal is refused by its format as well as its engine version |
| `DecisionMutation` (decision ingress, replay journal) | 5 variants | 7 variants | adds `extend_deadline { ticket_id, expected_version, deadline }` and `reassign { ticket_id, expected_version, controller_id }` |
| `DecisionOptionEvaluation` (in `PolicyDecision` and `DecisionTrace`, hot history and archive blobs) | — | — | gains `bias: i64`, omitted when zero, so an unbiased evaluation and trace encode as before; decision archive blob format stays 1 |
| `WeightedUtilityEvaluator` | — | — | gains `option_biases`, omitted while empty |
| `GuardedUtilityPolicy` semantic hash | — | — | unchanged without biases; a tagged option-bias section when any is declared |
| `DecisionState::validate` | deadline index | all open-ticket indexes and the batch | also refuses inconsistent decision-maker or controller indexes and a batch that repeats a ticket or names one outside hot history |

Exact replay reruns the boundaries, so it rebuilds the batch and the indexes.
Snapshot validation reconstructs the batch from admitted ingress (a tampered
batch is refused as `InvalidSnapshot`); a checkpoint without replay evidence is
authenticated through the decision commitment. The new mutations replay through
the same reconstruction as the others: the shared availability rule, then
`DecisionState::apply` on a candidate state.

## Refusals

Every refusal named above is a recorded rejection: the admission records a
`Rejected` attempt and the boundary settles. No new error code is added.

## What can still fail a boundary

Nothing new. The decision paths fail a boundary only where they already did:
an attempt the journal cannot append, or a hot-history leaf that cannot be
encoded. A boundary system that calls a new read without declaring
`canwu.core.decisions` gets `UndeclaredStateRead`, as for every view read, and
fails its own boundary if it propagates the error. Beginning a batch and
dropping archived IDs from it cannot fail.

## Public API delta

- `canwu-decision`: `DecisionMutation::ExtendDeadline` and
  `DecisionMutation::Reassign` (exhaustive matches must add them);
  `DecisionOptionEvaluation::bias` (struct literals must set it);
  `WeightedUtilityEvaluator::option_biases` and `with_option_bias`;
  `DecisionState::open_tickets_of_decision_maker`,
  `open_tickets_of_controller`, `begin_admission_batch`,
  `tickets_opened_in_batch`, and `ticket_ids_opened_in_batch`.
- `canwu-sim`: `SimulationView::decision_tickets_opened`,
  `open_decision_tickets_of`, and `open_decision_tickets_of_controller`;
  the same three on `Simulation` and `CompactedSimulation`.
- `canwu-api`: the same three on `Canwu` and `CompactedCanwu`; every type was
  already re-exported. No `&mut self` method is added, so `gated!` is
  unchanged.

## Durable tests

- `utility_option_bias_moves_the_score_and_is_recorded_apart_from_factors`
  (`canwu-decision`, `tests/policy_contracts.rs`): a bias, not a factor, turns
  a guarded utility choice; the evaluation's factors are unchanged, its bias is
  recorded apart and `score == Σ contributions + bias`; an unbiased evaluation
  encodes without `bias`; the semantic hash changes with the bias.
- `gap_g81_decision_deadline_and_reassign` (`canwu-api`): every refusal of
  both mutations is a recorded rejection; accepted ones in the same boundary
  see the versions the earlier ones advanced; a ticket survives its old
  deadline and an extended one expires at its new deadline; only the new
  controller resolves; a seat successor's ticket, linked to a cancelled parent
  of another decision maker by the seat alone, cannot move to an unseated
  controller (`InvalidDecision`, the lineage rule a strict load re-checks) and
  moves to a controller of the same seat; a controller whose authority person
  is captive cannot take a ticket (`IssuerUnavailable`); a strict reload right
  after the mutations and at the end gives back the same decision state (the
  live indexes compared with the rebuilt ones), and exact replay matches.
- `gap_g82_decision_ticket_reads` (`canwu-api`): a boundary system lists the
  tickets its boundary opened in admission order (not ID order), a refused
  `Open` is not listed, and the decision-maker and controller lists follow
  cancellation, reassignment, and expiry; the host lists on `Canwu` and on a
  `CompactedCanwu` continued from a continuation checkpoint agree, as do a
  strict reload and exact replay; a boundary that opens nothing empties the
  batch.

Checked inline (not committed) to fail: settlement without the batch reset
(the boundary system lists the previous boundary's tickets), reconstruction
without the per-boundary reset (the reload is refused), `ExtendDeadline`
without removing the old deadline entry and `Reassign` without removing the old
controller entry (the reload comparison), and `Reassign` without the lineage
check (the seat-successor refusal).

## Documentation delta

For the release documentation pass (canonical page and both website languages):

- `docs/architecture.md`, "Decision framework": done in this change (the bias
  in the guarded-utility paragraph and its hash, the two mutations, the reads
  and indexes, the batch in persisted decision state; the sentence "A ticket
  cannot be reassigned to another controller" is gone; the `Reassign` refusals
  name each code).
- `docs/end-state.md`, person availability: done in this change ("A ticket
  cannot be reassigned" now says the host may `Reassign` an open ticket before
  the kernel cancels it, and a cancelled one continues as a successor).
- `website/src/content/docs/architecture/decisions.mdx` and
  `website/src/content/docs/en/architecture/decisions.mdx`, "Who may
  resolve a ticket": done in this change (the first bullet names `Reassign`).
  Both pages still list only `decision_ticket`, `decision_controller`, and
  `decision_attempt` as boundary-system reads; the release pass adds the three
  new reads and the two mutations there.
- `docs/versioning.md` and both `developer/persistence.mdx` pages: commitment
  format 7 (they say 0.19.0 writes commitment format 6).
- `docs/terminology.md` and both website term pages: the `decision attempt`
  row's list of requests adds "extend the deadline of" and "reassign"; the
  candidates below.
- The engine-user skill's decision notes: hold a ticket back with
  `ExtendDeadline` instead of cancelling and reopening it; delegate an open
  ticket with `Reassign`; find a boundary's new tickets with
  `decision_tickets_opened` and a desk's with
  `open_decision_tickets_of_controller`.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| deadline extension | `DecisionMutation::ExtendDeadline` | 展期 | Moving an open ticket's deadline later under the same ticket ID, checked against the ticket version. |
| ticket reassignment | `DecisionMutation::Reassign` | 改派 | Moving an open ticket to another registered controller, which alone may then resolve it; the decision maker and lineage stay. |
| option bias | `WeightedUtilityEvaluator::with_option_bias`, `DecisionOptionEvaluation::bias` | 选项偏置 | A policy's standing integer preference for one option, added to its weighted score and recorded apart from the factor lines. |
| admission batch | `DecisionState::tickets_opened_in_batch`, `decision_tickets_opened` | 准入批次 | The tickets the latest boundary's admitted decision `Open`s created, in admission order. |

## Known limits

- A ticket without a deadline cannot gain one, and no mutation shortens or
  removes a deadline.
- `Reassign` re-checks lineage only against a hot parent, as snapshot
  validation does; an archived parent's link is not re-read.
- `Reassign` does not compare the policies of the two controllers; a host
  response or pending tie-break computed for the old controller is refused as
  stale by the version.
- The host's `decision_tickets_opened` shows only the latest settled
  boundary's tickets; a host that settles several boundaries before reading
  (`advance_canonical`) reads each boundary's from a boundary system or keeps
  its own list.
- No list by seat, definition, or deadline window; the end-of-boundary sweep
  of tickets whose person became unavailable still scans the hot tickets
  rather than the new indexes.
- Option biases are keyed by option ID, so a policy shared by ticket
  definitions that reuse an option ID applies the same bias to both.
