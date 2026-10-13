# Single-Step Completion Lease and Runtime Protected-Floor Install

Status: implemented in 0.15.0. It closes downstream gaps G-53 and G-56 in
`canwu-resource`. Both changes are in the resource plugin and its request
types. The kernel, the archive, and the shape of the resource root are
unchanged.

## Single-step completion lease (downstream gap G-53)

### Problem

Every irreversible resource operation (a consumption, a transfer start, a
terminal transfer disposition, a credit, an external outflow, an account-level
loss) carries the activation certificate of a completion lease. A lease
becomes active through four operations: acquire, grant, prepare, activate.
Each leaves an operation outcome and a lease receipt, and each asks the
archive whether its operation key was used before. With the receipt that
completion adds, one lease leaves 9 terminal records and makes 4 archive
lookups.

The steps are separate because a lease may have several participants, each of
which grants and prepares on its own. When the resource plugin is the only
participant nothing happens between the steps, and they are overhead: 18 of
the 27 terminal records a same-place transfer leaves, and 8 of its 15 archive
lookups. A hot terminal record is decoded, validated, re-encoded, and
journaled at every boundary that changes the resource root, and it counts
against the archive candidate budget, so this overhead decides how soon
archive backpressure applies and what each boundary costs.

`CompletionLeaseBookV1::activate_single_owner` already existed, but no
operation reached it.

### Decision

One completion operation takes such a lease from nothing to active:

```rust
ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1 {
    lease,            // RequestCompletionLeaseV1, as for Acquire
    grant_id,         // the one grant, owned by canwu-resource
    target_versions,  // every exact target the lease locks
    current_boundary,
})
```

**Only participant.** This means exactly that `lease.expected_participants`
is `{"canwu-resource"}`. The one grant is owned by the resource plugin and
locks every target. The plugin does not derive participants from the targets:
a resource grant may lock an external record version another plugin owns, on
this path as on the four-step path, and it is the host application that
decides whether that plugin should hold a grant of its own. The single step
is the lease behind a tracked `BeginTransfer`, exchange leg, `CompleteTransfer`
(`AcceptLocal`, `Lose`, `Return`), `ExternalOutflow`, or `RecordLoss`. Two
kinds of lease are not single-participant and keep the four operations: a
lease that needs a grant owned by another plugin (an adapter operation, whose
provider must hold its own participant grant), and a lease another plugin
coordinates with the resource plugin as an external participant
(`canwu-production`, the force-supply reference). A single step that names
any other participant set is refused with `invalid_lifecycle`.

**The same checks.** The operation runs the acquire, grant, prepare, and
activate steps in order with the checks each has as a separate operation: the
holder's run-budget partition; pending slots, request tokens, cooldown, and
the same-time cap; the full receipt reservation
(`MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE`) in archive backpressure; the named
report reservation; the deterministic fairness order, so a single step does
not pass a pending acquisition that sorts before it; exact current targets; no
conflicting target lock; and the activation guard. All four run at
`current_boundary`.

**The same certificate.** Because the operation calls the same four lease-book
functions, the `CompletionLeaseActivationCertificateV1` equals the one the
four steps produce for the same acquisition, grant id, and targets when their
activation runs at `current_boundary`. With all four steps at that boundary
the acquisition, grant, target locks, admission epoch, and reserved units are
equal too. An operation that consumes the certificate cannot tell which path
produced it.

**One outcome and one receipt.** The operation leaves one `CompletionLease`
outcome under the key `resource:completion:lease:{acquisition}`, with the
lease as its result reference and the eligibility envelope's exact evidence,
and one `Activated` receipt. Completion still adds its one receipt.

**Ingress.** A tracked command cannot carry the operation. The host
application queues it with `enqueue_resource_completion_operation`, as it
queues the grant, prepare, and activate steps. That is not the only source: a
plugin whose boundary system declares the resource completion ingress among
its `plugin_ingress_targets`, as `canwu-production` does, can schedule a
single step too. The resource plugin trusts such a packet as it trusts the
host application's, which is the trust the four-step operations already have.
The operation is kept from holders because an activated lease does not expire
and holds its exact target locks until it completes: a holder command that
could activate a lease on any target could withhold another custodian's
account from that custodian's own leases.

**Refusals.** A domain refusal is the operation's recorded rejection, and the
lease exists whole or not at all: a refused single step leaves no
acquisition, grant, lock, report reservation, or spent request token. This
includes what the plugin finds against the simulation view, on which the
four-step path fails its boundary: an eligibility time that is not the
boundary time, no room for the reserved reports, an external record version
that is not its record's current version, or a record kind the plugin was not
configured to read. Those checks read live state and the current-version
provenance index, never retained evidence, so a sealed run and its exact
replay decide the same. A single step never resumes an acquisition or grant
that exists; it is refused with `idempotency_conflict`, and the four-step
path finishes what it began. Identities that are valid by themselves but too
long to form the operation key are refused with `invalid_identifier`, under
the key `resource:completion:invalid:{digest}`. So are identities that form
the key of the single step but not the key of the `Release` that may have to
give the lease up, which names the acquisition and the grant: such a lease
could never be released. The completion ingress refuses to queue either
request in the first place.

**What can still fail a boundary.** Not every refusal is recorded. Like every
resource operation, the single step must fit the budgets of the root before
anything is recorded, and three things remain errors that fail the boundary
that settles it:

- *No room.* The check asks for the whole lease: `2 + 16` archive candidate
  slots (the outcome, the receipt, and the reserved terminal path) and
  `1 + 16` hot outcome slots, beside the 16 that every active lease already
  reserves. So a single step fits while `hot terminal records + 16 × active
  leases + 18 <= max_archive_candidates`: with the canonical budget of 256
  and no backlog, the fifteenth concurrently active lease is the last. The
  room is asked for even when the request would be refused, so under a full
  budget a refusal is not recorded either.
- *An archive lookup that fails.* A missing, forged, or unreadable archive
  object is an error, never a miss.
- *The state byte cap*, checked when the root is validated after the
  operation.

A host application therefore checks headroom before it queues a single step.
An archive commit frees room taken by hot terminal records, and it may be
queued behind the step, because commits settle first. It frees nothing held
by the reservations of active leases: that room returns only when their
operations complete or the leases are released.

**Operation keys.** The operation key is derived from the acquisition id. A
resend of the same request returns the recorded outcome while that outcome is
in the root and is answered from the archive afterwards; neither is a second
lease. A refused request has used its key, so a retry names a new acquisition
id.

A packet on the completion ingress whose key belongs to a different request,
in the root or the archive, settles nothing, and its boundary commits. This
holds for every completion operation, the four steps included. The skip
leaves no record of its own: the outcome under the key is the other
request's. A host application therefore compares
`ResourceOperationRequestV1::request_digest` of what it sent with the
`request_digest` of the outcome under the key, and does not read that outcome
as its own.

**Who can take a completion key.** A key under `resource:completion:`
(`RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`) is reserved for completion
operations: a tracked command that names one on any other request is refused
at admission, `resource_adapter_ingress` and `enqueue_resource_allocation`
refuse to queue one, a packet queued around them settles nothing, and
`ResourceState::apply_operation` returns an error without recording an
outcome, which would itself take the key. A holder can therefore take a
completion key only with a completion operation it may send as a tracked
command:

- A holder cannot take the key of a single step (`lease`), nor of a `grant`,
  `prepare`, `activate`, `release`, `expire`, or `participant-*` step.
- A holder can take `acquire:{id}` for any id, by acquiring a lease under it.
  The host application's `Acquire` with that id then settles nothing, and a
  single step with that id is refused with `idempotency_conflict` while the
  holder's acquisition is in the root. The host application uses another id.
- A holder can take `abort:{id}` of a lease it holds: its own `Abort` leaves
  an outcome under that key whether it is applied or rejected. It cannot take
  the abort key of another holder's lease or of an id the root does not hold.
  Admission resolves the holder of a tracked `Abort` from the acquisition in
  the root, not from the request, and refuses anyone else.

**Reserved keys in production output.** `canwu-production` does not yet refuse
an output request whose operation key lies under the reserved prefix when an
execution starts. The output batch of such an execution is skipped: its
boundary commits and nothing is credited. The execution never receives its
acknowledgement, and its resource participant grant stays `Consumed`, holding
its completion units, its target locks, and its 16 reserved outcome slots. A
host application keeps such keys out of the output requests it admits.

**Free-form keys of other packets.** Outside the reserved prefix nothing
changes for packets that are not tracked commands and not completion
operations. An allocation, adapter, or production output packet whose
free-form operation key another issuer used first for a different request
still fails its boundary, as on 0.14.0.

**Acquisition identities.** An acquisition id is unique among the
acquisitions in the root, not for the run. While the root holds an
acquisition, neither path creates another under its id: the single step is
refused, and so is an `Acquire`, which before this change could only meet an
acquisition its own resend had created. After a lease has left the root with
its archived records, the operation key is what remains, and the two paths
derive different keys: an `Acquire` with the id of an archived single-step
lease creates a new acquisition, and so does a single step with the id of an
archived four-step lease. A host application that needs acquisition ids
unique for the run keeps them unique itself.

**Beside an archive commit.** Archive commits settle before the other packets
of a boundary. A single step queued in the boundary of a commit, before or
after it, is taken on the root the commit left, and the commit applies to the
root it was prepared from instead of going stale behind the lease.

Alternatives considered and rejected:

- *Irreversible operations without a lease when the resource plugin is the
  only participant.* The lease is what reserves the terminal receipt path and
  locks the exact targets before a debit. Dropping it would change every
  irreversible request and remove both guarantees.
- *Carrying the lease request inside the operation it authorizes.* An exchange
  leg needs its custodian's certificate before the exchange is admitted, so
  the certificate must be able to exist first; and six request types would
  change shape.
- *Keeping four operations and recording one outcome.* Each operation is a
  separate ingress with its own idempotency key and needs its own outcome.
- *Admitting the single step as a tracked holder command.* See "Ingress".

### Invariants

- A lease is active only after the checks of all four steps passed against one
  state. The single step adds no path around any of them.
- The certificate is a function of the acquisition, its grants, and the
  activation boundary. It does not record how many operations produced it.
- A refused single step changes nothing but the outcome it records.
- An acquisition in the root is advanced only by the path that created it.
- The four-step path, the external-participant path, and their records are
  otherwise unchanged.

### What a host application has to watch

- **An active lease does not expire.** A single-step lease is active at once.
  If its operation is never issued, or is rejected, the lease holds its
  target locks, its completion units, and its 16 reserved slots until the
  host application sends `Release`. The four-step path has the same property
  once it activates; until then its grants expire.
- **A stray `Requested` acquisition blocks single steps.** A holder may still
  send a tracked `Acquire`. While that acquisition waits for its first grant
  it holds its place in the fairness order, and every single step that sorts
  after it (a later class, round, holder, or admission) is refused with
  `invalid_lifecycle` until the acquisition is granted or aborted. A host
  application that uses only single steps watches for such acquisitions and
  removes them. It sends `Abort`. If the holder's own rejected abort has
  already taken the abort key, that packet settles nothing; the host
  application then grants the stray acquisition and releases that grant.

### Measured

Counts, from the committed fixture
`gap_g53_resource_single_step_lease.rs`
(`same_place_transfer_leaves_fifteen_records_and_nine_lookups`), which asserts
them. It settles the same transfer twice in one plugin-owned run with an
archive head: a tracked `SubmitDemand`, an allocation pass, a lease and a
tracked `BeginTransfer` for the source custodian, then a lease and a tracked
`CompleteTransfer` with `AcceptLocal` for the destination custodian. Terminal
records are the entries added to `terminal_archive_candidates`. Archive
lookups are the loads of the archive head directory through the runtime's
archive provider; every lookup of an operation key loads it once. The
"before" column is the four-step path, which this change leaves as it was in
0.14.0.

| Count | Four steps | Single step |
| --- | ---: | ---: |
| Terminal records per lease, activation only | 8 | 2 |
| Terminal records per lease, with its completion receipt | 9 | 3 |
| Archive lookups per lease | 4 | 1 |
| Terminal records per same-place transfer | 27 | 15 |
| Archive lookups per same-place transfer | 15 | 9 |

A single step also starts no expiry clock, because it never leaves a grant
that could expire, so it queues no expiry tick.

### Persistence, replay, and formats

- **Plugin state.** No field of `ResourceState` changes and
  `RESOURCE_STATE_FORMAT_VERSION` stays 2. A single-step lease is stored as
  the acquisition, grant, certificate, and target locks the four steps store.
  Its outcome and its receipt use the existing record shapes with a new
  operation-key prefix, `resource:completion:lease:`, which
  `ResourceOperationOutcome::validate` accepts for the `CompletionLease` kind.
  A rejected outcome may carry the key `resource:completion:invalid:{digest}`.
- **Ingress.** `ResourceCompletionOperationV1` gains the variant
  `single_step_lease`. Packets of the other variants are encoded as before.
- **Archive.** Archived outcomes and receipts keep their format. An archive
  written before this change holds no record with the new key prefixes.

### First-party users

- `canwu-economy-reference` (grain loop): the lease of its transfer start
  locks only resource targets, so it now takes the single step. Its
  consumption, harvest credit, and transfer acceptance leases lock a record
  the economy plugin owns under the economy plugin's own grant, and its force
  lease is coordinated by the force-supply plugin; they are unchanged. The
  loop archives by record count, so with fewer records a month its settled
  delivery reaches the archive sooner; the one harness test that looked for
  that transfer in the root now reads the transfer and the outcome of its
  single-step lease from the root or the archive.
- `canwu-production` and `canwu-force-supply-reference` coordinate their own
  lease books with the resource plugin as an external participant. Unchanged.
- `examples/completion_lease.rs` drives a `CompletionLeaseBookV1` directly, as
  a coordinator does. It calls the book's functions and settles no resource
  operation, so there is no outcome or receipt to save. Unchanged.

## Runtime protected-floor install (downstream gap G-56)

### Problem

A protected floor is an immutable `ProtectedFloorPolicyRevision` that entered
the resource root only through `ResourceState::install_protected_floor_policy`
while the scenario was built. `SetProtectedFloor` could point an account at a
revision already in the root and nothing else, so every floor a custodian
might choose had to be authored in advance. A custodian could not name a
quantity.

### Decision

`SetProtectedFloor` carries the revision body. `ResourceProtectedFloorRequestV1`
gains an optional field:

```rust
pub install: Option<ProtectedFloorPolicyRevision>,
```

When it is present, `policy` must name its id. The operation installs the
revision if the root does not hold it and points the account at it, together
or not at all. It stays the token-only operation it was: no completion lease,
one `SetProtectedFloor` outcome, and one advance of the account revision.

A separate install command was considered and rejected. It would take two
operations and two outcomes to set one floor; it would create revisions that
no account points at, which need an owner, an authority rule, and a way to
free them; and it would let a holder fill the root's bound without ever
setting a floor. Installing and pointing in one operation means a revision
installed at runtime is always referenced when it enters the root.

**Content-addressed identity.** A revision installed at runtime must be
content-addressed:
`ProtectedFloorPolicyRevision::content_addressed(revision, floor,
override_classes)` derives `semantic_digest` from those three fields and the
id from the digest (`resource:protected-floor:{digest}`). A new floor is
therefore always a new revision, and one id names one content for the whole
run. That is what makes retirement safe: an archived demand that names a
retired revision still names exactly one content, and the revision can be
installed again only as itself. A free identity would allow a later install
to give an archived reference a different meaning, or would need a tombstone
for every identity ever used. A content-addressed revision names at most 16
override classes of 1 to 160 bytes each. The prefix is reserved: a scenario
install of an identity under it that its content does not derive is refused.

A content-addressed revision is always named with its body. A request that
names one by id alone is refused with `invalid_definition`, whether or not
the root holds the revision. Whether it does depends on what other accounts
and demands refer to, so an answer that differed would tell a custodian
whether anyone currently protects exactly that quantity. `policy` alone
names an authored revision, or `None`.

**The archive keeps the identity, not the body.** Retirement removes the only
copy of a revision's body the engine holds. A demand in the archive keeps the
id of the floor it was admitted under; the id names one content and does not
contain it. A host application that needs the floor quantity and override
classes an archived demand was admitted under keeps the body itself.

**Authority.** A tracked command must come from the account's custodian, as
before. An access grant is consent to draw on stock under a cap, not custody:
admission resolves the target holder of `SetProtectedFloor` from the account
alone, so a grantee cannot change the grantor's floor. Adapter ingress and
completion ingress cannot carry the operation.

**Bound.** A root holds at most `MAX_PROTECTED_FLOOR_POLICY_REVISIONS`
(4,096) revisions, authored and content-addressed together.
`install_protected_floor_policy` refuses a revision past it,
`ResourceState::validate` rejects a root past it, and a runtime install past
it settles as a recorded `limit_exceeded` rejection that changes nothing. The
boundary does not fail.

The bound is one quota shared by every custodian. It is below the canonical
`max_accounts` (8,192), so floors of different content on more than 4,096
accounts cannot coexist. A custodian's own demands keep the revisions they
cite in the root, so one custodian can pin revisions up to the bound, and
every other custodian's install is then `limit_exceeded` until revisions are
retired. A host application that lets holders set floors limits how many
distinct floors each of them may hold.

**Retirement.** Every applied `SetProtectedFloor` retires the
content-addressed revisions that no account and no demand in the root refers
to, before the bound is checked, so replacing a floor at the bound settles.
An account whose floor changes every day keeps one revision in the root.
A demand keeps the revision it cites while the demand is in the root; once
the demand is archived, the next floor change retires the revision. Archive
commits settle before the other packets of a boundary, so that may be a floor
change in the boundary of the commit itself. Accounts that set equal content
share one revision, which is retired when the last of them lets go. A retired
revision is installed again by sending its body.

How a revision arrived does not matter. An authored revision is never
retired: a scenario's ladder stays available by id whether or not an account
points at a rung. A revision a scenario built with `content_addressed` is
retired like any other content-addressed one: unless an account or a demand
refers to it, the first applied `SetProtectedFloor` removes it.

Retirement visits only the content-addressed revisions, which are one
contiguous key range, and reads the accounts and demands once. It runs in
`SetProtectedFloor` only, so no other operation does more work than before.

### Invariants

- A revision in the root never changes. An id names one content: an authored
  id for as long as the root holds it, which is always; a content-addressed id
  for the whole run.
- An account or a demand in the root never names a revision the root does not
  hold.
- The root does not record how a revision arrived. An authored revision and a
  content-addressed one are distinguished only by whether the identity is
  derived from the content.

### Refusals

- Recorded as the operation's rejected outcome: an install that is not
  content-addressed, an install that is not the revision `policy` names, or a
  content-addressed id without its body (`invalid_definition`); a root at its
  bound (`limit_exceeded`); a stale account revision (`version_conflict`); an
  authored id the root does not hold (`not_found`).
- Recorded on the command, at admission: a subject that is not the
  custodian; an operation key under `resource:completion:`; an operation key
  whose outcome is in the root or the archive for a different request.
- Left unsettled: a command admitted before the request that takes its key
  has settled. Admission cannot see that request's outcome yet, so the
  command is accepted. At settlement the key belongs to the first request,
  and the second command's packet settles nothing: the boundary does not
  fail and the first request keeps its effect. The skip leaves no record of
  its own, and the skipped command stays `Accepted`. The outcome under the
  key is the first request's, so a host application compares
  `ResourceOperationRequestV1::request_digest` of what it sent with the
  `request_digest` of that outcome and does not read the outcome as its own.
  Two such commands admitted in one boundary settle the same way, in their
  admitted order. Before this change the second packet failed its boundary
  and every later one. This holds for every tracked resource command.
- Still errors that fail the boundary, as for every resource operation: no
  room for the outcome (one archive candidate slot and one hot outcome slot,
  beside the 16 of each that every active lease reserves), an archive lookup
  that fails, and the state byte cap. A command that does not fit stays
  queued until an archive commit makes room, and the commit may be queued
  behind it.

### Readers

Every reader of a floor reads the root: `account_quantities` (and through it
allocation, `RecordLoss`, `ExternalOutflow`, and the stock observations of
holder reports), the override-class check of allocation, and the account and
demand contract checks. `BeginTransfer`, `BeginExchange`, and consumption
debit a reservation and read no floor. `canwu-production` and the
force-supply reference read floors only through the allocation legs they
consume. The economy reference also calls `account_quantities` for the stock
observations it records and reads the protected quantity of those
observations in its projections.

The root does not record how a revision arrived, so a content-addressed
revision reads the same whether a scenario or a custodian installed it: the
fixture `gap_g56_resource_floor_install.rs` installs one both ways and
asserts the two roots equal, directly after the operation and again after a
script of every reader. The same script against an authored revision of the
same content, pointed at by id, has the same effect operation by operation;
only digests differ, because the demands cite another identity.

An operation in flight when the floor changes:

- A reservation made before the change stays reserved. Its debit locks the
  account revision the change left.
- An open demand cites the floor it was admitted under. Its allocation pass
  settles as a `version_conflict` rejection until the demand is amended to the
  floor in force. A granted demand in that position is supplied nothing
  instead, which does not fail the grantee's pass.
- An operation whose lease locked the earlier account revision settles as a
  `version_conflict` rejection.
- A `SubmitDemand` that is admitted citing revision X and settles after the
  floor has changed to Y is the one case in which the kind of revision
  matters. If X is authored it is still in the root: the demand is applied
  and is then stale, as above. If X is content-addressed and the change
  retired it, the demand cites a revision the root does not hold and settles
  as a recorded `invalid_definition` rejection. If another account or demand
  still refers to X, it was not retired and the demand is applied. The
  requester submits the demand again citing the floor in force.

### Persistence, replay, and formats

- **Plugin state.** No field of `ResourceState` changes and
  `RESOURCE_STATE_FORMAT_VERSION` stays 2. `protected_floor_policies` gains
  and loses entries at runtime; it has no new marker for how a revision
  arrived.
- **Requests.** `ResourceProtectedFloorRequestV1::install` is omitted from the
  canonical encoding while it is `None`, so a request that installs nothing
  has the encoding and the request digest it had before.

## Compatibility

0.15.0 is a pre-1.0 minor release.

- `ResourceCompletionOperationV1` gains the variant `SingleStepLease`; an
  exhaustive match on it must add an arm.
- `ResourceProtectedFloorRequestV1` gains the field `install`; a struct
  literal must add `install: None`.
- `ResourceState::install_protected_floor_policy` and
  `ResourceState::validate` enforce `MAX_PROTECTED_FLOOR_POLICY_REVISIONS`. A
  scenario that installs more than 4,096 revisions is refused, and so is a
  scenario identity under `resource:protected-floor:` that its content does
  not derive.
- An operation key under `resource:completion:` is accepted only on a
  completion operation. Any other request that names one is refused: at
  admission for a tracked command, by `resource_adapter_ingress` and
  `enqueue_resource_allocation`, and as an error from
  `ResourceState::apply_operation`.
- An `Acquire` that names an acquisition the root holds is a recorded
  `idempotency_conflict` rejection.
- A tracked `Abort` is authorized by the holder of the acquisition in the
  root. One that names an acquisition the root does not hold, or another
  holder's, is refused at admission. Before, it was admitted on the holder
  the request named and settled as a recorded rejection.
- A packet on the completion ingress whose operation key belongs to a
  different request settles nothing. Before, its boundary failed.
- A completion operation whose identities do not form a valid operation key
  no longer panics. `ResourceCompletionOperationV1::operation_key` returns
  `resource:completion:invalid:{digest}` for it, the operation settles as a
  recorded `invalid_identifier` rejection, a tracked command that carries it
  is refused at admission, and `enqueue_resource_completion_operation`
  refuses to queue it.
- A tracked resource command whose operation key has an outcome in the root
  for a different request is refused at admission with
  `IdempotencyConflict`, as it already was for a key in the archive. One
  admitted before that outcome existed is left unsettled. Before, the
  boundary that settled either failed.
- `RESOURCE_SEMANTIC_HASH` changes. Registered commands, ingress, boundary
  systems, and schemas are unchanged.
- Snapshot, commitment, checkpoint, evidence-journal, and paged-checkpoint
  formats and the hash algorithms are unchanged.

These two changes leave a resource root written by 0.14.0 as it was: it
decodes, validates, and re-encodes to the same bytes, provided it holds at
most 4,096 protected-floor policy revisions. Its revisions are authored, so
nothing in it is retired; one whose identity happens to lie under
`resource:protected-floor:` can no longer be named by id. What 0.15.0 changes
for the archive such a root names is described in the
[archive lookup and volume proposal](resource-archive-lookup-and-volume.md).
As with any version change, the exact engine-version and plugin-identity
checks reject a 0.14.0 save.
