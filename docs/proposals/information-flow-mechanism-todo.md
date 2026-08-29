# Information Flow Mechanism Implementation Plan

Status: revision-3 implementation checklist for
`information-flow-mechanism.md`; implementation and verification are in the
final promotion gate.

Baseline: `origin/main` at `2d337f4` on 2026-08-21. The persisted decision
framework, engine-issued decision-controller provenance, published society
extension, commitment roots, exact replay, compaction reservations, and
bilingual case navigation are part of the baseline and must not be
reimplemented or bypassed.

## Invariants changed by this work

1. Plugins can publish append-only, schema-versioned knowledge to a person or
   institutional holder without owning `canwu.core.knowledge`.
2. Private knowledge is readable only through a holder-authorized view;
   `PublicObserver` cannot use an arbitrary actor ID to read it.
3. Holder-facing records omit audit evidence and hidden lineage.
4. Phase-4 and phase-13 publication uses a kernel-owned stage distinct from
   ordinary component/domain-record settlement.
5. Historical origin references name exact evidence versions and remain
   verifiable across save/load, replay, and compaction reconstruction.
6. Operation-keyed random outcomes do not shift when unrelated work is added
   or reordered.
7. The authoritative information extension, not voluntary helper usage,
   enforces information lifecycle rules.
8. Multi-recipient delivery is represented per recipient and per attempt.
9. Raw `Canwu` is an explicitly trusted admin capability; player, AI, network,
   and observer code receives only a restricted principal-bound `CanwuViewer`.
10. Archived evidence receipts and keyed-draw reservations preserve exact
    continuation after live evidence sealing.
11. The generic extension owns neutral lifecycle facts only; application
    semantics remain in application-owned knowledge schemas.
12. All new collections, payloads, queries, and fan-out work have versioned
    deterministic admission limits.

## Milestone 0: revised design approval

- [x] Rebase the design worktree to `origin/main@2d337f4`.
- [x] Run the lightest relevant baseline check (`cargo test -p canwu-knowledge`).
- [x] Replace person-only knowledge ownership with `KnowledgeHolderRef`.
- [x] Split holder-visible subjects from audit-only origin evidence.
- [x] Define a separate `KnowledgeViewContext` and reject `PublicObserver`.
- [x] Define versioned knowledge schemas and semantic hashes.
- [x] Define the kernel-owned phase-4/13 publication stage and read cuts.
- [x] Put visibility on each publication batch.
- [x] Version domain-record evidence with boundary/change identity.
- [x] Split dispatch from per-recipient delivery attempts.
- [x] Separate interpretation performer from commissioning holder.
- [x] Define immutable audience snapshots for restricted release.
- [x] Make `canwu-information` an authoritative extension plugin.
- [x] Add kernel-enforced `CreateOnly` domain-record mutation policy.
- [x] Define operation-keyed random draws.
- [x] Bind keyed draws to producer and admitted cause, not only caller text.
- [x] Define archived evidence receipts and compact keyed-draw reservations.
- [x] Define a restricted `CanwuViewer` and institution-aware observation
  principal/event audience.
- [x] Add `AnyEntity` plus explicit holder eligibility policy.
- [x] Define the authoritative information-operation ledger and output slots.
- [x] Define next-boundary cross-plugin ingress.
- [x] Define versioned admission/query/fan-out limits.
- [x] Declare the `0.5.0` public Rust source break explicitly.
- [x] Complete independent kernel, strategy, and generality review of revision 2.
- [x] Write concrete revision-3 contracts for every P0/P1 review finding.
- [x] Complete one final consistency review of the frozen revision-3 text.
- [x] Specify strict legacy routing: reject unknown/format-5 fields through
  nested `deny_unknown_fields` V4 wire structs before legacy validation and
  migration.
- [x] Freeze `RandomDrawAddress::OperationV1`, zero-based candidate retries,
  shared `DomainRecordVersionRef`, and the manual binary encoding contract.
- [x] Freeze generic `EvidenceRef`, journal/nested locator legality,
  evidence-index leaf/node/empty hashes, entry count, dependency inventory,
  and atomic provider requirements.
- [x] Remove the segment-ID hash cycle by separating `EvidenceItemLocator` from
  receipt-only `ArchivedEvidenceLocator`.
- [x] Specify two-phase evidence sealing with immutable prepare, idempotent host
  store/read-back, stale-token validation, atomic local commit, and safe orphan
  handling.
- [x] Replace the underspecified ledger generation with a root-bound
  `KnowledgeReadCut`, and freeze normalized query-hash material excluding
  cursor and page size.
- [x] Scope public read-cut and overlay roots to the holder-facing projection,
  keep the global knowledge root private, and bind holder/query/cut in a
  separate cursor hash.
- [x] Separate stable random entropy address from sequentially allocated
  evidence identity; changing only evidence numbering cannot change a keyed
  result.
- [x] Keep domain-record entropy targets to stable record identity plus version;
  exclude boundary/change establishment source.
- [x] Freeze backward-only output-slot lineage admission and reject self,
  duplicate, unresolved, cross-kind, forward, and cyclic edges.
- [x] Allow Audience/Open dispatch to provide direct Access context without a
  synthetic delivery attempt or release.
- [x] Freeze audience membership leaf/node/proof encoding and limits.
- [x] Use exact assignment record versions and descriptor-committed delegation
  authority grants with canonical subject/capability/time claims; decisions
  authorize only through admitted command/ingress.

Exit criteria:

- Every public type has one owning crate.
- Every authoritative write has a phase, owner, validation cut, rollback path,
  persisted evidence, hash domain, and replay rule.
- No holder-facing API can expose another holder or audit-only origin data.
- Both public cases and all internal profiles fit without case-specific fields.

## Milestone 1: format-5 fixtures and verified migration

Invariant: a format-4 state is validated under the 0.4 contract before any
identity or wire-shape migration, and 0.5 writes format 5 only.

- [x] Select snapshot format 5 for 0.5; do not claim additive format-4 write
  compatibility.
- [x] Check in 0.4 fixtures for: no-plugin empty state; a plugin with domain
  schema and boundary contract; at least one sequential draw; v1 boundary state
  hash; ReplayJournal; and checkpoint-journal plus compact continuation.
- [x] Record canonical JSON, state hash, domain roots, checkpoint hash, boundary
  hash, and negative tamper companions for those fixtures.
- [x] Implement legacy format-4 wire structs and validate old engine identity,
  roots, boundary chain, final state hash, replay envelope, and segment
  continuity before migration.
- [x] Introduce the format-5 `RandomDrawAddress` wire type,
  `Sequential { position }`, the reserved `OperationV1` shape, and every
  affected persistence/hash/validation/source-break surface.
- [x] Migrate sequential draws to tagged `Sequential`, add empty format-5
  generic/continuation state and defaults, then recompute roots under 0.5.
- [x] Until Milestone 6 enables keyed execution, reject every loaded or
  proposed `OperationV1` draw as `UnsupportedRandomDrawAddress`; never accept a
  persisted shape whose value cannot yet be recomputed.
- [x] State and test that old historical intermediate commitments require the
  0.4 runtime for exact replay; 0.5 exact replay begins after migration.
- [x] Document every public enum/struct/API source break, including viewer,
  descriptor, contract, receipt, random, event, error, and reference types.
- [x] Add an external-crate compile/API-delta fixture for old and new
  construction patterns.
- [x] Bump lockstep workspace crate versions and update `Cargo.lock`.

Files:

- `crates/runtime/canwu-sim/src/runtime/migration.rs`
- `crates/runtime/canwu-sim/src/runtime/persistence.rs`
- `crates/runtime/canwu-sim/src/runtime/hashing.rs`
- `crates/runtime/canwu-sim/src/runtime/random.rs`
- `crates/runtime/canwu-sim/src/runtime/replay.rs`
- `crates/runtime/canwu-sim/src/runtime/state.rs`
- `crates/runtime/canwu-sim/src/runtime/transactions.rs`
- `crates/runtime/canwu-sim/src/runtime/validation.rs`
- `crates/runtime/canwu-sim/tests/fixtures/`
- `docs/versioning.md`

Gate: no format-5 runtime model code lands until legacy validation and migration
ordering are executable and unambiguous.

## Milestone 2: holder-relative knowledge model

Invariant: generic knowledge is append-only, versioned, holder-relative, and
separate from truth, belief, access, and audit projection.

### canwu-core

- [x] Add `KnowledgeRecordId` with `define_id!`.
- [x] Add `HolderKnowledgeRecordId` with `define_id!`.
- [x] Add `KnowledgeRecordKind`.
- [x] Add `KnowledgeSchemaId { kind, version }`; runtime version-zero
  rejection remains part of schema admission.
- [x] Add `KnowledgeHolderRef::{Person, Entity}`; runtime rejection of
  `Entity(EntityRef::Person(_))`.
- [x] Add `KnowledgeHolderPolicy::{Allowed, Disallowed}` to domain entity
  schemas with a backward-compatible `Disallowed` default.
- [x] Define core holder eligibility and reject routes, territories, resources,
  retired entities, and deleted tombstones for new publication.
- [x] Add `KnowledgeRecordType::SCHEMA_VERSION`.
- [x] Re-export supported types.
- [x] Add canonicalization and serialization tests.

### canwu-knowledge

- [x] Add the workspace `serde_json` dependency.
- [x] Add `KnowledgeSubjectTarget` and `KnowledgeSubject`.
- [x] Add shared core `EvidenceRef`, `DomainRecordVersionRef`, and
  `DomainRecordVersionSource`; keep `KnowledgeEvidenceRef` as an alias only.
- [x] Add `KnowledgeOrigin`, `KnowledgeRecordDraft`, and stored
  `KnowledgeRecord`.
- [x] Add origin-free `KnowledgeRecordView`.
- [x] Add `KnowledgeHistoryView`, `KnowledgeCursor`, `KnowledgeQuery`, and
  `KnowledgeQueryResult` with a root-bound read cut.
- [x] Add the generic holder ledger to `KnowledgeSnapshot` with default and
  skip-empty serialization; do not add fields to legacy `ActorKnowledge`.
- [x] Reject non-empty scenario generic ledgers in the first slice and initialize
  the runtime generic ID counter to one.
- [x] Implement deterministic current-head, full-history, schema, subject, and
  cursor filtering in a standalone ledger; delta remains deferred by the
  first-slice gate.
- [x] Keep `ArmyKnowledge`, `ActorKnowledge`, `KnowledgeSnapshot`, and
  `KnowledgeSource` wire shapes unchanged until format-5 migration.

Durable tests:

- [x] person and institution holders serialize canonically;
- [x] current heads exclude superseded records but retain contradictions;
- [x] full history retains every record;
- [x] cursor pagination is stable across equal timestamps;
- [x] cursor query identity normalizes duplicate filters, permits page-size
  changes, rejects a forged binding, and rejects a different holder projection
  root;
- [x] holder-facing conversion omits origin evidence;
- [x] retired/deleted holders retain auditable history but accept no new
  publication and do not copy knowledge to a successor;
- [x] empty generic ledger preserves the baseline wire fixture.
- [x] scenario authors cannot preselect generic IDs, learning times, or origins.

Files:

- `crates/foundation/canwu-core/src/lib.rs`
- `crates/model/canwu-knowledge/Cargo.toml`
- `crates/model/canwu-knowledge/src/lib.rs`

## Milestone 3: schema ownership and immutable record policy

Invariant: schema identity is exact and a faulty plugin cannot rewrite a
create-only information record.

### Knowledge schemas

- [x] Add `KnowledgeSubjectTargetKind`, `KnowledgeSubjectSchema`, and
  `PluginKnowledgeSchema`.
- [x] Add typed `AnyEntity` to knowledge subjects and domain references; accept
  only registered entity-class targets.
- [x] Persist schema version, semantic hash, writable flag, payload schema, and
  subject roles.
- [x] Add `PluginDescriptor.knowledge_schemas` with default and skip-empty.
- [x] Add transactional registrar API and ownership maps.
- [x] Require one owner per kind and exactly one writable version per kind.
- [x] Include schemas in descriptor equality, semantic rehydration, run
  identity, snapshot validation, and replay.
- [x] Add `KnowledgeLimitsV1` to engine semantic identity and validate every
  collection/encoded byte limit before allocation.

### Domain record mutation policy

- [x] Add `DomainRecordMutationPolicy::{Versioned, CreateOnly}` to
  `DomainRecordSchema`, defaulting old descriptors to `Versioned`.
- [x] Include the policy in descriptor semantic identity.
- [x] Reject Update/Retire/Delete for `CreateOnly` in
  `apply_mutation_bundle`, not only in extension helpers.
- [x] Keep existing schemas behavior unchanged by default.

Durable tests:

- [x] duplicate kind or schema version rolls back registration;
- [x] two writable versions are rejected;
- [x] schema hash mismatch blocks rehydration and replay;
- [x] raw update/retire/delete of a create-only record is rejected atomically;
- [x] `AnyEntity` rejects a domain value record and accepts a future domain
  entity kind without changing the information schema;
- [x] each V1 knowledge limit accepts its boundary value and atomically rejects
  boundary plus one;
- [x] an old descriptor without mutation policy hydrates as `Versioned`.

Files:

- `crates/runtime/canwu-sim/src/runtime/records.rs`
- `crates/runtime/canwu-sim/src/runtime/plugins.rs`
- `crates/runtime/canwu-sim/src/runtime/mod.rs`
- `crates/runtime/canwu-sim/src/runtime/validation.rs`
- `crates/runtime/canwu-sim/src/runtime/persistence.rs`
- `crates/runtime/canwu-sim/src/runtime/replay.rs`
- `crates/api/canwu-api/src/lib.rs`

## Milestone 4: kernel-owned publication stage

Invariant: publication is an atomic knowledge operation, not an ordinary state
write and not a plugin-owned mutation of the knowledge store.

### Public types

- [x] Add `KnowledgeWriteGrant` to `BoundarySystemContract`.
- [x] Add batched `BoundaryDirective::PublishKnowledge` with holder,
  per-batch visibility, optional audit-only producer correlation, records, and
  summary.
- [x] Add `BoundaryKnowledgeChange` as one holder batch.
- [x] Add skip-empty `BoundaryRecord.knowledge_changes`.
- [x] Add `BoundaryEmissionKind::KnowledgeChange`.
- [x] Add the `knowledge_published` typed payload and encode it through the
  domain-neutral `EventKind` record.
- [x] Add `EventAudience::KnowledgeHolder(KnowledgeHolderRef)` and principal-
  aware visibility without resolving an institution to a current person.
- [x] Add batch and record counts to `BoundaryReceipt`.
- [x] Add knowledge-specific error codes.
- [x] Keep global record IDs in audit/boundary evidence only; derive stable
  holder-local projection IDs for views, relations, and cursors.

### Settlement algorithm

- [x] Extract publication directives before `apply_boundary_stage`.
- [x] Continue rejecting every non-publication directive in phase 4.
- [x] Allow publication only in phases 4 and 13.
- [x] Validate a complete phase bundle before allocating IDs.
- [x] Allocate IDs by plugin/system/directive/record order.
- [x] Stamp `learned_at` with boundary time.
- [x] Maintain a dedicated pending ledger and owned-result overlay.
- [x] Expose phase-4 SameBoundary publications to phases 5-14.
- [x] Hide phase-4 NextBoundary publications from every in-boundary read.
- [x] Keep phase-13 systems on one knowledge input cut.
- [x] Expose phase-13 SameBoundary publications only to phase 14.
- [x] Commit all pending publications immediately before final evidence/hash
  finalization.
- [x] Emit one holder-scoped built-in event per batch.
- [x] Keep the global first-record ID out of the holder-facing event; retain the
  exact range in `BoundaryKnowledgeChange`.
- [x] Roll back pending records, IDs, events, roots, and evidence on any later
  failure.

### View semantics

- [x] Add `SimulationView::knowledge_records` returning owned values.
- [x] Enforce `canwu.core.knowledge` declared reads.
- [x] Merge current ledger and the phase overlay without exposing a borrowed
  mutable/current snapshot.
- [x] Document that this is an omniscient plugin-system read, not an agent
  authorization API.

Durable tests:

- [x] phase-4 SameBoundary is visible in phase 5 but not to a phase-4 peer;
- [x] phase-4 NextBoundary is hidden until host read after success;
- [x] phase-13 publication sees stages 9/11/aggregation and phase-4 overlay;
- [x] a phase-13 peer cannot see another phase-13 publication;
- [x] one system can publish authorized SameBoundary and NextBoundary batches;
- [x] ordinary phase-13 directive behavior remains unchanged;
- [x] phase 7 and all other phases reject publication;
- [x] undeclared schema/visibility and foreign owner are rejected;
- [x] duplicate producer correlation in one producer/boundary is rejected;
- [x] holder-facing IDs contain no gaps caused by other holders' publications;
- [x] a later failure restores ledger, ID counter, events, and boundary data.

Files:

- `crates/model/canwu-event/src/lib.rs`
- `crates/runtime/canwu-sim/src/runtime/boundary.rs`
- `crates/runtime/canwu-sim/src/runtime/plugins.rs`
- `crates/runtime/canwu-sim/src/runtime/settlement.rs`
- `crates/runtime/canwu-sim/src/runtime/mod.rs`
- `crates/runtime/canwu-sim/src/runtime/state.rs`
- `crates/runtime/canwu-sim/src/runtime/transactions.rs`
- `crates/api/canwu-api/src/lib.rs`

## Milestone 5a: retained evidence, authorization, and public queries

Invariant: validation can prove the production cut while ordinary holders see
only the information intended for them.

### Evidence resolution

- [x] Extend `ValidationContext` to resolve command, event, ingress, boundary,
  random draw, and exact domain-record-version evidence.
- [x] Add a proposal-only resolver over current pending record changes, random
  draws, and events with exact index and established commit stage.
- [x] Replace bare pending vectors with staged wrappers mapped exactly from
  `DomainRecordCommitStage`; freeze their final indexes before constructing the
  shared phase-13 resolver.
- [x] Allow phase 13 to cite only pending evidence established in an earlier
  visible stage; never persist a pending resolver state.
- [x] Return retained, pending, archived, and missing availability explicitly;
  accept archived identity only through the completed Milestone 5b/7b receipt
  and dependency contracts.
- [x] Validate initial-scenario record versions separately from boundary change
  versions.
- [x] Validate boundary ID, change index, record reference, and record version
  as one tuple.

### Holder authorization and restricted API

- [x] Add private-field `KnowledgeViewContext`.
- [x] Add `ObservationPrincipal::{Person, Institution, Public}` derived only
  from persisted seat/policy, plus explicit Research/Developer principals.
- [x] Add `CanwuViewer<'a>` with no raw knowledge, events, boundaries, domain
  records, snapshot, export, or admin-upgrade method.
- [x] Add explicitly named admin-only `Canwu::admin_query_knowledge` and
  holder-authorized `CanwuViewer::query_knowledge` methods.
- [x] Map character seats to their person holder and institution seats to their
  institution holder.
- [x] Map CharacterBound, InstitutionBound, PublicObserver, ResearchFull,
  DeveloperDiagnostic, and LegacyUnspecified explicitly; institution binding
  never grants the simultaneously named person's private ledger.
- [x] Reject `PublicObserver` before reading any ledger.
- [x] Permit ResearchFull/DeveloperDiagnostic to select an existing holder.
- [x] Add a separately authorized audit-record query.
- [x] Keep existing detached raw reads explicitly trusted/admin-only and ensure
  bindings, AI clients, and remote adapters expose `CanwuViewer`, not `Canwu`.
- [x] Ensure all event/API projection matches include
  `KnowledgePublished` and cannot widen its audience.

Durable tests:

- [x] ActorBound cannot read another person or institution;
- [x] InstitutionBound reads its institution across personnel changes;
- [x] PublicObserver plus a valid arbitrary person ID still fails;
- [x] a forged/stale context is rejected;
- [x] public/actor/institution viewers have no type-level route to raw admin
  snapshot, journal, domain-record, or audit APIs;
- [x] normal queries omit audit origin/evidence while returning only the
  explicitly holder-visible subject roles;
- [x] audit query is limited to research/developer policies;
- [x] missing or wrong-version evidence is rejected.

Files:

- `crates/runtime/canwu-sim/src/runtime/validation.rs`
- `crates/runtime/canwu-sim/src/runtime/mod.rs`
- `crates/runtime/canwu-sim/src/runtime/persistence.rs`
- `crates/api/canwu-api/src/lib.rs`
- binding and remote-adapter crates that currently expose `Canwu`
- `crates/tools/canwu-debug/`
- `docs/architecture.md`
- `docs/end-state.md`
- `docs/engine-conformance.md`

Interim fail-closed rule: until Milestone 5b and the matching Milestone 7b
persistence work are complete, archived evidence is not an accepted runtime
resolution, the existing static sealing refusal remains in force for declared
command/event/ingress evidence readers, and compaction may not discard any
record needed by generic knowledge origin or pending-current-boundary proof.

## Milestone 5b: archived evidence acceptance and sealability

Invariant: archived identity can become an accepted cause only after its
receipt, segment, provider, dependency, and reconstruction contracts all pass
as one coherent slice with Milestone 7b.

- [x] Add committed `ArchivedEvidenceReceipt` continuation state for exact
  reachable evidence references.
- [x] Permit origin/cut validation from a valid receipt without duplicating the
  archived payload.
- [x] Require archive retrieval and receipt verification when a rule needs old
  payload bytes; otherwise fail with `EvidenceContentUnavailable`.
- [x] Rejoin archived segments before full snapshot/replay origin validation.
- [x] Replace the existing static compaction refusal with the complete
  receipt/provider/dependency-root/two-phase-seal implementation from
  Milestone 7b.

Durable tests:

- [x] archived evidence validates after checkpoint reconstruction;
- [x] a valid archived receipt supports a later derivation;
- [x] missing archived payload fails only when content inspection is required;
- [x] strict legacy-format validation rejects archive-state smuggling, while
  current compaction retains every dependency required by the completed 5b/7b
  contract.

## Milestone 6: operation-keyed deterministic randomness

Invariant: an existing operation's result is a function of its stable address,
not the count or order of unrelated draws.

- [x] Enable the Milestone-1 `RandomDrawAddress::OperationV1` wire variant for
  execution and validation; it selects `Blake3OperationV1` independently of
  sequential stream algorithms.
- [x] Add `RandomOperationAddressV1` bound only to producer plugin, declared
  operation kind, stable application operation ID, stable target, and
  schema-declared draw slot; persist admitted `EvidenceRef` separately from
  entropy identity.
- [x] Encode a domain-record target as stable `DomainRecordRef + version` and
  validate the full `DomainRecordVersionRef` only as non-entropy evidence.
- [x] Add `SimulationView::random_range_for_operation`.
- [x] Derive producer from execution context; accept explicit evidence because
  a handler may process several admitted items, then validate it against the
  proposal-visible cut without hashing sequential evidence IDs into the value.
- [x] Derive keyed values from root seed, algorithm version, stream key, complete
  operation address, bound, and canonical purpose hash.
- [x] Implement and fixture the exact `Blake3OperationV1` domain separators,
  manual length-delimited binary field order/discriminants, little-endian
  extraction, zero-based candidate index, and unbiased rejection reduction.
- [x] Add cross-implementation golden vectors covering every target
  discriminant, exact UTF-8 bytes, a rejection retry, and evidence-ID
  renumbering that leaves the entropy bytes unchanged.
- [x] Do not advance sequential stream state for a keyed draw.
- [x] Assign legacy sequential migration and the tagged wire/source break to
  Milestone 1; Milestone 6 must not revise that already-migrated shape.
- [x] Maintain a runtime uniqueness/idempotency index rebuilt from the draw
  journal.
- [x] Return the existing value for an exact retry.
- [x] Reject key reuse with a different bound or purpose.
- [x] Add sorted, committed `KeyedDrawReservation` continuation state for live
  evidence sealing, including operation evidence plus the sealed draw receipt,
  and rebuild it from full segments.
- [x] Include keyed evidence in random roots, boundary evidence, rollback,
  persistence, replay, validation, and compaction.

Durable tests:

- [x] add/reorder/remove an unrelated operation without changing prior keyed
  outcomes;
- [x] sequential stream results and positions remain unchanged;
- [x] exact keyed retry adds no new draw;
- [x] conflicting keyed reuse is rejected;
- [x] same caller text in a different plugin cannot collide;
- [x] inserting an unrelated command/event/ingress before an operation may
  renumber its evidence but does not change its keyed outcome;
- [x] same entropy address with different evidence is an idempotency conflict;
- [x] failed boundary rolls back keyed evidence/index state;
- [x] exact retry and conflict detection survive repeated seal/restore cycles;
- [x] tampered operation address, reservation, receipt, bound, purpose, or value
  is rejected;
- [x] old sequential random fixtures remain byte/hash compatible.

Files:

- `crates/runtime/canwu-sim/src/runtime/random.rs`
- `crates/runtime/canwu-sim/src/runtime/settlement.rs`
- `crates/runtime/canwu-sim/src/runtime/state.rs`
- `crates/runtime/canwu-sim/src/runtime/transactions.rs`
- `crates/runtime/canwu-sim/src/runtime/hashing.rs`
- `crates/runtime/canwu-sim/src/runtime/validation.rs`
- `crates/runtime/canwu-sim/src/runtime/persistence.rs`
- `crates/runtime/canwu-sim/src/runtime/replay.rs`
- `crates/api/canwu-api/src/lib.rs`

## Milestone 7a: generic persistence, hashing, and replay

Invariant: the first runtime slice is incomplete until every ordinary storage
and replay path preserves generic knowledge without relying on archive or live
compaction support.

- [x] Add `next_knowledge_record_id` to runtime counters, snapshot, rollback,
  control material, and validation with default/skip-one rules.
- [x] Add generic ledger to the knowledge root.
- [x] Add skip-empty knowledge changes to boundary hash material.
- [x] Prove the first-slice scenario generic ledger is empty, then reconstruct
  the generic ledger from ordered boundary batches and compare with final state.
- [x] Reconstruct the counter and compare with snapshot/control commitment.
- [x] Reject format-4 scenario generic records in the first slice and initialize
  the migrated generic counter at one.
- [x] Verify each batch's producer, phase, visibility, holder, ID range, event,
  and exact evidence cut.
- [x] Update flat snapshots, checkpoint shells, journal segments, ordinary full
  reconstruction, forks, and rollback without enabling archived resolution.
- [x] Update `migration.rs`, not only serde defaults.
- [x] Regenerate exact replay and compare generic knowledge batches.
- [x] Confirm the information replay path consumes persisted operation and
  interpretation-result records and exposes no external decoder, controller,
  policy, or language-model callback.

Tamper matrix:

- [x] holder, schema version/hash, payload, subject, confidence, or time;
- [x] supersedes/contradicts forward edge, other holder, or same-batch edge;
- [x] command/event/ingress/boundary evidence ID;
- [x] domain-record version, boundary, or change index;
- [x] producer plugin/system/phase or visibility;
- [x] batch order, first ID, count, event recipient, or emission index;
- [x] next record counter backward/forward;
- [x] holder projection/cursor root or binding hash.

Round trips:

- [x] flat snapshot;
- [x] exact replay journal;
- [x] fork before and after publication;
- [x] rollback across publication;
- [x] checkpoint plus journal segments;
- [x] complete format-4 fixture set and verified format-5 migration.

Gate: Milestones 1 through 5a and 7a are one first-slice acceptance unit. It
must pass ordinary persistence, hashing, replay, rollback, tamper, and
authorization proof while archived resolution stays fail-closed.

## Milestone 7b: archive and compact-continuation persistence

Invariant: Milestone 5b and 7b land atomically; no partial archive path may
relax the retained-evidence sealing guard.

- [x] Add skipped-when-empty archived-receipt, evidence-dependency, and
  keyed-reservation commitment roots to compact continuation material without
  duplicating full-snapshot evidence.
- [x] Make `ArchivedEvidenceReceipt` a generic persisted-evidence receipt and
  support random-draw references; do not put a knowledge-only evidence enum in
  keyed-random state.
- [x] Add typed journal/nested locators, per-segment evidence-index Merkle roots,
  exact leaf/node/empty hash domains, committed entry counts and archived
  segment headers, exhaustive locator legality, and verified provider lookup.
- [x] Implement `prepare_evidence_seal`, content-addressed idempotent
  `ArchiveStore`, and atomic `commit_evidence_seal` with stale-token and orphan
  semantics.
- [x] Replace static read-declaration sealing refusal with `IdentityOnly` versus
  `PayloadRequired` dependency inventory, committed dependency root, and atomic
  `ArchiveNotReady` behavior.
- [x] Rejoin segments before full reconstruction and byte-compare receipt,
  dependency, retry, and keyed-reservation indexes.
- [x] Regenerate exact replay and compare keyed draws/reservations after
  repeated seal and restore.

Tamper and round-trip tests:

- [x] keyed random address, evidence, reservation, or result;
- [x] archived evidence receipt or keyed reservation field/order/root;
- [x] archived segment omission, duplication, gap, overlap, or reorder;
- [x] compact live seal, restore, exact retry, and full reconstruction;
- [x] prepare/store/commit stale token, missing segment, conflicting same-ID
  bytes, and host-owned orphan-candidate identification without implicit
  deletion.

## Milestone 8: authoritative canwu-information extension

Invariant: shared information records can be changed only through the
extension's validated operations.

### Package and ownership

- [x] Add the published `crates/extensions/canwu-information` depending only on
  `canwu-api`, serde, and serde_json.
- [x] Add `InformationPlugin` with a fixed neutral namespace.
- [x] Register channel, content, representation, instance, dispatch,
  delivery-attempt, access, interpretation, audience, release, and operation
  schemas.
- [x] Mark immutable kinds `CreateOnly`.
- [x] Register only neutral information knowledge schemas; application-semantic
  claims remain owned and published by application plugins.
- [x] Register canonical versioned operation command and ingress descriptors.
- [x] Persist the operation state machine, canonical input hash, deterministic
  output slots, domain result refs, publication result IDs, continuation, and
  terminal rejection for exact retry and conflict rejection.
- [x] Add producer-declared, target-validated next-boundary
  `SchedulePluginIngress` and keep `ScheduleIngress` as self-target shorthand.
- [x] Prevent callers from submitting raw mutations under the extension's
  identity.

### Lifecycle

- [x] Implement bounded inline and digest-addressed external content bodies.
- [x] Keep external resource locators outside authoritative state and require
  canonical ingress for externally resolved interpretation.
- [x] Enforce per-parent content and representation DAG lineage, completeness,
  and fidelity; parents must be persisted or earlier output slots and all
  invalid/cyclic edge forms fail atomically.
- [x] Enforce SameContent versus DerivedContent relationships.
- [x] Treat a byte-for-byte copy as a new instance of the same representation.
- [x] Enforce instance terminal states.
- [x] Enforce dispatch and per-recipient attempt state machines.
- [x] Support addressed, audience, and open dispatch targets; require explicit
  final disposition before dispatch closure.
- [x] Enforce contiguous retry numbering.
- [x] Record nonexclusive access without changing delivery.
- [x] Allow direct Access context from active/completed Audience or Open
  dispatch while Addressed delivery still requires a recipient attempt.
- [x] Enforce interpretation performer/holder/input-access relationships and
  exact self/institutional/delegated authority evidence, assignment versions,
  semantic descriptor grants, and canonical `DelegationClaimV1` binding of
  performer, holder, capability, and validity interval.
- [x] Enforce explicit-member or resolved-group audience snapshots with exact
  membership version/root and `AudienceMembershipProofV1`, plus release
  transitions.
- [x] Preserve prior access/knowledge after withdrawal or expiry.
- [x] Split phase-7 mutations from phase-4/13 publications.
- [x] Add `InformationLimitsV1` and persisted continuation chunks for fan-out
  work that exceeds a boundary cap.

Files:

- `Cargo.toml`
- `Cargo.lock`
- `crates/extensions/canwu-information/Cargo.toml`
- `crates/extensions/canwu-information/src/lib.rs`
- `crates/extensions/canwu-information/src/model.rs`
- `crates/extensions/canwu-information/src/schema.rs`
- `crates/extensions/canwu-information/src/operation.rs`
- `crates/extensions/canwu-information/src/lifecycle.rs`
- `crates/extensions/canwu-information/src/plugin.rs`
- `crates/extensions/canwu-information/src/query.rs`

## Milestone 9: anonymous case library and conformance profiles

Invariant: examples teach reusable mechanics without source mapping or
case-conditioned shared behavior.

### Public case A

- [x] Add `confidential_copy_release.rs` and bilingual pages.
- [x] Use only role labels, relative minutes, neutral nodes, and synthetic
  content codes.
- [x] Preserve intended delivery after nonexclusive access.
- [x] Create copy and selected derivative lineage.
- [x] Activate an explicit audience release for two holders.
- [x] At the detached plan surface, prove originator and unrelated holder do not
  receive the hidden-operation publication drafts; kernel holder authorization
  remains covered by the shared runtime conformance tests.
- [x] Prove detached full-lifecycle save/load and replay, then seed that validated
  state into `Canwu` and prove authoritative final-operation snapshot restore,
  exact replay, and compact reconstruction.

### Public case B

- [x] Add `encoded_interception.rs` and bilingual pages.
- [x] Use a channel with no persistent primary instance.
- [x] Record access and failed interpretation independently.
- [x] Deliver the original dispatch normally.
- [x] Use a distinct performer interpreting for the intended holder.
- [x] Publish only the neutral `interpretation_recorded` fact through an explicit
  holder-scoped knowledge batch; decoded semantic claims remain owned by an
  application plugin.
- [x] Restrict review distribution through an audience record.
- [x] Prove authoritative replay consumes the admitted interpretation result and
  has no external interpreter callback to rerun.

### Internal profiles

- [x] ephemeral multi-observer with no instance;
- [x] partial multi-recipient delivery with retry;
- [x] institutional holder across personnel change;
- [x] distinct collector/performer/commissioning holder;
- [x] multi-hop relay, copied instance destruction, and lineage DAG;
- [x] contradictory multi-source reports and current-head/full-history query;
- [x] release withdrawal retaining prior access and knowledge;
- [x] `fixture.information.open-fanout-resource` with digest-addressed external
  body, open reception, 10,000 holders, and deterministic continuation chunks;
- [x] `fixture.information.claimed-source-divergence` separating semantic
  attribution from protected audit origin;
- [x] fourth independent synthetic channel without shared-type changes.

### Anonymous surface

- [x] Keep case/profile IDs neutral and lower-case.
- [x] Keep display names as roles only.
- [x] Use relative time and neutral node IDs.
- [x] Use synthetic claim/content codes.
- [x] Include no inspiration or source-mapping section.
- [x] Run an external, uncommitted source-term list against code, comments,
  tests, filenames, and public pages.

Case/navigation files:

- `crates/extensions/canwu-information/examples/confidential_copy_release.rs`
- `crates/extensions/canwu-information/examples/encoded_interception.rs`
- `crates/extensions/canwu-information/tests/information_lifecycle.rs`
- `crates/extensions/canwu-information/tests/case_conformance.rs`
- `website/src/content/docs/tutorials/cases/confidential-copy-release.mdx`
- `website/src/content/docs/en/tutorials/cases/confidential-copy-release.mdx`
- `website/src/content/docs/tutorials/cases/encoded-interception.mdx`
- `website/src/content/docs/en/tutorials/cases/encoded-interception.mdx`
- both bilingual tutorial indexes and case indexes, preserving the existing
  local-community diffusion case introduced by `570efa5`
- `agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/references/documentation-map.md`

## Milestone 10: performance, docs, and promotion audit

- [x] Measure snapshot growth at 10,000, 100,000, and 1,000,000 knowledge
  records.
- [x] Measure current-head, delta, history, and paged holder queries at 100,
  1,000, 10,000, and 100,000 records, including one hot holder.
- [x] Measure publication batches of 1, 10, 100, and 1,000 records.
- [x] Measure a 10,000-recipient dispatch, 10,000-member audience, 1,000 mixed
  holders, 100 schemas, and a 100-segment compact journal.
- [x] Record wall time, P50/P95 query time, replay throughput, peak resident
  memory, serialized bytes, and index rebuild time without shrinking fixtures.
- [x] Confirm all query indexes are rebuilt and excluded from authoritative
  hashes.
- [x] Update architecture, end-state, versioning, engine conformance, rustdoc,
  and debug projection.
- [x] Run an independent public API, kernel, persistence, replay, authority,
  determinism, and performance review.
- [x] Resolve every blocking finding and rerun affected gates.
- [x] Keep `canwu-information` private for the 0.5 line; any later publication
  requires a separate API, performance, and promotion review.

## Full verification before handoff

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] `cargo check -p canwu-debug`
- [x] `cargo run -p canwu-reference-world --example starter`
- [x] `cargo run -p canwu-api --example phased_boundary`
- [x] `cargo run -p canwu-api --example plugin`
- [x] `cargo run -p canwu-api --example decision_ticket`
- [x] `cargo doc --workspace --no-deps`
- [x] both public information examples
- [x] website's checked-in `check` and `build` scripts
- [x] generated route and internal-link inspection
- [x] `git diff --check`
- [x] old format-4 validation fixtures and explicit verified format-4 to
  format-5 migration proof
- [x] detached full-lifecycle replay plus seeded authoritative final-operation
  exact replay and compact reconstruction for both public cases
- [x] every named metamorphic, isolation, tamper, and archive-segment matrix
  listed above; do not use an unbounded "every test" acceptance claim
- [x] independent pre-performance implementation review and final promotion
  review complete.

## Resolved review findings

- [x] Add all newly introduced public enum variants/struct fields to the source
  break list and compile fixture: `BoundaryDirective`, `BoundaryEmissionKind`,
  `EventKind`, `EventAudience`, `ErrorCode`, `DomainReferenceTargetKind`,
  `RandomDrawRecord`, `DomainRecordSchema`, `BoundaryReceipt`,
  `BoundarySystemContract`, `PluginDescriptor`, `ViewerContext`, and
  `SimulationSnapshot`.
- [x] Add `Cargo.toml`, `Cargo.lock`, and all lockstep crate-version changes to
  the migration/review surface.
- [x] Keep generic knowledge queries limited to CurrentHeads/FullHistory and
  cursor reads. Batch, relation-graph, and changes-since queries remain deferred;
  separately tested cross-plugin ingress and bounded continuation fan-out are
  now part of the extension/runtime infrastructure.
- [x] Add phase-13 same-boundary pending evidence tests using a concrete
  proposal resolver.

## Implementation sequencing record

Runtime implementation started after:

- [x] the worktree matches current `origin/main`;
- [x] the user authorized design correction and implementation;
- [x] the light baseline check passes;
- [x] frozen revision-3 consistency review has no unresolved P0/P1;
- [x] snapshot format/fixture handling is selected explicitly: format 5 with
  verified format-4 migration.

The first coherent implementation slice covered Milestones 1 through 5a plus
7a: verified format-4 migration, the complete tagged random-address wire with
only `Sequential` executable, generic holder knowledge, schema ownership,
publication, pending-current-boundary retained evidence, restricted
`CanwuViewer`, CurrentHeads/FullHistory cursor queries, and their ordinary
persistence/hash/replay path. During that slice, archived resolution remained
fail-closed and `OperationV1` remained unsupported. Later slices added keyed
randomness, the atomic Milestone-5b/7b compact-receipt path, cross-plugin
ingress, bounded continuation fan-out, the authoritative `canwu-information`
extension, and the anonymous cases. Batch, relation-graph, and changes-since
knowledge queries remain deliberately deferred; the unchecked items above are
the current promotion gates.
