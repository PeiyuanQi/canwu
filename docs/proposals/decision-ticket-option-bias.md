# Decision Ticket Option Bias

Status: implemented in 0.21.0. It closes downstream gap G-84 (a bias that
differs from ticket to ticket needs a controller per bias value). It changes
`canwu-decision` (the option, the utility evaluator, the archive format, the
decision commitment), `canwu-sim` (the commitment format), and `canwu-api`
(a contract test; every type was already re-exported). Same version only: no
compatibility work with 0.20.0 bytes or hashes.

## Problem

0.20.0 (downstream gap G-70) gave the utility stage a bias:
`WeightedUtilityEvaluator::with_option_bias(option_id, bias)` keeps
`option_biases: BTreeMap<String, i64>` on the evaluator, and
`GuardedUtilityPolicy` hashes it into the policy's `semantic_hash`. The bias is
therefore part of the policy and of the identity its controller binding
names. `DecisionOption` carried `utility_inputs` and no bias.

A bias whose value is decided per ticket (an acceptance bias computed from the
facts of one request, a per-ticket weight hint) cannot ride the decision
maker's one controller: each distinct value is a distinct policy, so a host
registers one controller per value (41 for a bias clamped to ±20) for each
decision maker and class, and routes each ticket to the controller of its
value. Folding the value into a factor does not help: a factor is weighted by
each policy, so the drafter's value would be scaled, or dropped by a policy
that has no weight for it, and the trace would show it as a factor line.

## Invariant

- A policy still selects only an existing option. A bias changes a score,
  never the option set, the authority envelope, or the controller.
- A score is `Σ weight × utility_inputs[factor] + bias`, where `bias` is the
  policy's own bias for the option plus the option's per-ticket bias, both
  unweighted; the evaluation records that `bias` apart from the factor lines.
- The policy's bias is policy configuration (in its semantic hash); the
  option's bias is ticket content (in the ticket, its versions, the decision
  ingress, and the decision commitment, never in the policy's identity).

## Design

`DecisionOption` gains `bias: i64`, written by the ticket's drafter at `Open`
(`DecisionTicketDraft::options`) or `ReplaceOptions`, which advances the
ticket version as for any option change, so a response computed before a
re-bias is stale. It is zero and omitted from the encoding when the option has
none, as `DecisionOptionEvaluation::bias` is; the serde default exists for
that canonical omission, not to read older bytes. A builder,
`DecisionOption::with_bias(bias)`, sets it. Any `i64` is admitted; a blocked
option keeps its bias and is not scored.

`WeightedUtilityEvaluator::evaluate` adds the option's bias to its own bias for
the option (checked `i64`; an overflow is a policy error, as a factor's is),
adds the sum to the weighted factor sum, and records the sum in
`DecisionOptionEvaluation::bias`. `WeightedUtilityPolicy` and
`GuardedUtilityPolicy` score through the evaluator, so both read ticket
biases; in the guarded policy they apply before the near-equivalence margin,
as the policy's biases do. Blocked and guard-excluded options are not scored
and record no bias. The guarded policy's semantic hash is unchanged: it still
covers only the policy's own biases, so one binding serves every ticket of a
decision maker whatever the tickets' biases. The trace keeps one bias line:
the policy's part is the recorded bias less the bias of the option on the
ticket version the trace names.

Shapes not taken:

- Two fields on the evaluation (policy part and ticket part). The row asked
  for the sum on the existing field, which keeps `score == Σ contributions +
  bias` and the evaluation's shape; the ticket keeps the ticket part.
- Replacing the evaluator's map with the option field. 0.20.0 kept the bias on
  the policy so that two controllers with different doctrines read one ticket
  differently; that still holds, and the ticket bias is added beside it.
- A bias on `ExternalDecisionOption`. External and LLM requests carry no
  utility inputs either; a bias is a utility-stage input.

Cost: one `i64` per option, encoded only when nonzero, and one checked add per
scored option. No index, no new state.

## Persistence and replay

| Item | 0.20.0 | 0.21.0 | What changed |
| --- | --- | --- | --- |
| `DecisionOption` (in `DecisionTicketDraft`, `ReplaceOptions`, `DecisionTicket`: decision ingress, replay journal, snapshot and checkpoint decision state, continuation checkpoint, hot history, archive blobs) | — | — | `bias: i64`, omitted when zero; strict loading refuses an explicit `"bias": 0` as a non-canonical encoding |
| `DecisionOptionEvaluation::bias` (in `PolicyDecision` and `DecisionTrace`) | the evaluator's bias | the evaluator's bias plus the option's | same field and encoding, omitted when the sum is zero |
| Decision commitment | `canwu.commitment.decisions.v3` | `canwu.commitment.decisions.v4` | same inputs (controllers, hot-history root, archive receipt root, admission batch); the new tag marks that the committed tickets carry option biases |
| `COMMITMENT_FORMAT_VERSION` | 7 | 8 | commits to decision state through `canwu.commitment.decisions.v4`; a format 7 snapshot, checkpoint, or replay journal is refused by its format as well as its engine version |
| `DECISION_ARCHIVE_FORMAT_VERSION` (archive blob, receipt, verified archive commit) | 1 | 2 | an archived ticket's options carry their bias; a blob records no engine version, so its format alone names its shape, and a format 1 blob or receipt is refused |
| `GuardedUtilityPolicy` semantic hash | — | — | unchanged; ticket biases are not part of it |

Unchanged, on purpose: the hot-history leaf tag (`canwu.decision.hot-history-leaf.v2`)
and the decision ingress commitment (`canwu.decision.ingress-request.v1`)
hash the canonical encoding of what they cover, so a biased ticket or draft
already hashes apart from an unbiased one, and the records that carry them
are refused by commitment format 7 and the engine version; the decision
archive bucket page format (1) carries locators, not records.

Exact replay re-admits the drafts and replacements with their biases and
rebuilds the same tickets; snapshot validation reconstructs decision state from
the admitted decision ingress, so a snapshot whose ticket bias was edited is
refused (`InvalidSnapshot`, "decision ingress history does not reconstruct the
persisted decision state"). A host that evaluates a policy again after a
restore scores the same, because the bias is in the ticket it reads.

## Refusals

None is new. `Open` and `ReplaceOptions` admit any bias. An evaluator whose
policy and ticket biases, or whose score, overflow `i64` returns
`DecisionErrorCode::InvalidDecision` to its caller, as an overflowing factor
does: `prepare_decision` and `drive_decision` return it to the host before
any ingress is queued.

## What can still fail a boundary

Nothing new. A boundary system that evaluates a utility policy itself and
propagates the policy error fails its own boundary, as for a factor overflow
today; the decision paths of `canwu-sim` fail a boundary only where they
already did.

## Public API delta

- `canwu-decision`: `DecisionOption::bias` (struct literals that list every
  field must set it; those built with `..DecisionOption::new(..)` need
  nothing) and `DecisionOption::with_bias`; `DECISION_ARCHIVE_FORMAT_VERSION`
  is 2; the rustdoc of `WeightedUtilityEvaluator`, `with_option_bias`,
  `GuardedUtilityPolicy`, and `DecisionOptionEvaluation::bias` states the sum.
- `canwu-sim`: `COMMITMENT_FORMAT_VERSION` is 8.
- `canwu-api`: no new item; `DecisionOption` and both constants were already
  re-exported. No `&mut self` method is added, so `gated!` is unchanged.

## Durable tests

- `ticket_option_bias_joins_the_policy_bias_under_one_policy_identity`
  (`canwu-decision`, `tests/policy_contracts.rs`): one guarded policy with a
  standing bias decides two tickets differently by their option biases; the
  evaluation records the policy's plus the ticket's bias beside unchanged
  factor lines and `score == Σ contributions + bias`; a ticket bias that
  cancels the policy's records zero; a blocked option's bias is not applied;
  an overflowing sum is a policy error; a zero bias is not encoded.
- `gap_g84_ticket_option_bias_shares_one_controller` (`canwu-api`): one
  registered controller answers three tickets through `drive_decision` with
  one policy, biased at `Open`, unbiased, and re-biased by `ReplaceOptions`,
  and each trace records the summed bias; a strict reload gives back the same
  snapshot, a snapshot whose ticket bias was edited is refused because the
  admitted drafts no longer reconstruct it, and exact replay matches.

Checked inline (not committed) to fail: an evaluator that ignores the option's
bias fails both tests.

## Known limits

- A trace records one bias per option. The ticket part is the bias of the
  option on the ticket version the trace names: always the ticket's current
  options for a resolving trace (`Resolve` does not change options), but not
  for a deferring trace after a later `ReplaceOptions`, because hot history
  keeps only a ticket's latest version.
- Only the shipped utility evaluator reads the bias. Rule, human, external,
  and LLM policies ignore it, `ExternalDecisionRequest` does not carry it,
  and a host `UtilityEvaluator` reads `option.bias` only if it chooses to.
  A policy built on `WeightedUtilityEvaluator` cannot opt out of ticket
  biases; one that must ignore them implements its own evaluator.
- The bias is unbounded; a host that wants a clamp applies it when it
  drafts the option.

## Upgrading from 0.20.0

A decision host that registered a controller per bias value (a per-ticket
value folded into `with_option_bias`) registers one controller per decision
maker and class with the policy's standing biases only, and writes each
ticket's value on its option with `DecisionOption::with_bias` when it drafts
the ticket or replaces its options; the trace's `bias` is then the sum of
the two. A host whose tickets carry no bias scores exactly as in 0.20.0.
Struct literals of `DecisionOption` that list every field add `bias: 0`.
Saves are refused by engine version as for any release, and also by
commitment format 7; decision archive blobs written by 0.20.0 (format 1) are
refused, so keep them with the 0.20.0 engine.

## Documentation delta

- `docs/architecture.md`, "Decision framework": done in this change (the bias
  has two parts, the ticket's outside the policy hash) and commitment format
  8 in the persistence paragraph.
- `docs/end-state.md`, "Decisions and controllers": done in this change
  (per-ticket option biases are domain data; the evaluator records an
  option's bias apart from its factors).
- For the release pass: `docs/versioning.md` (the 0.21.0 entry, its format
  table, and the current contract's commitment format 8);
  `website/src/content/docs/architecture/decisions.mdx` and
  `website/src/content/docs/en/architecture/decisions.mdx` (the
  `GuardedUtilityPolicy` row: the score adds the policy's and the ticket's
  option bias); both `developer/persistence.mdx` pages (commitment format 8);
  `docs/terminology.md` and both website term pages (below); the engine-user
  skill's decision note (`canwu-engine-usage`, the `with_option_bias` line:
  a per-ticket value goes on the option with `with_bias`, not into a
  controller per value) and the documentation map row.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| option bias (widened) | `WeightedUtilityEvaluator::with_option_bias`, `DecisionOption::bias`, `DecisionOptionEvaluation::bias` | 选项偏置 | An integer preference for one option, added unweighted to its weighted score and recorded apart from the factor lines: the policy's standing bias plus the ticket's per-option bias. |
| ticket option bias | `DecisionOption::bias` | 票据选项偏置 | The bias a ticket's drafter writes on one option: ticket content outside the policy's identity, added to the policy's bias for that option, so tickets whose biases differ share one controller. |
