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
content fails. A tracked command that does so is refused at admission; a
packet that reaches settlement with such a key, on any ingress, settles
nothing and is named by a skip event (see "Skipped packets"). Keys under
`resource:completion:` are reserved for completion operations.

Allocation is deterministic by descending priority, due time, domain-provided
tie-break key, admitted sequence, and demand ID. Protected floors, minimum
useful quantity, partial-fulfillment policy, expiry, and rejection remainder
remain explicit.

## Integration

Add the extension as an optional dependency when the host feature-gates
resource simulation:

```toml
[dependencies]
canwu-resource = { version = "0.15.0", optional = true }

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
Consequently a full hot archive skips the next lifecycle before player cost
(see "Skipped packets") but keeps the room an accepted transfer or certified
consumption reserved.

### Single-step lease

A lease whose only participant is the resource plugin takes one operation:
`ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1)`
carries the acquisition (`RequestCompletionLeaseV1`), the id of its one grant,
the exact targets, and the boundary, and it acquires, grants, prepares, and
activates together or not at all. "Only participant" means exactly that
`expected_participants` is `{"canwu-resource"}`: the one grant is owned by the
resource plugin and locks every target. The plugin does not derive
participants from the targets, so a resource grant may lock a record version
another plugin owns, on this path as on the four-step path; whether that
plugin should hold a grant of its own is the host application's decision. The
single step is the lease of a tracked `BeginTransfer`, exchange leg,
`CompleteTransfer` (`AcceptLocal`, `Lose`, `Return`), `ExternalOutflow`, or
`RecordLoss`. A lease that needs a grant owned by another plugin (an adapter
operation, whose provider holds its own participant grant) or that another
plugin coordinates (production, force supply) is refused with
`invalid_lifecycle` and keeps the four operations.

The single step applies the checks of the four steps: the holder's run-budget
partition, pending slots, request tokens, cooldown and same-time cap, the full
receipt reservation, the deterministic fairness order, exact current targets,
no conflicting target lock, and the activation guard. Its certificate equals
the one the four steps produce for the same acquisition, grant, and targets
when their activation runs at the same boundary, so the operation it
authorizes is unchanged. It leaves one `CompletionLease` outcome, keyed
`resource:completion:lease:{acquisition}`, and one `Activated` receipt, where
the four steps leave four of each; completion still adds its one receipt. A
same-place transfer therefore leaves 15 terminal records and makes 9 archive
lookups instead of 27 and 15.

A tracked command cannot carry the single step. The host application queues
it with `enqueue_resource_completion_operation`, as it queues the grant,
prepare, and activate steps. A plugin whose boundary system declares the
resource completion ingress among its `plugin_ingress_targets`, as
`canwu-production` does, can schedule it too; the resource plugin trusts such
a packet as it trusts the host application's, which is the trust the
four-step operations already have.

A domain refusal is the operation's recorded rejection and leaves no
acquisition, grant, lock, or spent request token behind. That includes what
the plugin finds against the simulation view, which skips the packet on the
four-step path: an eligibility time that is not the boundary time, an
external record version that is not current, or a record kind the plugin was
not configured to read. A single step never resumes an acquisition or grant
that exists, and the four-step `Acquire` never resumes one either: both are
`idempotency_conflict`. Identities that do not form the operation key of the
step, or of the `Release` that may have to give the lease up, are
`invalid_identifier`. A resend of the same request returns its outcome,
before and after that outcome is archived, and is not a second lease. A
single step whose key belongs to a different request settles nothing, as
described under operation keys below. A refused request has used its
operation key, so a retry names a new acquisition id. An archive commit in
the same boundary settles first, so the commit applies to the root it was
prepared from and the lease is taken on the root the commit left.

Room is measured on the result, as for every resource operation. A refused
single step needs room for its outcome only. An applied one needs `2 + 16`
archive candidate slots (its outcome, its receipt, and the reserved terminal
path) and `1 + 16` hot outcome slots, beside the 16 of each that every active
lease reserves: it fits while `hot terminal records + 16 x active leases + 18
<= max_archive_candidates`, so with the canonical 256 and no backlog the
fifteenth concurrently active lease is the last. One that does not fit is
skipped, not failed: `archive_backlog` when an archive commit would make room,
`reserved_capacity` when the room is held by the reservations of active
leases, which only their completion, release, or expiry frees. An archive
lookup that fails (a missing, forged, or unreadable archive object, never a
miss) still fails the boundary.

What a host application has to watch:

- An active lease does not expire, and a single-step lease is active at once.
  If its operation is never issued, or is rejected, the lease holds its
  target locks, its completion units, and its 16 reserved slots until the
  host application sends `Release`.
- A holder may still send a tracked `Acquire`. While that acquisition waits
  for its first grant, every single step that sorts after it in the fairness
  order is refused with `invalid_lifecycle`, until the acquisition is granted
  or aborted. A host application that uses only single steps watches for
  stray `Requested` acquisitions and removes them: it sends `Abort`, and if
  the holder's own rejected abort has already taken the abort key, so that
  the packet settles nothing, it grants the stray acquisition and releases
  that grant.
- An acquisition id is unique among the acquisitions in the root, not for
  the run. After a lease has left the root with its archived records, the
  other path's first step with the same id creates a new acquisition, because
  the two paths derive different operation keys. A host application that
  needs ids unique for the run keeps them unique itself.

### Operation keys of completion operations

Every completion operation derives its key from the identities it names, as
`resource:completion:{step}:{identities}`. A key under `resource:completion:`
(`RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`) is reserved for them: a tracked
command that names one on any other request is refused at admission,
`resource_adapter_ingress` and `enqueue_resource_allocation` refuse to queue
one, a packet queued around them settles nothing, and
`ResourceState::apply_operation` returns an error without recording an
outcome. A holder can therefore take a completion key only with a completion
operation it may send as a tracked command:

- A holder cannot take the key of a single step (`lease`), nor of a `grant`,
  `prepare`, `activate`, `release`, `expire`, or `participant-*` step.
- A holder can take `acquire:{id}` for any id, by acquiring a lease under it.
  The host application's `Acquire` with that id then settles nothing, and a
  single step with that id is refused with `idempotency_conflict` while the
  holder's acquisition is in the root. The host application uses another id.
- A holder can take `abort:{id}` of a lease it holds, since its own `Abort`
  leaves an outcome under that key whether applied or rejected. It cannot
  take the abort key of another holder's lease or of an id the root does not
  hold: admission resolves the holder of a tracked `Abort` from the
  acquisition in the root and refuses anyone else.

A packet whose key belongs to a different request, in the root or the
archive, settles nothing on any ingress, and its boundary commits. That
includes a tracked command admitted before the request that takes its key had
settled; the command stays `Accepted`. The skip event names the packet with
the reason `operation_key_taken`, and the outcome under the key is the other
request's.

Identities that are valid by themselves may still be too long for the key of
a step (`has_canonical_operation_key`). For a single step that includes the
key of the `Release` that may have to give the lease up, which names the
acquisition and the grant. Such an operation is refused at admission or at
`enqueue_resource_completion_operation`, and one that reaches settlement is a
recorded `invalid_identifier` rejection.

`canwu-production` does not yet refuse an output request whose operation key
lies under the reserved prefix, or is already used, when an execution
starts: the output batch of such an execution is skipped with
`reserved_operation_key` or `operation_key_taken`, nothing is credited, and
the execution never receives its acknowledgement; its resource participant
grant stays `Consumed`, holding its completion units, target locks, and 16
reserved outcome slots.

## Protected floors

An account's protected floor is the `ProtectedFloorPolicyRevision` its
`protected_floor_policy` names. `account_quantities` derives the protected and
available quantities from it, allocation never reserves protected stock for a
demand outside the revision's `override_classes`, and `RecordLoss` and
`ExternalOutflow` take protected stock only with `allow_protected`.

A revision is immutable and is one of two kinds:

- **Authored.** `ResourceState::install_protected_floor_policy` installs a
  revision with an identity and digest of the author's choosing while the
  scenario is built. It stays in the root whether or not an account points at
  it, and an account is pointed at it by id.
- **Content-addressed.**
  `ProtectedFloorPolicyRevision::content_addressed(revision, floor,
  override_classes)` derives the digest from the content and the id from the
  digest (`resource:protected-floor:{digest}`), so one id names one content
  for the whole run. Only this kind is installed at runtime, and the root
  keeps it only while something refers to it, whoever installed it: a
  scenario that installs one no account points at loses it at the first
  applied `SetProtectedFloor`. An identity under the prefix must be the one
  its content derives.

`ResourceOperationRequestV1::SetProtectedFloor(ResourceProtectedFloorRequestV1)`
points an account at a revision, or at none. To set a floor of any quantity,
the custodian sends the content-addressed revision in `install` and its id in
`policy`: the operation installs the revision if the root does not hold it and
points the account at it, together or not at all. A content-addressed
revision is always named with its body; its id alone is refused with
`invalid_definition` whether or not the root holds it, so the answer tells a
custodian nothing about what other accounts protect. The operation is
token-only: it needs no completion lease, leaves one `SetProtectedFloor`
outcome, and advances the account revision. A tracked command must come from
the account's custodian. An access grant is consent to draw on stock, not
custody, so a grantee cannot set the grantor's floor; adapter ingress cannot
either.

A root holds at most `MAX_PROTECTED_FLOOR_POLICY_REVISIONS` (4,096) revisions
of both kinds. A scenario install past the bound is refused, and a runtime
install past it settles as a recorded `limit_exceeded` rejection. The bound is
one quota shared by every custodian and is below the canonical `max_accounts`
(8,192): floors of different content on more than 4,096 accounts cannot
coexist, and a custodian whose demands cite many different floors can fill
the quota, after which every other custodian's install is refused until
revisions are retired. A host application that lets holders set floors limits
how many distinct floors each may hold.

Every applied `SetProtectedFloor` retires the content-addressed revisions
that no account and no demand in the root refers to, before the bound is
checked, so a floor that changes every day keeps one revision in the root. A
demand keeps the revision it cites until the demand is archived; the next
floor change then retires it, which may be a change in the boundary of the
archive commit itself, because commits settle first. Accounts that set equal
content share one revision. A retired revision is installed again by sending
its body. Retirement removes the only copy of the body the engine holds: an
archived demand keeps the id of the floor it was admitted under and nothing
else, so a host application that needs that floor's quantity and override
classes later keeps the body itself.

A floor change advances the account revision like any other account change.
A reservation made before it stays reserved and is debited under a lease that
locks the new revision. An open demand cites the floor it was admitted under,
so its allocation pass settles as a `version_conflict` rejection until the
demand is amended to the floor in force; a granted demand in that position is
supplied nothing instead. An operation whose lease locked the earlier account
revision settles as a `version_conflict` rejection. A `SubmitDemand` that was
admitted citing a floor and settles after that floor was replaced depends on
the kind of revision: an authored one is still in the root, so the demand is
applied and then stale; a content-addressed one that the change retired is
gone, so the demand is a recorded `invalid_definition` rejection, unless
another account or demand still refers to it.

A refusal of `SetProtectedFloor` is recorded: as the operation's rejected
outcome, or on the command when authority or the operation key refuses it at
admission. A command admitted before the request that takes its key has
settled, or whose result does not fit the root, settles nothing and is named
by a skip event; the command stays `Accepted`. This holds for every tracked
resource command. An archive lookup that fails still fails the boundary.

## Skipped packets

No packet a host application or a holder can send fails a resource boundary.
A packet that cannot settle for an expected reason changes nothing, its
boundary commits, and the plugin emits one `RESOURCE_INGRESS_SKIPPED_EVENT`
(`canwu.resource.ingress_skipped.v1`) naming its ingress id, the operation key
it carried, and a reason; `ResourceIngressSkipV1::from_event` reads it back.
An event naming an entity the run does not hold would fail the boundary, so
the event names as its affected entity only a holder the run admitted, the
holder of a tracked command or the requester of an allocation pass, and only
when it is a registry entity rather than the entity of a domain record, which
may be gone when the packet settles. The holder of the external participant a
production output batch or an external participant step belongs to was never
checked against the run's entities, so the event names it in its detail only
(`the work belongs to holder {holder}`). For the same reason the settlement
event names only admitted, registered holders, and
`enqueue_resource_allocation` refuses a requester that is not an entity of
the run. The reasons (`ResourceIngressSkipReasonV1`):

- `operation_key_taken`: a different request holds the key;
- `reserved_operation_key`: a request that is not a completion operation
  names a key under `resource:completion:`;
- `failed_settlement_checks`: the packet does not decode or fails the checks
  its ingress makes at settlement (its tracked command, provider binding,
  certificate, boundary time, current evidence, or a refused credit of a
  production output batch);
- `archive_backlog`, `reserved_capacity`, `root_limit`: its result does not
  fit the root; the same request settles once room returns. The detail of a
  `reserved_capacity` skip says when no archive commit or lease release can
  ever make room: work that would not fit even with an empty backlog and
  every reservation free. Room reserved for finished participants whose
  reports wait returns once the reports publish and a commit clears them, for
  work an admitted lease certifies as well. A production
  output batch settles every credit or none, and its room is measured for the
  whole batch. Operations may fill the state byte cap only up to
  `ResourceLimitsV1::operation_state_bytes`, which leaves
  `MAINTENANCE_STATE_BYTE_RESERVE` (64 KiB, or a quarter of a smaller cap)
  for the root's maintenance writes.

Work another plugin scheduled is never handed to the host application. A
production output batch, or a completion step a plugin scheduled that can
settle later as the same step (`Prepare`, `Abort`, `Release`, and the
external participant's prepare, complete, and release), that does not fit the
root is deferred while waiting can make room: the plugin queues it to itself
as a `RESOURCE_DEFERRED_INGRESS` packet, an internal ingress no host can
queue, due one minute later, the delay doubling with each further deferral
up to 1,024 minutes, retimes a batch's credits to that instant, and emits the
event with `deferred_until` set and the participant's holder named in its
detail. Work waiting on an archive backlog or on room
leases reserve (or finished participants whose reports wait) is retried until
the commit, release, or publication that makes room; work
waiting on the byte cap is skipped after the 1,024-minute retry; work no
commit or release can make room for, and steps bound to their boundary time
(an acquisition, a grant, an activation, a consumption, an expiry, a single
step), are skipped at once. While a deferral waits the queue is not empty, so
the run cannot seal. Adapter packets a plugin scheduled are skipped, because
their sender (the force-supply reference) dispatches again every consumption
without an outcome. A batch skipped for room, or a deferred batch whose
output account changed revision while it waited or whose output key another
request took, leaves its execution pending. The acknowledgement the plugin
schedules to the production plugin once a batch settles carries `settled_at`,
the instant the credits settled.

A resend whose outcome is already recorded settles nothing and leaves no
event. What a completion step sets in motion (the production plugin's
acknowledgement, an expiry tick) follows only an applied outcome. An expiry
skipped for room stops the plugin's expiry ticks, which would otherwise meet
the same root at the same instant without end; the next applied archive
commit schedules a tick while an expiry is due, and
`enqueue_resource_completion_expiry_tick` queues one for a host application.

What still fails a resource boundary is an invalid record or an engine
invariant: an archive object that is missing, forged, or unreadable during a
key lookup; a forged archive commit or retention acknowledgement; a root that
does not validate after settlement for a reason other than a limit;
maintenance writes (stale commits' receipts and handles, the report system's
scheduling state) that pass the byte reserve in one boundary; and arithmetic
or encoding failure. See `docs/proposals/resource-refusals-without-stalls.md`.

A tracked `Acquire` must be eligible at the instant it is admitted and cite
the current version of its evidence; settlement checks it again as the
completion ingress does.

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
`prepare_resource_archive(candidate_limit, store)` selects only hot terminal
candidates within the configured budget,
`PreparedResourceArchiveBatchV1::store_and_verify` verifies content-addressed
objects, exact membership/temporal closure, and retention.
`enqueue_resource_archive` additionally requires the batch to equal the exact
current terminal candidates before committing the authenticated directory
through the plugin's permitted internal ingress.
`finalize_resource_archive_retention` acknowledges store-side retention after
restart or stale-source rejection. Archive roots, retention handles, receipts,
and candidate indexes are persisted in `ResourceState`. The maintenance
receipt of an applied commit becomes a terminal candidate and can itself move
cold; ordinary lifecycle work does not scan cold history.

An archive commit never fails for capacity. A commit that went stale because
another commit or an operation settled first is a terminal no-op: its receipt
is not a candidate, stays in the root outside the budgets until
`finalize_resource_archive_retention` acknowledges it, and is then removed.
A commit queued beside it against a full root still applies. An
acknowledgement retires the retention handle it names and every stale receipt
recorded under that handle, so the same batch queued twice leaves no stale
receipt behind. Finalizing reads how the commit settled from the state-side
retention handle, which stays until its acknowledgement settles, so a commit
can also be finalized after a later commit archived its receipt. Within one
boundary, archive
commits and retention acknowledgements settle before every other packet of
the resource plugin, so a commit prepared from the root the boundary started
from applies even when an operation was admitted ahead of it. Releasing a
retention handle removes its protection and never deletes an object: two
batches prepared from one root share their leading blobs, pages, and lookup
pages, so an object named by a rejected or abandoned handle may belong to an
applied commit. Deletion is by reachability only.

One commit carries up to `MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS` (4,096) records
as blobs of at most 256, each with its own membership and temporal page, and a
run may make any number of commits. `max_archive_candidates` defaults to 256
and may be raised to `ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES` (4,096);
`max_archive_maintenance_receipts` must be at least as large. The ceiling is
headroom for a burst, not a working size. Every boundary that changes the
resource root decodes, validates, re-encodes, and journals the whole root, so
one boundary costs time and journal bytes in proportion to the hot terminal
records: in the recorded baseline about 13 ms and 0.3 MB at 256 records,
52 ms and 1.2 MB at 1,024, and 246 ms and 4.6 MB at 4,000
(`benchmarks/baselines/2026-10-13-resource-archive-full.json`). Choose the
budget from that cost at the backlog you accept, and archive often rather than
run with a large backlog. Room is measured on what an operation leaves: a
refused operation needs one slot for its outcome, an applied one a slot for
every terminal record it adds, beside the slots reserved for admitted
completion lifecycles. An operation that does not fit settles nothing and is
named by a skip event with `archive_backlog` (an archive commit makes room) or
`reserved_capacity` (it does not); it is not a recorded rejection, and its
key stays free. Send it again once room returns. With `archive_backlog`, queue
one commit that selects every candidate, in the same boundary if you like,
because archive commits settle first. A commit queued behind another that was
prepared from the same root goes stale once the first applies, and an applied
commit leaves its own maintenance receipt as a candidate, so one commit makes
room for the budget minus one record. Operations beyond that are skipped
again and settle behind the next commit. Nothing is queued for a skipped
operation, so a long shortage costs events and resends, never a stopped run.

Every operation asks the archive whether its operation key was used before.
Each commit directory commits to a cumulative lookup index
(`ResourceArchiveLookupPageV1`, a 16-way trie over key digests), and
`archived_resource_terminal_record` reads the head directory and one lookup
page per index level, each authenticated by its content id: the cost follows
the logarithm of the archived records and does not depend on the number of
commits. A hit also reads and authenticates the one blob that holds the
record. `prepare_resource_archive` reads the lookup pages on the paths of the
new keys from the store and prepares their replacements. A lookup trusts the
head directory to name a faithful index: `enqueue_resource_archive` and the
checked restore wrappers establish that, and a root assembled by hand is not
checked.

A restore, an exact replay, or a fork reads the index of every head it passes
through, so the store must keep the lookup pages those heads reach. The
archive reachability participant marks every lookup page reachable from a
chain, queued, or unacknowledged directory; the index of the head and of each
queued commit must be complete, and any other is marked as far as the store
still holds it.

`superseded_lookup_pages` of a prepared batch names the lookup pages of the
previous head's index that the batch's index no longer reaches. The list may
be used only after the batch's commit settled as `Applied`. A batch whose
commit was rejected as stale carries a list too, and that list names pages of
the live head. Deleting the pages of an applied commit is safe only after a
durable snapshot or checkpoint taken after that commit exists, and it gives up
every earlier point of the run that reads one of those pages: a snapshot or
checkpoint taken before the commit no longer restores, a journal no longer
replays across it, and a fork whose head precedes it no longer continues. This
reaches back as far as the commit that wrote each page, not only to the commit
before, because consecutive heads share pages. A host application that needs
any of those keeps the pages. `live_resource_archive_lookup_pages` recomputes
what a live run reads when a list was lost: the pages reachable from the head
and from each queued commit, whose directory roots the host application
passes. A commit is queued when its store-side retention handle is in
`DurableIngress` and the root holds no handle with its id. A queued commit the
host application does not name is not covered.

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
