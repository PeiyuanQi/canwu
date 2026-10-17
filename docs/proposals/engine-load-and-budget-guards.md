# Engine Load and Budget Guards

Status: implemented in 0.18.0. Three small `canwu-sim` items from the engine
findings, with their `canwu-api` re-exports: the remaining half of E-27
(initial-scenario declarations no seal can accept), E-31 (the mid-boundary
checkpoint refresh), and E-33 (knowledge publication reservations), and one
bug the request-retention work found: a queued command with request ID 0. No
snapshot, commitment, checkpoint, or journal format changes.

## E-27: initial-scenario declarations no seal can accept

**Problem.** A seal, a continuation checkpoint, and the compact identity a
seal token binds all collect the evidence dependencies of every current
record (`Simulation::evidence_dependencies`). Its per-record checks refuse,
with `ArchiveNotReady`, a record whose payload-required continuation or
identity-only dependencies name an initial-scenario version (which has no
evidence to archive or keep), name the record's own version, are malformed,
or are a completed continuation that keeps dependencies. `Simulation::new`
and every restore accepted such records in the initial scenario, so the run
started, settled boundaries, and could never seal or write a continuation
checkpoint until a boundary rewrote the record.

**Decision.** The same checks run at load, through one shared function
(`validate_initial_evidence_declarations` in `persistence.rs`), so load and
seal cannot disagree:

- **Creation.** `register_plugin`, once the plugin's record schemas are
  known, checks the records of the bound initial scenario the plugin owns.
  Refusal: `InvalidDomainRecord`, "initial-scenario record R declares
  evidence no seal can accept: ...", and the registration rolls back. This
  covers every plugin-aware constructor, an exact replay (which recreates
  the run), and the rehydration of a restore.
- **Restore.** Snapshot validation (the snapshot, checkpoint-and-journal, and
  paged-checkpoint restores) and continuation-state validation check the
  snapshot's initial scenario against the descriptors' schemas. Refusal:
  `InvalidSnapshot`, "snapshot initial scenario is invalid: ...".

Not refused: an initial record that names evidence which does not exist yet
(the seal reports it not ready until it exists, then seals it like any other
dependency), and records written by boundaries (a plugin's rule, unchanged).
Nothing here can fail a boundary. Every first-party scenario, fixture, and
reference world still loads (workspace tests). The continuation gate's
deliberate `a_packet_first_declared_after_its_seal_is_not_ready` declares
identity evidence in a boundary-written record, not an initial one, and keeps
its meaning and its expected panic.

## E-31: the mid-boundary checkpoint refresh

**Problem.** A boundary-generated `ScheduleIngress` (the fiscal drain
packet's) appended its record through `append_ingress`, which called
`refresh_checkpoint_hash` and so hashed every dirty commitment root in the
middle of the boundary; settlement then hashed again at its end.

**Analysis.** The record store's Patricia nodes cache their hashes and the
journal commitments are append-only accumulators, so for record writes the
early refresh is the boundary's own hashing done early: the end hashes only
what changed after it. The work is repeated only for roots dirtied again
after a refresh, and in full for the roots computed whole (plugin
components, world, decisions, scheduler, random streams), once per
generated packet.

**Decision.** Ingress a boundary generates while it settles
(`ScheduleIngress`, `SchedulePluginIngress`, `CancelPluginIngress`, and
boundary-generated decision ingress) is appended by
`append_boundary_ingress` / `append_boundary_ingress_cancellation`, which do
everything `append_ingress` does except the refresh. Host-side ingress
refreshes at once, as before. Two refreshes inside a boundary remain: a
queued command the boundary admits refreshes once per admission (accepted or
rejected), and a command handler's `EnqueuePluginIngress` during that
admission refreshes through `append_ingress`. Both run before the first
phase, when no system has written anything, so they hash little; folding
them into the boundary's final refresh is a possible follow-up.

**Why it is safe.** A scheduled packet's identity is claimed from counters at
the same point as before: its ingress ID, issue time, due time, eligibility
cut (the boundary count plus one), class, priority, payload, cause
`Boundary(id)`, queue key, receipt, and `BoundaryIngressGeneration`. None
carries or depends on the checkpoint hash. That hash is observable only
through host calls after settlement returns; a `SimulationView` exposes
neither it nor the commitment roots. Settlement recomputes it over the whole
state as its last step, and the commitment cache is a memo of state, so the
checkpoint hash, state hash, and boundary hash after the boundary are the
ones the early refresh would have led to; replay, restore, and gate tests
compare them. A refresh fails only on an engine invariant (no commitment
cache, an unsupported commitment format), which the final refresh meets as
well, and a boundary that fails restores the state it started from, ingress
counters and journal included.

**Costs** (release build, shared machine, medians of seven, one boundary
updating 50,000 of 56,891 records in phase 7; "ingress" adds one same-instant
`ScheduleIngress` in phase 13):

| | Wall | Commit | Hashing |
| --- | ---: | ---: | ---: |
| 0.17.0, no ingress | 1,030 to 1,210 ms | 204 to 247 ms | 174 to 194 ms |
| 0.17.0, ingress | 1,105 to 1,236 ms | 311 to 370 ms | 55 to 64 ms |
| 0.18.0, no ingress | 1,089 to 1,246 ms | 201 to 236 ms | 170 to 186 ms |
| 0.18.0, ingress | 1,105 to 1,413 ms | 209 to 256 ms | 168 to 192 ms |

The ingress no longer moves 110 to 125 ms of hashing into phase 13, but the
boundary's wall time is unchanged within noise: in the filed shape the early
refresh was not repeated work. Where it was, it is gone: 20,000 plugin
components and 64 pairs of `SetComponent` and `ScheduleIngress` in one
phase-13 proposal took 1,796 to 2,101 ms in 0.17.0 (64 full component
hashes) and 100 to 113 ms in 0.18.0 (one). The probe was a one-off and is not
committed; no committed test, since nothing observable changes.

## E-33: knowledge publication reservations

**Problem.** The publication budget is spent in dispatch order, so a
publisher that runs earlier and spends all it is shown at every same-instant
boundary leaves later publishers (information, fiscal) deferring forever.

**Decision.** An optional per-plugin reservation, declared at registration:

```rust
pub struct KnowledgePublicationReservation {
    pub records: usize,
    pub holder_entries: usize,
    pub payload_bytes: usize,
}
impl PluginRegistrar<'_> {
    pub fn reserve_knowledge_publication(
        &mut self,
        reservation: KnowledgePublicationReservation,
    ) -> Result<(), CanwuError>;
}
/// Wraps any plugin: registers it, then reserves for it.
pub struct KnowledgeReservedPlugin<'a> { /* plugin, reservation */ }
impl<'a> KnowledgeReservedPlugin<'a> {
    pub const fn new(
        plugin: &'a dyn SimulationPlugin,
        reservation: KnowledgePublicationReservation,
    ) -> Self;
}
// PluginDescriptor gains
//   knowledge_publication_reservation: Option<KnowledgePublicationReservation>
```

A host reserves for a plugin it does not write by registering it through
`KnowledgeReservedPlugin`; a plugin may also offer its own knob that calls
the registrar. The plugin must register a knowledge-writing boundary system
(phase 4 or 13 with a knowledge write grant). A boundary holds the
reservations of the plugins with such a system due in it, decided from the
boundary's cadences and admitted events before the first phase, the same
facts that decide which systems run; it holds a share for the whole
boundary, whatever the plugin's other systems and cadences, so a plugin
publishing in phases 4 and 13 has one share for both, and a share its phase-4
system leaves unspent stays held through phase 13. Hosts size a reservation
knowing that it is held at every boundary where its publisher is due. A
system of plugin `q` reads, through the unchanged
`knowledge_publication_budget()`, the limits less everything charged so far,
less the unspent share of every other held reservation; its proposal is
charged against the same figure. Its own unspent share is part of what it
reads. Because the reservations together fit the limits and every charge
stays within what its plugin was shown, what remains always covers every
plugin's unspent share, so no publisher, whatever its order, can take
another's. Restore validation charges each knowledge change against the same
per-plugin rule, in the same order, holding the reservations the boundary
record's cadences and admitted events make due. Without reservations the figures are the
0.17.0 ones.

**Refusals.** At registration, `InvalidPluginRegistration` for an empty
reservation, a second one, reservations of the plugins registered so far
that together exceed `KnowledgeLimitsV1::CURRENT`'s per-boundary bounds, or a
reservation of a plugin with no knowledge-writing boundary system. On
restore, `InvalidSnapshot` for descriptors whose reservations are empty, do
not fit, or have no knowledge-writing system, and `PluginManifestMismatch`
when a plugin rehydrates with another reservation (or none). A boundary still fails with
`KnowledgeLimitExceeded` when a proposal publishes past what its system was
shown.

**Overshoot stays a failure.** A publisher that does not read the budget, or
reads it and overshoots, still fails the boundary (the undeclared-write
class). Cutting the publication would leave the same proposal's state writes
believing it was published; dropping the whole proposal would need new
boundary evidence (a format change) and turn a plugin bug into a silent
skip. Neither is cheap and safe.

**Identity and formats.** The reservation is part of the plugin descriptor,
and so of the run's identity, its commitment, and every save. The field is
omitted when absent, so every existing descriptor's bytes and hash are
unchanged and the descriptor format version stays at its value.

## Request ID 0 at enqueue

**Problem.** Command admission refuses request ID 0 ("command request IDs
must be nonzero"), decision ingress refuses decision request ID 0, and a save
refuses a queued command that carries it ("queued command ingress is not
canonical"). `enqueue_command` accepted it, so a host that queued ID 0 could
settle on but never save or restore, before or after the command was
admitted.

**Decision.** ID 0 is not a valid request ID, the rule everywhere else.
`enqueue_command` refuses it first, as a host error that queues nothing
(`InvalidPayload`, the admission's message): enqueue records no attempts, and
its other refusals are host errors too. A command submitted directly with ID
0 keeps its recorded rejection, which a save holds; validation is unchanged.

## Tests

In `canwu-sim` (`src/runtime/tests/load_and_budget_guards.rs`), each checked
to fail with its fix reverted:

- `an_initial_record_declaring_initial_scenario_evidence_is_refused_at_load`:
  a payload-required continuation and an identity declaration naming an
  initial-scenario version are refused at creation and, forged into an
  otherwise valid save with every identity rebound, on restore; declarations
  a seal accepts load and write a continuation checkpoint.
- `a_queued_command_request_needs_a_nonzero_id`: `enqueue_command` refuses
  ID 0 and leaves the run unchanged; a direct command with ID 0 is a recorded
  rejection and the run still saves and restores.
- `a_reserved_share_outlasts_an_earlier_publisher_that_spends_all_it_is_shown`:
  a phase-4 publisher spends every byte it reads, and a phase-13 publisher
  registered with a reservation still reads its share and publishes; the
  restore validates the boundary, a restore without the reservation is
  refused, reservations past the budget and a reservation with no
  knowledge-writing system are refused at registration, and a boundary in
  which the reserving plugin's publisher is not due holds nothing.

## Deviations

- E-31: the filed expectation of saving the 195 ms does not hold for that
  shape; the refresh was early, not repeated. The change is kept because it
  removes the repeated whole-root hashing and attributes hashing to the
  hashing phase.
- E-33: the reservation is set at registration (plugin descriptor), not in
  the run configuration. The run configuration is left out of the
  authoritative identity (a replay-purpose run of the same inputs must reach
  the same state), so a rule that changes what a boundary publishes does not
  belong there; plugin descriptors are in it.
