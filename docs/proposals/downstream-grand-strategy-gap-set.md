# Downstream Grand-Strategy Gap Set

Status: consolidated proposal drafted 2026-09-25 from the verified needs of a
downstream historical grand-strategy application built on the 0.11.1 public
crates. Every section names a contract that the application could not express
through `canwu-api` and the published extensions without becoming a second
owner of engine truth. Each section is independently implementable; the
suggested release grouping is at the end. No section is accepted until its
public-API fixture fails on 0.11.1 and passes on the implementation branch.

## Decision and invariant

Accept the gap set as two additive pre-1.0 minor releases. The simulation core
remains responsible for deterministic time, canonical ingress, authority,
scheduling, transaction atomicity, allocation, random evidence, persistence,
replay, and commitments. Every new contract below is one of:

- a typed field or enum variant with a serde default on an existing public
  record;
- a new boundary directive, canonical ingress, or host-facing API method whose
  effect is recorded in the replay journal;
- a pure function or policy adapter that reads only what the caller passes;
- a new domain extension crate above `canwu-api`.

No contract introduces application vocabulary, a period calendar, a world map,
a market, or a combat model into the core. Snapshot format 8 is retained; the
exact `engine_version` continues to make snapshots across the release boundary
intentionally non-interchangeable, as in 0.10 and 0.11.

The invariant every section preserves:

```text
validated intent (command, ingress, or directive from a declared writer)
  -> staged in its owning phase with declared reads/writes
  -> validated before atomic commit
  -> recorded as replayable evidence with a stable identity
  -> readable only actor-relatively
```

## Ownership map

| Concern | Owner after this proposal | Downstream application boundary |
| --- | --- | --- |
| Person life and custody state, admission consequences | `canwu-sim` core state keyed by `PersonId` (§1) | the application decides *when* a person dies or is captured through its own boundary systems |
| Runtime creation of a person | `canwu-sim` directive and id counter (§2) | the application supplies the draft and provenance |
| Cancellation of queued plugin ingress | `canwu-sim` scheduler, `canwu-api` facade (§3) | the application decides what to cancel |
| Multi-owner conditional transitions | `canwu-sim` manifest, phase-10 staging, phase-11 audit (§4) | participants and expected versions are application content |
| Delegated resource access | `canwu-resource` grant record and demand policy (§5) | who grants whom is application authority |
| Rule-evaluation evidence | `canwu-core` record type, `canwu-sim` evidence, `canwu-api` query (§6) | rule ids and term semantics are application content |
| Holder-relative planning snapshot | `canwu-correspondence` public builder (§7) | route-knowledge records are published by the application |
| Composite decision policy | `canwu-decision` policy SDK (§8) | guard rules and utility weights are application content |
| Decision ticket lineage | `canwu-decision` field (§9) | — |
| In-place loss, atomic exchange, local acceptance | `canwu-resource` request variants (§10–§12) | causes and evidence are application records |
| Realized output at completion | `canwu-production` operation extension (§13) | realization evidence is application content |
| Acting fiscal actor | `canwu-fiscal` binding fields (§14) | the authority basis is an application grant record |
| Movement lifecycle plugin, capacity pools, subject roles, seizure handoffs, external-condition reroutes | new `canwu-movement` extension; `canwu-transport` record additions (§15–§19) | incidents, hostility, and hazards remain application systems |
| Delegated carriers, carrier seizure | `canwu-correspondence` (§20–§21) | — |
| Authenticity finding | `canwu-information` (§22) | detection probability is an application draw |
| Weighted and advisory procedure stages | `canwu-law` (§23–§24) | seat weights and blocks are application content |
| Policy-pressure provenance, cohort headcount rebase | `canwu-society` (§25–§26) | the external stock is an application record |
| Culture boundary system and exposure ingress | `canwu-culture` (§27) | exposure signals are application events |
| External transmission source | `canwu-technology` (§28) | — |
| Fiscal archive, observer bound, batched consumption, production limits | `canwu-fiscal`, `canwu-resource`, `canwu-production` (§29, series) | — |
| Lockstep publication of `canwu-military` and its reference content | release procedure (§30) | — |

## Section format

Each section states: the verified 0.11.1 shape (file cited), the contract
sketch in Rust-like pseudocode, persistence/replay/versioning notes, and the
verification evidence expected. Fixture names are suggestions.

---

## 1. Person availability and admission consequences

Verified: `runtime/legacy_world.rs` `Person { id, name, government, current_location, roles: Vec<String>, transit }`; command admission and `DecisionControllerBinding` validation never consult a life or custody state.

```rust
// canwu-sim (re-exported by canwu-api)
pub enum LifeState { Alive, Dead, Missing }
pub enum CustodyState { Free, Detained, Hostage, Captive, Hiding, Exile }

pub struct PersonAvailability {
    pub life: LifeState,          // default Alive
    pub custody: CustodyState,    // default Free
    pub custodian: Option<EntityRef>,
    pub since: SimTime,
}

// Kept as a separate ordered core map keyed by PersonId, not a field on the
// legacy Person struct, so it survives the planned world-model move.
pub struct PersonAvailabilityState { entries: BTreeMap<PersonId, PersonAvailability> }

pub enum BoundaryDirective {
    // ...existing...
    SetPersonAvailability { person: PersonId, availability: PersonAvailability, summary: String },
}

impl StateKey { pub fn core_person_availability() -> StateKey }

pub enum ErrorCode { /* ... */ IssuerUnavailable, DecisionMakerUnavailable }
```

Rules:

- The directive is accepted in phase 7 from a boundary system whose contract
  declares `writes: [StateKey::core_person_availability()]`, and inside a
  phase-10 transition bundle. Two writers for the same person in one boundary
  fail the boundary.
- Admission (`process_command`, `enqueue_command`, decision ingress) rejects a
  request whose issuer resolves to a person with `life != Alive` or
  `custody ∈ {Detained, Captive}` with `IssuerUnavailable`; `Hostage`,
  `Hiding`, `Exile` are admissible (the application may restrict further).
- `prepare_decision` refuses a draft whose `decision_maker` is such a person
  with `DecisionMakerUnavailable`; an open ticket whose maker becomes
  unavailable is closed as `DecisionTicketState::Cancelled { reason: "decision_maker_unavailable" }`
  in the same boundary, with a trace.
- Knowledge: a dead person's holder ledger becomes read-only for admin and
  custodian reads; publication to it is rejected with `InvalidKnowledgeHolder`.

Persistence/replay: the map is a snapshot field with an empty default; the
directive is journaled like `SetComponent`; `engine_version` gates loading.
Versioning: new enum variants and error codes change exhaustive matches →
minor. Format 8 retained.

Evidence: `person_availability_blocks_issuer_and_closes_tickets` (command from a
dead issuer rejected with a stable code; open ticket cancelled; replay
reproduces both); `availability_write_requires_declared_writer`.

## 2. Runtime person creation

Verified: no `BoundaryDirective` creates a `Person`; `MutateRecord` creates domain records; ids come from the scenario.

```rust
pub struct PersonDraft {
    pub name: String,
    pub government: GovernmentId,
    pub current_location: TerritoryId,
    pub roles: Vec<String>,
    pub availability: PersonAvailability,
    pub provenance: EvidenceRef,
}

pub enum BoundaryDirective {
    // ...
    CreatePerson { draft: PersonDraft, correlation: String, summary: String },
}

pub struct BoundaryReceipt {
    // ...
    pub created_persons: Vec<(String /* correlation */, PersonId)>,
}
```

Rules: accepted in phase 7; ids allocated from `counters.next_person_id`
(persisted in the checkpoint, claimed with the same `claim_counter` discipline
as knowledge record ids); the new person exists with next-boundary visibility
(the proposing plugin binds it at the next boundary using the receipt);
`correlation` is unique per (plugin, system, boundary).

Persistence/replay: the counter is part of the checkpoint; replay reproduces
ids. Versioning: minor; format 8 retained.

Evidence: `create_person_ids_are_replay_stable` (fork and replay yield the same
`PersonId`s); `create_person_visible_next_boundary`.

## 3. Cancellation of queued plugin ingress

Verified: `runtime/ingress.rs` exposes enqueue paths and `IngressReceipt { ingress_id, issued_at, due_at }`; no withdraw path.

```rust
impl Canwu {
    pub fn cancel_plugin_ingress(
        &mut self,
        ingress_id: IngressId,
        permit: &PluginIngressPermit, // or host authority when the item was host-enqueued
        reason: &str,
    ) -> Result<IngressReceipt, CanwuError>;
}

pub struct IngressCancelled { pub ingress_id: IngressId, pub reason: String, pub at: SimTime }
```

Rules: only the issuing plugin's permit or the host may cancel; a due or
admitted item cannot be cancelled (`LateIngress`); the cancellation is
journaled as an expected terminal record ordered before the item's due time;
replay reproduces it; a cancelled item is a terminal no-op, never a rollback.

Versioning: new public method and journal record kind → minor; journal format
4 retained (additive kind under strict loading of the new engine).

Evidence: `cancelled_plugin_ingress_never_settles_and_replays` (enqueue, cancel,
advance, assert no admission, replay equality).

## 4. Transition manifest and omitted-participant audit

Verified: phase-10 directives are collected into the transition set
(`settlement.rs` `HistoricalCandidateEvaluation` arm) without a declaration of
required participants; no `TransitionManifest` symbol exists.

```rust
pub struct TransitionParticipant {
    pub plugin: String,
    pub expected_pre: Vec<DomainRecordVersionRef>,
    pub expected_post: Vec<DomainRecordVersionRef>,
}

pub struct TransitionManifest {
    pub lineage_id: String,
    pub attempt: u32,
    pub participants: Vec<TransitionParticipant>,
    pub ready_at: BoundaryId,
}

// registered as an internal ingress `transition_manifest_v1` by the coordinating plugin
pub enum BoundaryDirective {
    // ...
    StageTransitionWrite { manifest_id: String, writes: Vec<BoundaryDirective> },
}

pub struct TransitionAuditRecord {
    pub manifest_id: String,
    pub outcome: TransitionAuditOutcome, // Committed | MissingParticipants(Vec<String>) | VersionMismatch(Vec<DomainRecordVersionRef>)
}
```

Rules: `StageTransitionWrite` is accepted in phase 10 only from plugins listed
in the manifest; phase 11 fails the boundary when a listed participant staged
nothing or a pre-version differs from the snapshot; phase 12 receives the
read-only audit record as evidence; ordinary phase-10 directives without a
manifest keep today's behavior.

Versioning: new directive variant and evidence kind → minor; format 8
retained.

Evidence: `transition_missing_participant_fails_closed` (two-plugin manifest,
one participant silent, whole boundary rolls back); `transition_audit_is_replay_evidence`.

## 5. Delegated resource access grant

Verified: `canwu-resource/src/model.rs` 305 `ResourceDemandSourcePolicyV1::{Pooled, ExactAccounts}`; `docs/end-state.md` keeps cross-custodian delegation with the host.

```rust
pub struct ResourceAccessGrantV1 {
    pub grant_id: ResourceGrantId,
    pub grantor_custodian: EntityRef,
    pub grantee: EntityRef,
    pub resource_revision: ResourceDefinitionRevisionId,
    pub unit_revision: ResourceUnitRevisionId,
    pub cap_quantity: u64,
    pub valid_from: SimTime,
    pub valid_until: SimTime,
    pub authority_evidence: DomainRecordVersionRef,
}

pub enum ResourceOperationRequestV1 {
    // ...
    IssueAccessGrant(ResourceIssueGrantRequestV1),   // issuer must be the grantor custodian
    RevokeAccessGrant(ResourceRevokeGrantRequestV1),
}

pub enum ResourceDemandSourcePolicyV1 {
    Pooled,
    ExactAccounts(Vec<ResourceAccountId>),
    Granted { grant_id: ResourceGrantId, accounts: Vec<ResourceAccountId> },
}
```

Rules: `Granted` is validated before scarcity arbitration (grant current,
grantee = requester, accounts custodied by grantor, remaining cap ≥ quantity);
allocation and consumption decrement the cap; no fallback to pooled accounts;
revocation stops at the first reservation like amendments do today.

Versioning: new variants → minor; resource plugin semantic identity changes;
format 8 retained.

Evidence: `granted_demand_requires_current_grant_and_never_falls_back`;
`grant_cap_is_conserved_across_replay`.

## 6. Rule-evaluation trace record

Verified: `canwu-api` `Explanation { summary, causal_chain: Vec<ExplanationStep { label, event }> }` explains by events; `DecisionTrace` covers ticket options only.

```rust
// canwu-core
pub struct EvaluationTerm { pub term_id: String, pub contribution: i64, pub evidence: Vec<EvidenceRef> }
pub struct EvaluationTraceRecord {
    pub rule_id: String,
    pub rule_version: String,
    pub subject: EntityRef,
    pub terms: Vec<EvaluationTerm>,
    pub result: i64,
    pub boundary: BoundaryId,
}

// canwu-sim
pub struct BoundaryProposal { /* ... */ pub evaluation_traces: Vec<EvaluationTraceRecord> }
pub struct EvaluationLimitsV1 { pub traces_per_boundary: usize, pub terms_per_trace: usize }

// canwu-api
impl CanwuViewer { pub fn evaluation_traces(&self, subject: &EntityRef, cut: &KnowledgeReadCut) -> Vec<EvaluationTraceRecord> }
```

Rules: traces are boundary evidence emitted from phases 7 and 12, bounded,
content-addressed into the evidence journal, never state; the viewer returns
only traces whose `subject` the viewing holder may see under the existing
knowledge policy.

Versioning: additive type and evidence kind → minor; journal format 4 retained.

Evidence: `evaluation_trace_is_actor_filtered_and_replayed`.

## 7. Planning snapshot from holder knowledge

Verified: `canwu-correspondence/src/knowledge.rs` has `planning_knowledge_query()`, `KnownRoutingEndpoint`, `KnownRoutingConnection`, `KnownAddress`; the plugin builds a carrier-relative snapshot privately; no public builder.

```rust
pub struct KnowledgeReadCutDigest(pub String);

pub fn planning_snapshot_from_holder_knowledge(
    view: &SimulationView<'_>,
    holder: &KnowledgeHolderRef,
    read_cut: &KnowledgeReadCut,
    observed_at: SimTime,
) -> Result<(PlanningSnapshot, KnowledgeReadCutDigest), CanwuError>;
```

Rules: admits only endpoints and connections the holder's ledger asserts at the
cut; the digest is the canonical hash of the record ids and versions read, for
evidence; pure with respect to truth (a test asserts the function never reads
core route state).

Versioning: additive function → minor by lockstep.

Evidence: `holder_snapshot_excludes_unknown_connections`.

## 8. Guarded utility policy

Verified: `canwu-decision/src/policy.rs` `RuleChoice::{Select, Defer, NoMatch}`, `OrderedRulePolicy`, `WeightedUtilityPolicy`; one `DecisionPolicyIdentity` per binding.

```rust
pub enum RuleChoice { Select(String), Defer(String), Exclude { option_id: String, reason: String }, NoMatch }

pub struct GuardedUtilityPolicy {
    pub identity: DecisionPolicyIdentity,     // DecisionPolicyKind::Utility reused
    pub guards: OrderedRulePolicy,
    pub utility: WeightedUtilityEvaluator,
    pub near_equivalence_margin: i64,
    pub random_tie_break: bool,
}

pub enum DecisionStage { Guard, Utility, Random }
pub struct DecisionTrace { /* ... */ pub stage: Option<DecisionStage>, pub fired_guards: Vec<String> }

pub enum PolicyDecision { /* ... */ PendingRandom { candidates: Vec<DecisionOptionWeight> } }
```

Rules: guards run in order; `Select` wins lexicographically; `Exclude` removes
an option and records the reason; remaining options are scored; when the top
scores lie within the margin and `random_tie_break` is set, the policy returns
`PendingRandom`, which the boundary resolves with the existing
`ResolveDecisionRandomly` directive over those candidates only; the trace
records the stage and fired guard ids.

Versioning: new enum variants and a trace field → minor; format 8 retained;
historical traces keep `stage: None`.

Evidence: `guarded_policy_orders_guard_utility_random_and_traces_stage`.

## 9. Decision ticket lineage

Verified: `DecisionTicketDraft { id, definition, decision_maker, assigned_controller, summary, context, options, deadline }`.

```rust
pub struct DecisionTicketDraft { /* ... */ pub parent_ticket: Option<DecisionTicketId> }
pub struct DecisionTicket      { /* ... */ pub parent_ticket: Option<DecisionTicketId> }
```

Rules: the parent must exist, be terminal, and share the `decision_maker`
family (same holder or same institution); the chain is exposed through
`decision_history_location` and `DecisionTrace`. Versioning: struct literal
break → minor. Evidence: `ticket_lineage_requires_closed_parent`.

## 10. Account-level loss

Verified: `canwu-resource/src/model.rs` 614 `ResourceLoss { account: Option<_>, transfer: Option<_>, ... }`; `runtime.rs` 320–334 offers loss only through `CompleteTransfer(Lose)`.

```rust
pub struct ResourceAccountLossRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub account: ResourceAccountId,
    pub expected_account_revision: ResourceRevision,
    pub quantity: u64,
    pub cause: EvidenceRef,
    pub allow_protected: bool,
    pub at: SimTime,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}
pub enum ResourceOperationRequestV1 { /* ... */ RecordLoss(ResourceAccountLossRequestV1) }
pub enum ResourceOperationKind { /* ... */ Loss }
```

Rules: the phase-7 lifecycle writer settles a `ResourceLoss { account: Some, transfer: None }`;
phase 8 counts it as admitted loss in `ConservationTotalsV1`; protected floors
are respected unless `allow_protected`; outcome, receipt, and archive follow
existing terminal-operation paths; holder reports gain a `loss` observation.

Versioning: new variants → minor; resource semantic identity changes.

Evidence: `record_loss_conserves_and_respects_protected_floor`.

## 11. Atomic two-leg exchange

```rust
pub struct ResourceExchangeStartRequestV1 {
    pub operation_key: ResourceOperationKey,
    pub leg_a: ResourceTransferStartRequestV1,
    pub leg_b: ResourceTransferStartRequestV1,
    pub completion_certificate: CompletionLeaseActivationCertificateV1,
}
pub enum ResourceOperationRequestV1 { /* ... */ BeginExchange(ResourceExchangeStartRequestV1) }
```

Rules: both `BeginTransfer`s are admitted or both rejected with one outcome;
the outcome cites both transfer ids; terminal dispositions remain independent.
Evidence: `exchange_rejects_both_legs_when_one_is_invalid`.

## 12. Same-place acceptance

Verified: `runtime.rs` 107–133 `Accept { acceptance: ResourceTransportAcceptanceV1 { execution: TransportExecutionLink, .. } }` makes a transport execution mandatory.

```rust
pub enum ResourceTransferDispositionV1 {
    Accept { /* unchanged */ },
    AcceptLocal { destination: ResourceAccountId, expected_destination_revision: ResourceRevision, handover_evidence: DomainRecordVersionRef },
    Lose { .. }, Return { .. }, ExternalOutflow { .. },
}
```

Rules: valid only when the transfer's `transport` is `None` and both accounts
share the place scope the host declared on the definition revision or account
custodian; otherwise `InvalidPayload`. Evidence: `accept_local_requires_shared_scope`.

## 13. Realized output at completion

Verified: `canwu-production/src/model.rs` 856 `CompleteExecution { execution }`; no realized-output symbol.

```rust
pub enum ProductionOperation {
    // ...
    CompleteExecution {
        execution: ProductionExecutionId,
        realized_output_per_mille: Option<u16>,           // None = 1000
        realization_evidence: Option<DomainRecordVersionRef>,
    },
}
pub struct ProcessRevision { /* ... */ pub max_realized_per_mille: u16 /* default 1000 */ }
```

Rules: phase 7 scales each `ProductionOutputSettlementRequest.quantity` by the
per-mille (floor) and stores the evidence on the execution; phase 8 rejects a
value above the process bound or a value below 1000 without evidence; the
output acknowledgement path is unchanged.

Evidence: `realized_output_scales_settlement_and_requires_evidence`.

## 14. Acting fiscal actor

Verified: `canwu-fiscal/src/model.rs` 704 `FiscalAuthorityBinding { id, institution, authorized_actor: Option<PersonId> }`.

```rust
pub struct FiscalAuthorityBinding {
    pub id: String,
    pub institution: EntityRef,
    pub authorized_actor: Option<PersonId>,
    pub acting_actor: Option<PersonId>,                    // default None
    pub authority_basis: Option<DomainRecordVersionRef>,   // default None
}
```

Rules: `apply_fiscal_action_v1` admits a command from either actor while the
basis version is current; a stale basis rejects with a stable reason.
Evidence: `fiscal_action_from_acting_holder_admitted_with_current_basis`.

## 15. `canwu-movement` domain extension

Verified: `canwu-transport` is a record library with no plugin, ingress, phase-7 writer, root state, validator, or report; a plugin cannot live there because `canwu-sim` depends on `canwu-transport` (publish groups 3–5).

Add `crates/extensions/canwu-movement` (publish group 6, depends on
`canwu-api` only):

```rust
pub struct MovementState {
    pub executions: BTreeMap<TransportExecutionId, TransportExecution>,
    pub orders: BTreeMap<String, MovementOrder>,
    pub pools: BTreeMap<String, TransportCapacityPoolV1>,      // §16
    pub operation_outcomes: BTreeMap<String, MovementOperationOutcomeV1>,
    pub observation_heads: BTreeMap<KnowledgeHolderRef, u64>,
}
impl MovementState { pub fn validate(&self) -> Result<(), MovementError> } // leg continuity, one active revision, bookings inside leg windows, terminal closure

pub const MOVEMENT_COMMAND: &str = "apply_movement_operation_v1";
pub const MOVEMENT_LEG_DUE_INGRESS: &str = "movement_leg_due_v1";  // IngressClass::ScheduledSystem

pub enum MovementOperation {
    Order(MovementOrder), StartLeg { .. }, CompleteLeg { .. }, FailLeg { reason: String, .. },
    Reroute(ItineraryRevision), RecordHandoff(Handoff), RequestBooking(CapacityBooking), Cancel { .. },
}
```

Boundary systems: `movement_lifecycle_apply_v1` (phase 7, `EventDriven`,
`SameBoundary`), `movement_lifecycle_validate_v1` (phase 8),
`movement_report_publish_v1` (phase 13) with `MovementObserverGrant`
(operator, owner, delayed remote). Incident draws stay in the application's
systems; the plugin never draws.

Evidence: `movement_plugin_settles_legs_and_replays`; `movement_report_is_holder_relative`.

## 16. Capacity pools and deterministic booking allocation

```rust
pub struct TransportCapacityPoolV1 {
    pub id: String, pub resource: String, pub custodian: KnowledgeHolderRef,
    pub window_from: SimTime, pub window_until: SimTime,
    pub quantity: u64, pub booked: u64, pub consumed: u64, pub revision: u64,
}
pub struct BookingAllocationV1 { pub booking: CapacityBookingId, pub quantity: u64 }
pub struct CapacityBookingAllocationEvidenceV1 {
    pub pool: String, pub pool_revision: u64, pub booking: CapacityBookingId,
    pub quantity: u64, pub operation_key: String, pub semantic_digest: String,
}
pub fn allocate_capacity_bookings(pool: &TransportCapacityPoolV1, requests: &[CapacityBooking], at: SimTime)
    -> Result<Vec<BookingAllocationV1>, TransportError>;
```

Order: priority desc, `valid_from` asc, tie-break key, admitted sequence,
booking id. With §15, phase 6 offers pools and phase 7 confirms or fails
bookings. Evidence: `booking_allocation_is_order_stable`.

## 17. Persons-group movement subject

Verified: `lib.rs` 56–62 `MovementSubjectRole::{MovablePrincipal, Cargo, Carrier, Passenger, Attached}`; `quantity` only for `Cargo`.

```rust
pub enum MovementSubjectRole { MovablePrincipal, Cargo, Carrier, Passenger, Attached, PersonsGroup }
// validate: PersonsGroup requires a positive quantity (head count), like Cargo
```

`EntityRef::Group` in `canwu-core` is deliberately not proposed; a domain
record reference remains the subject identity. Evidence: `persons_group_requires_headcount`.

## 18. Seizure handoffs

```rust
pub enum HandoffKind { Planned, Seizure { by: EntityRef } }
pub struct Handoff { /* ... */ pub kind: HandoffKind /* serde default Planned */ }
```

## 19. External-condition itinerary reason

```rust
pub enum ItineraryRevisionReason {
    Initial, Disaster { explanation: String }, CapacityUnavailable { explanation: String },
    KnowledgeUpdate { explanation: String }, Recovery { explanation: String },
    ExternalCondition { record: DomainRecordRef, version: u64, kind: String },
}
```

Evidence for §18–§19: `reroute_cites_external_condition_record`; serde
round-trip tests for the defaults.

## 20. Delegated carrier authority

Verified: `canwu-correspondence/src/plugin.rs` 2394 rejects `carrier != request.sender`; `docs/end-state.md` names delegated-carrier disclosure and authority as future contracts.

```rust
pub struct CorrespondenceIntent { /* ... */ pub carrier_authority: Option<DelegationClaimV1> } // canwu-information type
```

Rules: when `carrier != sender`, a claim is required whose `performed_by`
resolves to the carrier, `performed_for` to the sender, `capabilities`
contains `carry_correspondence`, and whose validity window covers dispatch;
the lifecycle system reads the carrier's ledger for planning and disclosure;
without a valid claim the existing rejection stands. Evidence:
`delegated_carrier_requires_valid_claim`.

## 21. Carrier seizure incident

```rust
pub enum CorrespondenceIncidentKind {
    Disaster { .. }, Interception { .. },
    CarrierSeized { seized_by: EntityRef, custody_handoff: HandoffId },
}
```

Rules: terminates the delivery attempt; pairs with §18 so custody change is
transport evidence. Evidence: `carrier_seized_terminates_attempt`.

## 22. Authenticity finding on interpretation

Verified: `canwu-information/src/model.rs` 270 `InterpretationPayload { interpreted_at, status, capability, confidence_per_mille }`.

```rust
pub struct AuthenticityFinding { pub claimed_source_accepted: bool, pub basis: String, pub confidence_per_mille: u16 }
pub struct InterpretationPayload { /* ... */ pub authenticity: Option<AuthenticityFinding> }
```

Rules: written by the interpreting holder's `Interpret` operation; validation
requires the finding to cite the representation version being interpreted.
Evidence: `authenticity_finding_binds_to_representation_version`.

## 23. Weighted and unit-block procedure stages

Verified: `canwu-law/src/model.rs` 240 `ProcedureStageDefinition { quorum: u16, threshold: u16, .. }` counts seats equally.

```rust
pub struct ProcedureStageDefinition {
    // ...
    pub seat_weights: BTreeMap<String, u16>,      // empty = equal
    pub block_of_seat: BTreeMap<String, String>,  // seat -> block
    pub block_threshold: Option<u16>,             // blocks required when block_of_seat is non-empty
}
```

Evidence: `eight_seats_in_eight_blocks_threshold_five_with_chair_tie_break`.

## 24. Advisory stage kind

```rust
pub enum ProcedureStageKind { Deliberation, Veto, Signature, Review, Ratification, Consultation }
```

Rules: ballots persist as evidence and never count toward completion; the stage
completes at its deadline. Evidence: `consultation_with_zero_ballots_completes_and_opens_next_stage`.

## 25. Policy-pressure provenance

Verified: `canwu-society/src/model.rs` 308 `PolicyPressure` has no issuer.

```rust
pub struct PolicyPressure { /* ... */ pub issuer: Option<EntityRef>, pub decision_version: u64 /* default 0 */ }
```

## 26. Cohort headcount rebase

```rust
pub enum RebaseReason { ExternalStockChange, Correction }
pub struct CohortHeadcountRebaseV1 {
    pub cohort_id: String, pub new_headcount: u64,
    pub external_stock: DomainRecordVersionRef, pub reason: RebaseReason,
}
pub const COHORT_REBASE_INGRESS: &str = "cohort_headcount_rebase_v1";
```

Rules: buckets re-proportioned deterministically with remainders; a ledger entry
is written; a stale cited version rejects. Evidence for §25–§26:
`rebase_rejects_stale_stock_version_and_conserves_bands`.

## 27. Culture boundary system and exposure ingress

Verified: `canwu-culture/src/plugin.rs` registers only the state record schema and a maintenance participant; `settle_culture_society_boundary` (`society.rs` 537) is a host-called library function; `canwu-society` already registers five boundary systems.

```rust
pub const CULTURE_EXPOSURE_INGRESS: &str = "culture_exposure_v1";
pub struct CultureExposureSignalBatch {
    pub target_generation: u64, pub cohort_scope: Vec<String>,
    pub fidelity_per_mille: u16, pub evidence: Vec<EvidenceRef>, pub earliest_boundary: BoundaryId,
}
// boundary system `culture_lifecycle_settle_v1`: phase 7, SystemCadence::Monthly, SameBoundary
```

Rules: admits exposure batches and accepted institutional decisions, calls
`settle_culture_society_boundary`, persists `canwu.culture:state`, and hands
the society delta to the society plugin's owner path (one writer of
`canwu.society:state`); emits `CulturalSignalBatch` as next-boundary ingress
through compiled `emit_effect` bindings. Evidence:
`culture_settlement_runs_in_boundary_and_keeps_single_society_writer`.

## 28. External transmission source

Verified: `canwu-technology/src/query.rs` 841–849 requires an exact live `source_capability` for `Demonstration`, `Apprenticeship`, `PersonnelTransfer`.

```rust
pub enum TransmissionSource {
    Live { capability: DomainRecordVersionRef },
    External { evidence: EvidenceRef, declared_reliability_per_mille: u16 },
}
pub struct TransmissionOpportunityPayload { /* source_capability replaced by */ pub source: Option<TransmissionSource>, /* ... */ }
```

Rules: `External` is accepted for the three practice modes with the same
open/close immutability; validation requires the evidence to be a
manifest-bound content record. A serde alias keeps `source_capability`
readable on the new engine only. Evidence: `external_source_admits_practice_transmission`.

## 29. Series-scale items (deferred, recorded)

- `canwu-fiscal` terminal-record archive mirroring `canwu-resource`
  (`prepare_fiscal_archive`, `enqueue_fiscal_archive`,
  `finalize_fiscal_archive_retention`, `FiscalState.archive_roots`);
  fixture: fill `MAX_FISCAL_ASSESSMENTS`, close a cycle, show the next
  `OpenAssessment` rejected while closed records cannot move cold.
- `FiscalObserverBinding.actor: Option<PersonId>` and configurable
  `max_observers` (today `MAX_FISCAL_OBSERVERS = 64`, `actor` mandatory).
- `ResourceConsumptionBatchRequestV1` under one lease, only if profiling shows
  lease cost dominates.
- `ProductionLimitsV1` review (sites, work orders, executions ≥ 16,384 or
  sharding by site scope; mutation batching contract).
- `TraversalModel::Recurring { period, offsets, duration }` for compact
  timetables.

## 30. Lockstep publication of the military crates

`canwu-military` and `canwu-military-reference-content` use
`version.workspace = true` but are published only at 0.10.0 and are absent from
`docs/releasing.md` "Publish order". Add them to groups 6 and 7 and publish
them with the next release regardless of downstream adoption, so that "all
first-party crates version in lockstep" holds on the registry as well as in the
workspace.

---

## Release grouping

- **0.12.0 (first release):** §1, §2, §3, §7, §8, §9, §10, §11, §12, §13, §14,
  §17, §18, §19, §30, plus `versioning.md`, `end-state.md`, terminology, and
  website mirrors. All additive; format 8 retained.
- **0.13.0:** §4, §5, §6, §15, §16, §20, §21, §22, §25, §26, §27, §28, and
  §23–§24 if the downstream application confirms `canwu-law` adoption.
- **Later:** §29.

## Verification evidence expected for every section

- A public-API fixture that fails on 0.11.1 and passes on the branch, using
  only `canwu-api` and the extension's public types.
- Save/load, fork, and exact-replay equality on a run that exercises the new
  contract; strict JSON loading rejects unknown fields as today.
- Ownership and authority tests where a contract adds a writer or an admission
  rule (declared-writer rejection, duplicate-writer rejection).
- Determinism tests for anything ordered (§16 allocation, §2 ids).
- `docs/architecture.md`, `docs/end-state.md`, the crate README, and the
  terminology table updated in both languages; `docs/versioning.md` gains the
  release paragraph.

## Open questions

1. §1: should `Hostage` block command issuance by default, or is that an
   application rule? The sketch admits it.
2. §2: is the receipt-keyed correlation sufficient, or should `CreatePerson`
   accept a caller-proposed `PersonId` from a reserved application range?
3. §4: should a manifest be allowed to span two boundaries (`ready_at` in the
   future) so participants can stage across a report boundary?
4. §6: evidence-journal growth — should traces be sampled by rule id under a
   run-configuration trace policy rather than always recorded?
5. §15: name — `canwu-movement` versus `canwu-transit`; the terminology table
   must add the paired Chinese term before publication.
6. §28: replacing `source_capability` with an enum is a shape change; an
   alternative is an additional optional `external_source` field with a
   validation rule that exactly one is present.
7. §23: should block counting live in `canwu-law` or in a generic ballot
   helper under `canwu-decision` that both law and institutional society
   decisions reuse?
