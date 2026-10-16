---
name: canwu-engine-usage
description: Use Canwu through its public Rust and semantic agent APIs. Use when building a game client, renderer, research tool, Python or web binding, AI agent, debug consumer, or experiment that observes a Canwu world, queries actor-relative knowledge, discovers legal actions, submits validated commands, advances time, explains events, or compares snapshots and forks.
---

# Use Canwu Engine

Work through `canwu-api`. Read `README.md`, `docs/architecture.md`,
`docs/end-state.md`, and the public types in `crates/api/canwu-api` as needed.
Use the sibling
[`canwu-engine-docs`](../canwu-engine-docs/SKILL.md) skill when the primary task
is to find, compare, summarize, or explain Canwu documentation.

## Choose the interface

- Use typed commands, queries, events, snapshots, forks, and diffs for Rust
  programs and research tools.
- For actor-facing or AI-agent workflows, read through
  `viewer_for_actor(actor)` (a `CanwuViewer`: `query_knowledge`,
  `visible_changes_since`, `evaluation_traces`) or a `viewer_context(actor)`,
  explain outcomes with `explain`, advance with `wait`, and act with a
  validated `CommandEnvelope` (`Command::OrderMovement` or a plugin command).
- Use `viewer()` and `admin_query_knowledge` (omniscient reads) only for
  explicitly authorized research or debug tools.

## Follow the boundary

1. Never mutate live world state directly or reach into `canwu-sim` from a
   client.
2. Read the actor's own knowledge (`viewer_for_actor`) before choosing an
   action for it.
3. Turn an intended action into a validated `CommandEnvelope` and submit it
   through the run's ingress; handle validation errors instead of changing
   fields.
4. Advance simulation time explicitly. Do not couple time to rendering frames.
5. Consume emitted events and causal references to update external views.

Runtime plugins are also implemented through `canwu-api`; do not add a direct
client dependency on `canwu-sim`. Declare every core collection read with the
matching `StateKey::core_*` constructor, and declare every plugin-state read or
write with its owned `StateKey`. Handlers are stateless function pointers;
store deterministic plugin state in serialized components.

## Protect partial knowledge

- Use `viewer_for_actor(actor)` (`CanwuViewer::query_knowledge`,
  `visible_changes_since`) or `viewer_context(actor)` for an actor's view.
- Do not silently replace missing actor knowledge with `world()` ground truth.
- Show estimates, confidence, source, observation time, and information age when
  available.
- Keep agent context small. Start with a summary, then inspect details only when
  needed.

## Start with the vertical slice

Run:

```text
cargo run -p canwu-reference-world --example starter
```

The example demonstrates a validated army move, scheduled travel, and exact
save/load, fork, and replay.

Movement uses the canonical `Command::OrderMovement { subject, destination,
cargo }` command, with an army the issuer commands or the issuer's own person
as `subject`. A self-move is still an ordinary validated command: the issuer
must be the same person as the `EntityRef::Person` subject. A person may carry explicitly listed letter IDs;
arrival emits person and delivery events, and custody is not considered
delivered until the recipient handoff succeeds.

Run `cargo run -p canwu-api --example plugin` for an issuer-aware plugin command
with a machine-validated payload and declared core/component access.

## Integrate external clients

- Renderers consume snapshots, geometry, positions, relationships, and events;
  they decide whether to draw sprites, SVG, 3D scenes, tables, or text.
- Save with `snapshot` or `snapshot_json`; restore with
  `from_snapshot_json` for read-only inspection. If the snapshot contains
  plugins, use `from_snapshot_json_with_plugins` before advancing or submitting
  commands; continuation remains blocked until every exact stored plugin
  descriptor is rehydrated.
- Use `fork` and `diff` for counterfactual comparisons.
- Use `Canwu::replay_from_journal(plugins, &journal, archive_provider)` for
  journals containing plugin commands. Executable handlers are not
  serialized, and a replay without the matching plugins intentionally fails
  rather than silently omitting plugin behavior.
- Treat `Issuer` as an assertion made by a trusted in-process host. Authenticate
  external callers in the adapter before constructing a command envelope.

## Save, seal, and continue (0.16.0)

- Save a run the player will continue with
  `canwu.continuation_checkpoint()`, which returns the checkpoint and its
  `continuation_hash`; store both and restore with
  `CompactedCanwu::from_continuation_checkpoint_verified(checkpoint,
  &continuation_hash, &[&plugin, ...])`. It carries no evidence history, so it
  is small and restores to a runtime that settles, seals, forks, and writes
  further continuation checkpoints exactly as the uninterrupted run does. On
  that runtime `checkpoint()`, `prepare_paged_checkpoint`,
  `snapshot_with_segments`, and `replay_journal_with_segments` return
  `HistoryUnavailable`, so keep a `Canwu` run's `snapshot`, `replay_journal`,
  and checkpoint journal where debugging, exact replay, or an archive needs
  them.
- Save without pausing: `Canwu`, `CompactedCanwu`, and every fork are
  `Send` (`Sync` is not promised). Take `fork()` on the simulation thread,
  move the fork to a worker, and write the checkpoint journal, continuation
  checkpoint, or sealed segments there; the original keeps settling. A
  save-only fork never reads the archive provider; a provider or store the
  run holds is passed as `Arc<dyn … + Send + Sync>` (no longer `Rc`), gives
  one answer per object however calls interleave and blocks while busy, and
  the host keeps every lookup page or archive object a live fork still
  reads. A fork's seal belongs to the fork.
- Seal retained evidence (`seal_evidence`, or `prepare_evidence_seal` and
  `commit_evidence_seal`) whenever the retained window grows; sealing works
  with ingress pending and an unadmitted tail. A seal that returns
  `ArchiveNotReady` names a declared dependency the run no longer holds, so
  check an ingress record with `SimulationView::ingress_is_declarable` before
  a plugin declares it.
- A plugin never declares an initial-scenario record version as
  payload-required: it stays readable after every seal, and the engine refuses
  the declaration.
- A refusal is never a failed boundary: an inadmissible packet is a recorded
  rejection, a skipped packet with a traced reason
  (`canwu.resource.ingress_skipped.v1`, `canwu.production.ingress_refused.v1`,
  `canwu.fiscal.ingress_rejected.v1`, `canwu.law.ingress_rejected.v1`), or a
  deferral the plugin resends itself. Watch those events; do not retry a
  boundary.

## Choose the extension knobs

- Society transfers: `SocietyPlugin::with_transfer_settlement`. Keep the
  default `TransferSettlement::Batch` when many issuers move cohort shares in
  one period (a market, a migration wave, a levy): every due transfer is
  checked against the state the period's issuers saw, and all of them that
  still fit apply in admission order. Choose `OneByOne` only when each
  transfer must see the result of the previous one at the same settlement,
  such as a chain of hand-offs between the same cohorts; only the first
  transfer of the chain, in operation-ID order, completes per settlement, and
  the rest settle as `stale_source` (or `insufficient_source`) and must be
  reissued against the new state, one per settlement.
  The mode is part of the plugin identity, so fix it before the run starts.
- Fiscal auditors: give every audited institution a `FiscalAppointmentPolicy`
  (`FiscalState::add_appointment_policy`). Name an `appointer` outside the
  institution for an independent audit, list `nominators` and `petitioners`
  for a procedure of nomination and removal petition, or make the institution
  its own appointer with `appoints_without_nomination` and
  `removes_without_petition` for the 0.15.0 behaviour. Without a policy no
  audit binding can change at runtime.
- Reading the resource root from a plugin: the root persists as a live root
  and hot terminal buckets, so a boundary system or command handler reads it
  with `resource_root_in_view(view, keys)` or `ResourceRootReader`, naming
  the hot terminal records it needs, or `whole_resource_root_in_view`, and
  declares the read of `resource_hot_terminal_state_key()` beside the live
  root's; the host reads the whole root with `resource_state`. Commit a
  large archive backlog in batches and acknowledge each before the next.
- World-scale fiscal use: send actions as `FiscalActionBatch` values (up to
  4,096 entries, `fiscal_action_command`), each entry naming its target's
  current revision from `fiscal_target_revision`; send receipts in
  `fiscal_execution_receipts_v1` packets of up to 4,096 through
  `enqueue_execution_receipts`; close cycles with `CloseAccountingCycle` and
  archive closed books in batches with `FiscalState::prepare_fiscal_archive`,
  `enqueue_fiscal_archive`, and `finalize_fiscal_archive_retention`; and read
  reports through `fiscal_reports(&viewer)` or, as a trusted host,
  `fiscal_reports_of_holder`. An observer may be a government holder with no
  actor. See `docs/ming-fiscal-case.md` and the website fiscal page.

Keep the external client replaceable. If the public API cannot express a
needed read or command, improve `canwu-api` instead of bypassing it.
