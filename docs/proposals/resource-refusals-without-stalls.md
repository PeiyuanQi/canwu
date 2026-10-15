# Resource Refusals Without Stalls

Status: implemented in 0.16.0. It changes how the resource plugin
(`canwu-resource`) settles and admits packets, and leaves the archive lookup
index, the single-step lease semantics, and the shape of the resource root as
they were in 0.15.0.

It builds on two 0.15.0 contracts:
[resource archive lookup and volume](resource-archive-lookup-and-volume.md)
and
[single-step completion lease and runtime protected-floor install](resource-single-step-lease-and-floor-install.md).
The statements of those proposals that this one changes are listed under
[Compatibility](#compatibility).

## The rule

A failed boundary cannot be stepped past, and a due packet cannot be
withdrawn, so a packet that fails its boundary stops the run until the
packet itself can settle. In 0.15.0 a resource packet failed its boundary for
reasons a host application or a holder causes in ordinary use: a result that
did not fit the archive budgets of the root, an operation key that another
request had taken, and a packet that did not pass its settlement checks.

After this change, every resource packet that cannot settle for such a reason
is **skipped**:

- it changes nothing in the resource root and records nothing under its
  operation key, so the key stays free for a corrected or repeated request;
- its boundary commits, and every other packet of the boundary settles as it
  would have without it;
- the plugin emits one `canwu.resource.ingress_skipped.v1` event
  (`RESOURCE_INGRESS_SKIPPED_EVENT`) that names the packet's ingress id, the
  operation key it carried, a stable reason code, and a readable detail.
  `ResourceIngressSkipV1::from_event` reads it back.

Work another plugin scheduled is never handed to the host application,
because that plugin may never send it again. When a production output batch,
or a completion step a plugin scheduled that is not bound to its boundary
time, does not fit the root, and an archive commit or a lease release can make
room for it, the resource plugin **defers** it: it queues the same work to
itself for a later instant, and the event says until when. Work that no
commit or release can make room for is skipped at once, and work that waits
on the byte cap is skipped once its delay has reached its cap (see
[Plugin-scheduled work is deferred](#plugin-scheduled-work-is-deferred)).

A request that is refused by the domain (a stale revision, an unknown target,
a lease the run budget does not allow, and so on) is still the operation's
recorded rejection, as before. A request whose outcome is already recorded
for it, in the root or the archive, is a resend: it settles nothing and leaves
no event, because the outcome under its key is its own.

Only an invalid record or an engine invariant failure still fails a resource
boundary. The complete list is in
[What can still fail a boundary](#what-can-still-fail-a-boundary).

## The skip event

```text
resource ingress {ingress} skipped ({reason}) [{operation key}] {detail}
resource ingress {ingress} deferred to minute {minute} ({reason}) [{operation key}] {detail}
```

The second form is a deferral; `ResourceIngressSkipV1::deferred_until` holds
the instant. The operation key is empty when the packet could not be decoded,
and the detail is then fixed, because a decoder's message can repeat text the
sender supplied. For a production output batch the key is that of the credit
that decided the skip.

An event naming an entity the run does not hold fails its boundary, and a
boundary system cannot ask the entity registry, so the event names as its
affected entity only a holder the run has admitted: the holder of a tracked
command, which the kernel admitted with the command, or the requester of an
allocation pass, which `enqueue_resource_allocation` now refuses
(`EntityNotFound`) unless it is an entity of the run. Even such a holder is
named only when it is a registry entity, which the registry never loses, not
the entity of a domain record, which may be gone when the packet settles. A
holder named this way sees, under the `AffectedActors` audience, that its work
settled nothing. The holder of the external participant a production output
batch or an external participant step belongs to was vouched for by the
coordinator plugin, never checked against the run's entities, so the event
names it in its detail only (`the work belongs to holder {holder}`) and has no
affected entity; neither does the event of any other packet. A local lease
holder is not named at all: the host application supplies it, and the root
does not check it. For the same reason the
`canwu.resource.operation_settled.v1` event now names only admitted,
registered holders too; in 0.15.0 an allocation pass for a requester that was
not an entity of the run failed its boundary when it settled.

| Reason (`ResourceIngressSkipReasonV1`) | Code | Meaning | What makes it settle |
| --- | --- | --- | --- |
| `OperationKeyTaken` | `operation_key_taken` | A different request holds the key, in the root or the archive. | Nothing: the key belongs to the other request. Send the request under another key. |
| `ReservedOperationKey` | `reserved_operation_key` | A request that is not a completion operation names a key under `resource:completion:`. | Nothing: use a key outside the prefix. |
| `FailedSettlementChecks` | `failed_settlement_checks` | The packet does not decode, or does not pass the checks its ingress makes at settlement; the detail names the check. | A corrected packet, which may use the same key. |
| `ArchiveBacklog` | `archive_backlog` | The result does not fit the archive-candidate or hot-outcome budget, and an archive commit of every hot candidate would make room. | The same request, sent again once a commit has applied. |
| `ReservedCapacity` | `reserved_capacity` | The result does not fit even beside an empty backlog: the room is reserved by admitted completion leases or by finished participants whose reports wait, or the budget is smaller than the work. The detail says when no commit or release can make room: for work that does not fit even with an empty backlog and every reservation free. | The same request, once leases complete, are released, or expire, or once waiting reports publish and a commit clears them, or with a larger budget. An archive commit alone does not help. |
| `RootLimit` | `root_limit` | The result passes another limit of the root, in practice the share of the state byte cap operations may use (see below). | The same request once the root is smaller, or a larger cap. |

`ResourceIngressSkipReasonV1::is_capacity` is true for the last three.

What the event tells a holder: that its own request settled nothing and
why. `operation_key_taken` says its key is held by another request, which
admission already tells it. `archive_backlog` and `reserved_capacity` say
whether the root's backlog or the reservations of admitted leases are short,
an aggregate the holder could not tell apart before; they name no other
holder, lease, or request.

The skip decision and the event text read only the resource root, the
current-version index of other records, live record bodies, and the archive
the root names, never retained evidence. The one check that reads retained
evidence, whether a resource command packet matches the tracked command that
produced it, reports any failure with one fixed detail, so the event is the
same whether or not that command's evidence has been sealed. A host cannot
give a packet a command cause, so only a forged packet fails that check.

## Item 1: a result that does not fit the root

### Problem

Before an operation settled, the plugin checked that the root had room for
the most terminal records an operation of its kind could produce (between one
and eight, by kind), and for every completion acquisition it would reserve,
beside the reservations of admitted leases. An operation that did not fit
failed its boundary and stayed queued. One archive commit frees at most the
budget minus one slot (it leaves its own receipt), so a boundary with more
due operations than that could never settle. The check also asked for the
room of the applied result when the request would be refused: a single-step
lease asked for `2 + 16` candidate slots and `1 + 16` hot outcome slots even
when the simulation view refused it, so at the canonical budget of 256 about
the fifteenth concurrently active lease failed its boundary, refusals
included, and no archive commit could make room, because the room was held by
the reservations of active leases.

### Decision

**Room is measured on the result.** The operation is applied to the root, its
outcome is about to be recorded, and only then is room checked
(`ResourceState::room_shortage`):

- the outcome takes one archive-candidate slot and one hot outcome slot;
- an applied operation also needs room for every terminal record it added and
  every reservation it made;
- work that an admitted lease or participant certifies (a certified debit, a
  lease step on an admitted acquisition, an expiry) draws on the room that
  lease reserved, and must fit only beside the reports waiting to become
  archive candidates, as before;
- any other work must also leave every reservation of admitted leases and
  participants free, as before.

A refused operation therefore needs room for its own outcome only. The
reservation accounting itself is unchanged: an admitted acquisition reserves
`MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE` slots of each budget until it is
released or expires.

**A result that does not fit is a skip, not an error.** The root is restored
to what it was before the operation, nothing is recorded, and the packet is
skipped with `ArchiveBacklog`, `ReservedCapacity`, or `RootLimit`. The reason
is decided from the same numbers: `ArchiveBacklog` when the root holds at
least one hot candidate and the result would fit beside a single maintenance
receipt, which is what an archive commit of every candidate leaves;
`ReservedCapacity` otherwise. `ReservedCapacity` is **permanent** when no
archive commit or lease release can make room: when the work's records would
not fit even with an empty backlog and every reservation free. Any other
`ReservedCapacity` is curable, for work an admitted lease certifies too: such
work need not leave other leases' reservations free, but it must fit beside
the slots of finished participants whose reports have not yet published, and
those slots become ordinary hot records once the reports publish, which an
archive commit clears.
A production output batch settles every credit or none, so its room is
measured once, for the whole batch against the root before it, not credit by
credit. `RootLimit` is any other limit
`ResourceState::validate` finds on the result, in practice the state byte
cap. An operation may fill the cap only up to
`ResourceLimitsV1::operation_state_bytes`: the cap less
`MAINTENANCE_STATE_BYTE_RESERVE` (64 KiB), or less a quarter of it for a
smaller cap. The reserve is for the root's own maintenance writes, which are
not skipped: the receipt and retention handle of a stale archive commit
(about 4 KB for a commit of sixteen pages) and the report scheduling state
the report system persists. The skip applies on every ingress that settles
operations: tracked commands, adapter packets, production output batches
(every credit or none), allocation passes, completion operations, and the
completion expiry tick.

A detached caller of `ResourceState::apply_operation` has no packet to skip.
It receives `ResourceError::LimitExceeded`, with the root unchanged, as
before; only the amount of room asked for changed.

**The expiry tick stops instead of spinning.** The plugin schedules an expiry
tick at the same instant while expiry is pending. An expiry whose result does
not fit is skipped, and that tick schedules no successor: a tick at the same
instant would meet the same root, and `advance_canonical` would settle
boundaries at that instant without end. An expiry is certified work, so only
an archive commit makes room for it: an applied archive commit schedules one
tick while an expiry is due, and expiry resumes by itself. That tick cannot
spin, because a tick that still finds no room schedules none. The next grant
also schedules a tick, and a host application can queue one through the new
`enqueue_resource_completion_expiry_tick`. One boundary schedules at most one
expiry tick, however many settlements ask for one.

### Alternatives rejected

- *Deferral of every packet.* A packet queued again at the same instant is
  admitted by the next boundary at that instant, which meets the same root, so
  `advance_canonical` never returns while the budget stays short. A packet
  queued for a later instant cannot settle there when it names its boundary
  time: irreversible requests, single steps, acquisitions, and activations
  are bound to it, and a holder's debit must not move. A tracked command
  queued again would no longer carry its command cause. Host packets are
  therefore skipped, and their sender sends them again; only plugin-scheduled
  work, whose sender may not, is deferred (below).
- *Record a `limit_exceeded` rejection.* It needs the very slot that is short,
  and it takes the key, so the issuer would have to mint a new key for a
  request that was refused only for want of room.
- *Account reservations apart from hot records* (a separate reservation
  budget). The reservations are what guarantee that certified work's records
  fit the same candidate budget that bounds the root and one archive commit; a
  separate pool would need a new field of `ResourceLimitsV1`, which the root
  persists, so the root's shape would change, and it would move room between
  two budgets without adding any. The host application raises
  `max_archive_candidates` instead (up to 4,096, room for about 250
  concurrently active leases), and the `ReservedCapacity` reason tells it that
  an archive commit will not help.

### When the budget stays short for many boundaries

Every boundary settles archive commits and retention acknowledgements first,
then every other resource packet in admitted order. Each packet whose result
does not fit is skipped and named by one event; each packet whose result fits
settles. Nothing is queued, persisted, or retried on a skipped packet's
behalf, so nothing accumulates in the root, the ingress queue, or the
archive, however long the shortage lasts, and a skipped packet cannot delay
or starve another: the next packet is checked against the same root. Work a
lease of the resource plugin certifies keeps settling within the room that
lease reserved; the credits of a production output batch, whose participant
reserves hot outcome slots but no candidate slots, can find no room, and are
deferred while a commit or a release can make room. No boundary fails for a
short budget. Work that can never fit is skipped at once, and nothing waits
without bound on a shortage that no commit or release can end. Its cost is one event per
skipped packet, the requests the host application or the holder sends again,
and one small packet per retry of each deferral. While a deferral waits the
run cannot seal; an archive commit ends a wait on the backlog, and a lease's
completion, release, or expiry a wait on reserved room.

What the host application does:

- `ArchiveBacklog`: queue one archive commit of every candidate and send the
  skipped requests again, in the same boundary if it likes (commits settle
  first). A commit makes room for the budget minus one record; the requests
  beyond that are skipped again and settle behind the next commit.
- `ReservedCapacity`: complete or release leases, or raise
  `max_archive_candidates`.
- An expiry skipped for room resumes with the archive commit that makes
  room; nothing else is needed.
- Deferred work settles when it comes due after a commit or a release has
  made room: queue the commit, or complete or release the leases, that the
  reason names. Plugin-scheduled work skipped for room (it can never fit, or
  the byte cap stayed short through its longest delay) does not come back by
  itself; its execution stays pending, as described below.

### Plugin-scheduled work is deferred

A packet whose cause is a boundary was scheduled by a plugin's boundary
system; the kernel refuses that cause on ingress a host application queues.
Two kinds of such work have no sender that would send it again: the output
batch the production plugin dispatches once for a completed execution, and
the completion steps a plugin schedules, such as the production plugin's
`CompleteExternalParticipant`.

Only a completion step that can settle at a later instant as the same step is
deferred: `Prepare`, `Abort`, `Release`, `PrepareExternalParticipant`,
`CompleteExternalParticipant`, and `ReleaseExternalParticipant`. A preparation
names a boundary only for its activation guard, which gives the same answer
later, and the activation that follows checks the grant's deadline at its own
boundary. The steps bound to their boundary time are skipped like a host
packet: an `Acquire` (eligible at its boundary), a `Grant` or
`GrantExternalParticipant` (expiry counted from its boundary), an `Activate`
or `ConsumeExternalParticipant` (must name its boundary time), an `Expire` or
`ExpireExternalParticipants` (the same), and a single step. Settled later, each would fail its time checks
or count its expiry from the wrong boundary.

Whether the wait is worthwhile follows from the shortage. An archive backlog
waits until an archive commit makes room, room reserved by leases until they
complete, are released, or expire, and room held for finished participants
whose reports wait until the reports publish and a commit clears them; a
production output batch meets that last shortage when many completions wait
on delayed reports at once, and skipping it would leave its execution
pending for good. A permanent `ReservedCapacity` is
skipped at once: waiting cannot make room. A `RootLimit`, in practice the
operations' share of the byte cap, may pass or may not, so the work waits until its delay has reached
the cap of 1,024 minutes (eleven deferrals, about 34 hours in all) and is
skipped when it still finds no room after that wait.

When such a packet's result does not fit and the wait is worthwhile, the
resource plugin defers it:

- It queues the same work to itself as one `RESOURCE_DEFERRED_INGRESS`
  (`resource_deferred_packet_v1`) packet, an internal ingress no host can
  queue, that carries the packet type, the payload, and the count of its
  deferrals.
- The packet comes due one minute after the first deferral, and the delay
  doubles with each further deferral up to 1,024 minutes. It is never the
  same instant, so no instant loops, and a shortage of any length costs one
  small packet per retry: at most a little over one a day once the delay has
  stopped growing. Work waiting on a backlog or on reserved room is retried
  until the commit or release that ends the shortage; work waiting on the
  byte cap is skipped after its 1,024-minute retry.
- A production output batch is retimed to the instant it is due: every
  credit's `at` becomes that instant, which the batch's checks require. The
  production plugin's acknowledgement checks the credits' keys, accounts,
  quantities, statuses, and evidence, not their time or request digest, and
  the resource plugin checks the batch against the execution's output
  requests (key, account, account revision, resource, unit, quantity), which
  do not include the time either.
- The skip event says `deferred to minute {minute}` and names in its detail
  the holder of the external participant the work belongs to.
- The acknowledgement the resource plugin schedules to the production plugin
  once a batch settles carries `settled_at`, the instant the credits settled:
  the instant the batch was dispatched, or the instant a deferred batch came
  due. It is set on every acknowledgement. The production plugin's
  acknowledgement type reads the fields it knows, so the field is additive.
- While a deferral waits, the ingress queue is not empty, so the run cannot
  seal. The production plugin reads the retained body of the source version
  it pinned on the execution when the acknowledgement settles, and that body
  stays until the acknowledgement has settled. (Making that read
  independent of a seal belongs to the production plugin.)

Plugin-scheduled packets on the other ingresses are not deferred. Adapter
packets, which the force-supply reference schedules, are skipped: that plugin
dispatches again every consumption that has no outcome, and a deferred copy
would settle beside the one it sends again. Plugin-scheduled packets skipped
for any reason other than room are skipped, not deferred, because waiting
does not change the reason; the event names the participant's holder in its
detail.

What a deferred production output batch cannot survive, stated precisely:
the execution's output requests pin each output account's revision, and the
participant's certificate locks it, but a lock does not stop an operation
that needs no lease (a `SetProtectedFloor`) or a debit or credit under
another lease that locked a later revision from changing that revision. When
such a change happens while the batch waits, the batch's credit is refused
when it comes due, the batch is skipped, and the execution stays
`CompletedPendingOutputSettlement`. The same holds for an output key another
request took first: the batch is skipped as `operation_key_taken`, and since
no wait frees a key, the execution stays pending, its participant grant stays
`Consumed` and holds its completion units, target locks, and reserved hot
outcome slots. It holds too for a batch skipped for room: one whose credits
can never fit the candidate budget, or one the byte cap kept out through its
longest delay. All of these are prevented on the production side
(re-pinning outputs, refusing at execution start an output key that is
already used, or an execution with more outputs than the budget holds), not
by waiting.

### Invariants

- The root after a boundary is the root before it with exactly the settled
  packets applied in admitted order; a skipped packet contributes nothing.
- A skipped packet records nothing under its key. A request skipped for room
  settles under the same key when sent again; a request skipped because its
  key is taken never does.
- Hot terminal records never exceed `max_archive_candidates`, hot outcomes
  never exceed `max_operation_outcomes`, and work not certified by an admitted
  lease leaves every reservation free, as before.
- The skip decision reads only the root, the current-version index, live
  bodies, and the archive the root names, so a live run, a sealed run, a
  restore, and an exact replay skip the same packets with the same events.
- Plugin-scheduled work that does not fit is deferred only while waiting can
  make room: always for an archive backlog or reserved room, and for at most
  eleven deferrals on the byte cap. It is never retried at the same instant,
  and each deferral leaves one packet in the queue. Work no wait can fit, and
  completion steps bound to their boundary time, are skipped at once, so a
  deferral stays queued without bound only while an archive commit or a
  lease release the host application can make would let it settle.
- Operations leave the maintenance reserve of the byte cap free.

## Item 2: operation keys another issuer used first

### Problem

0.15.0 made a tracked command or a completion packet whose key a different
request held settle nothing. Allocation passes, adapter packets, and
production output batches still failed their boundary for good: their free
keys can be taken by any other request, for example a holder's tracked
command.

### Decision

The same skip on every ingress that settles operations. Before any check of
its own, each packet resolves who holds its key (`operation_key_holder`, root
then archive). A key this request holds is a resend and settles nothing
silently; a key another request holds is skipped with `OperationKeyTaken`. A
production output batch is checked credit by credit: one credit whose key
another request holds skips the whole batch; a batch whose credits are all
recorded for it settled before; and a batch with only some of them recorded,
which no settlement leaves, is skipped as failing its checks. A key under the
reserved prefix is skipped with `ReservedOperationKey` on every ingress, as it
was silently skipped before.

Keys namespaced by issuer were considered and rejected: they would change the
key, the request digest, and the archived records of every existing request
type, and a host application that composes keys today would have to learn a
second scheme, while the skip makes a collision harmless and visible.

A resend now settles nothing on every ingress. In 0.15.0 a tracked command or
a completion operation whose outcome was in the root was applied again: the
root was rewritten with the same outcome, the settlement event was emitted
again, and a resent `Grant` or `CompleteExternalParticipant` scheduled its
expiry tick or production acknowledgement again. An adapter packet resent
after its lease completed failed its boundary. A production output batch
resent after the production plugin had settled its execution failed its
boundary, and one resent before that was acknowledged again.

## Item 3: a trace for every skipped packet

Every skip, on every ingress, emits the skip event described above: the
ingress id names the packet (its record in the ingress log holds the payload,
and the command that produced it for a tracked command), and the reason says
whether to send it again. A host application no longer compares the request
digest of the outcome under a key with its own to learn that its request did
not settle, although that comparison stays valid.

The event type is registered with the `AffectedActors` audience and declared
in the lifecycle system's `emits`. That is the descriptor change, and the
plugin's semantic hash changes with it.

## Item 4: finalizing retention after a later commit archived the receipt

### Problem

`finalize_resource_archive_retention` read the commit's disposition from its
maintenance receipt. The receipt of an applied commit is archived by a later
commit, so a call made after that failed host-side, and the state-side handle
and the store-side `DurableIngress` handle stayed for the life of the run.

### Decision

The disposition is read from the state-side retention handle, which stays in
the root until its acknowledgement settles and records how the commit settled
(`Committed` once any commit of the batch applied, `RejectedStale` otherwise).
When the root still holds the receipt, the acknowledgement names it, as
before. When it does not, the acknowledgement names the handle by id and
directory root instead, and settlement retires that handle and every stale
receipt recorded under it. A handle the root no longer holds was
acknowledged before, so a repeated acknowledgement settles as a no-op.

Reading the handle first also corrects one case of 0.15.0: the same batch
queued twice settles once as applied and once as stale under one handle; if
the applied receipt had been archived and the stale one had not, finalize
read the stale receipt and finalized the store-side handle as
`RejectedStale` although the commit applied.

A call still fails host-side, without touching any boundary, when the commit
has not settled, and when both the handle and the receipt are gone because an
earlier call was acknowledged and a later commit archived the receipt. In
that case there is nothing left to finalize.

## Item 5: a tracked `Acquire` on the completion ingress's checks

### Problem

The completion ingress checks an `Acquire` against the simulation view: its
eligibility time is the boundary time, its reports fit the hot report
capacity, and every record version its eligibility envelope persists is the
current version of its record. A holder's tracked `Acquire` skipped all
three, so it could create an acquisition eligible at another time, or one
that persisted evidence that was never current.

### Decision

The tracked command is checked at admission and again at settlement:

- **Admission** refuses an `Acquire` whose eligibility time is not the
  admission time, the instant its packet settles, and one whose envelope
  evidence is not the current version of its record. The refusal is recorded
  on the command (`InvalidAuthority`).
- **Settlement** runs the checks the completion ingress makes of the same
  operation (`validate_completion_ingress`), the report reservation included.
  A packet that fails them is skipped with `FailedSettlementChecks`.

The completion ingress itself skips a four-step step that fails its checks
(an acquisition or grant that is not in the root, a target version that is
not current, a coordinator payload that does not authorize it, a time that is
not the boundary time) instead of failing the boundary. A single step keeps
its 0.15.0 rule: what the plugin finds against the view is the step's recorded
rejection, which now needs room for its outcome only.

**An activated lease still never expires.** Expiring or reclaiming an
activated lease was considered and not adopted. An active lease's
certificate may already be carried by an admitted command or a queued packet;
expiring it would turn that certified debit into a rejection at settlement,
which breaks the guarantee that an admitted lease cannot strand its certified
work, and a time limit would be a new policy every host application would
have to choose. The holder cannot activate a lease by itself (the grant,
prepare, and activate steps and the single step enter only through the
completion ingress), so the host application that activated a lease can
always release it with `Release`, as before. This is listed under open
decisions.

**A `Requested` acquisition holds its reservation until it is granted or
aborted.** A holder's tracked `Acquire` reserves sixteen slots of each budget,
and an acquisition that no grant follows does not expire, so a holder can hold
`reserved_capacity` for as long as it likes. How many such acquisitions there
are is bounded by the holder's partition (`reserved_pending_slots`) and by the
global pending-acquisition bound. A host application that sees
`reserved_capacity` skips aborts the stray acquisitions, as it already does
for the fairness order (see the single-step lease proposal).

**An applied step only sets anything in motion.** What a completion step sets
in motion after it settles (the coordinator's acknowledgement of a completed
external participant, the expiry tick a grant starts, and the tick that
cleans up an acquisition a prepare left aborting) follows only an outcome
that was newly recorded and applied. A refused `CompleteExternalParticipant`,
for an unknown participant or one whose grant is not yet consumed, is a
recorded rejection that sends the coordinator nothing; before this change the
first failed its boundary on every retry, and the second told the production
plugin that the participant had completed, which production refused, failing
its boundary.

## What can still fail a boundary

The resource plugin's boundary systems fail a boundary only for these
reasons. None is something an honest host application or a holder triggers
by sending or racing packets:

1. **An archive object that is missing, forged, oversized, or malformed**
   when a packet's key is looked up in the archive: the head directory, a
   lookup page, or the blob a hit names. A miss is decided by authenticated
   pages only, so this is never taken for a free key. It settles once the
   store can serve the object.
2. **A forged archive packet**: an archive commit that does not decode or
   whose verified batch does not validate, or a retention acknowledgement
   that does not decode, contradicts the receipt the root holds under its
   sequence, or names a handle with another directory root. Both are internal
   ingress: only `enqueue_resource_archive`,
   `finalize_resource_archive_retention`, or a plugin that declares the
   packet type among its ingress targets queues them.
3. **A root that does not validate after settlement** for a reason other
   than a limit (conservation, closure, index, or lease-book consistency), in
   the lifecycle system or in the invariant systems of phases 8 and 12. This
   is an engine invariant failure: every operation was validated as it
   settled.
4. **Maintenance writes past the byte reserve.** Operations leave
   `MAINTENANCE_STATE_BYTE_RESERVE` (64 KiB, or a quarter of a smaller cap) of
   the state byte cap free, and the writes that are not operations use it:
   the receipt and retention handle of a stale archive commit (about 4 KB for
   sixteen pages) and the report system's scheduling state. Sixteen stale
   commits of sixteen pages in one boundary fit the canonical reserve. A
   boundary whose stale commits and report writes together pass it fails,
   which takes a host queuing many commits prepared from the same root.
5. **Arithmetic, encoding, or kernel failure**: a revision, sequence, or
   count that overflows `u64`, a root that does not encode, or a kernel error
   reading the boundary's own admitted ingress or the plugin's own declared
   state.
6. **An applied external participant completion that leaves no participant
   in the root.** The follow-on acknowledgement looks the participant up only
   after the completion applied, and an applied completion keeps the
   participant among the terminal grants, so this is reached only through an
   engine defect. A refused completion sets nothing in motion (item 5).

Admission failures are command rejections, never boundary failures.

## Persistence, replay, and formats

- **Plugin state.** The shape of `ResourceState` and
  `RESOURCE_STATE_FORMAT_VERSION` (2) are unchanged. A skipped packet writes
  nothing. A resend no longer rewrites the root, so a run that resends a
  settled tracked command or completion operation has a different root
  revision, boundary records, and hashes than in 0.15.0; a run without
  resends or skips settles to the same roots, except where 0.15.0 failed a
  boundary.
- **Ingress.** The retention acknowledgement payload is now one of two
  forms, `{"receipt": ...}` as before or `{"handle": {"retention_handle_id",
  "directory_root"}}`; the packet type keeps its name, as 0.15.0 kept the
  names of the shapes it changed, and the exact engine-version check keeps a
  0.16.0 run from reading a 0.15.0 journal. One internal ingress type is new,
  `resource_deferred_packet_v1` (`ResourceDeferredPacketV1` inside the plugin:
  packet type, payload, deferral count). The output acknowledgement the
  resource plugin schedules to the production plugin
  (`production_output_ack_v1`) gains one field, `settled_at`, set on every
  acknowledgement. No other packet changes; a deferred production output batch
  carries its credits retimed.
- **Events.** Skips are ordinary plugin events
  (`canwu.resource.ingress_skipped.v1`) in the boundary's emissions. They are
  evidence, sealed and replayed like every event.
- **Plugin identity.** `RESOURCE_SEMANTIC_HASH` changes: the lifecycle system
  declares the new event type and the deferred ingress as a target, and the
  plugin registers the event's audience and the deferred ingress. Commands,
  the other ingress types, record schemas, and the report knowledge schema
  are unchanged.
- **Kernel formats.** Snapshot, commitment, checkpoint, evidence-journal, and
  paged-checkpoint formats and the hash algorithms are unchanged.
- **Replay.** A skip decision reads only the root, the current-version index,
  live bodies, and the archive the root names. The fixture
  `skips_agree_across_a_seal_a_fork_a_restore_and_an_exact_replay` settles
  skips for room, for a taken key, and for a forged command packet after a
  seal, and the sealed run, an unsealed fork, a run restored from the sealed
  segments, and an exact replay agree on checkpoint, snapshot, and events.
  `a_plugin_scheduled_output_batch_without_room_is_deferred_until_it_fits`
  defers a production output batch twice, makes room, settles it retimed,
  then seals; the sealed run, an unsealed fork, and an exact replay of the
  sealed run agree as the batch is sent again. A deferral is queued ingress,
  persisted and replayed like any other.

## Durable tests

Each fails on 0.15.0, and each was checked to fail with its fix reverted in
the working tree.

- `resource_budget_skip.rs`: more operations in one boundary than a commit
  makes room for are skipped and all settle, exactly once, as they are sent
  again behind further commits; a single-step lease without room beside the
  reservations of active leases is skipped as `reserved_capacity`, an archive
  commit does not help, a refused lease is recorded with room for its outcome
  only, and a release makes room; an expiry without room is skipped,
  schedules no tick at the same instant, and resumes with the archive commit
  that makes room; a production output batch the production plugin
  scheduled is deferred twice and settles retimed once a commit makes room,
  its acknowledgement carrying the instant it settled, across a seal; a batch
  of as many credits as the candidate budget holds is skipped at once as
  `reserved_capacity`, the queue empties, and the run seals; a batch that does
  not fit beside the slot of a finished participant whose report waits is
  deferred as `reserved_capacity`, and settles behind a commit once the report
  publishes; a batch the byte
  cap keeps out is deferred eleven times, to minute 2,047, and then skipped as
  `root_limit`, leaving nothing queued; a
  `CompleteExternalParticipant` a plugin scheduled is deferred, settles behind
  the commit that makes room, and its acknowledgement follows; a
  plugin-scheduled `Expire`, bound to its boundary time, is skipped as
  `archive_backlog` and not deferred; a refused `CompleteExternalParticipant`
  sets nothing in motion; a refusal and an applied operation past the byte
  reserve are taken back exactly and skipped as `root_limit`; and the seal,
  fork, restore, and replay fixture named above.
- `resource_ingress_skip_trace.rs`: a key another request holds, in the root
  and in the archive, skips a tracked command, an allocation pass, an adapter
  packet, a production output batch, and a single step, each with its event
  and holder; a reserved key, an undecodable payload, a resource command
  packet no command produced, an unauthorized production output batch, and a
  completion step for an unknown acquisition are skipped with their reasons
  and change nothing; an allocation pass for a requester that is not an
  entity of the run is refused when queued; and a resend of a tracked command
  or of a grant already recorded does not write the root again or schedule
  another tick.
- `resource_archive_late_finalize.rs`: a commit finalized after a later
  commit archived its receipt, then restored and replayed.
- `resource_tracked_acquisition_checks.rs`: a tracked `Acquire` eligible
  later, or citing evidence that is not current, is refused at admission, and
  one whose reports cannot be reserved is admitted and then skipped at
  settlement, recording nothing.

Four 0.15.0 tests that asserted a failed boundary now assert the skip:
`an_archive_commit_settles_ahead_of_an_operation_queued_before_it`,
`canonical_external_participant_ingress_requires_exact_coordinator_payload_and_replays`,
`installed_consumer_settles_once_and_survives_restore_and_exact_replay` (a
resend now settles nothing), and
`malformed_authorizations_and_forged_packets_roll_back_without_spending_lease`.

## Compatibility

0.16.0 is a pre-1.0 minor release.

- **Behavior.** A resource packet that does not fit the root, whose key
  another request holds, or that fails its settlement checks no longer fails
  its boundary; it is skipped with an event. Tests or hosts that relied on the
  failure must read the event instead.
- **Room.** `ResourceState::apply_operation` measures room on the result, so
  it settles some operations 0.15.0 refused for room, refusals in particular;
  its `LimitExceeded` messages for room changed.
- **Resends** settle nothing on every ingress (see item 2).
- **Admission.** A tracked `Acquire` must be eligible at the time it is
  admitted and cite current evidence.
- **Finalize.** `finalize_resource_archive_retention` succeeds after a later
  commit archived the receipt, and reads the disposition from the state-side
  handle first.
- **Events and allocation.** The resource settlement event names only
  admitted, registered holders, the skip event names an external
  participant's holder in its detail only, and `enqueue_resource_allocation`
  refuses a requester that is not an entity of the run.
- **Deferral.** Production output batches, and completion steps a plugin
  scheduled that are not bound to their boundary time, are deferred when they
  do not fit and waiting can make room, and skipped otherwise; adapter
  packets a plugin scheduled are skipped. An applied archive commit schedules
  an expiry tick while an expiry is due. Operations leave the maintenance
  reserve of the byte cap free.
- **Production acknowledgement.** The output acknowledgement the resource
  plugin schedules to the production plugin carries a new `settled_at` field,
  the instant the credits settled.
- **Added API.** `RESOURCE_INGRESS_SKIPPED_EVENT`,
  `ResourceIngressSkipReasonV1`, `ResourceIngressSkipV1` (with
  `deferred_until`), `RESOURCE_DEFERRED_INGRESS`,
  `ResourceLimitsV1::MAINTENANCE_STATE_BYTE_RESERVE` and
  `ResourceLimitsV1::operation_state_bytes`, and
  `enqueue_resource_completion_expiry_tick`. Nothing public is removed and no
  signature changes; the room classification (`RoomShortage`) stays internal
  to the crate.
- **Semantic hash.** `RESOURCE_SEMANTIC_HASH` changes; as with any version
  change, the exact engine-version and plugin-identity checks reject a 0.15.0
  save.

Statements of the 0.15.0 proposals that no longer hold:

- [Archive lookup and volume](resource-archive-lookup-and-volume.md): an
  operation that does not fit the budget "fails its boundary and stays
  queued", and "operations due at one boundary that need more than [the
  budget minus one] cannot settle". They are skipped, and settle when sent
  again behind further commits. A call to `finalize_resource_archive_retention`
  made after a later commit archived the receipt no longer fails, and leaves
  no handle behind.
- [Single-step completion lease](resource-single-step-lease-and-floor-install.md):
  "no room" and "the room is asked for even when the request would be
  refused" no longer fail the boundary; a refused single step needs room for
  its outcome only and one that does not fit is skipped. A skipped packet
  leaves an event of its own, so "the skip leaves no record of its own" and
  "a skipped command stays `Accepted`" without trace no longer hold; the
  command still stays `Accepted`. Allocation, adapter, and production output
  packets whose key another issuer used first are skipped, not failed.

## Open decisions

- **Expiring activated leases.** An activated lease still never expires (item
  5); a time limit would be a new policy for every host application.
- **Pending executions after a skipped output batch.** A production output
  batch skipped for room, for a taken key, or for an output account that
  changed revision leaves its execution `CompletedPendingOutputSettlement`
  and its participant grant `Consumed`. Preventing these belongs to the
  production plugin (re-pinning outputs, refusing at execution start an
  output key already used or more outputs than the budget holds).
- **Waits with no host action.** Plugin-scheduled work waiting on an archive
  backlog or on reserved room is retried, one small packet a day at most,
  until the host application commits or releases; the run cannot seal
  meanwhile. Whether such a wait should also end after a bound is open.

## Measurements

The change is not a performance change, but it reorders the hot path of every
operation: the operation is applied before room is checked, and a rejection
is taken back from its recorded position when it passes a limit, instead of
from a copy. The number of whole-root copies per operation is unchanged (one)
and so is the number of whole-root validations (one). The measurement below
was taken before deferral, the byte reserve, and whole-batch room were added;
none adds work to an operation that fits (the reserve changes the number the
encoded root is compared with, and a production output batch measures room
and validates once for the batch instead of once per credit).

The same workload was measured before and after: the `backlog_cases` of
`resource-archive-profile` (`benchmarks/performance-harness`, smoke preset),
in which each sampled boundary settles one requester-scoped allocation pass,
a rejection, against a root holding 0, 256, or 1,024 hot terminal records.
Release builds; "before" is the same binary built from `v0.15.0` (`df945ef`).
The two binaries ran alternately, three times each, on the shared 16-core
machine while other builds were running; the table gives the median of the
three runs, each run itself the median of five boundaries.

| Hot terminal records | One boundary, before / after | Detached operation, before / after | Journal bytes per boundary, before / after |
| --- | --- | --- | --- |
| 0 | 0.65 ms / 0.44 ms | 0.028 ms / 0.021 ms | 9,059 / 9,059 |
| 256 | 17.6 ms / 14.7 ms | 1.04 ms / 0.68 ms | 297,051 / 297,051 |
| 1,024 | 87.0 ms / 65.3 ms | 3.37 ms / 2.80 ms | 1,165,195 / 1,165,195 |

The journal bytes are exact and equal: the boundary records the same root and
the same evidence as before. The times vary by up to 40 % between runs of one
binary on this machine, so they show no regression and support no claim of a
speed-up. The raw reports are not kept under `benchmarks/baselines/`, since
this change makes no performance claim.
