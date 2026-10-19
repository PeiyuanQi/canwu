# Resource Transit, Account Place, and External Inflow

Status: implemented in 0.19.0. It closes downstream gaps G-63, G-65, G-67, and
G-68 and the engine finding E-45 in `canwu-resource`, and documents the ways
around downstream gap G-66, for which the owner declined an engine change. A
transfer can lose part of its escrow and stay open, a transfer carried by a
multi-leg execution records every leg and handoff in order and keeps them into
the archive, the host application can move an account to another place, and a
custodian can book value from outside the modelled accounts, only under a
lease the host application bound to exactly that credit.

Two independent reviews of the first implementation found that a partial
loss followed by acceptance failed the boundary when the transfer's demand had
already ended, that an allocation pass allocated escrow in flight a second
time, and that a lease did not bind the operation it was issued for, so a
custodian could mint any quantity under a lease meant for something else.
Delta reviews of those fixes found that a holder could still write a binding
into its own four-step lease, that a refused front-run still took the
provider's operation key, that an ended demand's records could be archived
while its transfer was in flight, and that returning escrow held back its
demand. This text describes the implementation with all of these fixes.

It builds on [single-step completion lease](resource-single-step-lease-and-floor-install.md)
(0.15.0), whose host-issued single step is the lease of every new irreversible
command here, on [resource refusals without stalls](resource-refusals-without-stalls.md)
(0.16.0), whose rule it keeps (an expected refusal is a recorded rejection or a
skip, never a failed boundary), and on [resource series scale](resource-series-scale.md)
(0.18.0), whose live buckets and attachment rules it follows.

## Problem and evidence

Each gap was verified in the published 0.18.0 source.

1. **A transfer could not lose part of its escrow (downstream gap G-65).**
   `CompleteTransfer { Lose }` settled on the whole remaining escrow
   (`complete_transfer` set `quantity = snapshot.escrow`, and the `Lose` arm
   set `lost = quantity`), and it ended the transfer. A cargo that loses some
   of its load on the way had to be booked as several transfers. Acceptance
   also credited the demand with the allocation leg's whole quantity
   (`settle_transfer_acceptance` called `record_fulfillment(&leg, ..)`, which
   added `leg.quantity`), whatever reached the destination.
2. **A multi-leg execution's intermediate legs left no trace on the transfer
   (downstream gap G-63).** `advance_transfer` admitted only
   `PendingDispatch → InTransit`, `InTransit → ArrivalPending`, and
   `→ ReturnPending`; an `InTransit → InTransit` advance was
   `invalid_lifecycle`. Each advance replaced `transfer.transport`, its
   evidence was pushed into `exact_evidence` and sorted, losing the order,
   and completion overwrote `exact_evidence` with the terminal evidence, so an
   archived transfer named only its last link.
3. **An account's place scope could not change (downstream gap G-67).** The
   scope was set when the scenario installed the account and was documented
   as immutable; a tracked `CreateAccount` with a scope was refused. After an
   application carried a store's contents elsewhere, the account kept its old
   scope, and `AcceptLocal` refused a same-place transfer at the new place.
4. **Value from outside the modelled accounts could not be credited
   (downstream gap G-68).** `Credit { ExternalInflow }` was refused as a
   tracked command (`validate_resource_authority`), and on adapter ingress it
   needed a provider participant grant (`validate_local_provider_participant`).
   There was no counterpart to the custodian's `ExternalOutflow`.
5. **An inflow credit kept no evidence (engine finding E-45).** The
   `ExternalInflow` arm of `credit()` returned no exact evidence, unlike a
   production credit and an outflow, against the rule that an outcome keeps
   the evidence it settled on.
6. **Two local acceptances that share a handover record cannot settle in one
   boundary (downstream gap G-66).** The terminal certificate of the first
   locks the record's exact version from its activation to its completion.
   The 2026-10-19 source read showed that a host can settle several boundaries
   at one simulated instant, so the second leg can settle at the same instant
   one boundary later; the day of delay came from settling one boundary per
   day.

Found in review of the first implementation, each reproduced:

7. **Acceptance after a partial loss failed the boundary for good when the
   demand had ended.** `record_fulfillment` always set the demand's status and
   added it to the dirty set. A demand cancelled, expired, or rejected for
   its minimum while its transfer was in flight became `PartiallyFulfilled`
   with no due or expiry entry, the root failed validation, and every later
   boundary failed (demand 30, allocate, `BeginTransfer`, `CancelDemand`,
   `RecordTransferLoss` 10, `AcceptLocal`). Without a partial loss the same
   happened to a demand whose leg was smaller than its remainder.
8. **An allocation pass ignored escrow in flight (0.18.0 behaviour).** A pass
   counted only active reservations against a demand's remainder; a transfer
   start consumes the reservation, so a pass allocated the in-flight quantity
   a second time (30 in flight, a second pass allocated another 30), or
   rejected the demand for its minimum when the stock left could not meet it.
   The owner's G-65 decision is that a loss reopens only what was lost.
9. **A lease did not bind the operation it authorizes.**
   `consume_completion_certificate` checked the operation key and the exact
   targets, not the kind or the quantity, and a tracked inflow `Credit` locks
   the same targets as an `ExternalOutflow` or an account `RecordLoss` with a
   record cause. A treasurer redeemed a host's outflow lease for an inflow of
   1,000,000 under its key, and a tracked inflow of 2,000,000 front-ran a
   provider's adapter credit of 25 under a four-step lease in which the
   provider held its own participant grant. Both were applied.

Found in the delta review of those fixes:

10. **A holder could write its own binding.** A holder may send a four-step
    `Acquire` as a tracked command, and its envelope was accepted with an
    `operation_binding`: a treasurer acquired a lease bound to an inflow of
    3,000,000, the host granted, prepared, and activated it with the usual
    targets, and the inflow was applied.
11. **A refused front-run took the provider's key.** A tracked command
    refused at settlement for redeeming another plugin's lease is a recorded
    rejection and takes its operation key, so the provider's own operation
    under that key (an adapter credit, or a tax outflow) could no longer
    settle until the host released the lease.
12. **An ended demand's records left the root before its transfer ended
    (0.18.0 behaviour).** A cancelled or expired demand's closure (demand,
    reservation, consumed leg) became an archive candidate while a transfer
    drawn on the leg was open; once archived, the transfer's acceptance was a
    recorded `invalid_definition` ("transfer lost its allocation leg").
13. **Returning escrow held back its demand.** The first fix counted a
    `ReturnPending` transfer's escrow as in flight, though it can never be
    accepted.

Found in the final check:

14. **A holder could fail the boundary through an amendment.** A demand of
    150 on two of the requester's accounts got legs of 100 and 50; the 100
    left in a transfer; an `AmendDemand` lowered `requested` to 100, which
    0.18.0 allowed (it compared with `fulfilled` only); the acceptance made
    the demand `Fulfilled` while the 50 were still reserved, so its closure
    was held back by the reservation, and the leaked-closure validation rule
    of the previous round, which knew only transfers as holds, failed the
    boundary.

## Decision

The owner decided the shape of each fix on 2026-10-19 (recorded in the
progress file, §6); this section records how it is built. Names, error codes,
and the order of checks are the implementer's, listed under
[implementer choices](#implementer-choices) for the owner to confirm.

### Partial transfer loss (downstream gap G-65)

`ResourceOperationRequestV1::RecordTransferLoss(ResourceTransferLossRequestV1)`
takes part of an open transfer's escrow:

```rust
pub struct ResourceTransferLossRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub loss_id: ResourceLossId,
    pub transfer: ResourceTransferId,
    pub expected_transfer_revision: ResourceRevision,
    pub quantity: u64,
    pub cause: EvidenceRef,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}
```

- **Effect.** The transfer keeps its state and the rest of its escrow:
  `escrow -= quantity`, `lost += quantity`, and its revision advances. A
  `ResourceLoss` with `account: None` and `transfer: Some(..)` is recorded and
  counts as admitted loss in `ConservationTotalsV1`. The outcome, of kind
  `ResourceOperationKind::Loss` (shared with `RecordLoss`), carries the lost
  quantity, the escrow left as its remainder, the loss as its result, and the
  cause version as its exact evidence when the cause is a record version.
- **States.** Every state that is not terminal: `PendingDispatch`,
  `InTransit`, `ArrivalPending`, and `ReturnPending`, any number of times. No
  `SplitTransfer`.
- **Quantity.** Above zero and below the escrow. A loss of the whole escrow
  ends the transfer, so it stays `CompleteTransfer` with `Lose`, which is
  terminal and loses whatever partial losses left (`lost += escrow`, where
  0.18.0 set `lost = escrow`). The transfer's quantity invariant
  (`quantity = escrow + accepted + lost + returned + external_outflow`) and
  the engine's conservation check hold at every step.
- **Lease.** The certificate locks the transfer at the expected revision and,
  for a domain-record cause, that record version; it is the single step the
  host application queues, as for every tracked irreversible command. The
  loss completes this lease only. The lease the transfer was started under
  stays until the transfer's terminal disposition completes it.
- **Authority.** A tracked command comes from the transfer's controller, as
  for `Lose`: the source custodian, or the grantee of a transfer started
  under an access grant. Canonical adapter ingress may carry it with the cause
  record as its provider source and the provider's participant grant, the
  path `RecordLoss` already takes.
- **Demand crediting.** Acceptance credits the demand with what is accepted
  (the escrow left), not with the allocation leg. A transfer of 30 that lost
  10 and is accepted records a fulfillment of 20; the demand keeps 10 of
  remainder, stays `PartiallyFulfilled`, and is marked dirty. A transfer lost
  whole, returned, or let out credits its demand nothing, as before. Without
  partial losses the accepted quantity is the allocation, so nothing changes
  for a transfer that loses nothing.
- **What a pass allocates.** An allocation pass counts the escrow of the open
  transfers drawn on a demand as drawn, beside its active reservations: in
  the quantity it still needs, in what remains of its minimum useful
  quantity, and in the guard that rejects it for its minimum. Escrow in
  flight is never allocated again, a pass never rejects a demand for its
  minimum while a transfer for it is in flight, and a loss or a return gives
  back exactly what it takes, which the next pass may supply (the owner's
  "a loss reopens only what was lost"). A returning transfer
  (`ReturnPending`) is not in flight for its demand: it is never accepted, so
  its escrow is needed again as soon as it turns back.
- **A demand that ended in flight.** A demand cancelled, expired, or rejected
  for its minimum while a transfer drawn on it is open keeps its terminal
  status when the transfer is accepted: `fulfilled` counts what was
  delivered, a fulfillment record names it, and the demand rejoins no due,
  expiry, or dirty index. Only an open demand changes status at acceptance.
  Its closure (the demand, its reservations, and its consumed legs) becomes
  an archive candidate only when no open transfer draws on its legs: the
  transfer's terminal disposition (acceptance, loss, return, or outflow)
  marks it then, so the demand and the leg stay in the live root while the
  transfer needs them.
  (With "what a pass allocates", a demand can no longer be rejected for its
  minimum while a transfer for it is in flight; the rule covers it anyway.)
- **Concurrent work.** The revision advances, so an `AdvanceTransfer` or
  `CancelTransfer` that expects the earlier revision is a recorded
  `version_conflict`. A lease that already locks the transfer's current
  revision for a terminal disposition keeps the loss's lease from locking it
  (the single step is refused); the host application releases that lease and
  takes a new one for the revision the loss leaves.

### Intermediate legs and the link history (downstream gap G-63)

`AdvanceTransfer` admits `InTransit → InTransit` when the new
`TransportExecutionLink` names the execution of the current link and either a
new leg, not lower than the current one, or a handoff the transfer has not
recorded. "Not lower" is the only order the engine checks: a link with a leg
compared with one without counts the link without a leg as lower, and any
other order is the transport's.

Each advance appends one `ResourceTransferLinkV1 { progress, transport,
evidence }` to `ResourceTransfer::link_history`, in order. `transport` stays
the current link (the last entry). While the transfer is open the history is
the only copy of its transport evidence: `exact_evidence` stays empty, and
the root's payload-required continuation declares the history's evidence
instead (0.18.0 kept the sorted transport evidence in `exact_evidence`, which
would now hold each version twice). A terminal disposition writes the
evidence it cites to `exact_evidence`, as in 0.18.0, so readers that take the
acceptance evidence from it are unchanged. The history is kept through
completion and into the archive.

The live root and the archive refuse a transfer whose history its advances
could not have recorded: more than 16 entries or 14 in transit, a transition
`advance_transfer` does not admit (intermediate links included), a
`transport` that is not the last link, an open transfer in a state its
history does not leave (other than returning after a cancellation) or with
`exact_evidence`, and an accepted transfer whose history does not end in an
arrival, unless it was accepted locally before any link.

The history holds at most `MAX_RESOURCE_TRANSFER_LINKS` (16) entries, two of
them kept for the arrival and the return: a transfer records its dispatch and
at most thirteen intermediate links (fourteen in-transit entries), an
intermediate advance past that is a recorded `limit_exceeded` rejection, and
an arrival, a return, and a terminal disposition are always admitted, since
each happens at most once. A cancellation moves the transfer to
`ReturnPending` without a link.

### Moving an account to another place (downstream gap G-67)

`ResourceOperationRequestV1::RescopeAccount(ResourceRescopeAccountRequestV1)`
is the host application's operation:

```rust
pub struct ResourceRescopeAccountRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub place_scope: Option<ResourceScopeId>,
    pub evidence: DomainRecordVersionRef,
}
```

- **Ingress.** The host queues it with `enqueue_resource_account_rescope` on
  the internal ingress `RESOURCE_ACCOUNT_RESCOPE_INGRESS`
  (`resource_account_rescope_v1`), whose permit the plugin hands out only
  through that call. A tracked command that carries it is refused at
  admission (`InvalidAuthority`), `resource_adapter_ingress` refuses it, an
  adapter packet that carries it fails its settlement checks, and a packet a
  plugin's boundary system schedules on the rescope ingress settles nothing,
  with a `failed_settlement_checks` skip event. A holder still never sets a
  place scope: a tracked `CreateAccount` with a scope stays refused.
- **Effect.** The account takes the new scope and its revision advances. No
  completion lease is needed. The outcome, of the new kind
  `ResourceOperationKind::RescopeAccount`, keeps `evidence` as its exact
  evidence. Reservations, allocation legs, demands, and access grants that
  name the account carry over unchanged; an operation whose lease locked the
  earlier account revision is a `version_conflict`, as after any account
  change. A new scope equal to the current one is applied like any other.
- **Evidence.** It must be its record's current version when the operation
  settles, read from the current-version index
  (`SimulationView::current_domain_record_version`), never from retained
  evidence, so a sealed run and its exact replay decide alike; and its record
  kind must be one of the adapter evidence kinds the plugin was configured
  with (`ResourcePlugin::new(adapter_evidence_kinds)`). The lifecycle system
  also reads the resource plugin's own records and the production runtime,
  and neither proves a move, so evidence of those kinds is refused before
  the read; evidence of a kind the system does not read is refused by the
  read. `enqueue_resource_account_rescope` checks currency when
  the host queues the operation, through `CanwuRun::current_domain_record_version`.
  A detached `ResourceState::apply_operation` has no view and does not read
  the evidence.
- **Refusals.** Recorded: the account is missing (`not_found`), the expected
  revision is stale (`version_conflict`), the account is the source or
  destination of a transfer that is not terminal (`invalid_lifecycle`), a
  local completion lease or an external participant grant locks the account's
  current revision (`invalid_lifecycle`), the evidence is not current
  (`version_conflict`), or the evidence is not of an adapter evidence kind
  (`invalid_authority`).

### External inflow credit (downstream gap G-68, engine finding E-45)

`Credit` with `ResourceCreditSourceV1::ExternalInflow(DomainRecordVersionRef)`
is admitted as a tracked command from the account's custodian, the mirror of
`ExternalOutflow`: the host application queues the single-step lease
(`enqueue_resource_completion_operation`) that locks the account revision and
the inflow's evidence, bound to exactly that credit (below), and the
custodian sends the command. A production
credit stays refused as a tracked command and on adapter ingress; it settles
only through the production output batch ingress. Adapter ingress keeps its
0.18.0 path for an inflow (the evidence is the provider source, under a lease
in which the provider holds its own participant grant).

The source now carries a record version (it carried an `EvidenceRef`), so
only record evidence is possible, and the applied outcome keeps it as its
exact evidence on every path (E-45), as a production credit keeps its source
and an outflow its authority. Outcome validation requires exactly one piece of
evidence on an applied credit. The inflow counts in
`ConservationTotalsV1::external_inflow`. The engine sets no cap on inflow, and
conservation totals stay global across resources and units (engine finding
E-46 stays open); what bounds a holder's inflow is the binding of the lease
the host issues.

### Lease bindings (found in review)

A lease's issuer can bind it to the operation it authorizes, a holder cannot,
and a tracked command redeems only a lease the resource plugin alone holds
grants of:

```rust
pub struct CompletionOperationBindingV1 {
    pub kind: ResourceOperationKind,
    pub quantity: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<ResourceTransferDispositionKindV1>,
}

pub struct EligibilityEnvelopeV1 {
    // ...the 0.18.0 fields...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_binding: Option<CompletionOperationBindingV1>,
}
```

- **Where it lives.** In the lease's eligibility envelope, set with
  `EligibilityEnvelopeV1::bound_to` (`CompletionOperationBindingV1::new`,
  and `with_disposition` for a `CompleteTransfer` lease). The envelope's
  digest covers it; the acquisition stores the envelope, and the activation
  certificate carries the digest (`eligibility_envelope_digest`, inside the
  certificate's own digest), so a binding cannot change after the lease is
  requested; restore also refuses a lease book whose certificate carries
  another envelope digest than its acquisition. Both lease paths take it
  unchanged: the single step's `lease` and the four-step `Acquire` are
  `RequestCompletionLeaseV1`. An unbound envelope encodes and digests as in
  0.18.0.
- **Who writes it.** The issuer: the host application through the
  completion ingress, or a plugin that coordinates the lease there. A
  holder's tracked `Acquire` whose envelope carries a binding is the
  acquisition's recorded `invalid_authority` rejection, so no lease exists
  for the host to grant.
- **What is checked.** Every redemption (`consume_completion_certificate`)
  asks the binding of the acquisition the root holds whether it admits the
  redeeming operation (`CompletionOperationBindingV1::admits`): the same
  kind; the same disposition, when the binding names one; and the quantity
  the operation moves: the credit, outflow, or loss quantity; the allocation
  leg's quantity for `Consume` and `BeginTransfer` (each leg's own for
  `BeginExchange`); the items' legs together for `ConsumeBatch`; and the
  escrow a `CompleteTransfer` disposition settles. `RecordLoss` and
  `RecordTransferLoss` share the kind `Loss`; their targets differ. A
  redemption the binding does not admit is the operation's recorded
  `invalid_authority` rejection. An absent binding keeps 0.18.0 behaviour,
  except below.
- **Tracked commands, at admission.** A tracked command that redeems a lease
  is refused at admission (`InvalidAuthority`), so it settles nothing and
  takes no operation key, when its lease's expected participants name
  another plugin or another plugin holds a participant grant of it (that
  lease authorizes the other plugin's operation, which reaches the root
  through its adapter ingress; the expected participants catch a command
  sent before the other plugin's grant settles), when the lease's binding
  does not admit it as far as the request shows (kind, disposition, and the
  quantity unless it is a disposition's escrow), and when it is an inflow
  `Credit` whose lease is not bound. The operation the lease was issued for
  then settles under its key. Settlement checks the same again if the lease
  appeared or changed in between: another plugin's lease is a
  `failed_settlement_checks` skip, which takes no key, and a binding that
  does not admit the command is its recorded rejection. A tracked inflow therefore
  needs a lease bound to exactly its kind and quantity. Adapter ingress is
  unchanged: an inflow needs no binding there, and a bound lease is checked
  as everywhere.

### Two acceptances on one handover record (downstream gap G-66)

No engine change. The crate README documents the two ways around it: settle
the second leg in a further boundary at the same simulated instant (take its
lease once the first acceptance has settled, as each holder's lease cooldown
allows), or give each leg its own handover record, since the lock is keyed by
the exact record version.

## Implementer choices

- **A request of its own for the partial loss (for the owner to confirm).**
  The owner's decision says
  `RecordLoss` may take part of a transfer's escrow. `RecordLoss` carries
  `ResourceAccountLossRequestV1`, whose account, account revision, and
  `allow_protected` do not apply to escrow, so the partial loss is the
  sibling variant `RecordTransferLoss(ResourceTransferLossRequestV1)`: same
  operation kind (`Loss`), same `ResourceLoss` record, same adapter path
  (cause equals provider source). The account-loss request keeps its encoding
  and digest. This follows the crate's pattern of `Consume` beside
  `ConsumeBatch` and `BeginTransfer` beside `BeginExchange`.
- **Partial-loss check order:** what the plugin found first (an archived
  loss with its identity, or a tracked lease rule), then loss identity unused
  among hot losses (`idempotency_conflict`),
  transfer present (`not_found`), expected revision current and escrow above
  zero (`version_conflict`), quantity above zero (`invalid_definition`),
  quantity below the escrow (`invalid_lifecycle`), then the certificate
  (`invalid_authority` or `version_conflict` as for every operation).
- **Link history cap:** sixteen entries in all, fourteen in transit, so the
  history has one hard bound and the arrival and return always fit.
  Intermediate-advance check order: same execution, leg not lower, something
  new (`invalid_lifecycle`), then room (`limit_exceeded`). An arrival or a
  return is not checked against the current link, as in 0.18.0.
- **Host-only operation pattern.** The crate exposes host-only work as an
  internal ingress whose permit only a crate helper uses, as for
  `enqueue_resource_allocation` and `enqueue_resource_completion_operation`.
  The rescope follows it with its own ingress and permit, and additionally
  refuses a packet a plugin scheduled (`PacketOrigin`), because the decision
  says host-only, where the completion ingress trusts plugins that declare it
  as a target. The plugin-scheduled refusal is a skip, not a recorded
  rejection, since such a packet has no admitted holder and takes no key.
- **Rescope check order:** account present, revision current, no open
  transfer naming it, no lease on its current revision, then the evidence
  (current, readable kind).
- **Rescope evidence read errors.** A read of the evidence that fails for a
  reason other than an undeclared kind (an engine invariant) skips the packet
  with `failed_settlement_checks`, as the single step does.
- **Allocation and in-flight escrow.** A pass computes the escrow in flight
  per demand once, from the open transfers that are not returning and the
  allocation legs they draw on, both in the live root while their demand is
  open. It counts that
  escrow with the active reservations wherever 0.18.0 counted reservations
  alone. A leg that is not attached belongs to an ended demand, which no
  pass reads.
- **A terminal demand at acceptance** keeps its status and indexes; its
  revision advances and it records the fulfillment, so the archive keeps
  what was delivered.
- **Lease binding names and place.** `CompletionOperationBindingV1` (kind
  and quantity) in `EligibilityEnvelopeV1::operation_binding`, set with
  `bound_to`. The envelope was chosen because its digest already reaches the
  acquisition and the certificate, so no lease, acquisition, or certificate
  struct changes and no lease encoding changes for an unbound lease. Kind and
  quantity only, as reviewed: the targets already pin the account and the
  record. A redemption checks, in order, the operation key, the acquisition,
  the binding, then the targets. An external participant grant (a production
  credit or consumption under another plugin's lease) is not a lease the host
  issues and checks no binding.
- **Tracked lease refusals at admission**, as the delta reviews asked: the
  lease's expected participants, grants, and binding are in the root when
  the command is admitted, so a front-run takes no key. Settlement repeats
  the checks for a lease that appeared or changed in between: another
  plugin's lease is a skip (no key), a binding mismatch a recorded
  rejection. A binding mismatch a tracked command cannot be told at admission
  (a disposition whose escrow differs) is the recorded rejection.
- **A holder's bound `Acquire` is a recorded rejection**, as reviewed,
  rather than refused at admission: it takes only the holder's own key.
- **Binding the disposition.** `disposition` is optional and names a
  `ResourceTransferDispositionKindV1` (`Accept`, `AcceptLocal`, `Lose`,
  `Return`, `ExternalOutflow`, from `ResourceTransferDispositionV1::kind`); a
  `Lose` with a record cause and an `ExternalOutflow` citing the same record
  lock the same targets, so a lease bound to one is not redeemed for the
  other. Without it, any disposition of the bound escrow is admitted.
- **One rule for a held closure.** `demand_closure_held` (an active
  reservation, a reserved leg, or a leg an open transfer draws on) is what
  `mark_terminal_demand_closure` waits on and what validation accepts as
  held, so the two cannot drift. The amendment bound below keeps an ended
  demand from holding a reservation in practice.
- **Amendment bound.** `AmendDemand` refuses a `requested` below
  `fulfilled` plus active reservations plus escrow in flight (returning
  escrow excluded, as for a pass) with a recorded `invalid_lifecycle`,
  before the contract checks.
- **Closure hold.** `mark_terminal_demand_closure` treats a consumed leg an
  open transfer draws on like an active reservation, and every terminal
  disposition marks the closure of its leg's demand after leaving the open
  transfers; acceptance leaves them before its closure, so the order of
  terminal sequences is unchanged where no demand ended in flight. Every
  terminal disposition, acceptance included, measures the closure's room on
  its whole result, as a consumption batch does: a closure that does not
  fit is a no-room skip, which takes no key (0.18.0 refused an acceptance
  whose closure did not fit as a recorded `limit_exceeded`).
- **Loss identities across the archive.** An account loss, a partial loss,
  and a `Lose` disposition whose loss identity is not hot read the archive
  for it, through the same lookup a lease identity uses, and are a recorded
  `idempotency_conflict` when an archived loss has it, so a later loss never
  shadows an archived one on lookup.
- **Rescope evidence kinds.** The lifecycle system's reads are fixed at
  registration and a boundary system sees no plugin configuration, so the
  settlement refuses the two record kinds the system reads for itself (the
  resource plugin's records and the production runtime) before the read, and
  the read refuses kinds it does not declare; what is left is the declared
  adapter evidence kinds. An application that also declares the production
  runtime as adapter evidence cannot cite it for a move.
- **Inflow source type.** `ExternalInflow` carries a `DomainRecordVersionRef`
  instead of an `EvidenceRef`, which makes "only record evidence" a type
  rule; the one first-party construction (the economy reference's harvest
  credit) already used a record version.
- **Semantic hash.** `RESOURCE_SEMANTIC_HASH` is
  `3a1e04cb2e61f397db3ecb7b13ebaf0f7af0cc87f621121b726cf1e2221a32ac`, the
  BLAKE3 hash in lowercase hex of this UTF-8 string, on one line:
  `canwu-resource plugin semantic hash 0.19.0: partial transfer loss, transfer link history, account rescope, external inflow command, lease operation binding, escrow in flight, demand closure hold`.
  It replaces the values of the earlier rounds, which were never released.

## Persistence and formats

Nothing written by 0.18.0 is read; there is no migration.

| Item | Change |
| --- | --- |
| `RESOURCE_STATE_FORMAT_VERSION` | 5 (was 4). `ResourceTransfer` gains `link_history: Vec<ResourceTransferLinkV1>`, omitted from the canonical encoding while empty. |
| Validation (`validate_within`) | Each transfer's link history must be one its advances could record (at most 16 entries and 14 in transit, each a transition `advance_transfer` admits from the state before it, intermediate links included, `transport` its last link, an open transfer in the state its history leaves or returning after a cancellation, with empty `exact_evidence`, and an accepted transfer accepted after an arrival or locally before any link). An applied `Credit` and `RescopeAccount` outcome must keep exactly one piece of evidence. |
| Open transfer evidence | An open transfer keeps its transport evidence in `link_history` only; `exact_evidence` is empty until its terminal disposition. The payload-required continuation declares the history's evidence. |
| Lease envelopes | `EligibilityEnvelopeV1` gains `operation_binding`, omitted while `None`, so an unbound envelope, its digest, and every 0.18.0 lease record encode as before; a bound one digests its binding, whose `disposition` is omitted while `None`. Lease book validation (restore, `validate`) also requires each certificate's `eligibility_envelope_digest` to equal its acquisition's envelope digest. |
| Demand closures | An ended demand whose consumed leg an open transfer draws on stays in the live root, with its reservation and leg, until that transfer's terminal disposition makes the closure a hot terminal candidate. Validation refuses an ended demand that is neither an archive candidate nor held back by the same rule the closure waits on (an active reservation, a reserved leg, or an open transfer): a leaked closure. |
| Archive | The blob format stays 1. An archived transfer carries its `link_history`; archive validation (`PreparedResourceArchiveBatchV1::validate`, lookups, `validate_resource_archive_store`) also refuses a terminal transfer whose accepted, lost, returned, and let-out quantities do not add up to its quantity, or whose link history is not valid (the terminal rules above included). A loss of an open transfer is a hot terminal record from the moment it is recorded and may be archived before its transfer. |
| Requests and outcomes | `ResourceOperationRequestV1` gains `record_transfer_loss` and `rescope_account`; `ResourceOperationKind` gains `rescope_account`; `ResourceCreditSourceV1::ExternalInflow` encodes a record version (was an evidence reference). |
| Ingress | New internal ingress `resource_account_rescope_v1`. Tracked commands admit `Credit` with an external inflow and `RecordTransferLoss`; adapter ingress admits `RecordTransferLoss`. |
| `RESOURCE_SEMANTIC_HASH` | Changes (above). |
| Live root, buckets, ledgers | Unchanged shapes. A rescope rewrites the account's live bucket; a partial loss rewrites the live root (the transfer) and adds its records to hot buckets. |
| Kernel formats | Unchanged. |

## What can still fail a boundary

The lists of [resource refusals without stalls](resource-refusals-without-stalls.md#what-can-still-fail-a-boundary),
[resource root write cost](resource-root-write-cost.md#what-can-still-fail-a-boundary),
and [resource series scale](resource-series-scale.md#what-can-still-fail-a-boundary)
hold. The new operations add nothing to them:

- `RecordTransferLoss`: every domain refusal is a recorded rejection; a key
  taken or reserved, a failed tracked-command or adapter check, or no room is
  a skip. What still fails the boundary is what fails every resource
  operation: an archive lookup that fails (a missing, forged, or unreadable
  archive object), a root that does not validate after settlement for a
  reason other than a limit (a tampered root), and arithmetic or encoding
  failure.
- Loss identities: `RecordLoss`, `RecordTransferLoss`, and a `Lose`
  disposition whose identity is not hot now read the archive when the root
  has one. An archived identity is a recorded rejection; a lookup that fails
  fails the boundary, the archive failure class above, as for a lease
  identity.
- Lease bindings and tracked lease refusals: a refusal at admission, a skip
  (another plugin's lease found at settlement), or a recorded rejection,
  never a failure. A held demand closure is marked by the transfer's
  terminal disposition, whose room is measured on its whole result: no room
  is a skip.
- Acceptance of a transfer whose demand has ended: settles (item 7 of the
  problem no longer fails the boundary). An allocation pass reads the open
  transfers and adds no failure.
- `AdvanceTransfer` (intermediate links): every refusal is recorded; the same
  engine failures remain.
- `RescopeAccount`: every domain refusal is recorded, including stale or
  unreadable evidence; a packet that does not decode, a plugin-scheduled
  packet, a key taken or reserved, an evidence read failing for another
  reason, or no room is a skip. Queuing evidence that is not current is
  refused at `enqueue_resource_account_rescope`, before anything is queued.
  The same engine failures remain.
- External-inflow `Credit` as a tracked command: as `ExternalOutflow`. A
  production credit and a credit to another custodian's account are refused
  at admission.

## Behaviour changes for the versioning notes

Beyond the new operations, a run on this release settles differently from
0.18.0 in these ways:

- acceptance credits the transfer's demand with what is accepted, not with
  the allocation leg;
- an allocation pass counts the escrow of open transfers that are not
  returning as drawn: it does not allocate it again or reject the demand for
  its minimum while it is in flight;
- a tracked command is refused at admission when it redeems any lease whose
  expected participants or grants name another plugin (not only an
  inflow), when its lease's binding does not admit it, and, for an external
  inflow `Credit`, when its lease is unbound; another plugin's lease found
  only at settlement is a skip;
- a loss identity that an archived loss used is refused
  (`idempotency_conflict`);
- an ended demand's closure is held until no open transfer draws on its
  legs, so the demand and its leg leave the root after the transfer ends;
- a terminal transfer disposition whose demand closure does not fit the
  archive candidates is a no-room skip, not a recorded `limit_exceeded`;
- a demand that ends while its transfer is in flight keeps its status when
  the transfer is accepted;
- `Lose` adds the escrow left to `lost` (0.18.0 set it);
- an `AmendDemand` that asks for less than the demand's fulfilled, actively
  reserved, and in-flight quantities together is a recorded
  `invalid_lifecycle` rejection (0.18.0 compared `requested` with `fulfilled`
  only, so a later acceptance could fulfil the demand while a reservation was
  still active, or exceed it).

## Public API delta

0.19.0 is a pre-1.0 minor release; same-version crates only.

- **Added:** `ResourceTransferLossRequestV1`,
  `ResourceOperationRequestV1::RecordTransferLoss`,
  `ResourceRescopeAccountRequestV1`, `ResourceOperationRequestV1::RescopeAccount`,
  `ResourceOperationKind::RescopeAccount`, `ResourceTransferLinkV1`,
  `ResourceTransfer::link_history`, `MAX_RESOURCE_TRANSFER_LINKS`,
  `RESOURCE_ACCOUNT_RESCOPE_INGRESS`, `enqueue_resource_account_rescope`,
  `CompletionOperationBindingV1` (with `new`, `with_disposition`, and
  `admits`), `ResourceTransferDispositionKindV1`,
  `ResourceTransferDispositionV1::kind`,
  `EligibilityEnvelopeV1::operation_binding` and
  `EligibilityEnvelopeV1::bound_to`,
  `ResourceOperationRequestV1::completion_certificates` and
  `ResourceOperationRequestV1::loss_identity`.
- **Changed (breaking):** `ResourceCreditSourceV1::ExternalInflow` holds a
  `DomainRecordVersionRef` (was `EvidenceRef`); new variants of the exhaustive
  enums `ResourceOperationRequestV1` and `ResourceOperationKind`;
  `ResourceTransfer` and `EligibilityEnvelopeV1` gain a field (struct
  literals break; no first-party crate builds an envelope literally);
  `RESOURCE_STATE_FORMAT_VERSION` is 5; an applied credit outcome must keep
  exactly one piece of evidence.
- **Behavior:** `AdvanceTransfer` admits `InTransit → InTransit` under the
  rules above; acceptance credits the demand with the accepted quantity;
  `Lose` adds the escrow left to `lost`; a tracked external-inflow `Credit`
  from the custodian is admitted; an inflow credit's outcome keeps its
  evidence; a tracked production `Credit` is refused with a message naming
  the output batch ingress; a tracked or adapter `RescopeAccount` is refused;
  an allocation pass counts escrow in flight as drawn; a demand that ended in
  flight keeps its status at acceptance and its records in the live root
  until the transfer ends; a returning transfer's escrow is needed again; a
  bound lease is redeemed only by its operation; a holder's tracked
  `Acquire` writes no binding; a tracked command is refused at admission for
  a lease with another plugin's grant, a binding that does not admit it, or
  an unbound inflow lease; an open
  transfer's `exact_evidence` is empty; a loss identity an archived loss took
  is refused; rescope evidence must be of an adapter evidence kind.
- **Other crates, in this change:** `canwu-economy-reference` builds its
  harvest credit's `ExternalInflow` from the record version directly, and that
  credit's outcome now keeps the harvest record as its evidence. No other
  dependent changes: none builds a `ResourceTransfer`, matches the changed
  enums exhaustively, or credits an inflow. The performance harness
  (`benchmarks/performance-harness`) builds only production credits and was
  not rebuilt here.

## Durable tests

Each was checked to fail with its fix reverted once in the working tree.

`gap_g65_resource_partial_loss.rs`:

- `a_partial_loss_keeps_the_transfer_open_and_its_demand_is_credited_with_what_is_accepted`:
  in a plugin-owned run with an archive, the source custodian loses 10 of a
  transfer of 30 by tracked command under a host single step; the transfer
  stays `PendingDispatch` with escrow 20 and lost 10, its revision advances,
  admitted loss is 10, the loss's lease leaves the root and the start lease
  stays. A zero quantity, the whole escrow, and a stale revision are recorded
  rejections, a holder who does not control the transfer is refused at
  admission, and no boundary fails. An archive commit takes the loss while the
  transfer is open (the archived loss names it) and a resend is answered from
  the archive. Local acceptance of the 20 left gives the destination 20, the
  demand a fulfillment of 20 and a remainder of 10 (dirty), and the root
  conserves; both leases have finished. A loss of the terminal transfer is a
  recorded `version_conflict`. An account loss that names the archived loss's
  identity is a recorded `idempotency_conflict` and loses nothing. A checked
  restore and an exact replay with the archive store agree. It fails when
  acceptance credits the allocation and without the archive lookup of loss
  identities.
- `a_partial_loss_is_taken_in_every_open_state_and_the_whole_loss_takes_the_rest`:
  in `PendingDispatch`, `InTransit`, `ArrivalPending`, `ReturnPending` by
  advance, and `ReturnPending` by cancellation, a loss of 5 keeps the state,
  and `Lose` then loses 25 more: `lost` is 30, admitted loss 30, the demand is
  credited nothing, and the root validates. It fails when `Lose` sets `lost`
  instead of adding to it.
- `a_demand_that_ends_while_its_transfer_is_in_flight_keeps_its_status_and_counts_the_delivery`
  (review items 7 and 12): in a plugin-owned run, once with a cancellation
  and once with an expiry in a pass while the transfer is in flight, an
  archive commit leaves the ended demand and its leg in the root; a loss of
  10 and local acceptance of 20 settle; the demand stays `Cancelled` or
  `Expired` with 20 fulfilled and 10 of remainder, out of the due, expiry,
  and dirty indexes; its closure is then a candidate and the next commit
  takes it; the root validates and conserves; a later pass allocates
  nothing; a checked restore and an exact replay with the archive store
  agree. A demand rejected for its
  minimum while in flight can no longer arise (next test), so the third case
  is covered by the same rule, not by a run. It fails when an ended demand's
  status or dirty entry is set at acceptance, and without the closure hold.
- `a_demand_cancelled_in_flight_is_archived_once_its_transfer_is_returned`
  (review item 12): a demand cancelled and its transfer called back stay in
  the root through an archive commit; the tracked `Return` restores the
  source and makes the closure a candidate, and the next commit takes it. It
  fails when a disposition other than acceptance does not mark the closure.
- `a_root_with_an_ended_demand_neither_archived_nor_held_is_refused`: an
  ended demand that is not an archive candidate and that nothing holds back
  fails validation. It fails without the leaked-closure rule.
- `an_amendment_asks_for_no_less_than_is_fulfilled_reserved_and_in_flight`
  (final check, item 14): a demand of 150 gets legs of 100 and 50 on two
  accounts and the 100 leave in a transfer; amendments to 100 and 149 are
  recorded `invalid_lifecycle` rejections, one to 160 settles; the 100 are
  accepted, the demand is cancelled, and an archive commit takes its
  closure, with no failed boundary. It fails without the amendment bound.
- `a_returning_transfer_does_not_hold_back_its_demand` (review item 13): a
  pass allocates nothing while the transfer is in flight and 30 again once
  it is cancelled. It fails when returning escrow counts as in flight.
- `a_lease_bound_to_one_escrow_or_disposition_is_not_redeemed_for_another`:
  after a loss of 10, a local acceptance under a lease bound to an escrow of
  30 is a recorded `invalid_authority` (only the settlement knows the
  escrow); a tracked `Return` under a lease bound to a `Lose` of 20 is
  refused at admission, and the `Lose` then settles under the lease's key. It
  fails without the redemption's binding check and without the disposition
  check.
- `an_allocation_pass_counts_what_is_in_flight_and_a_loss_reopens_only_what_was_lost`
  (review item 8): a demand of 60, useful only whole, has its 60 in flight
  with 40 left at the source; a pass allocates nothing and leaves the demand
  `Open` (0.18.0 rejected it for its minimum); after a loss of 10 a pass
  allocates exactly 10; after acceptance of the other 50 the demand holds 50
  fulfilled and 10 reserved and a pass allocates nothing. It fails when a
  pass does not count escrow in flight.

`gap_g63_resource_transit_history.rs`,
`a_transfer_records_its_legs_and_handoffs_in_order_and_archives_them`: a
provider advances a transfer through its dispatch, a second leg, and a
handoff within that leg; nothing new, a lower leg, another execution, and a
link without a leg after one with a leg are recorded `invalid_lifecycle`
rejections that change nothing. Eleven more legs fill the fourteen in-transit
links, the next is a recorded `limit_exceeded` rejection, and the arrival and
the return are admitted: sixteen entries with their progress, legs, and cited
evidence in order. A tracked `Return` completes the transfer at the cap and
keeps the history. Forged archived transfers (two legs out of order, the
current link dropped, a seventeenth link, a quantity its dispositions do not
account for, a transfer accepted after a return, and one accepted in
transit) are refused by the archive batch's validation. The archived transfer
names all sixteen links, and a checked restore and an exact replay agree. It
fails without the intermediate advance, without the cap, without the
lower-leg check, without the archive's history and quantity checks, and
without the accepted-after-arrival rule.

`gap_g63_resource_transit_history.rs`,
`a_root_whose_open_transfer_has_a_history_its_advances_could_not_record_is_refused`:
a root with a transfer in transit on its third link keeps its transport
evidence in the history only, its payload-required continuation declares a
boundary-established version from the history, and it restores from its
records and starts a run. With its live-root transfer forged (a lower leg, a
seventeenth link, a `transport` that is not the last link, in transit without
a link, transport evidence also kept in `exact_evidence`) it is refused both
by `ResourceState::from_records` and when a run starts from it. A snapshot
cannot carry such a root: the run's boundary hash chain and scenario
manifest hash refuse a forged snapshot before the resource plugin reads it,
which the test found first. It fails without the live-root history check,
without the empty-evidence rule, and when the continuation does not read the
history.

`gap_g67_resource_rescope_account.rs`:

- `the_host_moves_a_store_and_its_stock_is_accepted_locally_at_the_new_place`:
  a tracked rescope is refused at admission, `resource_adapter_ingress`
  refuses one, a rescope another plugin schedules settles nothing with a
  `failed_settlement_checks` skip event, and evidence that is not current is
  refused when queued; a stale revision (`version_conflict`) and evidence of
  a kind the plugin does not read (`invalid_authority`) are recorded. The
  host moves the granary from the depot to the frontier citing the carriage
  record: the outcome keeps it, the revision advances, and the reservation,
  leg, and demand carry over, so a transfer drawn on them to a frontier
  account is accepted locally. While that transfer is open neither account
  moves, and a lease on the granary's current revision keeps it in place until
  the lease's outflow completes; then it moves back. A checked restore and an
  exact replay agree. Evidence of the resource plugin's own runtime record,
  current when queued, is a recorded `invalid_authority`. It fails without
  the plugin-origin check, the open transfer check, the lease check, or the
  own-record refusal.
- `a_rescope_whose_evidence_was_superseded_is_refused_alike_sealed_and_unsealed`:
  the host cites the carriage record's version a boundary established while
  it is current; the record moves on before the rescope settles, and a sealed
  fork seals before each settlement. The fork and the unsealed run settle
  equal boundaries, keep equal checkpoint hashes, and both record the rescope
  as `version_conflict`. It fails when the check reads evidence existence.
- `an_account_another_plugin_locked_for_its_lease_is_not_rescoped_until_released`
  (detached): a coordinating plugin's prepared external participant grant
  locks the granary's current revision and no resource lease does; a rescope
  is a recorded `invalid_lifecycle` until the coordinator releases the grant,
  then it settles. It fails without the external participant lock check.

`gap_g68_resource_external_inflow.rs`:

- `a_custodian_books_an_external_inflow_and_its_outcome_keeps_the_evidence`:
under a host single step bound to an inflow of 40, the treasurer books 40 by
tracked command; the
outcome keeps the remittance version, the balance and
`conservation.external_inflow` follow, and the lease leaves the root. A
production credit and an inflow to another custodian's account are refused at
admission, as is a zero inflow the bound lease does not admit, and a resend
changes nothing. A provider's adapter inflow under a lease with its own
participant grant keeps its evidence too, `validate_resource_runtime` finds
it, and a restored run and an exact replay agree. It fails when the inflow
outcome keeps no evidence and when a tracked inflow is refused.
- `a_tracked_inflow_redeems_only_a_lease_bound_to_exactly_that_credit`
  (review items 9 to 11 and the second delta review): the treasurer's
  tracked inflow of 1,000,000 under
  an unbound outflow lease, of 1,000,000 under a lease bound to an outflow of
  5, of 2,000,000 under a four-step lease in which the provider holds its own
  participant grant, and of 41 under a lease bound to an inflow of 40 are
  each refused at admission, settle nothing, and leave the key free: the
  outflows, the provider's adapter credit of 25, and the inflow of 40 then
  settle under those keys. The treasurer's own tracked `Acquire` bound to an
  inflow of 3,000,000 is a recorded `invalid_authority` and acquires
  nothing. A front-run of 2,000,000 under a provider's lease sent once its
  acquisition settled, before any grant, is refused at admission; one sent
  with the acquisition is admitted and then skipped with
  `failed_settlement_checks`; in both cases the provider's credit of 25
  settles under the key. The root conserves, and a restored run and an exact
  replay agree. It fails without the holder-binding refusal, without the
  admission refusal, without the expected-participant check, and without
  the settlement skip.
- `a_root_whose_certificate_carries_another_envelope_than_its_lease_is_refused`:
  a detached root with an active bound single step restores from its
  records; with its acquisition's envelope rewritten to a valid one bound to
  1,000,000, `ResourceState::from_records` refuses it. It fails without the
  lease book's envelope cross-check.

The existing suites of `canwu-resource` and its dependents run unchanged; the
debug-build whole-root equivalence check runs in every new test.

## Documentation delta

The integrator applies these with the release; the crate README and rustdoc
are already updated.

- `docs/architecture.md`, the `AcceptLocal` paragraph (about line 1158):
  replace "The scope is host-declared when the scenario installs an account,
  immutable, and defaults to `None`; a tracked `CreateAccount` that sets it
  fails with `InvalidAuthority`." with "The scope is host-declared when the
  scenario installs an account, is changed only by the host application's
  `RescopeAccount` (`enqueue_resource_account_rescope`, citing the current
  version of the record of the move; refused while a transfer that is not
  terminal names the account or a lease locks its current revision), and
  defaults to `None`; a tracked `CreateAccount` that sets it, or a tracked
  `RescopeAccount`, fails with `InvalidAuthority`." After the `RecordLoss`
  paragraph (about line 1133) add: "`ResourceOperationRequestV1::RecordTransferLoss(ResourceTransferLossRequestV1)`
  takes part of an open transfer's escrow: the transfer keeps its state and
  the rest of its escrow, `lost` grows by the quantity, the revision advances,
  and a `ResourceLoss` naming the transfer counts as admitted loss. The
  quantity is above zero and below the escrow; a whole loss stays
  `CompleteTransfer` with `Lose`, which loses what is left. It comes from the
  transfer's controller, or through adapter ingress with the cause as provider
  source, under its own lease; acceptance credits the demand with what is
  accepted, so a loss reopens that much of it. `AdvanceTransfer` also admits
  an intermediate `InTransit` link of the same execution (a new leg not lower
  than the current one, or a new handoff), and each advance appends its link
  and evidence to `ResourceTransfer::link_history`, at most 16 entries (14 in
  transit), kept into the archive. A custodian may book `Credit` with
  `ResourceCreditSourceV1::ExternalInflow`, a record version, by tracked
  command under the host's single step, the counterpart of `ExternalOutflow`;
  every credit outcome keeps its evidence."
- `docs/end-state.md`, about line 415: replace "which the scenario sets at
  installation and the engine never infers." with "which the scenario sets at
  installation, only the host application changes, citing the record of the
  move, and the engine never infers." After that paragraph add: "Since 0.19.0
  a transfer may lose part of its escrow and stay open, a multi-leg carriage
  leaves every leg and handoff on the transfer it carries, in order and into
  the archive, and value from outside the modelled accounts enters as a
  custodian's evidence-backed inflow, the counterpart of an outflow."
- `docs/terminology.md` and both website term pages
  (`website/src/content/docs/reference/terminology.mdx`,
  `website/src/content/docs/en/reference/terminology.mdx`): change the place
  scope row's meaning to "A host-declared place identity for an account's
  stock, set when the scenario installs the account and changed only by the
  host application (`RescopeAccount`). It is never inferred; without it, local
  acceptance is unavailable." (中文: "由上层应用在场景安装账户时声明的账户存量地点标识，此后只有上层应用可以更改（`RescopeAccount`）；引擎从不推断它，未声明时不能本地验收。"),
  and add the rows under [terminology candidates](#terminology-candidates).
  The account-level loss row stays.
- `docs/versioning.md`, the 0.19.0 entry: the rows of
  [persistence and formats](#persistence-and-formats) and
  [public API delta](#public-api-delta), and under "Known limits" the list
  below.
- `website/src/content/docs/en/architecture/resources-production.mdx` and
  its Chinese page: in the admitted loss row, "`RecordLoss` on an account, a
  `RecordTransferLoss` on part of a transfer's escrow, or the `Lose`
  disposition of a transfer"; in the tracked command row, add
  `RecordTransferLoss`, external-inflow `Credit`; in the adapter row, add
  `RecordTransferLoss`; add a row "Host application `resource_account_rescope_v1`
  (`enqueue_resource_account_rescope`) | The host application only; a packet
  another plugin schedules settles nothing. | `RescopeAccount`"; in the
  lifecycle step 5, after "Transfer progress and `Accept` arrive as adapter
  operations that cite transport evidence.", add "A transfer carried over
  several legs records each leg and handoff in its link history, and part of
  its escrow may be lost on the way while it stays open."
- `website/src/content/docs/en/developer/production-economy.mdx` and its
  Chinese page: in "When it applies" add "external-inflow `Credit`, and
  `RecordTransferLoss`" to the tracked commands behind a single step; in
  "Local acceptance", replace "The scenario declares `place_scope` when it
  installs an account, and the value stays fixed." with "The scenario
  declares `place_scope` when it installs an account; afterwards only the host
  application changes it, with `enqueue_resource_account_rescope`, citing the
  current version of the record of the move.", and add the two ways around a
  shared handover record (the README's "Both legs of an exchange" bullet);
  after "Account-level loss" add a section "Partial transfer loss" with the
  README's text, and after "Local acceptance" a section "External inflow" with
  the README's "External inflow credit" text.
- `docs/architecture.md`, the completion lease paragraphs: add "A lease's
  eligibility envelope may bind the one operation it authorizes
  (`CompletionOperationBindingV1`: kind and quantity), which every
  redemption checks; a tracked command never redeems a lease in which another
  plugin holds a grant, and a tracked external-inflow `Credit` requires a
  bound lease." In the allocation paragraph, add "Escrow of open transfers
  counts as drawn: it is not allocated again, and a loss or a return gives
  back exactly what it takes."
- `website/src/content/docs/en/developer/production-economy.mdx` and its
  Chinese page, "When it applies": add the README's "Lease bindings" section.
- Release notes: downstream gap G-66 is closed by documentation only.

## Terminology candidates

Following `docs/terminology.md` (转移 for transfer, 运输关联 for transport
link, 路段 for leg, 交接 for handoff, 外部流入 for external inflow, 地点范围
for place scope):

| 中文规范写法 (suggested) | English | Code or API | Meaning |
| --- | --- | --- | --- |
| 转移部分损耗 | partial transfer loss | `ResourceOperationRequestV1::RecordTransferLoss` | A loss of part of an open transfer's escrow; the transfer keeps its state and the rest, and the loss counts as admitted loss. |
| 运输关联历史 | link history | `ResourceTransfer::link_history`, `ResourceTransferLinkV1` | The ordered transport links a transfer recorded as it advanced, each with its evidence, at most 16, kept into the archive. |
| 中间路段 | intermediate leg | `TransferProgressV1::InTransit` from `InTransit` | A later leg or handoff of the same execution recorded while the transfer stays in transit. |
| 账户改换地点 | account rescope | `ResourceOperationRequestV1::RescopeAccount`, `enqueue_resource_account_rescope` | The host application's change of an account's place scope, citing the current record of the move. |
| 外部流入入账 | external inflow credit | `ResourceCreditSourceV1::ExternalInflow` | A custodian's evidence-backed credit of value from outside the modelled accounts, the counterpart of an external outflow; as a tracked command, only under a lease bound to exactly that credit. |
| 租约操作绑定 | lease operation binding | `CompletionOperationBindingV1`, `EligibilityEnvelopeV1::operation_binding` | The one irreversible operation (kind, quantity, and for a terminal transfer disposition optionally which one) a completion lease authorizes, written by the lease's issuer, never a holder, in its eligibility envelope; redeeming the lease for another is refused. |

## Known limits

- **Live-root size of an open transfer.** Every open transfer lives in the
  live root, which each boundary that settles a resource packet re-encodes.
  Measured once on this branch (JSON encoding of `ResourceTransfer`, fixture
  identities of 20 to 30 characters; not a committed benchmark): 506 bytes
  with no link, 895 with one, and 5,219 with the full sixteen, about 286 per
  link. Keeping each link's evidence in `exact_evidence` as well, as the first
  implementation did, would have cost 1,036 and 7,496 bytes. At the canonical
  `max_transfers` of 8,192, all with full histories, open transfers take about
  43 MB of the live root (4.1 MB without links, 61 MB with the evidence kept
  twice), inside `MAX_AUTHORITATIVE_STATE_BYTES` (256 MiB) but re-encoded at
  every settling boundary; longer identities grow it in proportion. Each open
  transfer also declares up to sixteen payload-required versions (those a
  boundary established), up to 131,072 for the root.
- **An allocation pass reads every open transfer** to count escrow in
  flight: once per pass, bounded by `max_transfers`, in memory.
- **Operation-key squatting before a lease exists.** A tracked command that
  names a key no lease in the root carries yet is admitted, and its refusal
  at settlement (the lease is missing) takes the key, as any recorded
  rejection does. Host applications should not use guessable keys for a
  provider's operations, or should take the lease before the key is known.
- **A closure larger than the archive candidates never closes.** A demand's
  closure is `2 × legs + 1` records (each leg and its reservation, and the
  demand), and it becomes archive candidates in one step. A demand with more
  than `(max_archive_candidates − 1) / 2` legs (127 at the canonical 256,
  8,191 at the 16,384 maximum) can never close: the operation that ends it
  (an acceptance, a loss, a return, or an outflow of its last transfer, or a
  consumption in a batch) is a permanent no-room skip, and a single
  consumption, a cancellation, or an expiry a recorded `limit_exceeded`. Nothing bounds a demand's legs below that; each
  allocation pass adds at most one per source account.
- **A refusal at settlement takes its key.** A redemption refused when it
  settles (an adapter packet, a disposition whose escrow differs from its
  binding, or a tracked command whose lease changed after admission) takes
  its operation key, as every recorded rejection does; the lease stays
  active and the host releases it.
- **Binding is the issuer's choice** except for a tracked inflow: an unbound
  lease for any other operation authorizes whatever its targets allow, as in
  0.18.0.
- **A transfer does not list its losses.** Each loss names its transfer and
  `lost` totals them, but nothing indexes a transfer's losses; a reader of an
  archived transfer finds its partial losses by their own records.
- **Partial losses per transfer are not capped.** Each takes its own lease and
  leaves five hot terminal records (the single step's outcome and receipt,
  the loss, its outcome, and the completion receipt); the escrow bounds their
  number only by quantity.
- **The link history keeps fourteen in-transit links.** A longer execution's
  later intermediate links are refused and stay in the movement root only;
  the arrival and the return still record.
- **The engine checks only "not lower" leg order** and does not compare an
  arrival's or a return's link with the current one.
- **A rescope reads its evidence's currency, not its content.** The engine
  does not check that the cited record shows the stock at the new place; that
  stays the host application's. Its record kind must be an adapter evidence
  kind.
- **A detached root does not read rescope evidence** (`apply_operation` has no
  view).
- **No engine cap on inflow**, and conservation totals stay global across
  resources and units (engine finding E-46).
- **Two acceptances on one handover record still cannot settle in one
  boundary** (downstream gap G-66, declined).

## Review record

The code was reviewed by two independent reviewers, one for persistence and
determinism and one for domain semantics and authority, then in four delta
rounds until neither had a blocking finding. Every finding was accepted.

- First round, blocking: accepting a transfer that lost part of its escrow
  failed the boundary for good when its demand had already ended (both
  reviewers); a custodian could mint any quantity under a lease the host
  issued for another operation, or by front-running a provider's lease.
  Fixed by keeping an ended demand's status at acceptance, counting escrow in
  flight in allocation passes, and the lease operation binding with the
  refusal of a lease that holds another plugin's grant.
- First round, should fix and optional: the live-root history check was
  untested (tamper tests through `ResourceState::from_records`); the live
  root grew with duplicated evidence (transport evidence now lives only in
  the link history, measured under known limits); terminal histories were
  not tied to their terminal state; archived loss identities could be
  reused; open evidence was unchecked; rescope evidence admitted the
  plugin's own records; the external-lock rescope check was untested alone.
- Second round, blocking: a holder could write the binding into its own
  four-step `Acquire`. Fixed by refusing a tracked `Acquire` that carries a
  binding. Also: refusals moved to admission so a refused command takes no
  key; an ended demand's closure is held while a transfer draws on it;
  returning escrow no longer counts as in flight; certificate and
  acquisition envelope digests are cross-checked; a binding may name the
  disposition.
- Third round: a timed front-run before the provider's grant settled could
  still take its key (refused at admission by expected participants, and a
  settlement skip otherwise); a closure without room is a skip; a leaked
  closure is refused by validation.
- Fourth round, blocking: that validation rule failed a boundary after an
  amendment lowered a demand below what was still reserved. Fixed by
  refusing such an amendment and deriving the closure's wait and the
  validation rule from one predicate. A final re-check found no remaining
  way to reach an ended demand that is neither a candidate nor held.

Left for the owner to confirm: `RecordTransferLoss` as a sibling of
`RecordLoss` rather than a target of it, and the lease operation binding as
the way a host's lease approves exactly one inflow.
