# Versioning and Persistence

Canwu is pre-1.0. Format 8 is a deliberate clean break: the 0.13 runtime
writes and reads only its current contracts. There is no implicit loader or
runtime migration for pre-8 data. Applications that need old
records must keep the old engine or run an explicit, application-owned export
outside the Canwu runtime.

## Current contract

The workspace version is `0.15.0`. A live `SimulationSnapshot` has:

- snapshot format `8`;
- commitment format `4`;
- checkpoint/evidence-journal format `4`;
- state revision format `3`;
- admission cursor format `3`;
- exact replay revision format `3`;
- a declared `RunManifest`, a declared or `CompatibilityV1` run
  configuration, and canonical `initial_scenario`;
- a non-zero `authority_root_seed`, derived independently from the simulation
  random streams.

Typed loading and strict JSON loading reject any other engine or contract
version. Strict JSON loading also rejects unknown fields at every nested
object and rejects a wire value whose canonical re-encoding changes shape.

Version 0.15.0 is a pre-1.0 minor release. Each group of changes has a
proposal that gives its design, invariants, and measurements:

- [society settlement cadence and runtime revision](proposals/society-cadence-and-runtime-revision.md)
  (downstream gaps G-40 and G-45);
- [same-instant supersession and unplaced persons](proposals/same-instant-supersession-and-unplaced-persons.md)
  (G-58 and G-49);
- [keyed-draw index, draw ID, and boundary profile](proposals/keyed-draw-index-and-settlement-profile.md)
  (G-50, G-51, and G-47);
- [resource archive lookup and volume](proposals/resource-archive-lookup-and-volume.md)
  (G-52 and G-54);
- [single-step completion lease and runtime protected-floor install](proposals/resource-single-step-lease-and-floor-install.md)
  (G-53 and G-56);
- [fiscal throughput and authority](proposals/fiscal-throughput-and-authority.md)
  (G-41, G-44, G-48, and G-57, and fiscal reports).

New and changed public contracts, by crate:

- `canwu-sim`, re-exported by `canwu-api`:
  - `SimulationView::knowledge_heads(holder, kind, subjects)` returns
    `KnowledgeHeads { records, truncated }`: the global IDs of a holder's
    current records of one schema kind (those no record of the holder
    supersedes) that carry every listed subject. A publishing system uses it
    to name the records its next record supersedes. It needs the
    `canwu.core.knowledge` read, returns at most 64 IDs (the newest when more
    match, with `truncated` set, which is not an error), and reads committed
    state and the boundary being settled, so live, sealed, restored, and
    replayed runs return the same IDs. Additive.
  - `PersonDraft::placement: Option<PersonPlacement>` replaces
    `PersonDraft::government` and `PersonDraft::current_location`, so a draft
    is placed or it is not. A draft without a placement creates an unplaced
    person: an identity in the entity registry with person availability and a
    created-person receipt, and no `Person` record in the `WorldSnapshot`
    projection. The creation evidence `BoundaryPersonCreation::person` is the
    new flat `BoundaryCreatedPerson`, with `placement()`, `is_placed()`, and
    `person_record()`. Both changes are source-incompatible.
  - `SimulationView::random_draw_for_operation` takes the arguments of
    `random_sample_for_operation` and returns
    `RandomOperationDraw { draw_id, sample }`: the sample with the
    `RandomDrawId` of its draw evidence, so a record can cite the draw it was
    computed from. `RandomSample`, `random_sample_for_operation`, and
    `random_range_for_operation` are unchanged. Additive.
  - An opt-in wall-time profile of settled boundaries:
    `set_boundary_profiling`, `boundary_profiling`, and
    `take_boundary_profiles` on `Canwu`, `CompactedCanwu`, `Simulation`, and
    `CompactedSimulation`, with `BoundaryProfileBatch { profiles, dropped }`,
    `BoundaryProfile { boundary_id, settled_at, total_micros, handlers }`,
    `BoundaryHandlerTiming { kind, phase, plugin, name, elapsed_micros }`,
    `BoundaryHandlerKind` (`BoundarySystem`, `CommandHandler`, `EventSystem`),
    and `MAX_RETAINED_BOUNDARY_PROFILES` (1,024). Profiling is off in a new,
    restored, replayed, and forked runtime. It is a host diagnostic outside
    authoritative state: a profiled run produces the same receipts, evidence,
    hashes, snapshots, and replay journal. Additive.
- `canwu-society`:
  - `SocietyPlugin::with_settlement_cadence(cadence)` runs the society's four
    settlement systems on a host-chosen `SystemCadence` category;
    `SocietyPlugin::DAILY` and `SocietyPlugin::default()` keep `Daily`, and
    `EventDriven` is refused at registration. A transition rate applies once
    per settlement, whatever the cadence. The cadence is recorded in the
    persisted plugin descriptor, so a run is restored and replayed only with
    a plugin of the same cadence (`PluginManifestMismatch` otherwise);
    `from_society_snapshot_json_with_plugin` restores a run on another
    cadence. `SocietyPlugin` is now a struct with a private field, `Clone`
    but not `Copy`, which is source-incompatible: code that wrote
    `SocietyPlugin` as a value writes `SocietyPlugin::DAILY`.
  - On a cadence coarser than `Daily`, everything the society's phase-7
    system writes waits for a settlement: due transfers, rebases, lifecycle
    deltas, pressure revisions, institutional decisions, and transitions.
    Of the due transfers that share a cohort, one completes per settlement
    and the others settle as `stale_source`, as transfers due on one day
    of a `Daily` society already do. The owner-side queue record is
    rewritten whole by every boundary that admits a packet, so the
    evidence of a queue that waits a period grows with the square of its
    packets, and a packet that arrives while `MAX_SOCIETY_INGRESS_QUEUE`
    (4,096) wait is dropped with only a rejection event.
  - The `revise_society_pressure` command (`PressureRevisionDecision`, built
    into a ticket by `pressure_revision_ticket`) lets the institution behind
    an alignment revise, within the alignment's affected cohorts, policy
    pressure rows it issued and the activity and reach of its target's
    influence edges and organization nodes; an organization row needs an
    alignment that covers every cohort. A decision revises at most
    `MAX_PRESSURE_REVISION_ROWS` (256) rows, applies as a whole or not at
    all, and takes effect at the first society settlement after the boundary
    that admits its internal `society_pressure_revision_v1` packet.
    `SocietyCohortExchangeLedger::pressure_revisions`, a new public field
    that is omitted while empty, keeps the latest applied and the latest
    rejected decision of each alignment. The new field is
    source-incompatible for struct literals.
  - Also new: `SocietyPlugin::settlement_cadence`,
    `PolicyPressureRevision`, `InfluenceEdgeRevision`,
    `OrganizationRevision`, `PressureRevisionChoice`,
    `PressureRevisionOutcome`, `PressureRevisionOutcomes`,
    `SocietyPressureRevisionV1`, the events `society_pressure_revised_v1`
    and `society_pressure_revision_rejected_v1`, and constants for the
    command, ingress, event, and rejection-reason names.
- `canwu-resource`:
  - Each archive commit directory commits to a cumulative lookup index
    through the new required field
    `ResourceArchiveIndexDirectoryV1::lookup_root`: a content-addressed
    16-way trie over key digests, made of `ResourceArchiveLookupPageV1`
    objects of at most 4 KiB in the namespace
    `canwu.resource.archive.lookup-page`.
    `archived_resource_terminal_record` and
    `archived_resource_operation_outcome` read the head directory and one
    lookup page per index level, plus one blob on a hit, instead of walking
    and authenticating every commit. The cost of a cold lookup therefore
    follows the logarithm of the archived records and does not depend on the
    number of commits, and a lookup no longer returns
    `ResourceError::LimitExceeded`. The 2,048-commit limit
    `MAX_RESOURCE_ARCHIVE_LOOKUP_DIRECTORIES` is removed, so a run may make
    any number of archive commits. A missing or malformed archive object is
    an error, never a miss.
  - `ResourceState::prepare_resource_archive(candidate_limit, store)` takes
    the archive store, which it reads to extend the index; `()` implements
    `ResourceArchiveStore` for a root that has no archive head yet.
    `PreparedResourceArchiveBatchV1` gains `additional_pages`,
    `lookup_pages`, and `superseded_lookup_pages`. All of these are
    source-incompatible.
  - One commit carries up to `MAX_RESOURCE_ARCHIVE_COMMIT_RECORDS` (4,096)
    records as up to `MAX_RESOURCE_ARCHIVE_COMMIT_PAGES` (16) blobs of 256.
    `max_archive_candidates` may be set up to
    `ResourceLimitsV1::MAX_ARCHIVE_CANDIDATES` (4,096); the canonical value
    stays 256. `ResourceLimitsV1::validate` now requires
    `max_archive_maintenance_receipts` to be at least
    `max_archive_candidates`, and its hard maximum rises from 1,024 to 4,096.
    One rule is unchanged and matters when sizing the budget: an operation
    that does not fit the candidate budget fails its boundary and stays
    queued until an archive commit makes room; it is not a recorded
    rejection. The budget must cover the backlog plus every terminal
    record the operations due at one boundary can produce. Every boundary
    that changes the resource root also costs time and journal bytes in
    proportion to the hot terminal records: in the proposal's
    measurements, about 50 to 95 microseconds and 1.1 KB of journal for
    each hot record.
  - Also new: `ResourceArchiveLookupEntryV1`,
    `PreparedResourceArchiveLookupV1`, `PreparedResourceArchivePageV1`,
    `prepare_resource_archive_lookup`, `resource_archive_lookup_key_digest`,
    `live_resource_archive_lookup_pages`, and the lookup constants
    `MAX_RESOURCE_ARCHIVE_LOOKUP_LEAF_ENTRIES`,
    `MAX_RESOURCE_ARCHIVE_LOOKUP_DEPTH`, and
    `MAX_RESOURCE_ARCHIVE_LOOKUP_PAGE_BYTES`.
  - `ResourceCompletionOperationV1::SingleStepLease(SingleStepCompletionLeaseV1)`
    takes a completion lease whose only participant is the resource plugin
    from nothing to active in one operation. "Only participant" means that
    the lease's `expected_participants` is exactly `{"canwu-resource"}`. The
    operation runs the acquire, grant, prepare, and activate steps with the
    checks each has on its own, and its activation certificate equals the one
    the four steps produce for the same acquisition, grant, and targets at
    the same boundary, so the operation it authorizes cannot tell the paths
    apart. It leaves one `CompletionLease` outcome, keyed
    `resource:completion:lease:{acquisition}`, and one receipt, where the
    four steps leave four of each: in the committed fixture a lease with its
    completion receipt goes from 9 terminal records and 4 archive lookups to
    3 and 1, and a same-place transfer from 27 and 15 to 15 and 9. A tracked
    command cannot carry it; it enters through
    `enqueue_resource_completion_operation` or a plugin that declares the
    completion ingress. A domain refusal is the operation's recorded
    rejection and leaves no part of a lease behind. The four-step path stays
    for a lease with any other participant. The new variant is
    source-incompatible for exhaustive matches.
  - `ResourceProtectedFloorRequestV1::install` lets an account's custodian
    set a floor of any quantity at runtime: `SetProtectedFloor` carries a
    content-addressed revision
    (`ProtectedFloorPolicyRevision::content_addressed`) and installs it and
    points the account at it together or not at all. A content-addressed
    revision is always named with its body. A root holds at most
    `MAX_PROTECTED_FLOOR_POLICY_REVISIONS` (4,096) revisions, authored and
    content-addressed together, as one quota shared by every custodian;
    `ResourceState::install_protected_floor_policy` and
    `ResourceState::validate` enforce it, and a runtime install at the bound
    is a recorded `limit_exceeded` rejection. Every applied
    `SetProtectedFloor` retires the content-addressed revisions that no
    account and no demand in the root refers to, whoever installed them; an
    authored revision is never retired. The new field is source-incompatible
    for struct literals, which add `install: None`, and it is omitted from
    the canonical encoding while `None`.
  - Also new: `RESOURCE_COMPLETION_OPERATION_KEY_PREFIX`,
    `PROTECTED_FLOOR_POLICY_CONTENT_ID_PREFIX`,
    `MAX_PROTECTED_FLOOR_OVERRIDE_CLASSES`,
    `ProtectedFloorPolicyRevision::is_content_addressed`,
    `ResourceCompletionOperationV1::has_canonical_operation_key`, and
    `ResourceOperationRequestV1::request_digest`.
- `canwu-fiscal`:
  - Several fiscal actions settle in one boundary, in admission order. Each
    is compared with the procedure revision the state had when the boundary
    began and is validated against the state the previous action left. One
    boundary applies at most `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (64) actions;
    only an action that is current and authorized counts, and a further one
    is `Rejected` with `FISCAL_BOUNDARY_ACTION_LIMIT`. Admission bounds a
    request (`MAX_FISCAL_ACTION_JSON_BYTES`, 32 KiB), its free text
    (`MAX_FISCAL_ACTION_TEXT_BYTES`, 2,048 bytes), and its units and
    referenced IDs (`MAX_FISCAL_IDENTIFIER_BYTES`, 160 bytes). An action
    whose result would not fit the state byte budget, less a 16 KiB reserve,
    is rejected as its own outcome with `FISCAL_STATE_BUDGET_EXHAUSTED`.
  - `FiscalAction::RebindAuthority` replaces a binding's authorized actor,
    acting actor, and authority basis in one commit. It is issued under the
    binding it replaces, by its authorized actor or by the institution
    deciding as a body (a decision whose origin is the institution with no
    responsible actor), never by an acting actor or a third person. Every
    person it names must exist, and a new basis must be the current version
    of its record at admission and in the state the settling boundary began
    with. The binding records the action and its issuer in the new public
    field `FiscalAuthorityBinding::rebound_by` (`FiscalRebinding`).
  - `FiscalAuditBinding`, held in the new `FiscalState::audit_bindings`,
    admits an auditor from outside an institution for `RecordAudit` on that
    institution's scopes and for nothing else, while its own authority basis
    is current. `FiscalAction::RebindAudit` installs, replaces, or removes
    one, under the rules of an authority rebind. Every finding settled from
    0.15.0 names who entered it in the new public field
    `FiscalAuditFinding::entered_by` (`FiscalActionIssuer`,
    `FiscalIssuerRole`). The audited institution installs and removes its
    own auditors, so an audit binding establishes who entered a finding, not
    that the auditor is independent.
  - An observer's fiscal report is one summary record and parts of
    `FISCAL_REPORT_FACTS_PER_PART` (16) facts: `FiscalReportRecord`, version
    2 of the `fiscal_report` knowledge schema. Aggregate IDs are stable, a
    boundary republishes only the parts whose facts changed, and each record
    supersedes the current record of its own line, which the report system
    finds with `SimulationView::knowledge_heads`. A reader calls
    `fiscal_reports(&viewer)` or `assemble_fiscal_reports` and gets one
    `FiscalReport` for each observer binding. One boundary publishes at most
    `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024) report records; the
    parts that are left are named in `pending_parts` and published by the
    next boundary that changes the fiscal state.
    `FiscalState::report_plan` (`FiscalReportPlan`) is a new public field.
  - The two new `FiscalAction` variants are source-incompatible for
    exhaustive matches, and the new public fields on
    `FiscalAuthorityBinding`, `FiscalAuditFinding`, and `FiscalState` for
    struct literals. Also new: `FiscalReport`, `FiscalReportSummary`,
    `FiscalReportPart`, `MAX_FISCAL_REPORT_PARTS`, and the stable reasons
    `FISCAL_AUTHORITY_BASIS_NOT_CURRENT` and `FISCAL_ISSUER_NOT_BOUND`.

The source-incompatible changes are the `PersonDraft` and
`BoundaryPersonCreation` shapes; the `SocietyPlugin` type; the new variants
of `ResourceCompletionOperationV1` and `FiscalAction`; the new public fields
on `SocietyCohortExchangeLedger`, `ResourceArchiveIndexDirectoryV1`,
`PreparedResourceArchiveBatchV1`, `ResourceProtectedFloorRequestV1`,
`FiscalAuthorityBinding`, `FiscalAuditFinding`, and `FiscalState`; the
`prepare_resource_archive` signature; and the removed constant. The release
is therefore a minor version.
`PersonDraft` also rejects unknown fields when decoded: a draft written as
JSON in the 0.14.0 shape fails to decode instead of becoming unplaced. That
protection does not reach a draft an application flattened into its own
struct with `#[serde(flatten)]`, where the old shape decodes as unplaced, so
an application must not flatten a `PersonDraft`.

Version 0.15.0 also changes these behaviors, which are visible to existing
code and runs:

- Settlement and snapshot validation now apply one rule to the records a
  published knowledge record may supersede or contradict. A target must be a
  record of the same holder and schema kind that an earlier boundary
  committed, whatever its learned time, or that the same boundary published
  with `SameBoundary` visibility in an earlier phase. Under 0.14.0 snapshot
  validation compared learned times instead. It refused a target learned at
  the same instant in an earlier boundary, which settlement admits, so a
  plugin that superseded a holder's record in two boundaries of one instant
  wrote a save that no load accepted. It also accepted a phase-4 record
  naming an earlier phase-4 record of its own boundary, which settlement
  refuses. What settlement admits is unchanged.
- An operation-keyed request for an address the run has already drawn no
  longer depends on evidence retention. It returns the recorded draw when
  the evidence reference, bound, and purpose match, and
  `RandomOperationConflict` when they do not; 0.14.0 returned
  `InvalidRandomOperationEvidence` once a seal had removed the evidence, so a
  sealed run refused an exact retry that its unsealed replay answered. In a
  view with a random session, an undeclared stream or a malformed address is
  now reported before the evidence is looked at. A fresh draw still needs
  operation evidence that exists at the reader's cut.
- The runtime keeps its index of operation-keyed draws as derived state,
  extended when a boundary commits and rebuilt on restore, instead of
  rebuilding it at every boundary and copying it for each due system. A
  boundary no longer does work in proportion to the keyed draws the run has
  kept. A checkpoint, a paged checkpoint, a seal, a fork, and a restore
  still cost time per keyed draw ever made, and the index keeps one entry in
  memory for each drawn address for as long as the run lives (522 bytes each
  in the proposal's measurement).
- A society settlement writes the society root only when it changes more
  than a timestamp. `last_transition_at` is the time of the last settlement
  that changed a distribution or a carried remainder. `last_mobilization_at`,
  `last_aggregation_at`, and `last_projection_at` are the times those views
  were last materialized, and a set timestamp means the view matches the
  authoritative state. A projection's `observed_at` is the time that
  observer's own estimate last changed; it is recorded history that
  validation only bounds from above. A host that read these fields as the
  time of the latest settlement uses the time of the latest boundary that
  carried the cadence. `settle_transitions` returns `false` and leaves the
  state untouched when it changes nothing. An idle society no longer emits
  record-change events, so it no longer wakes other plugins' event-driven
  systems at the next boundary.
- A refused society command is a recorded rejection, and the run continues.
  `set_institutional_policy` refuses bad fields as `InvalidPayload`, an
  unknown alignment or a missing society root as `InvalidAuthority`, and a
  version that is not newer than the applied or pending one, including a
  resend, as `IdempotencyConflict`. `transfer_cohort_population` refuses the
  same cohort as source and destination as `InvalidPayload`, a missing
  society root as `InvalidAuthority`, an unknown cohort as `EntityNotFound`,
  the operation ID of a pending transfer with other data as
  `IdempotencyConflict`, a due time in the past as `InvalidDuration`, and a
  stale expected source version as `ValueOutOfRange`. A defective
  cohort-transfer packet is skipped with a `society_ingress_rejected_v1`
  event (`malformed_payload`, `unauthorized_origin`, `authority_unavailable`,
  `unknown_cohort`, or `operation_id_conflict`), and its boundary commits.
  Under 0.14.0 each of these failed the boundary and every later canonical
  step. A host that matched the old codes, or that detected a refusal by a
  failed boundary, reads the rejected command attempt or the event instead.
- An archive commit in `canwu-resource` no longer fails its boundary for
  capacity. The maintenance receipt of a stale commit is not an archive
  candidate: it stays in the root, outside the budgets, until its retention
  is acknowledged, and is then removed. Under 0.14.0 a stale commit against
  a full candidate budget failed every later boundary. A retention
  acknowledgement that arrives twice, or after a later commit archived the
  receipt it names, settles instead of failing its boundary. Within one
  boundary the resource plugin now settles archive commits and retention
  acknowledgements before its other packets, so a commit prepared from the
  root the boundary started from applies even when an operation was admitted
  ahead of it.
- A host application that archives resource records must now keep the
  archive's lookup pages: a restore, an exact replay, or a fork reads the
  lookup index of each archive head it passes. The engine never deletes one.
  `superseded_lookup_pages` names the pages a host application may delete if
  it gives up restoring, replaying, and forking from points before a commit:
  only after that commit settled as `Applied`, and only once a durable
  snapshot or checkpoint taken after it exists.
  `live_resource_archive_lookup_pages` recomputes the pages a live run reads.
- A tracked resource command that reuses an operation key for a different
  request no longer fails a boundary. When the key's outcome is in the root
  or the archive, the command is refused at admission with
  `IdempotencyConflict`; when it was admitted before the first request
  settled, its packet settles nothing and the outcome under the key names
  the request that owns it. Under 0.14.0 the boundary that settled a reused
  key still in the root failed, and so did every later one.
- An operation key under `resource:completion:` is reserved for completion
  operations. Any other request that names one is refused: at admission for
  a tracked command, by `resource_adapter_ingress` and
  `enqueue_resource_allocation`, and as an error from
  `ResourceState::apply_operation`. A holder can still take a completion key
  with the two completion operations it may send as tracked commands: an
  `Acquire`, and an `Abort` of a lease it holds.
- A packet on the completion ingress whose operation key a different request
  holds settles nothing, and its boundary commits. Under 0.14.0 that boundary
  failed, and so did every later one.
- A tracked `Abort` is authorized by the holder of the acquisition in the
  root, not by the holder the request names. An abort by another holder, or
  for an acquisition the root does not hold, is rejected at admission with
  `InvalidAuthority` and leaves nothing under the abort key.
- A single-step lease whose later `Release` key would not be a valid
  operation key is refused with `invalid_identifier`.
- An `Acquire` that names an acquisition the root already holds is a recorded
  `idempotency_conflict` rejection. A completion operation whose identities
  are too long to form its operation key is refused at admission or by
  `enqueue_resource_completion_operation`, and one that reaches settlement
  is a recorded `invalid_identifier` rejection instead of a panic.
- A scenario that installs more than 4,096 protected-floor policy revisions
  is refused, and so is a scenario revision whose identity lies under
  `resource:protected-floor:` without being derived from its content.
- Fiscal actions admitted to one boundary now settle instead of all but the
  first being rejected as stale. A later action of the same boundary is
  stale only when it would overwrite an adoption, authority binding, or
  audit binding that an earlier action of that boundary created or changed.
  An execution receipt or a historical-context change that settles in the
  same boundary no longer makes an action stale; one that settles in the
  boundary that admits the action's command still does.
- A fiscal action never fails a boundary for the others. An oversized
  request, over-long text or references, and a non-canonical ID of a new
  entry are rejected at admission as `InvalidPayload`, and so is a rebind
  that names a person who does not exist, as `EntityNotFound`. An action
  that settlement can give no outcome is dropped: it changes nothing, its
  action ID is not consumed, and one `canwu.fiscal.action_dropped.v1` event
  names it and the institution it was issued for. An accepted command whose
  action has no outcome after the next boundary was dropped. Under 0.14.0
  such a boundary failed on every retry.
- Settlement re-checks the role an issuer held at admission against the
  state the action settles on. An action of an authorized actor or auditor
  who has since been replaced is rejected with `FISCAL_ISSUER_NOT_BOUND`,
  and the basis a rebind or an audit binding relies on must still be current
  (`FISCAL_AUTHORITY_BASIS_NOT_CURRENT`). Authority bases are read from the
  current-version provenance index at admission and at settlement, which a
  seal cannot change.
- An invalid fiscal catalog or state is refused when the run is created or
  restored, because plugin activation now validates both. Under 0.14.0 it
  was accepted and failed the first boundary that admitted anything. A state
  is also refused when an observer binding names a person who does not
  exist (the state record now references each observer's person under the
  role `observer`, so encode it with `FiscalState::into_record`), when an
  assessment's unit or accounting cycle, or the namespace, name, and ID of a
  scope's domain-record institution together, exceeds 160 bytes, and when
  the scenario's knowledge carries a `fiscal_report` record of schema
  version 1. A binding may now cite a record of an external evidence kind as
  its authority basis.
- The four fiscal boundary systems return before they decode the fiscal
  records on a boundary that has nothing for them; a state still at its
  initial-scenario version is derived as before. Activation does not
  recompute derived fields, so a restored state whose transition candidates
  or aggregates were altered outside the engine keeps them until its next
  fiscal write.
- A reader that decoded one `fiscal_report` record as a `FiscalProjection`
  calls `fiscal_reports` instead. A boundary that changes nothing an
  observer can see publishes nothing to that observer, so a report's `as_of`
  is the time it last changed. A holder with several observer bindings gets
  one report for each. A report larger than one knowledge record is
  published instead of failing its boundary, which under 0.14.0 happened
  past about 196 aggregates. A sealed run supersedes the reports it
  published before the seal. Nothing is published to an observer whose actor
  is dead: the changed parts are deferred and go out with the first fiscal
  change after the person is alive again. A report plan that comes with a
  scenario is ignored.

Known limits carried by this release, each stated in its proposal:

- An operation that does not fit the resource archive-candidate budget
  fails its boundary and stays queued until an archive commit makes room,
  and a commit can free at most the budget minus one
  ([choosing the budget](proposals/resource-archive-lookup-and-volume.md#choosing-the-budget)).
- Every boundary that changes the resource root still decodes, validates,
  and journals all of its hot terminal records
  ([choosing the budget](proposals/resource-archive-lookup-and-volume.md#choosing-the-budget)).
- The restore audit of the resource archive is linear in the archive, the
  production archive still has a lifetime limit of 8,192 commits, and the
  force-supply reference archive still walks its commit chain
  ([measurements](proposals/resource-archive-lookup-and-volume.md#measurements),
  [other archives](proposals/resource-archive-lookup-and-volume.md#other-archives)).
- A single-step lease that finds no room in the archive-candidate or hot
  outcome budget, a failed archive lookup, or the state byte cap still fails
  the boundary that settles it, so a host application checks headroom first
  ([single-step completion lease](proposals/resource-single-step-lease-and-floor-install.md#single-step-completion-lease-downstream-gap-g-53)).
- An active completion lease never expires, and a stray `Requested`
  acquisition blocks every single step that sorts after it
  ([what a host application has to watch](proposals/resource-single-step-lease-and-floor-install.md#what-a-host-application-has-to-watch)).
- The 4,096 protected-floor revisions are one quota shared by every
  custodian, below the canonical account limit, and an archived demand keeps
  only the identity of the floor it was admitted under
  ([runtime protected-floor install](proposals/resource-single-step-lease-and-floor-install.md#runtime-protected-floor-install-downstream-gap-g-56)).
- A first operation-keyed draw that cites evidence a seal archived is
  refused where an unsealed run would draw
  ([an exact retry after a seal](proposals/keyed-draw-index-and-settlement-profile.md#an-exact-retry-after-a-seal)).
- A checkpoint, a paged checkpoint, a seal, a fork, and a restore still do
  work for every keyed draw the run has made, and the keyed-draw index keeps
  one entry in memory for each drawn address
  ([what it costs, and what still grows](proposals/keyed-draw-index-and-settlement-profile.md#what-it-costs-and-what-still-grows)).
- Under a coarse society cadence one due transfer per cohort completes per
  settlement, and the owner-side queue record is rewritten whole by every
  boundary that admits a packet
  ([open decisions and follow-up](proposals/society-cadence-and-runtime-revision.md#open-decisions-and-follow-up)).
- A fiscal execution receipt or historical-context change that does not fit
  the state byte budget still fails its boundary, and the fiscal caps are
  lifetime caps because the crate has no archive
  ([several actions per boundary](proposals/fiscal-throughput-and-authority.md#several-actions-per-boundary-g-44)).
- Deferred fiscal report parts are published only by the next boundary that
  changes the fiscal state, and the engine's cost of carrying published
  knowledge grows with every publication
  ([reports](proposals/fiscal-throughput-and-authority.md#reports)).
- An audit binding establishes attribution, not independence, and who
  appoints an auditor is left open
  ([audit bindings](proposals/fiscal-throughput-and-authority.md#audit-bindings-g-57)).
- The record versions that `rebound_by`, an audit binding, and a finding's
  `entered_by` name are not declared as required evidence, so a superseded
  one may no longer be provable after `seal_evidence`
  ([authority rebinding](proposals/fiscal-throughput-and-authority.md#authority-rebinding-g-48)).

Snapshot format 8, commitment format 4, checkpoint/evidence-journal format 4,
paged-checkpoint envelope format 4, and the hash algorithms are unchanged, and
the simulation core adds no persisted field: the keyed-draw index and the
boundary profile are never persisted or committed, and a placed person
creation encodes as it did in 0.14.0. The society root keeps schema version 2
and its shape, and the cohort exchange ledger keeps schema version 1: a
ledger without pressure revisions encodes to the same bytes. The resource
plugin state shape and `RESOURCE_STATE_FORMAT_VERSION` are unchanged.
`FISCAL_RUNTIME_SCHEMA_VERSION` stays 2: every new fiscal field is omitted
while unused, so the payload of a fiscal state written by 0.14.0 decodes and
encodes back to the same bytes. The state record around it is not the same:
its references now carry an `observer` entry for each observer's person, so
a record that 0.14.0 encoded with observers is refused when the run is
created and has to be built again with `FiscalState::into_record`. The
`fiscal_report` knowledge schema moves to version 2, and version 1 is no
longer registered.

Three plugin semantic identities differ from 0.14.0: `canwu-society`,
`canwu-resource`, and `canwu-fiscal`. The society descriptor gains the new
command, the internal ingress type, and its events. The fiscal descriptor
gains one event type on the settlement system, the person-availability read
on the aggregate and report systems, the `observer` reference role on the
state record schema, and the new report schema version. The semantic
identities of the other first-party plugins are unchanged.
Resource archive directories written by 0.14.0 do not decode, which affects
no run, because no 0.15.0 run can reach one. As with any version change, the
engine version in every checkpoint hash changes, first-party plugins that
take their plugin version from the crate version report a new one, and the
exact engine-version check rejects 0.14.0 saves; retain the 0.14.0 engine to
read them.

Version 0.14.0 is a pre-1.0 minor release. It replaces 0.13.3's day read with
a read of any window of simulated time (see the
[boundary window proposal](proposals/boundary-window.md)):
`SimulationView::boundaries_between(from, to)` returns `BoundaryWindow`, and
the host forms are `Simulation::boundaries_between`,
`CompactedSimulation::boundaries_between`, `Canwu::boundaries_between`, and
`CompactedCanwu::boundaries_between`. A boundary system declares how far back
it reads settled boundaries as the new public field
`BoundarySystemContract::boundary_schedule_horizon` (zero by default, at most
`MAX_BOUNDARY_SCHEDULE_HORIZON`, and only with the `canwu.core.boundary_schedule`
read); each system may look back only as far as its own horizon, the run keeps
settled boundaries for the longest declared one, and
`boundary_schedule_horizon` on `Canwu`, `CompactedCanwu`, `Simulation`, and
`CompactedSimulation` reports it. `boundaries_on_day` and
`DayBoundaries` are removed. The new contract field and the removal are
source-incompatible, so the release is a minor version. The field is omitted
from a serialized contract while it is zero, so plugin descriptors and semantic
hashes of systems that do not declare a horizon are unchanged. Snapshot format
8, commitment format 4, checkpoint/evidence-journal format 4, paged-checkpoint
envelope format 4, and the hash algorithms are unchanged; the ledger adds
nothing to saves or commitments. As with any version change, the engine
version in every checkpoint hash changes, first-party plugins that take their
plugin version from the crate version report a new one, and the exact
engine-version check rejects 0.13.3 saves; retain the 0.13.3 engine to read
them.

Version 0.13.3 is a patch release. It adds a read of the boundaries of one
simulated day, so a boundary system can count the boundaries its day has
settled, is settling, and still has queued, including those other sources
open, before it carries work into another boundary:
`SimulationView::boundaries_on_day` behind the new
`StateKey::core_boundary_schedule` read; the host forms
`Simulation::boundaries_on_day`, `CompactedSimulation::boundaries_on_day`,
`Canwu::boundaries_on_day`, and `CompactedCanwu::boundaries_on_day`; the types
`DayBoundaries`, `BoundarySummary`, `BoundaryIngressCount`,
`BoundaryIngressKind`, and `BoundaryIngressOrigin`; and in `canwu-time`,
`SimTime::day` and `SimTime::MINUTES_PER_DAY`. Only phased boundary systems
may declare the read: like `canwu.core.ingress`, it is refused for plugin
commands and synchronous event-driven systems, because it names other
plugins' queued work. The settled ledger is a runtime index derived from
the boundary and ingress journals and rebuilt on restore, so snapshot format 8,
commitment format 4, checkpoint/evidence-journal format 4, paged-checkpoint
envelope format 4, public type shapes, and the hash algorithms are unchanged,
and every change is additive and source-compatible. As with any version
change, the engine version in every checkpoint hash changes, first-party
plugins that take their plugin version from the crate version report a new
one, and the exact engine-version check rejects 0.13.2 saves; retain the
0.13.2 engine to read them.

Version 0.13.2 is a patch release. It adds public API calls that write and
restore the paged checkpoint, which `canwu-sim` already implemented:
`Canwu::prepare_paged_checkpoint`, `Canwu::from_paged_checkpoint_and_journal`,
and `Canwu::from_paged_checkpoint_and_journal_with_plugins`; the same three
calls and `journal_segment_since` on `CompactedCanwu`; and in `canwu-sim`,
`CompactedSimulation::from_paged_checkpoint_and_journal`, its `_with_plugins`
form, and `Simulation::from_paged_checkpoint_and_journal_with_plugins`.
[Format 8 state pages and decision history](#format-8-state-pages-and-decision-history)
describes them. Every change is additive and source-compatible. Snapshot
format 8, commitment format 4, checkpoint/evidence-journal format 4,
paged-checkpoint envelope format 4, public type shapes, and the hash
algorithms are unchanged. The engine version is part of every checkpoint hash,
so every run's checkpoint hashes, and the paged roots that record them, differ
from 0.13.1 exactly as they do across any version change, and first-party
plugins that take their plugin version from the crate version report a new
one. The exact engine-version check rejects 0.13.1 saves; retain the 0.13.1
engine to read them.

Version 0.13.1 is a patch release. Snapshot format 8, commitment format 4,
checkpoint/evidence-journal format 4, and public type shapes are unchanged. It
adds `SimulationView::replay_stable_domain_record_version`,
`canwu_military::MILITARY_REJECTION_EVENT`, and `OperationState::validate`,
and fixes these behaviors, which are visible to existing runs:

- A `Deferred` resolution no longer removes its ticket from the decision
  deadline index, so a deferred ticket expires at the first boundary after its
  deadline in live, restored, and replayed runs alike. Under 0.13.0 a live run
  kept such a ticket `Open` indefinitely, yet every later mutation was rejected
  as `ClosedTicket`; a person-availability sweep that tried to cancel it failed
  the boundary; and a run restored from a snapshot expired it and so diverged
  from the live run.
- A record's current version keeps its exact-version evidence after
  `seal_evidence`: `domain_record_version_evidence_exists`,
  `domain_record_version_is_current`, `domain_record_version`, and
  `evidence_time` resolve it in a sealed run as they do in full exact replay.
  Under 0.13.0 a sealed run lost that evidence with its establishing boundary,
  so it could reject a continuation that its exact replay accepted. A sealed
  non-current version still has no body or time.
- `canwu-production` admits an exact domain-record version as evidence only
  when it is the current version of its record: the realization evidence of a
  `CompleteExecution`, and every version a `StartExecution` or
  `CreateFacilityProject` cites or reads through a cited record.
  `AdvanceFacilityProject` no longer rereads a project's provider and
  technology evidence, which its creation validated, and still rechecks its
  resource evidence. Under 0.13.0 an earlier version was accepted while its
  evidence was retained, but the command was rejected, or its boundary failed,
  once `seal_evidence` had removed that evidence, so a compact run could
  diverge from its exact replay. Such an earlier version is now rejected in
  every run.
- `canwu-history-research` and `canwu-fiscal` resolve the exact versions they
  cite through `SimulationView::replay_stable_domain_record_version`, which
  returns a version's body and establishment time only when no seal can change
  them: for the committed current version, a version proposed earlier in the
  boundary and not superseded since, or an initial-scenario version. Every
  exact domain-record version a historical assessment command names (its
  subject, contradictions, supersessions, and record-version citations) must
  resolve through it when the command is admitted. Settlement no longer
  re-checks that evidence, so a later update to the subject cannot fail the
  boundary, and an identical resubmission of a recorded assessment stays a
  no-op. Each external evidence version a fiscal execution receipt cites must
  resolve the same way when the receipt settles, and
  `enqueue_execution_receipt` rejects one that would not; an identical resent
  receipt settles unchanged before its evidence is checked. If a cited
  evidence record changes after the receipt is enqueued and before it
  settles, the settling boundary fails and the due receipt cannot be
  cancelled, so receipts should cite create-only evidence kinds. As with
  production, an earlier version that 0.13.0 accepted only while its evidence
  was retained is now rejected in every run, so an assessment can no longer
  name a superseded version that the scenario did not provide. Generic
  history-research citations, such as events and boundaries, are still
  checked against retained evidence and remain a known way for a sealed run
  to differ from its exact replay.
- `canwu-military` no longer panics or jams its queue. A successful special
  operation completes and returns the force to its start as `Ready`, instead
  of panicking. Apart from host setup errors, admission returns only errors
  the engine records as rejected attempts. A command that fails in phase 7,
  including a stale `expected_force_revision`, a duplicate ID, a missing
  opposing force, or an over-long operation ID, becomes a `Rejected` ledger
  outcome and a `canwu.military.ingress_rejected.v1` event
  (`MILITARY_REJECTION_EVENT`); a bad provider acknowledgement only emits that
  event. Two provider acknowledgements due in one boundary now compose,
  reports skip dead commanders and send one knowledge batch per commander (at
  most 64 commanders per boundary), and combat losses come off subunits in ID
  order, so a force that has fought still validates. Under 0.13.0 each of
  these failed the boundary, and every later boundary failed the same way. A
  failure in a tick the plugin scheduled for itself still fails the boundary.
  Authority is tightened: `CreateForce` must name the issuer as commander,
  force and occupation commands require the current commander,
  `AssignCommander` rejects a missing or dead commander, `AdvanceTick` sent as
  a command is rejected, and hosts can no longer enqueue the internal
  `military_command_v1` packet. `MAX_RECORDS` (4,096 records per kind over a
  run), `MAX_COMPOSITION_ENTRIES` (at most 64 branch profiles, 64 tactics, and
  64 terrain modifiers in a ruleset), and `MAX_OPERATION_PARTICIPANTS`
  (distinct forces) are now enforced. Admission validates the installed
  ruleset, so a ruleset over these limits rejects every military command as
  `InvalidPayload`.
- `canwu-fiscal` admission rejects a bad command instead of failing the
  boundary. Under 0.13.0 a stale procedure revision, a missing quote, a quote
  of an undeclared kind, a remission naming an unknown assessment, an unknown
  scope, or a malformed action ID failed the boundary, and every later
  boundary failed the same way with time and revision frozen. Admission no
  longer checks the revision; settlement records a stale action as a
  `Rejected` outcome, which consumes its action ID and advances the procedure
  revision like any settled action, so a client retries with a new action ID.
  A missing assessment, scope, or audit target is rejected as
  `EntityNotFound`, and so is a commutation quote of an undeclared kind or one
  that does not resolve through `replay_stable_domain_record_version`, such as
  a superseded version that the scenario did not provide. A malformed action
  or binding ID is rejected as `InvalidPayload`. A missing fiscal catalog or
  state still fails the boundary, as a host setup error.
- `canwu-resource` checks the exact versions it cites against the
  current-version provenance index rather than retained evidence, so a sealed
  run admits exactly what its replay admits. The authority evidence of
  `IssueAccessGrant` (0.13.0 accepted any available exact version), the
  eligibility-envelope evidence of an `Acquire`, and adapter packets that name
  a force-supply record must cite the record's current version; an `Acquire`
  whose eligibility evidence is no longer current when it settles fails that
  boundary. A production output batch's pinned provider source, a
  certificate's locked targets, and a force `Consume` may cite an earlier
  version, which is checked only for consistency with the current version (an
  earlier version number and establishing source); a force `Consume` must
  keep as its consumer evidence the exact force version its certificate
  locked. Under 0.13.0 output settlement read the retained bodies of those
  earlier versions, so a sealed production run stalled when its output
  settled. `SimulationView::current_domain_record_version` also accepts the
  administrative domain-record read.
- The economy and force-supply reference integrations no longer read
  historical record bodies that a seal can remove. Requisition coordinator
  commands, force supply observations, and route and price observations cite
  the current version of the record they name; other observation source
  versions are checked only for consistency with the current version. The
  economy plugin reads live state, or the resource package archive for an
  archived record, instead of a cited body, and a second intent holding the
  same lock can no longer change a local economy the first one already
  changed. A force-supply resource outcome or externality packet whose cited
  version is no longer current when it applies fails its boundary, so a host
  cites the version current when it enqueues a packet that is due at once.
  Under 0.13.0 a sealed run of the requisition saga could stall or diverge
  from its exact replay. The grain loop also applies each month's resilience
  and force postures in the month they were chosen rather than a month late,
  and its requisition branch applies its local cost.

The production, history-research, fiscal, military, resource, economy
reference (its plugin and its typed-evidence adapter), and force-supply
reference plugin semantic identities change. The engine version is part of
every checkpoint hash, so every run's checkpoint hashes differ from 0.13.0; a
run in which a deferred ticket was still open when a boundary passed its
deadline also records the ticket as `Expired`, which changes its decision
commitment. The exact engine-version check rejects 0.13.0 saves; retain the
0.13.0 engine to read them.

Version 0.13.0 shipped the second and final release group of the
[downstream grand-strategy gap set](proposals/downstream-grand-strategy-gap-set.md);
only the §29 series items remain future work. Every contract is additive:

- `canwu-core`, re-exported by `canwu-api`: `EvaluationTraceRecord` and
  `EvaluationTerm`, the application-neutral explanation of how one rule
  produced one integer result for one subject at one boundary.
- `canwu-sim`, re-exported by `canwu-api`: rule-evaluation traces recorded by
  the `RecordEvaluationTrace` boundary directive from phase 7 or phase 12 into
  `BoundaryRecord::evaluation_traces` (`BoundaryEvaluationTrace`), bounded per
  boundary by the run configuration's `EvaluationLimitsV1`
  (`RunConfiguration::with_evaluation_limits`) and failing the boundary with
  `EvaluationTraceLimitExceeded` when exceeded. Transition manifests add the
  `RegisterTransitionManifest` and `StageTransitionWrite` directives, the
  `canwu.core.transitions` state key, `TransitionManifest`,
  `TransitionParticipant`, `TransitionRecordVersion`, `TransitionManifestId`,
  `PendingTransitionManifest`, and a phase-11 audit recorded as
  `TransitionAuditRecord` (`Committed` or `Expired`) on the boundary record and
  receipt; a partially staged manifest fails the boundary with
  `TransitionParticipantMissing` and a wrong expected version with
  `TransitionVersionMismatch`. Pending manifests persist in
  `SimulationSnapshot::pending_transition_manifests`.
- `canwu-api`: `CanwuViewer::evaluation_traces` with the evidence-free
  `EvaluationTraceView` and `EvaluationTermView`, and
  `pending_transition_manifests` on `Canwu` and `CompactedCanwu`.
- `canwu-decision`: a ticket's `parent_ticket` may also name a terminal ticket
  whose assigned controller is bound to the same non-empty seat as the new
  ticket's controller, so a successor continues a predecessor's decision.
- `canwu-resource`: delegated access grants (`ResourceAccessGrantV1`, the
  `IssueAccessGrant` and `RevokeAccessGrant` requests, the
  `ResourceDemandSourcePolicyV1::Granted` source policy, grant-backed
  transfers, and the holder-bound `resource_access_grant_status` read). A
  granted debit settles only in a boundary whose time equals its certified
  lease time, and the grant must be current then.
- `canwu-transport`: capacity pools and deterministic booking allocation
  (`TransportCapacityPoolV1`, `CapacityBookingRequestV1`,
  `allocate_capacity_bookings`, `BookingAllocationV1`,
  `CapacityBookingAllocationEvidenceV1`, `CapacityAllocationFailureV1`),
  booking requests on an execution, booking confirmation or cancellation
  before the booking window opens, execution cancellation, arrival settlement
  for an execution without a delivery attempt, and a terminal seizure after
  which an execution can only be closed. `TRANSPORT_SEMANTIC_VERSION` is `canwu-transport.v5`.
- `canwu-movement` (new crate): `MovementPlugin`, the movement lifecycle
  extension that owns transport executions and capacity pools in one
  `MovementState` record, settles legs at their due times, allocates pools in
  phase 7, and publishes holder-relative movement reports. It never draws
  randomness.
- `canwu-correspondence`: delegated carriers (`delegate_carrier_v1`,
  `CarrierDelegationRequest`, `InitiateCorrespondenceRequest::carrier_delegation`,
  `CorrespondenceIntent::carrier_authority`), recorded by the plugin as the carrier's current delegation for each sender
  (citable from the next boundary, replaced by a newer delegation); the terminal
  `CorrespondenceIncidentKind::CarrierSeized` incident; and the carrier's
  `attempt_report` knowledge schema.
- `canwu-information`: `AuthenticityFinding` on `InterpretationPayload`.
- `canwu-law`: weighted and unit-block procedure stages (`seat_weights`,
  `block_of_seat`, `block_threshold`, and the `status-quo` and
  `casting-seat:<seat>` tie-breaks) and the advisory
  `ProcedureStageKind::Consultation`.
- `canwu-society`: policy-pressure provenance (`PolicyPressure::issuer`,
  `decision_version`), the `cohort_headcount_rebase_v1` ingress, the internal
  `society_lifecycle_delta_v1` ingress, and the queued phase-12 society intake.
- `canwu-culture`: the new `CultureBoundaryPlugin`, which settles the culture
  lifecycle in a Monthly phase-7 system, admits `culture_exposure_v1` batches,
  and hands society deltas to the society plugin. `CulturePlugin` and its
  host-driven flow are unchanged.
- `canwu-technology`: an optional `external_source` on practice transmission
  opportunities (`ExternalTransmissionSourceV1`).

New enum variants (including `ErrorCode`, `BoundaryDirective`,
`ResourceOperationRequestV1`, `ResourceOperationKind`,
`ResourceDemandSourcePolicyV1`, `CorrespondenceIncidentKind`,
`ProcedureStageKind`, and `TransportError`) and new public struct fields (for
example on `RunConfiguration`, `BoundaryRecord`, `BoundaryReceipt`,
`SimulationSnapshot`, `ResourceState`, `ResourceTransfer`,
`InitiateCorrespondenceRequest`, `CorrespondenceIntent`,
`InterpretationPayload`, `ProcedureStageDefinition`, `PolicyPressure`, and
`TransmissionOpportunityPayload`) are source-incompatible: exhaustive matches
and struct literals must be updated, so the release is a pre-1.0 minor version.
New serialized fields are omitted while empty or at their default, and runs
that never register a manifest or record a trace hash exactly as before.
Snapshot format 8, commitment format 4, and checkpoint/evidence-journal format 4
are retained.

The resource, correspondence, information, society, technology, and law plugin
semantic identities change. `MovementPlugin` and `CultureBoundaryPlugin` are new
plugin identities. The `CulturePlugin`, production, fiscal, military,
history-research, and reference-world plugin semantic identities are unchanged. Existing law plans keep their canonical encoding and
content hash.

Version 0.13.0 also fixed these behaviors, which are visible to existing runs:

- A live production completion now settles its output. The resource side of an
  execution's completion lease locked the production runtime at the version
  current when the lease was granted, so a live completion could not settle its
  credit on 0.11 or 0.12; the credit now cites the version production pinned
  as the execution's output source.
- `AmendDemand` can no longer change a demand's lifecycle status, rejection
  reason, or requester; such an amendment is a recorded rejection instead of a change that
  jammed later boundaries.
- A law procedure stage that stops accepting ballots expires its pending seat
  work; a late seat response is recorded as a rejected outcome instead of
  failing the plugin boundary; and the pre-settlement budget check counts
  ticket work emitted at the exact deadline minute.
- The society Daily boundary after an applied cohort transfer no longer fails:
  applying a transfer invalidates the derived aggregates, mobilization
  candidates, and projections it affects, and a transfer's digest binds only
  the state the transfer depends on. An initial scenario cannot seed queued
  society ingress, and a restored snapshot's queue is validated like admitted
  packets.
- Snapshot restore accepts initial domain records that reference entities
  listed only in `Scenario::entities`, as the live state check already did.

Exact engine-version and plugin-descriptor checks still apply: a 0.12.0
snapshot must not be relabeled or loaded into 0.13.0; retain the previous
engine or use an explicit application-owned export. `canwu-movement` joins
publish group 6, so all 24 publishable library crates move to 0.13.0 together.

Version 0.12.0 shipped the first release of the
[downstream grand-strategy gap set](proposals/downstream-grand-strategy-gap-set.md).
Every contract is additive:

- `canwu-sim`, re-exported by `canwu-api`: core person availability
  (`PersonAvailability`, `LifeState`, `CustodyState`) changed only by the
  `SetPersonAvailability` boundary directive. Commands from an unavailable
  issuer are rejected with `IssuerUnavailable`; decision tickets whose person
  decision maker or controller authority person is unavailable cannot be
  opened, prepared, or resolved (`DecisionMakerUnavailable`,
  `IssuerUnavailable`), and open ones are cancelled at the end of the boundary
  that made the person unavailable (`decision_maker_unavailable`,
  `controller_authority_unavailable`). Runtime person creation adds
  `CreatePerson`, `PersonDraft`, and correlation-keyed `created_persons`
  receipts. Queued plugin ingress can be withdrawn before its due time through
  `cancel_plugin_ingress`, `cancel_permitted_plugin_ingress`, or the
  `CancelPluginIngress` directive, journaled as
  `IngressPayload::PluginCancellation`.
- `canwu-decision`: `GuardedUtilityPolicy`, `RuleChoice::Exclude`,
  `DecisionStage`, `DecisionOutcome::PendingRandom`, the controller opt-in
  `random_tie_break`, `DecisionPolicyIdentity::semantic_hash`, and ticket
  `parent_ticket` lineage.
- `canwu-resource`: account-level `RecordLoss`, atomic `BeginExchange`,
  same-place `AcceptLocal` gated by the host-declared
  `ResourceAccount::place_scope`, and loss observations in holder reports.
- `canwu-production`: realized output at `CompleteExecution`, bounded by
  `ProcessRevision::max_realized_per_mille` and the process's declared
  realization evidence kinds.
- `canwu-fiscal`: an acting actor admitted while its exact authority basis
  record version is current (`FiscalPlugin::with_authority_basis_kinds`).
- `canwu-transport`: the `PersonsGroup` subject role, `HandoffKind::Seizure`,
  and the `ExternalCondition` itinerary revision reason.
- `canwu-correspondence`: `planning_snapshot_from_holder_knowledge`, its pure
  form `planning_snapshot_from_knowledge_result`, and `KnowledgeReadCutDigest`.

New enum variants (including `ErrorCode`, `DecisionAttemptErrorCode`,
`BoundaryDirective`, `IngressPayload`, `RuleChoice`, `DecisionOutcome`,
`ResourceOperationRequestV1`, `ResourceOperationKind`,
`ResourceTransferDispositionV1`, `MovementSubjectRole`, and
`ItineraryRevisionReason`) and new public struct fields (for example on
`BoundaryRecord`, `BoundaryReceipt`, `SimulationSnapshot`, `DecisionTicket`,
`DecisionTrace`, `DecisionControllerBinding`, `ResourceAccount`,
`ProcessRevision`, `FiscalAuthorityBinding`, `Handoff`, and the
`CompleteExecution` operation) are source-incompatible: exhaustive matches and
struct literals must be updated, so the release is a pre-1.0 minor version. New
serialized fields are omitted while empty or at their default, so records that
do not use a contract keep their canonical encoding and hashes. Snapshot format
8, commitment format 4, and checkpoint/evidence-journal format 4 are retained.
The resource, production, and fiscal plugin semantic identities change, as does
the resource holder-report knowledge schema; `TRANSPORT_SEMANTIC_VERSION` is
`canwu-transport.v4`; the correspondence plugin semantic hash is unchanged.
Host-authored decision ingress that carries random draw evidence is now
rejected, both live and on load: only a boundary `ResolveDecisionRandomly`
resolution may produce draw evidence, for random-policy controllers and
guarded-utility tie-breaks alike. Exact engine-version and plugin-descriptor
checks still apply: a 0.11.1 snapshot must not be relabeled or loaded into
0.12.0; retain the previous engine or use an explicit application-owned export.
The documented publish order now lists every publishable crate, including
`canwu-law` and the military crates `canwu-military` and
`canwu-military-reference-content` (last published at 0.10.0), so all 23
publishable library crates move to 0.12.0 together.

Version 0.11.0 added a persisted source policy to resource demands: `Pooled`
retains shared-pool behavior, while bounded `ExactAccounts` names lawful
requester-custodied sources. Admission, amendment, allocation, restore and
replay validate that policy before reservation; there is no silent fallback to an
unlisted account. This public Rust field is an additive semantic capability but
struct literals are source-incompatible before 1.0, so the release is a minor
version and retains snapshot format 8. The resource plugin semantic identity
changes. A 0.10.1 snapshot must not be relabeled or loaded into 0.11.0; retain
the previous engine or use an explicit application-owned export. A downstream
application's pinned registry version does not change with this local checkout.

Version 0.10 adds optional resource, production, reference economy-content,
force-supply, and runnable economy-reference packages. They are built above
`canwu-api`; the simulation core gains no production, market, resource, or
military-doctrine semantics. The release retains format 8. Exact plugin
descriptors and semantic hashes make a 0.9 save intentionally incompatible
with a 0.10 run that enables the new packages, and no implicit migration is
provided.

Version 0.9 added the public Random decision policy, random option-weight and
draw-evidence fields, a decision-ticket random-operation target, and a
boundary-generated random resolution directive. These additions change public
Rust exhaustive matches and struct literals, so they require a pre-1.0 minor
version bump. The snapshot format number remains 8, while the exact
`engine_version` continues to make 0.8 and 0.9 snapshots intentionally
non-interchangeable; applications must use an explicit export if they need to
carry state across that engine boundary.

Format 8 keeps the exact decision-ingress commitment and adds explicit plugin
descriptor versioning, typed decision-history locations, canonical domain-record
primary/reverse/successor commitment roots, and content-addressed state-page
prepare/store/verify contracts. Format-7 snapshots, replay journals,
checkpoints, evidence segments, and plugin descriptors therefore require the
0.7 engine or an explicit application-owned export; the 0.8 runtime rejects
them before constructing mutable state.

The runtime no longer contains `migration.rs` or `legacy_v4.rs`. An older
snapshot must not be silently relabeled as format 8, and a legacy replay
journal cannot be promoted to exact replay.

Random streams persist their reduction algorithm. Existing
`SplitMix64V1` streams retain their historical modulo reduction, while new
streams use `SplitMix64V2`, which applies unbiased rejection sampling. This
keeps old format-7 journals replayable while making the current behavior
explicit in the persisted state.

## Format 8 state pages and decision history

`StatePageBlob` is the canonical uncompressed page envelope. Its `page_id` is
the domain-separated BLAKE3 hash of the bytes; hosts may store it in any
backend, but `StatePageProvider` must return the exact bytes or the runtime
returns `state_page_unavailable`. `prepare_state_delta` and
`verify_state_delta` are deterministic and idempotent: they do not mutate the
authoritative simulation revision or replay journal.

The paged-checkpoint envelope format is `4`. It stores a compact
`SimulationCheckpoint`, not an unaudited state-only snapshot. Restoring a
checkpoint whose journal cut is non-empty requires the exact contiguous
evidence prefix through `from_paged_checkpoint_and_journal`; the empty-journal
shorthand is valid only for a zero cut.

Hosts write and restore paged checkpoints through the public API.
`Canwu::prepare_paged_checkpoint(source, provider)` returns a
`PreparedPagedSimulationCheckpoint` whose delta leaves out the pages the
provider already holds. A runtime restored from a paged checkpoint also
leaves out the archived decision pages it never loaded, so its provider must
already hold every page of the checkpoint it was restored from.
`store_and_verify` stores the delta and checks only its new pages and the
root; `StatePageRetentionLedger::prepare` and `verify` check a checkpoint's
complete page set.
`Canwu::from_paged_checkpoint_and_journal(checkpoint, provider, segments)`
and `from_paged_checkpoint_and_journal_with_plugins` restore it from the
provider and the evidence prefix, which a host can collect as it goes with
`journal_segment_since`. Every segment must advance at least one journal, so
a host skips a segment whose start equals its end. Preparing changes nothing:
the authoritative revision, the checkpoint hash, and every journal are
unchanged. The checkpoint carries no evidence records; apart from compact
continuation metadata, such as one archived-segment header per seal, its size
follows current state rather than the length of the run, and each delta holds
only the changed pages. Preparing can still read retained evidence, and a
restore still validates the whole evidence prefix.

`CompactedCanwu` offers the same calls. Its evidence prefix is the host's
sealed segments in cursor order followed by `journal_segment_since` the end of
the last one. A compact restore needs the `_with_plugins` form whenever the
checkpoint names a plugin, and keeps the whole supplied prefix as retained
evidence. Its next seal therefore returns one segment from the zero cut that
replaces every earlier sealed segment: prefixes for later checkpoints start
from that segment, and only checkpoints taken before it still need the
earlier ones. The flat `CompactedCanwu::from_checkpoint_and_journal` behaves
the same way. A paged root also commits to which evidence is sealed, so it is
not a state identity: any restore from sealed segments, through `Canwu` or
`CompactedCanwu`, keeps the original checkpoint hash but gets a different
paged root, which a later seal does not bring back. A runtime that has never
sealed evidence restores to the same paged root.

Decision history is queried by `DecisionHistoryKey` (`Ticket`, `Attempt`, or
`Trace`) and reports `Hot`, `Archived`, `Unresolved`, or `Absent` through
`DecisionHistoryLocation`. Terminal tickets, attempts, and traces can move to
verified archive blobs through canonical maintenance ingress. The compact
checkpoint keeps a bounded hot decision page plus a two-level locator
directory: 4,096 stable hash-prefix buckets, each split into 16 deterministic
subsegments. A segment is capped at 64 entries and 1 MiB; unchanged pages are
reused by content address. The locator page table is itself chunked into
authenticated directory pages of at most 1,024 page references, so the root
manifest remains below the 4 MiB state-page limit at one million keys. An exact
lookup loads exactly one locator segment from a provider. The hot-history root is maintained incrementally from count, XOR, and
modular-sum digests and is rebuilt and verified on restore. Missing pages return
`DecisionHistoryUnavailable`, never `Absent` or an empty success. Verification
produces a provider-authenticated full replacement for every touched locator
page, bound to its previous page ID. Boundary commit therefore needs no provider
read, is replay-safe, and can append into an existing segment after a root-only
restart without discarding earlier receipts.
Before hot-state validation, paged restore derives the exact archived ticket or
trace references still named by hot traces, resolved tickets, and accepted
attempts, then authenticates only those locator pages against the committed
directory. Partially archived dependency graphs therefore restart correctly
without loading unrelated history.
Restore rejects noncanonical locator bucket/segment placement and any archived
locator or terminal payload duplicated in hot metadata. Repeat delta
preparation reads only directory pages for changed locator segments rather than
walking all 65,536 possible pages.

Offline GC starts from `ArchiveReachabilityManifest`. The kernel contributes
retained state pages, evidence segments, decision locator pages, pending
maintenance locators, and decision blobs. Every frozen plugin archive
participant extends the same manifest automatically; callers do not append law
marks manually. The law plugin contributes committed or leased content, blob,
membership, effective-time, and recorded-time page IDs. The retention ledger
derives and verifies the exact transitive page closure from the provider,
persists `Prepared`, `Verified`, `DurableIngress`, and terminal root handles
across restart, interlocks each ownership handoff with the active GC epoch, and
keeps a pending handle as a new-object delta plus the proposed current-page
closure while the previous committed root protects older data. Commit moves
the prior object closure into the new root, retires the superseded root, and
clears the terminal handle payload. Root-only restore and overlapping-root
tests prove that shared pages and plugin marks remain protected after restart. Internal
plugin ingress may also carry authenticated archive-retention roots. A pending
verified legal transition carries one directory root rather than a
history-sized list. The law reachability participant authenticates and expands
that root into blob, membership, and temporal objects before mark/sweep.
Snapshot/restart preserves the root; applying or terminally rejecting a stale
item transfers or releases it deterministically.
The legal commit itself binds the exact directory root, object count, shard,
token, source root, and expected record versions; neither a caller-provided
summary nor a different authenticated tree can be substituted.
The synchronous helper authenticates the store retention handle and complete
root closure before mutation, applies to a detached clone, finalizes retention,
and only then swaps the live runtime. Canonical ingress records terminal
outcomes by retention handle until the host finalizes the store from the
authoritative reloaded runtime and a private acknowledgement retires that
recovery metadata. Multiple archive commits in one boundary therefore remain
independently recoverable after restart.

The legal archive object remains format `1`; its authenticated membership and
bitemporal index is format `2`. Format `2` uses the full 16-bit deterministic
bucket space and rejects any membership or temporal page above 64 entries or
1 MiB before storage or root admission. A dense effective-time or recorded-time
cell is represented by an ordered vector of authenticated page segments, so 65
or more versions in one cell remain exact instead of overflowing one page.
Provider-backed temporal queries stream those segments and enforce candidate,
intersection, provider-call, segment, and decoded-byte budgets during I/O;
one query-wide meter covers directory, membership, temporal, and blob reads
across every shard. Pathological overlap fails before an unbudgeted next
segment is read.

Generic domain records use four canonical Patricia roots: primary records,
reverse references, successor edges, and predecessor edges. Forks and rollback
captures share these roots. Incremental checkpoints stop at provider-known
subtree pages, so changing one record emits only its missing Patricia paths and
the bounded manifest path rather than reserializing every record. The paged
checkpoint builder constructs its compact checkpoint body without first
cloning the full domain-record and decision payloads. Boundary proposal
validation likewise filters empty stages first, overlays only changed records
onto a structurally shared root, and validates the affected reference closure.

Plugin descriptors also persist owner-authorized maintenance dependency
resolver declarations. A cross-plugin retirement commit must include the exact
target schema owner and every declared dependent owner. Each proposal is bound
to the plugin semantic hash and may mutate only schemas owned by that plugin;
the complete proposal set is validated against one source domain root and
applied atomically through replay-visible maintenance ingress.
Plugin-owned maintenance packets use registrar-issued opaque internal-ingress
permits; ordinary host-authored plugin ingress cannot forge a verified archive
commit. Applied and post-admission stale maintenance both produce chained
terminal records. A stale item is a successful terminal no-op, not a boundary
rollback that can retry forever. Plugin descriptors persist the exact internal
ingress names so this authority and pending-retention ownership survive restart.

## Self-contained exact replay

`ReplayJournal` is the complete replay boundary. It carries the canonical
initial scenario, declared run identity, run configuration, plugin descriptors,
authority root, commands, attempts, ingress, boundaries, random draws, and
final revision/checkpoint commitments. `replay_from_journal(plugins, journal)`
does not accept a second scenario supplied by the caller; the scenario in the
journal is authoritative and is validated against the manifest.
`replay_from_journal_json` is the strict JSON counterpart and rejects unknown
fields recursively before replay begins.

Executable policy implementations are not replay inputs. Decisions, outcomes,
and evidence already admitted to the journal are replayed as records.
`DecisionPolicyKind::Random` therefore does not execute an RNG during replay.
Its format-8 evidence consists of an operation-keyed draw targeting the exact
ticket version, a `DecisionSelection` outcome, canonical option weights in the
decision trace, and boundary-generated decision ingress. New optional random
evidence fields remain absent on historical non-random decision traces. Since
0.12.0, a guarded-utility random tie-break uses the same evidence, with weights
naming only the pending near-equivalent candidates and the trace recording the
`Random` stage; the stage, fired-guard, and parent-ticket fields are absent on
traces that do not use them.

Since 0.13.0, rule-evaluation traces, transition-manifest registrations, and
transition audits are hash-chained boundary evidence. Exact replay regenerates
and compares them with the rest of the boundary, snapshot validation rebuilds
the pending manifests from that evidence, and sealing archives traces with
their boundary records. A trace is never read back as state, so replay needs no
separate trace input.

## Durable outbox

Boundary emissions are exposed through `Canwu::outbox_entries()`. Each entry
has a stable `delivery_id` derived from the run manifest, boundary, event, and
emission index. The host application must deliver entries at least once and
deduplicate by `delivery_id`. Exact replay regenerates the same outbox
identity; it does not re-send external effects.

In compact mode, `CompactedCanwu::outbox_entries()` returns entries from the
retained evidence tail. Once evidence is sealed, the caller owns the returned
`EvidenceJournalSegment` and must keep its boundary emissions with the host's
delivery/acknowledgement state; compaction does not acknowledge or deliver an
external effect on the host's behalf.

## Granularity boundary

The engine exposes the domain-neutral `SimulationGranularity` enum:

| Value | Meaning |
| --- | --- |
| `aggregate` | A coarse aggregate or population-scale state. |
| `group` | A bounded social, institutional, military, or organizational group. |
| `actor` | A person or other principal with its own knowledge and authority. |

These are engine simulation levels, not a fixed historical ontology. A host
game may map them to its own terms. In Celestial Mandate, for example,
`aggregate` can map to Population, `group` to Special Group, and `actor` to
Character. That mapping belongs in a CM reference integration or host adapter,
not in Canwu core.

## Reference integrations

Southern Ming and WWII are content and ruleset integrations for Celestial
Mandate. They are not Canwu adapters and are intentionally absent from this
repository. Canwu provides the generic state, authority, boundary, evidence,
granularity, persistence, replay, and outbox contracts that those integrations
consume.

## Source compatibility

The 0.8 change removes caller-supplied replay wrappers from the public facade
and adds schema-declared identity-only evidence dependencies plus compact,
Merkle-bound plugin-ingress provenance.
Use the constructor APIs for new runs, `snapshot`/`checkpoint` for persistence,
`replay_from_journal` for exact replay, and `outbox_entries` for host delivery.
Downstream crates must update their code to these contracts; no deprecated
alias is retained before 1.0.


Version 0.11.1 is a patch release. It fixes completion-lease abort and release cleanup so every released held or prepared grant is removed from the bounded expiry index before validation; snapshot format and public type shapes remain unchanged.
