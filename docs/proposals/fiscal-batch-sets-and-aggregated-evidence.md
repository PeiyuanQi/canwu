# Fiscal Batch Sets and Aggregated Evidence

Status: implemented in 0.23.0 (package commit `8d819f7`, from branch
`fix/g96`). It closes downstream gap G-96 (`canwu-fiscal`): the actions of
several offices due at one instant settle in one round, and one execution
across many scopes is one evidence record. It changes `canwu-fiscal`, and in the non-published
reference integration `canwu-ming-fiscal-reference` one host-helper guard and
its README. Both additions are additive: the batch command and per-scope
evidence behave as in 0.22.1. The plugin identity changes (version
`0.6.0-experimental` and a new semantic hash); no fiscal record format
changes. The fixtures are `fiscal_action_batch_sets` and
`fiscal_aggregated_evidence` under `crates/extensions/canwu-fiscal/tests/`.

## Problem

Verified in the 0.22.1 source:

- A `FiscalActionBatch` names one `authority_binding_id`, and the command
  `apply_fiscal_action_v1` carries one batch, so a person who holds several
  offices sends one command per office.
- Where a person issues through a decision (a declared run refuses
  `Issuer::Actor`), each command rides on a decision. Decision ingress
  requires its expected revision to be the run's revision when it is
  enqueued and when it is admitted (`canwu-sim` `runtime/decision.rs`), and
  every applied or rejected command advances that revision, so a second
  decision with a command cannot be enqueued for the same instant until the
  first is admitted. Each office pays its own round: one boundary admits the
  decision and its command, the next settles the command's packet. A holder
  with thirty offices due on one day pays thirty rounds.
- `FiscalExecutionEvidence` names one `scope_binding_id` and `request_id`,
  and a receipt's quantity is its evidence's. An office that collects across
  many scopes in one operation writes one evidence record per scope, so the
  evidence volume grows with the scopes rather than with the operations
  (downstream estimate: about 3,800 records a month for one roster).

## Decision

1. **Batch sets.** `FiscalActionBatchSet { batches }` carries 1 to
   `MAX_FISCAL_BATCH_SET_BATCHES` (64) batches, at most
   `MAX_FISCAL_BATCH_SET_JSON_BYTES` (16 MiB) serialized, each batch within
   the batch limits (4,096 entries, 4 MiB), and at most
   `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (65,536) entries in all, more than one
   boundary could apply. The new command `apply_fiscal_action_set_v1`
   (`APPLY_FISCAL_ACTION_SET_COMMAND`, built by `fiscal_action_set_command`)
   admits every batch exactly as the batch command admits one, under the
   set's one command issuer, sharing the admission's working records and its
   set of named `(target, action ID)` pairs, and queues one existing
   `fiscal_action_v1` packet per batch, in order. The next boundary settles
   them as it settles the packets of several batch commands admitted in one
   boundary. The settlement packet and its settlement are unchanged.
2. **A decision in one's own name.** One command has one issuer, and a
   decision's command subject is one entity, so a decision for an
   institution cannot cover another institution's batch. The set command,
   and only the set command, also admits a validated decision whose origin
   is `DecisionOrigin::Actor { actor }` and whose command subject is
   `EntityRef::Person(actor)`: the actor issuing in his own name. Admission
   resolves it as that actor issuing directly: under an authority binding,
   the principal or acting role he holds (an acting role still needs its
   current authority basis and still cannot rebind or appoint); under an
   audit binding or an appointment policy, that actor as the issuing person.
   It never holds the institution role. A person who is the authorized or
   acting actor of several offices therefore issues one set for all of them.
   The batch command keeps 0.22.1's rule (the subject is the binding's
   institution) unchanged.

   This widens the 0.22.1 contract. In 0.22.1 every fiscal admission of a
   decision required the institution the binding (or audit binding, or
   appointment policy) names as the decision's command subject, so a
   controller whose command subject is a person never touched fiscal state
   through the roles that person holds as an actor. That no longer holds
   for set commands: offered one by a ticket, such a controller holds that
   person's fiscal roles. It holds nothing the person does not hold when he
   issues directly, and the decision kernel's ticket and controller checks
   are unchanged.
3. **Aggregated evidence.** `FiscalExecutionEvidence` gains
   `parts: Vec<FiscalExecutionEvidencePart { scope_binding_id, request_id,
   quantity }>`. Aggregated evidence leaves the top-level scope and request
   empty, lists 1 to `MAX_FISCAL_EVIDENCE_PARTS` (4,096) parts, at most one
   per scope, and its parts' quantities sum to its `quantity`; every other
   field (identity, external operation, unit, payment form, execution kind,
   resource, source, target, disposition) holds for every part. Each scope's
   receipt is an ordinary `FiscalExecutionReceiptPacket` of that scope and
   request citing the one exact version, and settles the part naming its
   scope at the part's quantity. An office's collection day is one evidence
   record and one receipts packet (`enqueue_execution_receipts`, up to 4,096
   receipts) with one receipt per part. Every other receipt rule applies per
   scope as before.

Rejected alternatives: a receipt packet with parts of its own would have
needed a second receipt shape and a second refusal model for one packet that
settles some parts and refuses others; per-part receipts keep each scope's
refusal on its own. Admitting the own-name decision for the batch command
too would have changed the 0.22.1 batch command. Keeping that command
unchanged is this change's own choice, not something the gap row asks: it
confines the widening to the new set command.

## Acceptance

- **One round for several offices.** `fiscal_action_batch_sets` settles two
  offices' batches as one set in one round, and the fiscal state and the
  settled-action events equal those of two successive rounds. It also pins
  the own-name rule to sets: a decision in one's own name is refused for
  one batch, whether an assessment or an audit finding, and admitted for
  both in a set.
- **Per-part receipts and refusals.** `fiscal_aggregated_evidence` receipts
  each part's scope at its own quantity from one record, and records an
  unknown-scope part, a missing part, and an over-quantity part as
  refusals; the boundary commits.
- **Departure from the gap row: the state hash.** The gap row asks that the
  state hash equal the per-scope path's. That cannot hold: each receipt
  cites the one shared record rather than its own scope's, and the citation
  is part of the ledger record. What holds is that the books are equal but
  for each receipt's citations (`fiscal_aggregated_evidence` compares the
  fiscal state with the citations set aside). This departs from the row's
  text, and is named here and in the versioning notes.

## Refusals

A set is one command: anything admission refuses rejects it as a whole as a
recorded command attempt, and nothing is queued. The whole set is refused as
`InvalidPayload` for its size, a batch count outside 1 to 64, more than
65,536 entries in all, or an action ID it names twice for one target across
its batches; for a batch, with `batch N:` before what the batch command
would say (`batch 1: entry 0: fiscal decision subject does not match the
bound institution`).

A receipt citing aggregated evidence is refused, with one
`canwu.fiscal.ingress_rejected.v1` event, never a failed boundary, while the
packet's other receipts settle:

| Case | Reason |
| --- | --- |
| The receipt's scope or request is unknown (a part naming a scope with no such hot request) | `request_unknown` |
| The evidence names no part for the receipt's scope, or the part's request is not the receipt's | `claim_mismatch` |
| The part asks more than its request still authorizes | `quantity_exceeded` |
| The evidence names a top-level scope or request beside its parts, more than 4,096 parts, two parts for one scope, or parts that overflow or do not sum to its quantity (every receipt citing it) | `claim_mismatch` |
| The scope already settled a receipt citing that version | `evidence_reused` |
| The scope already settled a receipt of that `(evidence kind, external_operation_id)` | `operation_reused` |

The other reasons (closed book, evidence count and kind, a version that is
not current, a claim field that differs from the request) are unchanged.

## Invariants

- A set's batches settle exactly as the same batches would as separate
  commands admitted in one boundary: each entry against its target's
  revision when the settling boundary began, within the same per-boundary
  limits, its role re-checked at settlement against the state the earlier
  batches left. Batches whose entries change disjoint records settle to the
  same books and outcomes as the same batches sent one round after another
  at that instant (`fiscal_action_batch_sets` compares the fiscal state and
  the settled-action events).
- A set admits nothing that the same batches sent as separate commands by
  the same issuer would not, except a decision in the issuer's own name,
  which holds only what that person holds when he issues directly.
- An exact evidence version settles at most one receipt in each scope it
  names; per-scope evidence names one scope, so it still settles at most
  one receipt.
- The books one aggregated record leaves are the books per-scope records of
  the same operation leave, but for each receipt's `external_evidence`
  (`fiscal_aggregated_evidence` compares the fiscal state with the citations
  set aside).
- Per-scope evidence serializes byte for byte as in 0.22.1.

## Ownership and affected surfaces

| Surface | Change |
| --- | --- |
| `canwu-fiscal` `model.rs` | `FiscalActionBatchSet`, `FiscalExecutionEvidencePart`, `FiscalExecutionEvidence::parts`, serde defaults for an empty scope and request, three constants. |
| `canwu-fiscal` `plugin.rs` | the set command, its admission, the own-name rule (set only), `fiscal_action_set_command`, the plugin identity. The batch command's admission is the same code path, with the rule off. |
| `canwu-fiscal` `settle.rs` | receipt validation reads the part naming the receipt's scope; aggregated evidence is read and decoded once per boundary. |
| `canwu-fiscal` `lib.rs` | re-exports. |
| `canwu-ming-fiscal-reference` | `enqueue_reference_execution_result` refuses aggregated evidence (the reference adapter and its validator record and check per-scope results); README; struct literals. |
| `benchmarks/performance-harness` | struct literal only; the workload is unchanged. |
| `canwu-api`, `canwu-sim`, `canwu-decision`, `canwu-debug`, examples | none. The decision kernel is unchanged; the own-name rule reads the command context it already passes. |

## API surface

Added to `canwu-fiscal` (all re-exported at the crate root):

- `pub struct FiscalActionBatchSet { pub batches: Vec<FiscalActionBatch> }`
- `pub fn fiscal_action_set_command(set: &FiscalActionBatchSet) -> Result<Command, serde_json::Error>`
- `pub const APPLY_FISCAL_ACTION_SET_COMMAND: &str = "apply_fiscal_action_set_v1"`
- `pub const MAX_FISCAL_BATCH_SET_BATCHES: usize = 64`
- `pub const MAX_FISCAL_BATCH_SET_JSON_BYTES: usize = 16 * 1_024 * 1_024`
- `pub struct FiscalExecutionEvidencePart { pub scope_binding_id: String, pub request_id: String, pub quantity: u64 }`
- `pub parts: Vec<FiscalExecutionEvidencePart>` on `FiscalExecutionEvidence`
- `pub const MAX_FISCAL_EVIDENCE_PARTS: usize = 4_096`

Adding a public field to `FiscalExecutionEvidence`, whose fields are all
public, breaks code that builds it with a struct literal: add
`parts: Vec::new()`. Nothing is removed or renamed, and no `&mut self`
method is added to `Canwu`, so `gated!` is unchanged.

## Cost

- **A set** costs the admission of its batches, sharing one set of working
  records (an office several batches name is decoded once), and settles in
  two boundaries however many offices it covers, where separate decisions
  take two per office. Its entries are capped at one boundary's action limit,
  so admission reads at most what 16 full batch commands read.
- **Aggregated evidence** is one record and one ingress for an operation
  across N scopes instead of N records. Settlement still writes one receipt
  and one ledger revision per scope, so ledger writes, receipts, and the
  per-scope reuse checks are unchanged. Each aggregated version is read and
  decoded once per boundary into a map of its parts by scope, and each
  receipt looks up its part in `O(log parts)`; the decoded claims are held
  until the settlement system returns. Per-scope evidence is read once per
  receipt, as before, and not held. The evidence record grows by one part
  per scope, so the evidence domain's own record limits bound its parts.

## Persistence, hashes, and replay

- **Plugin identity.** `canwu-fiscal` reports version `0.6.0-experimental`
  and semantic hash
  `8b37008533b60bd6b891617475e601a797b3645d4dc2c448b4ffc8bd139c9afe`
  (BLAKE3 of `canwu-fiscal plugin semantic hash 0.6.0-experimental: batch
  sets and aggregated execution evidence`), and its descriptor gains the
  command `apply_fiscal_action_set_v1`. Every run that registers the plugin
  therefore has a different plugin semantic environment, and so a different
  checkpoint hash, from the same inputs under 0.22.1. Same version only: a
  0.22.1 save that registers the fiscal plugin does not load, and none is
  migrated.
- **Formats.** No fiscal record, settlement packet, receipts packet,
  snapshot, journal, or format number changes; `fiscal_action_v1` packets are
  what 0.22.1 queues. `FiscalExecutionEvidence` writes `parts` only when it
  has some and omits an empty scope or request, so per-scope evidence (which
  always names both) serializes byte for byte as before; aggregated evidence
  has no 0.22.1 form.
- **Against the per-scope path.** One aggregated record leaves the books
  per-scope records of the same operation leave, but every receipt's
  `external_evidence` cites the one aggregated version instead of its own
  scope's record, and the evidence domain holds one record instead of N, so
  the ledger records and every state and checkpoint hash over them differ
  from the per-scope path's. This is the named change, and the departure
  from the gap row's equal state hash (see Acceptance); the per-scope path
  itself changes only through the plugin identity.
- **Against successive rounds.** A set leaves the fiscal state of the same
  batches sent in successive rounds at that instant when they change
  disjoint records; the run differs in its command attempts, ingress, and
  revisions, and takes fewer boundaries, so its checkpoint hash differs.
- **Replay.** Admission reads only what the batch command reads, and the
  evidence read is the replay-stable current version, as before, so a
  sealed run, its fork, a restore, and an exact replay settle alike; both
  fixtures check a restore and an exact replay.

## Known limits

- A controller whose authority is an actor and whose command subject is
  that same person, as a host may register for other self-directed
  commands, holds the person's fiscal roles in any set command a ticket
  offers it, even a one-batch set. This is the widening of the 0.22.1
  contract named under the decision; the host decides which tickets offer
  a set command.
- A set covers one issuer. Offices held by different persons take one set
  per person, and where those persons decide through decisions, each
  person's decision still takes its own round.
- The reference integration records per-scope evidence only. Its enqueue
  helper refuses aggregated evidence, but its boundary system does not yet
  skip it: aggregated evidence sent as raw plugin ingress can still fail
  that boundary, as invalid evidence could in 0.22.1. Skipping it inside the
  boundary is left for later.
- Aggregated evidence saves records and ingress, not ledger writes: each
  scope's book still takes its own receipt.

## Rollout

Downstream gap G-96 is implemented in 0.23.0. A host adopts it at the
`=0.23.0` pin: build sets with `fiscal_action_set_command`,
register an own-name decision controller (`DecisionAuthority::Actor` with
the same person as command subject) where a person decides for several
offices, and write aggregated evidence where one operation serves several
scopes. Per-scope evidence and batch commands keep working unchanged.
