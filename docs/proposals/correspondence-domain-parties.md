# Correspondence Domain-Entity Parties, Withheld Reports, and Refused Seeds

Status: implemented in 0.20.0. It closes downstream gap G-72 (a domain-entity
knowledge holder can be no correspondence party), downstream gap G-78 (a
seizure report to a carrier who is no longer alive fails the boundary), and
the correspondence half of engine finding E-41 (malformed connection
knowledge fails the boundary) of `opus-progress.md`. It changes
`canwu-correspondence` only; `canwu-sim`, `canwu-api`, `canwu-information`,
and `canwu-routing` are untouched. Same version only: no compatibility work
with 0.19.0 bytes or hashes.

"Today" means 0.19.0 as it is on `main`.

## Contents

1. [Problem](#problem)
2. [Invariant](#invariant)
3. [Design](#design)
4. [Persistence and replay](#persistence-and-replay)
5. [Refusals](#refusals)
6. [What can still fail a boundary](#what-can-still-fail-a-boundary)
7. [Public API delta](#public-api-delta)
8. [Durable tests](#durable-tests)
9. [Documentation delta](#documentation-delta)
10. [Terminology candidates](#terminology-candidates)
11. [Known limits](#known-limits)

## Problem

**G-72.** The plugin wrote every party reference as a core target:
`operation_references` named the sender through `core_reference` and the
recipient and carrier through `holder_reference`, which wrapped a holder's
entity in `DomainReferenceTarget::Core`; `opportunity_references` did the same
for the sender and each candidate, and the knowledge-seed receipt for its
holder. The delivery-attempt binding (`recipient`) and the interception access
binding (`holder`) that the plugin hands to `canwu-information` were built by
the same helper. `canwu-sim` refuses a core alias of a domain entity in a
record's references, so any record naming a domain-entity holder (an
institution modelled as a domain entity with `KnowledgeHolderPolicy::Allowed`)
failed validation and the boundary with it. `initiate_correspondence_command`
also matched the prepared dispatch's `sender` reference only as
`DomainReferenceTarget::Core(request.sender)`, while `canwu-information`
requires a dispatch to name a domain-entity sender by its domain record, so a
domain-entity sender could never match. The carrier delegation record was not
affected: it holds no references and is keyed by a hash of the carrier and
principal.

**G-78.** A triggered `CarrierSeized` incident closes the attempt through an
information operation; when that operation completes, the lifecycle schedules
the internal `correspondence_attempt_report_v1` packet, which the host can
neither author nor withdraw. Phase 13 then published the
`CorrespondenceAttemptReport` to `operation.intent.carrier` with no holder
check. `canwu-sim` refuses a publication to a dead person's ledger
(`validate_holder_for_publication`, `InvalidKnowledgeHolder`), so a carrier who
died between his seizure and its closure failed the boundary that reported it
to him, and every retry of that boundary. A knowledge seed had the same
defect: a seed queued for a holder who died, or a domain entity retired, before
its admission failed the boundary at publication.

**E-41 (correspondence half).** A seed's connection facts were published as
they came (the knowledge schemas take `PayloadSchema::Any`). A connection that
no planning network admits, such as a `TraversalModel::Recurring` whose offset
is not inside its period, then failed `RoutingNetwork::new` in every planning
read of that carrier (start, replan, and retry), and
`carrier_planning_snapshot` reported it as `InvalidDomainRecord`, failing each
of those boundaries.

## Invariant

- Every correspondence record names a party in the form the runtime admits: a
  domain entity by its domain record, any other entity by its core identity.
- A carrier takes new carrying (a correspondence, a retry, a delegation) only
  while the engine would publish knowledge to it; records that already name a
  retired party keep naming it.
- The knowledge system never proposes a publication the engine refuses for its
  holder or its size, and never installs a connection fact that no planning
  network admits. Each such refusal is recorded (on the seed's receipt, or as
  an event for a report); the boundary stands.

## Design

### Party references (G-72)

`entity_reference(role, &EntityRef)` writes `DomainReferenceTarget::Domain`
for `EntityRef::Domain(record)` and `DomainReferenceTarget::Core` otherwise,
as `canwu-information`'s `custodian` and `holder` roles name theirs;
`holder_reference` now goes through it. Every party reference uses it: the
operation's `sender`, `recipient`, and `carrier`; the opportunity's `sender`
and each `candidate`; the seed receipt's `holder`; and the delivery-attempt
`recipient` and access `holder` bindings passed to `canwu-information`. The
dispatch's sender reference is matched by the entity it names, in either
form.

A domain-entity sender or carrier issues its commands under
`DecisionOrigin::Institution { institution: EntityRef::Domain(..), .. }`; the
authority rule (`origin_controls_sender`) and the delegation record are
unchanged.

**Deviation from the row: party roles admit a retired entity.** The row
recommends only the domain target. With that alone every party entity would be
pinned: when a record changes, the runtime re-validates every record that
references it (`validate_affected_record_store`), and a role with
`allow_retired: false` refuses a retired target, so a correspondence record
(history that is never deleted) would fail its party owner's retirement
boundary. The operation's `sender`, `recipient`, and `carrier`, the
opportunity's `sender` and `candidate`, and the seed receipt's `holder` are
therefore declared with `allow_retired: true`. Deleting a referenced party is
still refused: a tombstone holds no references, and no record may reference
one.

### Carriers that take no new carrying

`allow_retired: true` on the `carrier` role removed the only lifecycle check
on a carrier: the delegation record, `current_carrier_authority`,
`validate_delegation_claim`, the engine's command-authority check, and the
carrier's knowledge read all accept a retired domain entity. The sender and
recipient keep one through `canwu-information`, whose dispatch roles refuse a
retired target. So the carrier is now checked with
`knowledge_holder_accepts_publication`, the same rule as the reports and
seeds, and refused with `InvalidAuthority` (an expected command rejection,
recorded on the command attempt):

- when `initiate_correspondence_v1` is admitted;
- when a `RetryDelivery` recovery is admitted;
- for the claim's `performed_by` when `delegate_carrier_v1` is admitted.

The rule refuses a dead person as well as a retired, deleted, or ineligible
domain entity. That changes 0.19.0, which let a dead person carry: a dead
carrier is no plausible input, and its seizure report would be withheld
anyway. One rule for every party check is simpler than an active-entity check
beside the holder rule.

The carrier can change between admission and settlement (the start settles at
the next boundary, the retry likewise). Both settlements check it again: a
start whose carrier is refused creates no operation and emits
`canwu.correspondence.start_skipped.v1` (`START_SKIPPED_EVENT`, declared by the
lifecycle system), with the sender affected; a retry is recorded in
`recovery_history` without a new attempt, with `last_error` "retry carrier
accepts no knowledge", as a retry with no route is. Replanning the current
attempt does not check the carrier: the attempt already in flight continues.

### Withheld attempt reports (G-78)

At the report boundary the knowledge system asks
`SimulationView::knowledge_holder_accepts_publication(carrier)`, the engine's
own holder rule against the state the publication would be checked against.
It is broader than "alive" (it also refuses a missing army or government, a
retired, deleted, or ineligible domain entity, a person named as an entity,
and a kind that holds no knowledge), as `canwu-information` does. When it
refuses, the system proposes `BoundaryDirective::Emit` of
`canwu.correspondence.attempt_report_withheld.v1`
(`ATTEMPT_REPORT_WITHHELD_EVENT`, declared in the system's `emits`, with the
sender as its affected entity) instead of the publication. The check runs
where the report is published, not when it is
scheduled, so a death anywhere between the seizure and the report is covered.
The attempt itself still closes as failed; only the report is withheld.

### Refused seeds (G-78, E-41)

Each admitted seed key now settles to one `KnowledgeSeedReceipt` whatever the
outcome, with `refusal: Option<KnowledgeSeedRefusal>`:

1. the holder is checked first with `knowledge_holder_accepts_publication`;
   a refusal is `HolderUnavailable`;
2. then the seed's size against one publication directive's limits: more
   than `KnowledgeLimitsV1::records_per_batch` (1,000) endpoint, connection,
   and address records together, or one record whose payload, measured as the
   engine charges it (`KnowledgeRecordDraft::payload_bytes`), is more than
   `payload_bytes_per_record` (65,536), is
   `TooLarge { records, largest_record_bytes }`;
3. then each connection fact in seed order: it is compiled with
   `RoutingNetwork::new` in a network of its own two endpoints, which checks
   what the fact itself states (distinct endpoints, risk at most 1,000 per
   mille, an availability window that is not inverted, a well-formed traversal
   model); the first failure is `InvalidConnection { connection, reason }`,
   with the router's message as `reason`.

A refused seed publishes nothing. Its receipt references the holder only when
the holder accepts knowledge, since a refused holder may be one no record can
reference (a missing army, a deleted entity); the `holder` role is therefore
no longer required. The key is spent as for a published seed: an exact resend
is ignored, and a resend with different input under the same key is the
existing idempotency conflict.

Endpoint facts and address facts state nothing a network could refuse on its
own. Whether the holder knows a connection's endpoints depends on the rest of
its ledger and is still decided when the carrier plans.

### Routing index

Correspondence still plans with plain `plan_route` over a snapshot built for
each planning read. Compiling a `RoutingIndex` for that one query gains
nothing; the index pays only when it is kept across queries, which needs a
cache keyed by the carrier's read-set digest and its persistence story. Not
trivial, so left as a known limit.

## Persistence and replay

| Item | 0.19.0 | 0.20.0 |
| --- | --- | --- |
| `KnowledgeSeedReceipt` (record `canwu.correspondence.knowledge_seed`) | `seed_key`, `canonical_input_hash`, `holder`, `published_at` | `published_at` is renamed `settled_at` (the boundary time the seed settled, published or refused); adds `refusal: Option<KnowledgeSeedRefusal>`, omitted when `None` |
| `KnowledgeSeedRefusal` | — | new, internally tagged (`type`: `holder_unavailable`; `too_large` with `records` and `largest_record_bytes`; or `invalid_connection` with `connection` and `reason`), unknown fields refused |
| Party references | `Core` targets only | a domain-entity party is a `Domain` target; 0.19.0 could not write one, so no 0.19.0 run held one |
| Record schemas (plugin descriptor) | party roles `allow_retired: false`; seed `holder` required | party roles of the operation, opportunity, and seed receipt `allow_retired: true`; seed `holder` optional |
| `correspondence-lifecycle-v1` contract | no `emits` | `emits: ["canwu.correspondence.start_skipped.v1"]` |
| `correspondence-knowledge-ingress-v1` contract | no `emits` | `emits: ["canwu.correspondence.attempt_report_withheld.v1"]` |
| `SEMANTIC_HASH` | `a197a514…5685` | `c26226b9…cfcf`, BLAKE3 of `canwu-correspondence plugin semantic hash 0.20.0: domain-entity parties, withheld attempt reports, refused knowledge seeds` |
| Knowledge schema hashes | — | unchanged: no knowledge payload changes; the seed check narrows what the seed path installs, not what a stored record may hold |
| Snapshot, continuation, journal formats | — | unchanged |

The semantic hash changes, so a run saved by 0.19.0 with the correspondence
plugin does not restore; under the same-version policy none is expected to.

Replay: each new decision reads current state only (holder and carrier
eligibility from the engine's holder rule, the seed's own content, the
persisted operation), no retained evidence, so a sealed run and its replay
decide alike. The receipts, the events, and the command rejections are
ordinary boundary outputs, replayed exactly.

## Refusals

| Situation | 0.19.0 | 0.20.0 |
| --- | --- | --- |
| A record names a domain-entity party | failed the boundary | admitted |
| A dispatch names its domain-entity sender by its domain record | the handler's `InvalidDomainRecord`, not an expected command rejection, failed the boundary | admitted |
| Attempt report to a carrier that accepts no knowledge | failed the boundary, on every retry | withheld; event `canwu.correspondence.attempt_report_withheld.v1` |
| Correspondence, retry, or delegation whose carrier accepts no knowledge (retired, deleted, dead, ineligible) | admitted for a dead person; a domain entity could be no carrier (G-72) | command rejected (`InvalidAuthority`) |
| Carrier refused between admission and settlement | — | start: no operation, event `canwu.correspondence.start_skipped.v1`; retry: recorded with `last_error`, no new attempt |
| Seed for a holder that accepts no knowledge | failed the boundary | receipt with `HolderUnavailable`, no holder reference |
| Seed over one directive's record count or record size | failed the boundary | receipt with `TooLarge`, nothing published |
| Seed with a connection no network admits | published; every later planning of that carrier failed its boundary | receipt with `InvalidConnection`, nothing published |

## What can still fail a boundary

- A seed key reused with different input (`IdempotencyConflict`), and a seed
  or incident payload that does not decode (`InvalidPayload`), as in 0.19.0.
- Seeds that fit one directive but together pass the boundary's publication
  budget (32,768 records, 64 MiB): the knowledge system does not read
  `knowledge_publication_budget()` and does not defer.
- A domain-entity carrier deleted in the same boundary in which its start or
  retry settles: the settlement writes a reference to the tombstone
  (`DomainRecordReferenced`). A carrier deleted earlier, after its delegation
  was recorded, is refused at admission or skipped at settlement like a
  retired one.
- An opportunity naming a missing or deleted sender or candidate: the
  opportunity record fails reference validation (a missing core entity
  `InvalidDomainRecord`, a missing domain entity `DomainRecordNotFound`, a
  deleted one `DomainRecordReferenced`); a retired one is admitted.
- An interception naming an interceptor that `canwu-information`'s access
  `holder` role refuses (a retired, deleted, or missing domain entity, or a
  missing core entity): confirmed in its source, the access plan checks only
  the holder's kind (`single_holder_role` in `validate_access_context`), so
  the access record then fails reference validation (`allow_retired: false`)
  in the boundary that applies the information operation. A dead person is a
  valid reference; `canwu-information` leaves its publication unpublished.
- Planning reads. A carrier ledger whose planning network still does not build
  (a known connection whose endpoint the carrier does not know; a malformed
  fact installed by the initial scenario or another publisher of these
  schemas), a missing recipient address, or a ledger past the bounded query
  fails a start, replan, or retry boundary as `InvalidDomainRecord`; a start
  with no route also fails (a replan or retry with no route is recorded, as
  in 0.19.0).
- Deleting a domain entity that a correspondence record references
  (`DomainRecordReferenced` in the deleting owner's boundary), and changing an
  entity record referenced by more records than the runtime's affected-record
  closure (16,384).
- Command refusals the handlers raise as `InvalidDomainRecord`, which the
  engine does not count as an expected command rejection
  (`is_expected_command_rejection`), so they fail the command's boundary as in
  0.19.0: on `initiate_correspondence_v1` a malformed key or profile, a zero
  execution or dispatch version, a deadline before admission, a prepared
  dispatch that is missing, stale, not prepared, not single-recipient, or of
  another channel profile, or a sender that does not match it; on
  `resolve_correspondence_v1` a missing operation, an overlapping
  information operation, a recovery the operation's state does not allow, or
  a non-terminal attempt or inactive dispatch. Authority refusals (including
  the new carrier check) are `InvalidAuthority` and recorded. Recoding these
  refusals is outside this package; a candidate engine finding.
- The other lifecycle and incident refusals of 0.19.0: sequence gaps, stale
  dispatch evidence, two updates of one operation in a boundary, and a
  malformed incident (a zero custody handoff, a malformed seizer, the carrier
  seizing itself, a terminal operation, a probability or extent over 1,000).

## Public API delta

`canwu-correspondence` only:

- `KnowledgeSeedRefusal` (new enum, re-exported from the crate root),
  `KnowledgeSeedReceipt::refusal` (new public field), and
  `KnowledgeSeedReceipt::settled_at` (renamed from `published_at`).
- `ATTEMPT_REPORT_WITHHELD_EVENT` and `START_SKIPPED_EVENT` (new constants).
- Behaviour: domain-entity parties are admitted everywhere; the dispatch
  sender is matched in either form; a carrier the engine refuses as a
  knowledge holder is refused for a correspondence, a retry, or a
  delegation; seeds and reports are refused as above.

The new and renamed fields break struct literals and field reads of
`KnowledgeSeedReceipt` downstream, as any 0.x minor release may. No `canwu-api` or `canwu-sim` change, so no
`gated!` change.

## Durable tests

`crates/extensions/canwu-correspondence/tests/gap_g72_correspondence_domain_entity_parties.rs`:

1. **`gap_g72_correspondence_domain_entity_parties`**: three court entities of
   a fixture plugin (holder policy `Allowed`). The east court's prepared
   dispatch names it and the west court by their domain records; the envoy
   court holds the route seed and accepts carrying for the east court under
   its own institution authority; an automatic opportunity of the east court
   selects the west court. The seed receipt, the opportunity, and the
   operation name every court by its domain record, the delivery settles,
   the owner retires the envoy court after it, a new correspondence naming
   the retired court as carrier is a recorded `InvalidAuthority` rejection,
   and restore and exact replay match.

`crates/extensions/canwu-correspondence/tests/gap_g78_correspondence_withheld_reports_and_refused_seeds.rs`:

2. **`gap_g78_correspondence_report_to_a_dead_carrier_is_withheld`**: a sender
   carrying its own letter is seized on the road and dies in the same
   boundary. The attempt fails, the carrier's ledger holds no attempt report,
   one withheld event is recorded, a later seed for the dead carrier is
   refused (`HolderUnavailable`, no reference, nothing published), every
   boundary stands, and restore and exact replay match.
3. **`e41_correspondence_malformed_connection_seed_is_refused`**: in one
   boundary a seed with a recurring connection whose offset lies outside its
   period is refused (`InvalidConnection` naming it, holder referenced) and a
   valid seed for the same holder is published; an exact resend of the
   refused seed changes nothing; restore and exact replay match.

Each was checked inline (not committed) to fail under a plausible regression:

| Mutation | Test 1 | Test 2 | Test 3 |
| --- | --- | --- | --- |
| party references written as core aliases (0.19.0) | fails (the seed boundary: `InvalidDomainRecord`) | not run | not run |
| dispatch sender matched only as a core target (0.19.0) | fails (the handler's `InvalidDomainRecord` fails the start command's boundary) | not run | not run |
| no carrier check when a correspondence is admitted | fails (the later command reaches the completed-dispatch check, whose `InvalidDomainRecord` fails its boundary) | not run | not run |
| party roles refuse a retired entity | fails (the retirement: `DomainRecordReferenced`) | not run | not run |
| no holder check before the attempt report | not run | fails (`InvalidKnowledgeHolder`) | passes |
| no holder check before a seed | not run | fails (`InvalidKnowledgeHolder`) | passes |
| no connection check on a seed | not run | passes | fails |

The other new refusals (the delegation and retry admission checks, the
settlement skips, `TooLarge`) have no committed test, per the owner's
instruction against extensive testing.
The existing correspondence tests and the `routed_correspondence` example pass
unchanged.

## Documentation delta

Done in this package: `docs/architecture.md`, the correspondence section (the
withheld report; a paragraph on domain-entity parties and refused seeds), and
the crate rustdoc (`lib.rs` sections "Parties" and "Knowledge seeds").

For the release documentation pass (canonical page and both website languages
each):

- `website/src/content/docs/{,en/}architecture/knowledge-information.mdx`:
  the phase-13 sentence on `correspondence-knowledge-ingress-v1` adds that a
  seed whose holder the engine refuses, or with a connection no network
  admits, publishes nothing and its receipt records why, and that an attempt
  report to a carrier the engine refuses is withheld with
  `canwu.correspondence.attempt_report_withheld.v1`; the knowledge table row
  for `attempt_report` adds "unless the carrier accepts no knowledge then".
- `website/src/content/docs/{,en/}tutorials/cases/routed-correspondence.mdx`:
  the `CarrierSeized` bullet adds the withheld report; the seed paragraph adds
  the receipt and its refusal; one sentence that any knowledge holder,
  a domain entity included, can be a sender, recipient, carrier, or seed
  holder.
- `docs/versioning.md`, the release entry: the notes below.
- `docs/terminology.md` and both website term pages: the candidate below.

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| seed refusal | `KnowledgeSeedRefusal`, `KnowledgeSeedReceipt::refusal` | 知识种子拒收 | Why a knowledge seed published nothing: its holder accepts no knowledge, one publication cannot carry it, or it states a connection no planning network admits. Recorded on the seed's receipt; the seed key is spent. |

The withheld report needs no term: prose says the report "is not published"
(不发布), as the movement page says of its withheld reports.

## Known limits

- **`canwu-information` still pins a party.** Its dispatch `sender` and
  `intended_recipient`, delivery-attempt `recipient`, and access `holder`
  roles refuse a retired target, so a domain entity that sent or received a
  dispatch cannot be retired while those records exist; only a party that
  appears in correspondence records alone (a carrier, a seed holder, an
  opportunity candidate) is now retirable. A candidate engine finding for
  `canwu-information`.
- **Re-validation cost.** Every change to a party's entity record re-validates
  each correspondence record that references it, so an entity named by many
  correspondences pays that on each of its updates, and a change whose
  referencing records pass the runtime's affected-record closure (16,384
  records) fails its owner's boundary. `canwu-information`'s holder roles
  have the same cost.
- **Planning-time failures** listed above still fail the boundary; the seed
  check covers only what one fact states.
- **Routing index.** Correspondence plans with plain `plan_route`; no
  compiled `RoutingIndex` is kept across planning reads.
- **A refused seed spends its key.** A host that corrects a refused seed sends
  it under a new key; a holder that becomes eligible later (a domain entity
  created afterwards) needs a new seed key too.
- **No report to the sender.** As in 0.19.0, a delegating sender learns of a
  seizure only from the application; a withheld report is not redirected.
- **An attempt in flight keeps its carrier.** A carrier retired, deleted
  (impossible while an operation references it), or dead after its attempt
  started does not stop that attempt: its legs, a replan, and its delivery
  proceed; only a seizure report to it is withheld, and a retry is refused.

## Versioning notes (for the release entry)

`canwu-correspondence` admits domain-entity parties: its operation,
opportunity, and seed records, and the delivery-attempt and access bindings it
hands to `canwu-information`, name a domain-entity sender, recipient, carrier,
candidate, interceptor, or seed holder by its domain record (downstream gap
G-72), and its party roles admit a retired entity, while a carrier the engine
refuses as a knowledge holder (retired, deleted, dead, or ineligible) takes no
new correspondence, retry, or delegation (`InvalidAuthority` at admission; a
start skipped with `canwu.correspondence.start_skipped.v1` or a retry recorded
without an attempt when it became so after admission). An attempt report to a
carrier the engine refuses is withheld with
`canwu.correspondence.attempt_report_withheld.v1`, and a knowledge seed whose
holder the engine refuses, that one publication cannot carry, or that states a
connection no planning network admits, publishes nothing and records a
`KnowledgeSeedRefusal` on its receipt (downstream gap G-78, engine finding
E-41). The plugin semantic hash changes.

Upgrading from 0.19.0: a host that worked around G-72 by using persons as
parties may now name a domain entity (holder policy `Allowed`) as sender,
recipient, carrier, or seed holder; its prepared dispatch names a
domain-entity sender by its domain record, and its commands for that entity
use `DecisionOrigin::Institution`. A dead person can no longer carry: a
correspondence, retry, or delegation naming one is rejected. A host that
withheld seeds or seizures for dead holders may keep doing so; the engine now
refuses them itself. A seed receipt no longer means "published": check
`KnowledgeSeedReceipt::refusal` (for example in a "seed installed" check), and
send a corrected seed under a new key. `KnowledgeSeedReceipt::published_at` is
now `settled_at`, and struct literals add `refusal`.
