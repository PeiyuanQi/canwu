# Seat Rebinding

Status: implemented in 0.23.0 (package commit `99b4fcb`, from branch
`fix/g97`). It closes downstream gap G-97 (`canwu-sim`, `canwu-api`): a run
names its seat's person once, at run start, and had no way to move the seat
to another person while it runs. When the seat's character died and an heir
took the seat, the engine still treated the dead person as the human seat.

## Problem

`RunConfiguration::seat_binding` (`SeatBinding { seat_id, controller_id,
actor, institution, permission_profile_id }`) is the human seat of a declared
run. It is fixed for the whole run: `RunManifest` binds the configuration's
semantic hash, `manifest::validate_run_configuration` checks it on load, and
nothing changes it. Everything that follows the seat reads it:

- the human-only command gate (`validate_command_ingress_policy`): an
  `Issuer::Human` command must name the binding's controller, seat, and
  permission profile, and its decision origin must match the binding's actor
  or institution;
- the actor-bound viewer (`Canwu::viewer`, `Canwu::viewer_for_actor`): the
  principal is the binding's actor or institution;
- snapshot validation, which re-checks every recorded command attempt against
  the same gate.

When the seat's person dies, the existing availability path cancels the
tickets the person decides or whose controller acts for them
(`decision_maker_unavailable`, `controller_authority_unavailable`), and
seat succession (`DecisionTicketDraft::parent_ticket` across controllers
bound to one `seat_id`) lets a successor's controller continue a decision.
But the run's seat still binds the dead person: the heir's human commands
are refused by the seat gate, the dead person stays the actor-bound viewer,
and no call moves the seat.

## Decision

A **seat rebinding** is host-authored canonical ingress that moves the run's
seat to another existing person at the current instant.

1. **Request.** `SeatRebindingRequest { seat_id, actor, controller_id,
   permission_profile_id, expected_revision, expected_time }`. The seat's
   identity and institution never change. `controller_id` and
   `permission_profile_id` are optional and keep the current values when
   absent: a successor whose decision controller is registered under
   another ID (controller bindings are immutable) names it.
2. **Ingress.** `rebind_seat(request)` on `Simulation`, `CompactedSimulation`,
   `Canwu` (through `gated!`), and `CompactedCanwu` appends one
   `IngressPayload::SeatRebinding { request }` record of class `Command`,
   priority 0, no cause, due at `expected_time`, which must be the current
   time. The call fails, queuing nothing, before the runtime is ready, after
   direct command history (`MixedCommandIngress`), in a read-only run
   (`InteractionReadOnly`), in a run without a declared seat binding, for
   another seat, or for a seat bound to an institution without a person
   (`InvalidAuthority`), for non-canonical controller or profile
   IDs (`InvalidPayload`), while another rebinding of the seat is queued
   (`IdempotencyConflict`), and for a stale time or revision
   (`SimulationTimeConflict`, `SimulationRevisionConflict`). An identical
   queued request returns its receipt.
3. **Admission.** The boundary that admits it decides every admitted seat
   rebinding, in admission order, **before any other ingress it admits**,
   from the state committed before the boundary: the binding in force, the
   revision, the person registry, and person availability. The outcome is
   one of:
   - **Applied.** The new binding is in force from this boundary. The open
     tickets whose assigned controller is bound to the seat
     (`DecisionControllerBinding::seat_id`) and acts for the previous person
     (its authority person, as the availability path defines it) are
     cancelled with `seat_rebound` (`SEAT_REBOUND_REASON`), in ticket-ID
     order, through the same cancellation routine as the availability
     sweep. Only a change of person cancels tickets; a rebinding that keeps
     the person and changes the controller or profile cancels none.
   - **Rejected**, changing nothing else: an unknown person
     (`ActorNotFound`), a dead or missing one (`IssuerUnavailable`), a stale
     revision (`SimulationRevisionConflict`), or the binding already in
     force (`InvalidPayload`).
   A refusal is a recorded rejection, never a failed boundary. Custody does
   not matter: a detained or captive person may hold the seat, and the
   command gates refuse their commands (`IssuerUnavailable`) until they are
   free, as for any unavailable issuer.
4. **Evidence.** `BoundaryRecord::seat_rebindings` and
   `BoundaryReceipt::seat_rebindings` list `BoundarySeatRebinding { ingress,
   seat_id, outcome }` in admission order, with `SeatRebindingOutcome::
   Applied { previous, binding, cancelled_tickets }` or `Rejected { error }`.
5. **The binding in force.** `seat_binding()` on every handle returns the
   last applied rebinding's binding, or else the configuration's own. The
   human-only command gate, the actor-bound viewer, and snapshot validation
   of every command attempt read it. The run configuration and its manifest
   stay as declared. A replay-purpose run is read-only, so it never rebinds
   and keeps the configured binding.
6. **Persistence.** The binding in force is runtime metadata, persisted as
   `SimulationSnapshot::rebound_seat` (absent while the configuration's own
   binding is in force, so a rebinding back to it leaves no field) and
   bound by the checkpoint hash.

## Invariant

- The binding in force is a function of the configuration's binding and the
  applied rebindings of the journal: rebuilt boundary by boundary, it equals
  the persisted `rebound_seat` (normalized to absent when it equals the
  configuration's binding).
- Every command attempt a boundary admits meets the binding in force after
  that boundary's rebindings; any other attempt meets the binding after all
  boundaries. A run with direct command history never rebinds.
- A rebinding's outcome, and its cancelled tickets, are a deterministic
  function of the state committed before its boundary and of the
  rebindings decided before it in the same boundary.

## Ownership and affected surfaces

| Surface | Change |
| --- | --- |
| `canwu-sim` `runtime/seat.rs` (new) | The request and evidence types, the shape check, the shared admission rule, the ticket selector, the seat history snapshot validation rebuilds, `Simulation::seat_binding`, `Simulation::rebind_seat`, settlement's `apply_seat_rebindings`. |
| `canwu-sim` ingress, settlement, policy | The new `IngressPayload` variant; admission before other ingress; `validate_command_ingress_policy` takes the binding in force. |
| `canwu-sim` state, transactions, persistence, continuation | `RuntimeMetadata::rebound_seat`, captured and restored by the boundary transaction; `SimulationSnapshot::rebound_seat`, written by `checkpoint_state` and read by `assemble_runtime` (full, checkpoint-journal, paged, and continuation restores). |
| `canwu-sim` hashing | Boundary-record hash material gains `seat_rebindings`, checkpoint hash material gains `rebound_seat`; both omitted while empty. |
| `canwu-sim` validation, replay | Ingress-record shape (full and continuation); per-boundary reconstruction of every outcome and its cancellations on the rebuilt decision state; the persisted binding against the rebuilt history; per-attempt seat gate; exact replay re-enqueues through `rebind_seat`. |
| `canwu-sim` boundary window | `BoundaryIngressKind::SeatRebinding`. |
| `canwu-api` | Re-exports; `Canwu::rebind_seat` (through `gated!`), `CompactedCanwu::rebind_seat`, `seat_binding()` on both; the viewer principal reads the binding in force. `CanwuRun` is unchanged. |
| Extensions, `canwu-debug`, examples | None. |

## API surface

New public items (re-exported by `canwu-api`):

```rust
pub const SEAT_REBOUND_REASON: &str = "seat_rebound";

pub struct SeatRebindingRequest {
    pub seat_id: String,
    pub actor: PersonId,
    pub controller_id: Option<String>,
    pub permission_profile_id: Option<String>,
    pub expected_revision: u64,
    pub expected_time: SimTime,
}
impl SeatRebindingRequest {
    pub fn new(seat_id: impl Into<String>, actor: PersonId,
               expected_revision: u64, expected_time: SimTime) -> Self;
    pub fn with_controller(self, controller_id: impl Into<String>) -> Self;
    pub fn with_permission_profile(self, permission_profile_id: impl Into<String>) -> Self;
}

pub struct BoundarySeatRebinding {
    pub ingress: IngressId,
    pub seat_id: String,
    pub outcome: SeatRebindingOutcome,
}

pub enum SeatRebindingOutcome {
    Applied {
        previous: Box<SeatBinding>,
        binding: Box<SeatBinding>,
        cancelled_tickets: Vec<DecisionTicketId>,
    },
    Rejected { error: CanwuError },
}

// Simulation, CompactedSimulation, Canwu, CompactedCanwu:
pub fn rebind_seat(&mut self, request: SeatRebindingRequest)
    -> Result<IngressReceipt, CanwuError>;
pub fn seat_binding(&self) -> Option<&SeatBinding>;
```

Changed public items: `IngressPayload::SeatRebinding { request:
Box<SeatRebindingRequest> }`, `BoundaryIngressKind::SeatRebinding`,
`BoundaryRecord::seat_rebindings`, `BoundaryReceipt::seat_rebindings`,
`SimulationSnapshot::rebound_seat`. Neither enum is `#[non_exhaustive]`, and
the structs are built by literal, so an exhaustive match or a literal outside
the workspace breaks: a pre-1.0 minor change.

## Persistence and replay

No format number changes: snapshot `9`, commitment `8`, checkpoint/evidence
journal `6`, paged checkpoint `7`, continuation checkpoint `4`, state
revision `3`. A run that never rebinds writes the same bytes and the same
state, boundary, and checkpoint hashes as before: every new field is omitted
while empty, and so is every new hash member. A run that rebinds writes:

- an ingress record `{"type":"seat_rebinding","request":{...}}`;
- `seat_rebindings` on the boundary that admitted it, a member of that
  boundary's hash material (`canwu.boundary-record.v1`) and therefore of the
  boundary chain and the authoritative state hash;
- `rebound_seat` on the snapshot (and so in checkpoint journals, paged
  checkpoints, and continuation checkpoints) while a rebound binding is in
  force, a member of the checkpoint hash material (`canwu.checkpoint.v4`).

The binding stays out of the commitment roots and the authoritative state
hash, as the run configuration does ("seat policy is save identity, not
result identity"); the journal roots already commit to the rebinding
itself. Within one engine version, a full snapshot or a checkpoint journal
rebuilds the binding from its boundaries and requires the persisted one; a
continuation checkpoint, which holds no history, checks the binding's shape
and recomputes the checkpoint hash, which binds it. A full snapshot without
ingress or boundaries is at genesis or holds only direct command history,
so it must hold no rebound binding. Because a continuation's authoritative
state hash does not cover the binding, a host restores a continuation
through `from_continuation_checkpoint_verified` with the continuation hash
it stored at the save; pinning only the state hash does not protect the
seat. Exact replay re-enqueues
the record through `rebind_seat` and compares every regenerated boundary,
so it reproduces the outcome, the cancellations, and the binding. Following
the same-version policy, nothing written by an earlier release is read; an
earlier engine refuses a save holding a rebinding (unknown variant or
field), as it refuses every save of another version.

## What can still fail

- The call's refusals above, with nothing journalled.
- A recorded rejection at admission, as above; the boundary settles.
- Load: `InvalidSnapshot` when a seat-rebinding ingress record is not
  canonical (class, priority, cause, due time, expected time, seat, IDs,
  read-only run), when a boundary's evidence disagrees with its admitted
  rebindings or with the deterministic outcome or cancellations, when the
  persisted binding is malformed or does not follow the history, or when a
  continuation's binding does not reproduce its checkpoint hash.

## Alternatives not taken

- **Mutate the run configuration.** The manifest binds the configuration's
  hash; changing it would change the run's identity and break manifest
  validation. The rebound binding sits beside the configuration instead.
- **A command (`Command::RebindSeat`) through the existing gate.** Who may
  rebind is the host's decision (the gate's own subject), and a command
  would pass the gate it changes. A host ingress keeps the authority with
  the trusted host, as decision and calendar ingress do.
- **Commit the binding in a commitment root.** It would change the
  authoritative state hash's material and contradict its documented
  exclusion of seat policy; the journal roots already commit to the
  rebinding.
- **Decide rebindings in queue order among the other ingress.** A command
  queued before the rebinding in the same boundary would meet the old
  binding; deciding them first makes "from this boundary" exact.
- **Refuse at the call for a dead or unknown person.** The brief requires a
  recorded rejection, and availability can change between the call and the
  boundary; the boundary decides from committed state.
- **Refuse every unavailable person, captives included.** The engine's
  availability rule (`PersonAvailability::is_available`) also excludes the
  detained and the captive. Refusing them would leave a dead person in the
  seat while a living heir is held, and the command gates already keep a
  captive from acting; so only life state decides.
- **Bind the binding in force in the control commitment.** Raised in review:
  a continuation checkpoint carries no history, so the authoritative state
  hash alone does not fix the binding there; the checkpoint hash and the
  continuation hash do. Adding `rebound_seat` to the control commitment
  material would make two continuations with the same state hash also share
  their seat. Not taken: it would make the authoritative state hash cover
  seat policy, which it excludes for the configured binding by design, and
  integrity of a continuation already rests on its own hashes.

## Known limits

- A run with direct command history (`process_command` without canonical
  ingress) cannot rebind (`MixedCommandIngress`); read-only and
  replay-purpose runs cannot rebind (`InteractionReadOnly`), so a
  replay-purpose run keeps its configured binding.
- A boundary-window summary counts a seat rebinding under the command
  class (its kind is `BoundaryIngressKind::SeatRebinding`); a host that sets
  apart player commands filters on kind.
- One rebinding per seat may be queued at a time. A retry after its boundary
  is refused at the call as a stale revision, not answered from history.
- Only the person, controller, and permission profile change; a seat cannot
  be moved to another institution or removed.
- A seat bound to an institution without a person (no `actor` in the
  configured binding) cannot be rebound: the call refuses with
  `InvalidAuthority`, and a persisted rebound binding for such a seat is
  refused on load.
- Tickets are cancelled only for the seat's controllers that act for the
  previous person; the host continues them for the new person through seat
  succession (`parent_ticket`), as after an availability cancellation.
  Controllers stay registered; the engine does not tie a controller's
  `seat_id` to the run's seat binding.
- `CanwuRun` does not gain `rebind_seat`; hosts call it on a handle.

## Tests

One committed test, `crates/api/canwu-api/tests/seat_rebinding.rs`, through
the public API on a declared character-bound run with a seat controller, a
court controller, and a person who dies at the first daily boundary:

- the call refuses another seat and a stale instant;
- (c) a rebinding to an unknown and to a dead person is recorded as
  `ActorNotFound` and `IssuerUnavailable`, and the binding, the seat's open
  ticket, and the holder's command authority are unchanged;
- a queued rebinding answers its own retry and refuses a different one;
- a boundary that applies the rebinding and then fails (a monthly system
  that returns `Err`) leaves the binding, revision, checkpoint hash,
  boundaries, and the seat's ticket as they were, with the rebinding and the
  decrees still queued; this pins the boundary transaction's capture and
  restore of the binding;
- (a) in the boundary that applies the rebinding, the heir's human command
  is admitted and the holder's is refused by the seat gate
  (`InvalidAuthority`), though both decrees are queued ahead of the
  rebinding at a higher priority, which pins deciding rebindings before
  other admitted ingress; the seat's open ticket is cancelled with
  `seat_rebound`, the court's is not; the viewer follows the heir; the run
  configuration is unchanged;
- (d) the boundary record shows the rebinding at its boundary; a full save
  and an exact replay of the journal reproduce the snapshot and checkpoint
  hash; a save whose `rebound_seat` disagrees with its history is refused;
- (b) a continuation saved after the rebinding restores with the heir bound,
  the same checkpoint and authoritative state hashes, and a seat gate that
  still admits the heir and refuses the holder; a changed binding is refused
  by the checkpoint hash even when the continuation hash is recomputed
  over it.

Checked once inline and not committed: a rebinding queued before a
continuation save and decided after its restore; a sealed compacted run that
rebinds back to the configured binding (the field disappears) and restores
from a continuation; a checkpoint-journal restore and exact replay of a run
that rebinds twice; a captive person who takes the seat (applied, the
seat's ticket cancelled, their command refused as `IssuerUnavailable`, the
holder's as `InvalidAuthority`, the save loading and replaying); a
host cancel of the seat's ticket queued ahead of the rebinding in the same
boundary (recorded as a rejection, the ticket cancelled with
`seat_rebound`); a pending rebinding through a full save, exact replay, and
a checkpoint journal; the test under `continuation-gate`; the refusal
reason of each tamper. Two mutations confirm the committed test pins its
claims: skipping the rollback restore of `rebound_seat`, and deciding
rebindings in queue order instead of before other ingress, each fail it.

## Upgrading notes

Code that matches `IngressPayload` or `BoundaryIngressKind` exhaustively adds
the `SeatRebinding` arm. Code that builds `BoundaryRecord`,
`BoundaryReceipt`, or `SimulationSnapshot` by literal adds the new field
(`Vec::new()`, `None`). A host that reads the seat's person from
`run_configuration().seat_binding` reads `seat_binding()` instead.

## Documentation delta

`docs/architecture.md` (seat rebinding after decision lineage; the host
ingress list; the boundary-window ingress kinds; the checkpoint paragraph),
`docs/versioning.md` (the 0.23.0 entry), `docs/README.md` and the
engine-docs documentation map (this proposal), the website settlement pages
(the new ingress kind),
`docs/terminology.md` and both website terminology pages (seat rebinding,
席位改绑), the website decision pages and integration guides in both
languages (the seat gate reads the binding in force; moving the seat to an
heir), and the rustdoc.

## Terminology candidates

Seat rebinding (席位改绑): moving the run's seat to another existing person
while the run continues. Added to the terminology tables.

## Release notes text (for `docs/versioning.md`)

See the 0.23.0 entry of `docs/versioning.md`.

## Review record

An independent review (one reviewer, reading the code and docs without the
author's conclusions) found no correctness, determinism, authority, or
persistence defect: settlement and load reconstruction share the admission
rule and decide from the same inputs, the cancellations and the
end-of-boundary availability sweep agree on both paths, command attempts meet
the binding of their boundary, rollback, fork, and exact replay carry the
binding, and runs that never rebind keep their bytes and hashes. Findings
and their resolution:

1. Blocking: the boundary-window docs listed the ingress kinds without seat
   rebinding. Added to `docs/architecture.md` and both settlement pages,
   with the note to filter player commands on kind, not class.
2. The docs said the frozen-replay gate follows the binding in force; a
   replay-purpose run is read-only and cannot rebind. The claim is removed
   and the limit stated.
3. The control commitment does not bind the binding in force; recorded
   under alternatives, not taken.
4. The forged-continuation check failed on the continuation hash, not on the
   checkpoint hash that binds the binding; it now recomputes the
   continuation hash first. The failed-boundary rollback was probed once
   inline (above).
5. Precision: the continuation path checks shape, not history; the
   `Canwu::rebind_seat` rustdoc lacked the not-ready refusal; the English
   decisions page wording. Fixed.
6. Chinese copy: the viewer is 查看器, not 观察者. Fixed.
7. A detained or captive person was refused, beyond the brief's dead or
   unknown. Changed: only life state refuses (see alternatives).

A second independent review approved the change with nothing blocking. Its
findings and their resolution:

1. Test strength: the decrees in the rebinding's boundary were queued after
   it, so deciding rebindings in queue order would still pass; they now
   queue ahead of it at a higher priority. A failing-boundary step now pins
   the boundary transaction's restore of the binding.
2. Hardening: a full snapshot without ingress or boundaries accepted a
   rebound binding when it held no command attempts; it now must hold none.
3. A seat bound to an institution without a person would have gained an
   actor beside its institution; the call now refuses it
   (`InvalidAuthority`), and the persisted shape check refuses it on load.
4. Docs: the Chinese pages use 上层应用输入 for host ingress, and the
   proposal and integration guides say to restore a continuation through
   `from_continuation_checkpoint_verified` with its stored hash.
