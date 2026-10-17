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

## Tracked requests and retries (0.18.0)

- A tracked `CommandRequest` carries a request ID. An exact retry is answered
  with its original outcome (`process_command`) or receipt
  (`enqueue_command`) only while its attempt is inside the run's request
  outcome retention (`RunConfiguration::with_request_outcome_retention`,
  default 30 simulated days, at most 366, fixed when the run is created);
  after that the request is retired, its ID stays used, and any request with
  that ID, an exact retry or not, is refused with
  `ErrorCode::RequestOutcomeRetired` without an attempt, a revision, or an
  event. Every host branch that treats `IdempotencyConflict` as "this ID is
  taken, take another" accepts `RequestOutcomeRetired` the same way.
- The window is simulated time: a wall-clock pause retires nothing, and a
  fast speed retires sooner in wall time. Choose the retention from the
  highest speed and the longest client retry or reconnect delay; a client
  that may be away longer is answered from the outcomes the host received.
- Request ID 0 is refused at `enqueue_command` as at admission. Allocate IDs
  consecutively (a counter the run's state restores, or the revision a batch
  is built against plus an offset): retired IDs then collapse into a few
  canonical ranges (`RequestIndex::retired_commands`), where hashed IDs cost
  about 59 bytes each for the life of the run. After a restore, the next free
  ID is one more than the largest ID in the checkpoint's `retired_commands`,
  `commands`, `queued_commands`, `decision_commands`, unadmitted attempts, and
  pending ingress. An exact retry is itself the safe status query.
- After the next seal, `archived_evidence_receipt` and
  `load_archived_evidence_segment` no longer find a retired request's
  attempt, command, events, or ingress through the kept receipts; the host's
  own stored segments still hold them. A plugin that reads the provenance of
  a command ingress record (`plugin_ingress_matches`) declares it as an
  identity-only dependency, as the evidence rule already requires.

## Save, seal, and continue (0.16.0 contract; 0.18.0 writes continuation format 3)

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
- Paged restores take a budget (0.18.0, paged format 7):
  `from_paged_checkpoint_and_journal(checkpoint, provider, segments,
  budget)` and its `_with_plugins` form, on `Canwu` and `CompactedCanwu`,
  end with a `PagedRestoreBudget` that bounds the bytes the restore
  assembles from pages (not its heap) and fail with
  `RestoreBudgetExceeded` past it, restoring nothing. For a checkpoint the
  host wrote pass `PagedRestoreBudget::new(checkpoint.assembled_bytes)`;
  `PagedRestoreBudget::DEFAULT` (2 GiB) is a ceiling for the host's own
  larger checkpoints; for untrusted pages pass a budget near the expected
  size and authenticate the root page ID first. A page-store adapter of
  the host's own passes the budget through. The knowledge ledger is paged,
  so a world of thousands of report holders saves as a paged checkpoint;
  the continuation checkpoint stays the save when the evidence prefix is
  too long to verify. A seal now costs about 2.6 µs an index entry.
- Save without pausing: `Canwu`, `CompactedCanwu`, and every fork are
  `Send` (`Sync` is not promised). Take `fork()` on the simulation thread,
  move the fork to a worker, and write the checkpoint journal, continuation
  checkpoint, or sealed segments there; the original keeps settling. A
  save-only fork never reads the archive provider; a provider or store the
  run holds is passed as `Arc<dyn … + Send + Sync>` (no longer `Rc`), gives
  one answer per object however calls interleave and blocks while busy, and
  the host keeps every lookup page or archive object a live fork still
  reads. A fork's seal belongs to the fork. `fork()` is constant-time
  (0.18.0) and copies no journal; the first append on either side while
  both still hold the shared journals copies them (about 53 ms at 65,536
  records), so a save fork still alive at the original's next boundary
  moves that copy onto the original; finish the save before the next
  boundary when that matters, or seal, since a sealed run copies only its
  tail.
- Production at scale (0.18.0): install one `ProductionState` per scope
  with `scope` set (`production_scope_runtime_reference`) beside the
  primary root, give each its `limits` (`ProductionLimitsV1`, canonical
  16,384 sites and executions, 4,096 operations a boundary), name the scope
  in every `ProductionCommandEnvelope` and through the `_in_scope` enqueue
  helpers, prefix a scope root's lease and output keys with `{scope}/` and
  keep the primary root's free of `/`, and send a holder's monthly
  operations against one root as one `apply_production_batch_v1`
  (`ProductionBatchEnvelopeV1`; `max_batch_items` at most
  `max_operations_per_boundary`; a batch with a stale
  `expected_runtime_revision` is refused whole and sent again). Keep each
  root's archive lookup pages; ended work orders, ended leases, and
  unclaimed outcomes archive as residues. Refused production packets draw
  no incident and publish no report.
- Seal retained evidence (`seal_evidence`, or `prepare_evidence_seal` and
  `commit_evidence_seal`) whenever the retained window grows; sealing works
  with ingress pending and an unadmitted tail. A seal that returns
  `ArchiveNotReady` names a declared dependency the run no longer holds, so
  check an ingress record with `SimulationView::ingress_is_declarable` before
  a plugin declares it.
- A plugin never declares an initial-scenario record version as
  payload-required: it stays readable after every seal, and since 0.18.0 the
  engine refuses such a declaration in an initial-scenario record when the
  plugin registers (`InvalidDomainRecord`) and when a save restores
  (`InvalidSnapshot`), not only at the seal.
- A fiscal host that archives closed books seals at each save, before it
  writes a continuation checkpoint: the archive helpers refuse a book citing
  evidence newer than the run's own sealed cut (`sealed_evidence_cursor`),
  and a run that retains all its evidence refuses nothing.
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
- Reading the resource root from a plugin: the root persists as a live root,
  hot terminal buckets, and (0.18.0) live buckets holding accounts, grants,
  definitions, units, and a custodian index, so a boundary system or command
  handler reads it with `resource_root_in_view(view, keys)` or
  `ResourceRootReader`, naming the hot terminal records it needs and the live
  records with `attach_live(keys)` or `attach_custodian_accounts(holder)`, or
  `whole_resource_root_in_view`, and declares the read of
  `resource_hot_terminal_state_key()` beside the live root's; a consumption
  names its fulfillment (`ResourceConsumption::fulfillment`), so follow it
  instead of scanning. The host reads the whole root with `resource_state`
  and installs opening accounts with `ResourceState::into_records`
  (`into_record` refuses a root with live records). Commit a large archive
  backlog in batches of about 2,048 records and acknowledge each before the
  next.
- Resource at scale (0.18.0): a provider that settles many small
  consumptions sends one `ConsumeBatch` (`ResourceConsumptionBatchRequestV1`,
  up to 256 items under one lease, through its adapter ingress with a typed
  consumption intent per item under `resource_consumption_batch_item_key`);
  check `ResourceState::consumption_batch_can_fit(items)` before taking the
  lease, expect each item to be refused alone or applied with its own
  outcome, and treat a `boundary_budget` skip as a deferral to the next
  boundary at the same instant. `max_batch_items_per_boundary` (canonical
  32, hard 2,048) replaces the unused `max_mutations_per_boundary`; raise
  `max_archive_candidates` with it. Expire grants with
  `enqueue_resource_completion_expiry_tick`: a host `Expire` is refused at
  enqueue. A finished lease leaves the root, so `resource_completion_status`
  no longer finds it.
- Routing at scale (0.18.0): compile a `PlanningSnapshot` once per network
  revision into a `RoutingIndex`, move it to a new read cut with
  `reobserve`, plan with `plan_route` or `plan_route_with_conditions`
  (an `EdgeConditionSet` of closures and adjustments applied by departure
  instant), and key a cache with `RoutingCache::index_key`. A repeating
  timetable is one `TraversalModel::Recurring { period, offsets, duration }`
  pattern, not a slot per sailing. Validate connection payloads before
  publishing them as knowledge: `plan_route` now refuses a network
  `RoutingNetwork::new` refuses.
- Society (0.18.0): author intergroup relations with
  `IntergroupRelations::into_record` beside the root, revise many per
  packet with `intergroup_relation_revision_v1` (each change names the edge
  in full with its `expected_revision`, zero to create), read them with
  `load_intergroup_relations`, and publish what actors learn of them as
  your own knowledge; a refused revision is a
  `society_intergroup_relation_rejected_v1` event. Bound organization
  relation kinds with `SocietyState::relation_vocabulary`. Choose
  `SocietyPlugin::with_outcome_retention(OutcomeRetention::LatestSettlement)`
  to keep only the latest settlement's rebase and lifecycle outcomes in the
  ledger; it is plugin identity, like the cadence.
- Law at scale (0.18.0): declare a `CaseloadProfileDefinition` on the
  `LegalDefinition` (`caseload`) and report routine justice per
  jurisdiction and contiguous period with `enqueue_caseload_intake`; read
  the settled records with `load_legal_caseload(&run, &plan, jurisdiction)`;
  every refused report is a `canwu.law.caseload_rejected.v1` event. Cite a
  provider packet within `max_provider_packets` (default 256) packets of its
  admission, or raise the budget knowing each kept packet costs about 9 µs
  per law boundary. An information delegation must be in force when the
  interpretation is admitted (an expired one authorizes nothing, even for an
  interpretation dated inside its window).
- World-scale fiscal use: send actions as `FiscalActionBatch` values (up to
  4,096 entries, `fiscal_action_command`), each entry naming its target's
  current revision from `fiscal_target_revision`; send receipts in
  `fiscal_execution_receipts_v1` packets of up to 4,096 through
  `enqueue_execution_receipts`; close cycles with `CloseAccountingCycle` and
  archive closed books in batches with
  `prepare_fiscal_archive(&run, book_limit, &store)`,
  `enqueue_fiscal_archive(&mut run, &prepared, &store)`, and
  `finalize_fiscal_archive_retention` (0.18.0: the helpers take any
  `CanwuRun`, read only the records a commit needs, and take the sealed cut
  from the run itself; there is no `sealed_cut` argument); and read
  reports through `fiscal_reports(&viewer)` or, as a trusted host,
  `fiscal_reports_of_holder`. An observer may be a government holder with no
  actor. See `docs/ming-fiscal-case.md` and the website fiscal page.
- Fiscal reports at scale (0.17.0): give each `FiscalObserverBinding` a
  `cadence` (`FiscalReportCadence::OnChange`, the default, or
  `Interval { every, offset }`; spread offsets with
  `FiscalReportCadence::staggered(every, index, stride)`) and a
  `granularity` (`FiscalReportGranularity::Scope`, the default, or
  `Institution` for a court that wants one row per institution and
  mechanism), before `FiscalState::add_observer`; set the state's
  `historical_context.updated_at` to the run's start first, since an
  interval binding's first due time is the first aligned time at or after
  it. An interval report is published by the first boundary at or after each
  due time that runs the report system and never between due times; once
  the report has been published, its summary is republished at every due
  time, so silence never means "no report". If your boundaries can pass
  without any event, register `FiscalPlugin::with_report_cadence(cadence)`,
  the wake cadence: the `SystemCadence` on which
  `wake-fiscal-reports-on-cadence-v1` runs, distinct from a binding's report
  cadence. The report system publishes within its share
  of the engine's publication budget (`with_report_budget`, default half,
  at least `FiscalReportBudget::MINIMUM`) and clears a superseded report
  record at publication (`with_report_retention`, default
  `KnowledgeRetention::RetireWhenSuperseded`; choose `Keep` to read a closed
  cycle's figures from old records). These three knobs are plugin identity:
  choose them before the first boundary. Reports are `fiscal_report` schema
  version 6 (0.18.0, a compact part payload that `FiscalReportPart`
  decodes); the plugin is `0.5.0-experimental`.

## Publish knowledge from a plugin (0.17.0)

- A `BoundaryDirective::PublishKnowledge` carries `supersession` and
  `groups`: `KnowledgePublicationGroup { holders, records }` publishes every
  draft to every holder of the group (holders sorted and unique). One group
  of one holder with `KnowledgeSupersession::Explicit` is the 0.16.0
  directive; use `CurrentOnSubjects` with empty `supersedes` and
  `contradicts` on the drafts when each holder of a group should supersede
  its own current record on the same line, and the engine computes the
  lists per holder. The same body is stored once for every holder.
- Read the budget before you publish: `let mut budget =
  view.knowledge_publication_budget();` then `budget.take(bodies, entries,
  bytes)` per directive, with `bytes` the sum of `draft.payload_bytes()?`
  and `entries` the holders × drafts of its groups. Publish only what `take`
  admitted and defer the rest to a later boundary, or record a rejection,
  as the plugin's own state can see; a proposal past the budget fails the
  boundary with `KnowledgeLimitExceeded`. There is no per-system count any
  more (`batches_per_system_boundary` is gone); the budget is 32,768 bodies,
  32,768 entries, and 64 MiB per boundary, 1,000 bodies and 4,096 entries
  per directive, spent in dispatch order.
- Declare `retention` on every `PluginKnowledgeSchema`: `Keep` for records
  whose history a holder may read, `RetireWhenSuperseded` for lines a newer
  record replaces. It is part of the plugin descriptor.
- Reserve a share (0.18.0) when a plugin must publish whatever runs before
  it: register it through `KnowledgeReservedPlugin::new(&plugin,
  KnowledgePublicationReservation { records, holder_entries, payload_bytes
  })`, or call `registrar.reserve_knowledge_publication(reservation)` in
  `register`. The share is held at every boundary where the plugin's
  knowledge-writing system is due, other plugins cannot take it, and the
  plugin reads it through the same `knowledge_publication_budget()`. It is
  plugin identity; overshooting it still fails the boundary.
- Read a boundary's publications from `BoundaryKnowledgeChange::records_of(&holder)`
  and notice new knowledge through `KnowledgePublished { holders,
  record_count, entry_count }` (audience `EventAudience::KnowledgeHolders`).
  `KnowledgeQueryResult::retention` tells a client when `FullHistory` shows
  only current entries.

Keep the external client replaceable. If the public API cannot express a
needed read or command, improve `canwu-api` instead of bypassing it.
