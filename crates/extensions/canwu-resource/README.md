# canwu-resource

`canwu-resource` is Canwu's optional, generic owner of conserved physical
resource truth. It provides revision-bound definitions and units, accounts,
demands, reservations, deterministic allocation, transfer escrow,
consumption/loss/fulfillment evidence, immutable operation acknowledgements,
completion-capacity leases, and holder-relative reports.

The crate deliberately does not own recipes, prices, markets, transport reach,
military readiness, or historical capability content. Those domains cite its
exact public versions and submit adapter operations; only `ResourceState`
changes a physical balance.

## Contract

Each account has one authoritative `balance`. Available, reserved, and
protected quantities are derived with `ResourceState::account_quantities`.
Active transfer escrow remains conserved outside account balances. Every
accepted transition preserves:

```text
balances + active transfer escrow
  = opening balances + opening escrow + admitted credits/inflow
  - admitted consumption - admitted loss - external outflow
```

Definitions, units, accounts, demands, allocation legs, transfers,
consumptions, fulfillments, and outcomes carry exact typed identities and
revisions. Implicit unit conversion is forbidden. Every operation has a stable
`ResourceOperationKey`; replaying the same request returns the original
terminal `ResourceOperationOutcome`, while reusing the key with different
content fails.

Allocation is deterministic by descending priority, due time, domain-provided
tie-break key, admitted sequence, and demand ID. Protected floors, minimum
useful quantity, partial-fulfillment policy, expiry, and rejection remainder
remain explicit.

## Integration

Add the extension as an optional dependency when the host feature-gates
resource simulation:

```toml
[dependencies]
canwu-resource = { version = "0.13.0", optional = true }

[features]
resource = ["dep:canwu-resource"]
```

Create scenario state with `ResourceState::empty`, install immutable
definitions/units and opening accounts, then use `ResourceState::into_record`
to obtain the one authoritative `DomainRecord` root. Activate
`ResourcePlugin::new(adapter_evidence_kinds)` with the exact external record
kinds allowed to prove adapter consumption, external-inflow credit, loss, or
outflow. Production output credits do not use adapter ingress; they settle
through the production output batch ingress.

Player and institution decisions use tracked command ingress:

- `resource_command(&ResourceCommandV1)` constructs the plugin command.
- `RESOURCE_COMMAND`, `RESOURCE_COMMAND_INGRESS`,
  `RESOURCE_ADAPTER_INGRESS`, and `RESOURCE_ALLOCATION_INGRESS` are the
  persisted protocol names.
- `resource_adapter_ingress` creates an internal exact-evidence packet.
- `enqueue_resource_adapter_operation` additionally verifies that the cited
  source version is present before enqueueing it.
- `enqueue_resource_allocation` creates the canonical provider packet for one
  exact requester. Its request must carry the current `ResourceState` revision;
  settlement only scans that requester's due/dirty demands, so a caller cannot
  allocate another holder's demand or relabel the allocation owner.

The plugin has one Phase 7 lifecycle writer, Phase 8 conservation/exact-version
validation, bounded Phase 12 summary validation, and Phase 13 holder-relative
knowledge publication.

### Independent consumers

Production, force supply, and other consumers should retain these exact DTOs:

- `ResourceAllocationLegVersionV1` for accepted input allocation;
- `ResourceConsumptionRequestV1` to consume that allocation once;
- `ResourceConsumptionVersionV1` and `ResourceFulfillmentVersionV1` as exact
  consumption/fulfillment evidence;
- `ResourceCreditRequestV1` with
  `ResourceCreditSourceV1::Production(DomainRecordVersionRef)` for output
  credit;
- `ResourceOperationOutcomeVersionV1` as the immutable ACK, including status,
  accepted quantity, remainder, result reference, and semantic digest.

Use `resource_allocation_leg`/`exact_resource_allocation_leg` and
`resource_consumption`/`exact_resource_consumption` to validate input evidence.
Use `resource_operation_outcome`, `resource_operation_outcome_by_id`, and
`exact_resource_operation_outcome` to validate an ACK against live state. Use
`latest_resource_fulfillment` and `exact_resource_fulfillment` for downstream
fulfillment evidence. A rejected operation is still a durable terminal outcome
and never changes conservation totals.

Adapter packets bind `provider_plugin`, an exact
`DomainRecordVersionRef`, and the operation request. The resource plugin checks
the cited record body, owner, configured evidence kind, and request-specific
source field before settlement.

## Account loss, exchange, and local acceptance

- `ResourceOperationRequestV1::RecordLoss(ResourceAccountLossRequestV1)`
  debits one account in place and settles a `ResourceLoss` with
  `account: Some(..)` and `transfer: None`. It counts as admitted loss in
  `ConservationTotalsV1`, never touches reserved stock, respects the protected
  floor unless `allow_protected` is set, requires the exact account revision,
  and uses the `Loss` operation kind. The completion certificate locks the
  account revision and, for a domain-record cause, that record version. A
  tracked command must come from the account custodian; canonical adapter
  ingress may cite the cause record as its provider source.
- `ResourceOperationRequestV1::BeginExchange(ResourceExchangeStartRequestV1)`
  starts two `ResourceTransferStartRequestV1` legs atomically: both transfers
  are created or neither is, and the single `BeginExchange` outcome cites both
  transfer IDs in `cited_transfers`, whether applied or rejected. The parties
  first agree on `ResourceExchangeTermsV1` (exchange key plus each leg's
  transfer ID, exact allocation, and destination);
  `ResourceExchangeTermsV1::leg_operation_keys` derives each leg's operation
  key from the terms digest. Each leg carries its own completion certificate
  for that derived key, held by the leg's source custodian, so each lease
  consents to the whole exchange and cannot be reused for other terms. Leg
  keys become the transfers' operation identities; only the exchange key
  receives an outcome. A tracked command must come from the `leg_a` source
  custodian (or its grantee for a granted leg, which must be certified at the
  exchange's settlement time; see "Delegated access grants"). Terminal
  dispositions of the two transfers stay independent.
- `ResourceTransferDispositionV1::AcceptLocal` settles a transfer without a
  transport execution when the transfer is still `PendingDispatch` with no
  transport link and both accounts declare the same `ResourceAccount::place_scope`.
  The scope is host-declared when an opening account is installed (tracked
  `CreateAccount` commands cannot set it), is immutable, and defaults to
  `None`, which makes local acceptance unavailable. The exact handover record
  is locked by the terminal certificate and kept as evidence. A tracked
  command must come from the destination custodian; canonical adapter ingress
  may cite the handover record as its provider source.

Holder observation heads, reports, and witnesses carry optional
`ResourceLossObservationV1` entries, gated like transfer details. New fields
are omitted from canonical JSON while empty, so existing digests are unchanged.

## Completion capacity

`CompletionLeaseBookV1` and `RunBudgetRevisionV1` are public coordinator
contracts for consumers that must reserve bounded terminal receipts, mutations,
reports, and bytes before a first debit. The acquisition/grant/prepare/activate/
consume/abort lifecycle uses exact revisions and digest-bound eligibility
envelopes. `CompletionLeaseStatusDtoV1` is holder-bound. Guaranteed work sorts
ahead of shared burst work, and the public cap/refill/cooldown constants make
fairness and replay behavior independently testable.

All irreversible requests (`ResourceConsumptionRequestV1`,
`ResourceTransferStartRequestV1`, `ResourceTransferDispositionRequestV1`,
`ResourceCreditRequestV1`, `ResourceExternalOutflowRequestV1`, and
`ResourceAccountLossRequestV1`) require a
non-optional `CompletionLeaseActivationCertificateV1` plus exact locked target
revisions. The persisted lease book verifies that certificate before any debit,
escrow move, loss, outflow, or credit. Use `resource_completion_certificate`,
`resource_completion_grant`, `resource_completion_status`, and
`exact_resource_completion_certificate` for detached exact queries. Canonical
lease transitions enter through `enqueue_resource_completion_operation` and
`RESOURCE_COMPLETION_INGRESS`.

An acquisition persisted in `ResourceState` reserves
`MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE` terminal slots. Admission includes all
active reservations in archive backpressure, while grant/prepare/activate and
already-certified debit or terminal settlement consume those reserved slots.
Consequently a full hot archive rejects the next lifecycle before player cost
but cannot strand an accepted transfer or certified consumption.

## Reports and restore

`ResourceReportGrantV1` is an explicit allowlist. `ResourceReportDtoV1` and
`ResourceObservationWitnessV1` are detached, holder-bound observations; neither
can authorize a balance mutation. Reports do not fall back to trusted-client
ground truth. Each stock observation carries the authoritative
`ResourceScopeId` derived from the account's exact resource-definition revision;
consumers must preserve it and cannot rewrite distant stock as local. The
persisted observation head is the only report source, so materialization never
backdates current balances into an older observation cut.

Terminal resource records are moved through the bounded package-owned archive:
`prepare_resource_archive` selects only hot terminal candidates within the
configured budget, `PreparedResourceArchiveBatchV1::store_and_verify` verifies
content-addressed objects, exact membership/temporal closure, and retention.
`enqueue_resource_archive` additionally requires the batch to equal the exact
current terminal candidates before committing the authenticated directory
through the plugin's permitted internal ingress.
`finalize_resource_archive_retention` acknowledges store-side retention after
restart or stale-source rejection. Archive roots, retention handles, receipts,
and candidate indexes are persisted in `ResourceState`. Maintenance receipts
become bounded terminal candidates and can themselves move cold; ordinary
lifecycle work does not scan cold history.

Use `validate_resource_runtime` after activation and the checked wrappers
`from_resource_snapshot_json`, `from_resource_checkpoint_journal`, and
`replay_resource_from_journal` for restore/replay. They reject forged semantic
digests, conservation failures, revision mismatches, and unavailable exact
evidence.

See `examples/resource_lifecycle.rs` and `examples/completion_lease.rs`.

## Limits

`ResourceLimitsV1::canonical()` provides bounded authoritative collections,
allocation candidate work, query pages, and serialized state size. Completion
lease constants bound recipes, pending acquisitions, reserved slots, tokens,
same-time roots, TTL, and activation guard. Capacity pressure is reported as a
stable error or terminal rejection; it must not partially mutate physical
truth.

## Demand source policy

Since 0.11.0, every `ResourceDemand` persists a `source_policy`:

- `ResourceDemandSourcePolicyV1::Pooled` preserves allocation across all open accounts with matching resource and unit revisions, in deterministic account-ID order.
- `ExactAccounts(Vec<ResourceAccountId>)` lists 1–256 accounts in strictly increasing ID order, with no duplicates. Each must exist, be open, match both exact revisions, and have the demand requester as its custodian. Invalid lists reject the command before resources change; allocation validates them again.
- `Granted { grant_id, accounts }` lists 1–256 accounts the same way, but each must be custodied by the grantor of the named access grant, whose grantee must be the requester (see "Delegated access grants").

The allocator computes available supply, minimum useful quantity and partial fulfillment from the selected accounts only. An insufficient exact list never falls back to the pool. Existing protected floors and later transfer/consumption authority checks still apply. The exact path visits only the listed accounts, bounded by `MAX_DEMAND_SOURCE_ACCOUNTS`; pooled selection retains its existing account scan.

A demand may change its policy before its first allocation, with the expected revision. Once it has any reservation, including a consumed reservation backing in-flight transfer escrow, or any fulfillment, its policy is fixed. Cancel the demand and submit a new one for a new policy; cancellation does not cancel a separate transfer. Reservation history remains indexed until the demand is terminal and its archive closure is eligible; terminal demands cannot be amended. An amendment also cannot change the demand's lifecycle status, rejection reason, requester, fulfilled quantity, or resource and unit revisions; such an amendment settles as a durable `invalid_lifecycle` rejection.

`Pooled` is a selection policy, not permission to spend another custodian's stock. A host application must control who may submit pooled demands and which requester it uses. An `ExactAccounts` list proves no delegation either. Cross-custodian delegation with a cap and a window is expressed only by an access grant and a `Granted` policy; appointments, purposes and who may grant whom remain application/domain responsibilities. Transfer custody and completion-lease checks remain separate requirements.

The policy is included in request digests, runtime snapshots, exact replay and terminal demand archive payloads. Restore validates live source references and retained reservation membership; archive validation preserves the policy and its digest. Holder-relative reports retain their existing visibility contract and do not expose the authoritative source list automatically. Missing `source_policy` fields deserialize as `Pooled` in a standalone DTO, but this does not migrate old snapshots: exact engine-version and plugin-identity checks still apply. Rust struct literals must add the field; this is a 0.11.0 source break.

## Delegated access grants

`ResourceAccessGrantV1` is a grantor custodian's explicit, bounded consent that
a grantee may draw on its stock of one exact resource and unit revision:
`grant_id`, `grantor_custodian`, `grantee`, `resource_revision`,
`unit_revision`, `cap_quantity`, the half-open window `valid_from..valid_until`,
and the exact `authority_evidence` record version that justifies it (for
example an application's accepted requisition record).

- `ResourceOperationRequestV1::IssueAccessGrant(ResourceIssueAccessGrantRequestV1)`
  is admitted only as a tracked command whose subject is the grantor custodian,
  and only when the authority evidence is the exact current version of its
  record. Admission reads the current-version provenance index, not retained
  evidence, so a sealed run admits exactly what its replay admits.
  Adapter ingress cannot issue or revoke grants. The applied outcome keeps the
  authority evidence as its exact evidence.
- `RevokeAccessGrant(ResourceRevokeAccessGrantRequestV1)` also comes from the
  grantor custodian with the expected grant revision. Like a source-policy
  amendment it stops at the first reservation: a grant with reserved or
  debited quantity is fixed and settles a `invalid_lifecycle` rejection.
- A `Granted { grant_id, accounts }` demand is admitted only when the grant is
  active, its grantee is the requester, the demand's resource and unit match the
  grant, `due_at >= valid_from` and `expires_at <= valid_until`, every listed
  account is open and custodied by the grantor, and the requested quantity fits
  the grant's remaining cap.
- Allocation checks the grant before scarcity arbitration: a revoked or
  out-of-window grant supplies nothing (a demand that has drawn nothing is
  rejected with `access_grant_unavailable`), no allocation exceeds the remaining
  cap, and there is no fallback to pooled or other accounts. A listed account
  whose protected-floor policy the grantor changed after admission supplies
  nothing instead of failing the grantee's allocation pass.

Cap accounting is persisted on `ResourceAccessGrantRecordV1`:
`cap_quantity = remaining + reserved_quantity + debited_quantity`. Allocation
moves quantity from remaining to reserved; the debit of that allocation
(consumption or a transfer start, including an exchange leg) moves it from
reserved to debited, so each unit is charged exactly once. Released or expired
reservations return to remaining because the stock never left the grantor's
account; a debit stays charged even if its transfer is later returned.
`ResourceState::validate` rejects any grant whose reserved quantity differs from
the active reservations drawn under it or whose charges exceed its cap.

Completion-lease authority for delegated stock is explicit. The grantor's
consent is the grant; the debit of a granted allocation runs under the
grantee's own completion lease and only while the grant is current. An
activated lease does not expire, so the plugin settles a granted debit only in
a boundary whose time equals the debit's certified time (the request `at`,
which is the lease eligibility time), as adapter ingress already requires of
every irreversible operation, and the grant window must contain that time. A
debit certified inside the window but settled after it, or settled at any other
time than its certified one, is rejected. In a `BeginExchange` the rule applies
per leg: a granted leg must be certified in the same instant the exchange
settles, so a granted party certifying earlier than its counterparty rejects
the whole exchange, while an ordinary leg keeps the plain exchange rules. A
granted allocation pass likewise supplies nothing unless the grant is current
at both its requested time and the settling boundary. A lease
held by anyone else, including the grantor custodian, cannot debit a granted
allocation, and a tracked `BeginTransfer` or `BeginExchange` leg
on a granted allocation must come from the grantee. A transfer started this way
records `access_grant`, and its grantee (not the source custodian) controls its
cancellation, return, and loss; local acceptance still belongs to the
destination custodian.

Demand expiry stays lazy, as for every demand: reservations of a granted
demand past its `expires_at` (and so past the grant window) are released, and
returned to the cap, by the next allocation pass for the grantee, which the host
runs with `enqueue_resource_allocation`. Revocation cannot withdraw unused cap
after the first reservation, so size the cap and window to the delegation.
Validation also requires every grant's debited quantity to cover the hot
transfers and consumptions debited under it, and a grant-backed transfer to
name exactly the grant its (hot) demand drew on.

`resource_access_grant_status` is the holder-bound read of a grant and its
accounting for the grantor or grantee. Grants stay in hot state, bounded by one
global `MAX_RESOURCE_ACCESS_GRANTS`; they are not archived, so a host should
control who may issue grants, as it does for account creation. Grants, granted demands,
and grant-backed transfers persist, hash, restore, and replay with the resource
root; states without grants keep their canonical encoding.

## Production output credit

A production output credit carries the execution's completion certificate. The
resource participant grant of that lease locked the production runtime at the
version current when the lease was granted, before the execution existed, and
every later production transition, including the completion itself, advances
that record. The credit therefore cites the exact production version the
coordinator pinned on the execution when it dispatched the output; the resource
runtime accepts it when it is the locked record at or after the locked version,
and the production output batch ingress additionally requires it to equal the
execution's pinned `output_source`. Production credits settle only through that
batch ingress; `RESOURCE_ADAPTER_INGRESS` and `resource_adapter_ingress` reject
them. The participant grant was consumed at
the execution's start time, and the credit settles it at or after that time.
Completing an execution through a live command, at the start time or later,
settles its output.
