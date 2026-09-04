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
canwu-resource = { version = "0.10.0", optional = true }

[features]
resource = ["dep:canwu-resource"]
```

Create scenario state with `ResourceState::empty`, install immutable
definitions/units and opening accounts, then use `ResourceState::into_record`
to obtain the one authoritative `DomainRecord` root. Activate
`ResourcePlugin::new(adapter_evidence_kinds)` with the exact external record
kinds allowed to prove consumption, production credit, or outflow.

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
`ResourceCreditRequestV1`, and `ResourceExternalOutflowRequestV1`) require a
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
