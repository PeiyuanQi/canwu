# Society Intergroup Relations and Settlement Cost

Status: implemented in 0.18.0, in `canwu-society`. It closes downstream gap
G-34 (no relation primitive between cohort groups) and downstream gap G-33
(organization relation kinds are untyped strings), and it takes the cheap,
safe part of the society row of the 0.17.0 "Beyond fiscal" table
([world-scale fiscal](world-scale-fiscal.md#beyond-fiscal-caps-that-block-the-target)):
a settlement writes the society root once instead of up to four times,
rebases and transfers no longer copy and validate the whole root each, the
pending transfer record holds only transfers in flight, and an opt-in
retention bounds the cohort exchange ledger. The root itself is still one
record; that limit is stated with numbers below.

The contracts this proposal changes are
[society settlement cadence and runtime revision](society-cadence-and-runtime-revision.md)
and [fiscal and society limits](fiscal-and-society-limits.md). The durable
fixtures are, under `crates/extensions/canwu-society/`,
`tests/gap_g33_g34_society_relations.rs`,
`latest_settlement_retention_bounds_the_rebase_outcomes` in
`tests/society_ingress_queue.rs`, the pending-record and late-packet
assertions of `a_batch_completes_every_transfer_that_still_fits` in
`tests/society_transfer_settlement.rs`, and the one-write assertions of
`tests/gap_g40_society_settlement_cadence.rs`.

## Intergroup relations (downstream gap G-34)

### Problem

Every society target is an idea or an affiliation: a distribution says how a
cohort stands toward a target, an influence edge carries a target's signal
into a cohort, and an organization relation links two organizations. Nothing
says how one group of people regards another. The downstream application
keeps its own record family for that (one record per unordered pair of
groups and scope, with acceptance, tension, and trust, memory references, and
directional offsets for the cases where A's view of B differs from B's view
of A). The filed fix is a record
`IntergroupRelationEdge { source_class, target_class, region: EntityRef,
acceptance_per_mille, tension_per_mille, trust_per_mille, evidence }`.

### Decision

**A directed, typed, bounded relation between two cohort classes in a
region.** A cohort class is the set of cohorts that carry one classification
value: `CohortClass { dimension, value }` names every cohort whose
`classification[dimension]` is `value`. Both are the classification's
authored text, compared exactly: any non-empty text of at most
`MAX_COHORT_CLASS_TEXT_BYTES` (4,096) bytes, spaces and non-ASCII text
included, so a relation can name any class a cohort was authored with
(`canwu-culture` bounds classification text at 4,096 bytes by default too).
An `IntergroupRelationEdge` has

| Field | Meaning |
| --- | --- |
| `id` | canonical identifier, the key |
| `kind` | what the relation is about; one of `IntergroupRelations::kinds` when that set is not empty |
| `source_class`, `target_class` | the class whose stance this is, and the class it is held toward; never equal |
| `region` | a territory, a government (its jurisdiction), or a domain entity record an application uses for a region; absent for wherever the classes live |
| `acceptance_per_mille`, `tension_per_mille`, `trust_per_mille` | the stance, 0 to 1,000 |
| `evidence` | up to `MAX_INTERGROUP_RELATION_EVIDENCE` (16) `EvidenceRef`s, as the application gave them, without repeats |
| `revision` | one when authored or created; each applied change adds one |
| `revised_by`, `revised_at` | the admitted packet and the time of the latest revision; absent on an authored edge no packet has revised |

The kind, the two classes, and the region are the edge's identity: they never
change, and no two edges share them. The edge is directed, so a relation
whose sides differ is two edges, and a symmetric one is two equal edges; that
replaces the downstream's directional offsets without a second mechanism.

**Its own record.** The edges live in `IntergroupRelations` (record
`canwu.society:intergroup-relations` / `root`, schema version 1, payload
`{ schema_version, kinds, edges }`), not in the society root. The root is
rewritten whenever a settlement changes it, and the relations change at the
pace of a host's model, so keeping them apart means a settlement never
rewrites the relations and a revision never rewrites the root. It also gives
the relations a record version of their own that changes only with them,
which a knowledge record or a host log can cite as exact evidence.

**A region is an identifier, not a reference.** The society checks an
edge's region once, when a revision creates the edge: it must be an existing
territory or government, or an active domain entity record. The relations
record binds no record references (validation refuses one that carries
any), so the region's owner stays free to retire or delete it. A reference
would let any host pin another plugin's entity with a public packet: edges
are never removed and the region is part of their identity, so the owner's
later deletion would be refused by the engine's reference check and fail
the owner's boundary. An edge whose region was retired or deleted since
keeps it as recorded history: `load_intergroup_relations` returns it as
stored, a revision of the edge still applies (its identity is unchanged and
was checked when it was created), and a new edge cannot name that region
(`unknown_region`). A host that needs to know whether a region still exists
asks the engine.

**Authored by the scenario.** `IntergroupRelations::into_record` encodes the
record beside the society root. The run's creation (`validate_activation`)
and every restore (`validate_society_runtime`) check that some cohort of the
root carries each class an edge names. Cohorts and their classifications
never change at runtime (no operation of a running society plugin adds,
removes, or reclassifies a cohort), so the check holds for the whole run.

**Changed by canonical ingress, applied at admission.** The public
`intergroup_relation_revision_v1` packet (`IntergroupRelationRevisionV1`,
class `Information`) carries one to `MAX_INTERGROUP_RELATION_CHANGES`
(1,024) changes in strictly ascending ID order. A change states the edge in
full (identity, values, evidence) with an `expected_revision`: zero creates
the edge, otherwise it must be the edge's current revision. The event-driven
system `intake-intergroup-relations` (phase 7, `SameBoundary`) applies the
packets of a boundary in admission order, each as a whole or not at all, so
the systems of later phases in that boundary read the revised relations. A
change to an existing edge is checked for its revision and its unchanged
identity; one that creates an edge is also checked for its kind, its two
classes, its region (an existing territory or government, or an active
domain entity), and its identity against every other edge. Relations are
host-model state, not decisions, so the packet carries no actor authority,
as a cohort rebase does not.

| Refusal | Code |
| --- | --- |
| The payload does not decode, or is not a well-formed revision | `malformed_payload` |
| `expected_revision` is not the edge's current revision (zero while it does not exist), including a resend of an applied change, or the edge is at the last revision (`u64::MAX`) | `stale_revision` |
| A change alters an edge's kind, classes, or region, or a new edge repeats another's | `identity_conflict` |
| A new edge's kind is not in `kinds` | `unknown_kind` |
| A new edge names a class no cohort carries | `unknown_class` |
| A new edge's region is not a live territory, a government, or an active domain entity | `unknown_region` |
| The set would hold more than `MAX_INTERGROUP_RELATIONS` (4,096) edges | `relations_full` |

A refused revision changes nothing and is reported by one
`society_intergroup_relation_rejected_v1` event whose summary names the
ingress and, when one change caused it, that relation, and ends with the
code. The boundary commits.

**Read by hosts; actor knowledge is published.** `load_intergroup_relations`
returns the authoritative relations, as `load_society_state` returns the
root: a host and research read. Society keeps no observer model for
relations and gives an observer no estimate of one; an actor learns of a
relation through knowledge the application publishes, which can cite the
relations record version it read. A society projection is unchanged.

**Not used by the settlement.** No transition, mobilization candidate, or
projection reads the relations. They are a state primitive for hosts: Canwu
validates, versions, persists, and replays them and leaves their causes and
effects to the application, which keeps the period-specific model (what
raises tension, what a relation does to recruitment or unrest) above the
engine.

**Kept.** An edge is never removed. The edges a society can hold are bounded
by `MAX_INTERGROUP_RELATIONS`, and their classes and kinds by what the
scenario authored. Removing one would need a tombstone that
keeps its revision, or a re-created edge could accept a change computed
against its predecessor; a host that no longer needs a relation sets the
values it reads as neutral. The earlier values stay in the record-change
evidence until the history is sealed.

## Organization relation vocabulary (downstream gap G-33)

`SocietyState::relation_vocabulary: BTreeSet<String>`, as filed: while it is
empty any canonical identifier is admitted as `OrganizationRelation::relation`;
otherwise validation refuses a root whose relation names another kind. Each
entry must be a canonical identifier. It is omitted from the encoding while
empty, so a root without one encodes byte for byte as in 0.17.0, and the
society root keeps schema version 2. Organization relations are authored only
(no society operation creates or changes one), so validation at creation,
restore, and every write is the whole check. `IntergroupRelations::kinds`
applies the same mechanism to relation kinds, in the relations record rather
than the root, so the relations validate on their own record.

## Settlement cost

### Problem

The 0.17.0 table records that the society's state, ledger, and pending
records are whole roots rewritten at every settlement, and that the ledger
keeps every outcome for good. Measured at 4,000 cohorts and two targets (a
5.3 MB authored root, 10 MB once the derived views are materialized), with
two transition rules over every cohort, one influence edge per cohort, and
two observers, on a release build of this machine:

| Workload, 0.17.0 code | Time | Root writes | Record-change evidence |
| --- | --- | --- | --- |
| A `Daily` settlement that moves people | 0.85 to 0.91 s | 3 to 4 | 53 MB |
| The same with 50 queued rebases | 1.61 s | 4 | 62 MB |
| A `Monthly` settlement of 4,000 queued rebases (one per cohort) | 32.5 s, then 66.5 s in the second month | 4 | 52 MB |
| Ledger after 4,000 rebase outcomes a month | | | grows 1.4 MB a month, for good |

Three things cost most. A settlement that moved people wrote the root up to
four times: phase 7 cleared the derived views, and the phase-10, phase-12,
and phase-13 systems each decoded and validated the root, materialized one
view, and wrote it again, each write journaled with its previous record. A
rebase or a cohort transfer cloned the whole root and validated all of it,
so a settlement of `n` rebases cost `n` whole-root validations. And the
pending transfer record kept every transfer of the run, rewritten whole by
every boundary that admitted one, and the transfer intake decoded and
validated the whole root to read two cohorts and an alignment.

### Decision

- **One write per settlement.** The phase-7 settlement materializes the
  derived views its changes cleared (mobilization candidates, aggregates,
  and the projections of observers whose estimates changed) before it
  writes, and the three later systems are removed. The views are computed
  from the same state by the same functions, so the root a boundary commits
  is byte for byte the root the four systems committed: a probe that settled
  the workload above for six days, with 50 rebases on day 2, printed equal
  canonical hashes of the final root every day on the 0.17.0 code and on
  this change, at 1,000 and at 4,000 cohorts. Only record versions and the
  number of record changes differ.
- **Local checks, one validation.** A rebase and a cohort transfer read and
  write only their cohorts and those cohorts' distributions, located by key
  range, and build each new distribution canonical and totalling its
  cohort's new headcount; they change nothing when they fail. Transitions
  inside the settlement skip their own entry and exit validation. The whole
  root is validated once, before it is written. The public
  `SocietyState::rebase_cohort` and `settle_transitions` still validate the
  whole state, for hosts that own one outside the plugin.
- **A bounded pending record.** The boundary that queues a cohort transfer
  rewrites the pending record without the transfers the ledger already
  records as settled. The intake now checks the ledger before the pending
  record, so a second delivery of a settled transfer is still a no-op and one
  that reuses its operation ID with other data is still
  `operation_id_conflict`. It reads the two cohorts and the alignment it
  needs from the committed root without decoding the rest.
- **Opt-in ledger retention.** `SocietyPlugin::with_outcome_retention` takes
  `OutcomeRetention::Keep` (the default, every release's rule) or
  `OutcomeRetention::LatestSettlement`: a settlement that records rebase or
  lifecycle delta outcomes first drops those recorded at an earlier time, so
  the ledger holds one settlement's outcomes and those of packets refused for
  a full queue since. Transfer outcomes, which make a resent transfer
  idempotent, and the one or two pressure revision outcomes of each
  alignment are kept under both. Nothing the plugin decides reads an earlier
  rebase or lifecycle delta outcome (an ingress ID is queued once), so the
  retention changes only what a host can still read. The default stays
  `Keep`, so a host and the culture fixtures that count outcomes see no
  change. The retention is part of the plugin's identity, like the transfer
  settlement mode.

### Measurements after

The same probe on this change:

| Workload | 0.17.0 | 0.18.0 |
| --- | --- | --- |
| A `Daily` settlement that moves people, 4,000 cohorts | 0.85 to 0.91 s, 3 to 4 writes, 53 MB | 0.23 to 0.31 s (one outlier 0.41 s), 1 write, 14 to 20 MB |
| The same with 50 queued rebases | 1.61 s, 62 MB | 0.30 s, 20 MB |
| A `Daily` settlement, 1,000 cohorts | 166 to 174 ms, 13.4 MB | 54 to 68 ms, 3.5 to 5.0 MB |
| 4,000 queued rebases in one `Monthly` settlement | 32.5 s (66.5 s in month 2) | 0.46 to 0.92 s over six months |
| Ledger under `Keep`, 4,000 rebases a month | 1.4 MB a month | 1.4 MB a month (8.5 MB after six months; the settlement journals 17 MB in month 1 and 35 MB in month 6) |
| Ledger under `LatestSettlement` | not available | 1.40 to 1.42 MB in every month; 22 MB journaled per settlement |
| An intergroup revision at 4,096 edges (a 1.03 MB record), changing 1, 64, or 1,024 edges | not available | 29 to 41 ms, 2.06 to 2.09 MB journaled |

The machine was shared with other builds, so the times vary by about a
third between runs; the byte counts do not.

## Persistence, replay, and format

Snapshot, checkpoint, commitment, and evidence-journal formats are unchanged.

- The society root keeps schema version 2 and gains the optional
  `relation_vocabulary`, omitted while empty.
- The new record kind `canwu.society:intergroup-relations` (schema version 1)
  holds `IntergroupRelations`; it exists only when a scenario authors it or a
  revision creates it.
- The ledger and the pending record keep their shapes and schema version 1.
- The plugin descriptor loses the systems `evaluate-mobilization-candidates`,
  `aggregate-social-state`, and `materialize-society-projections`; gains the
  system `intake-intergroup-relations`, the public ingress
  `intergroup_relation_revision_v1`, the event
  `society_intergroup_relation_rejected_v1`, and the relations record schema;
  and the `intake-cohort-transfers` system now reads the ledger. The four
  pairs of transfer settlement mode and outcome retention have their own
  semantic hashes, all new.

A run is restored and replayed only with a plugin of its cadence, transfer
settlement mode, and outcome retention (`PluginManifestMismatch` otherwise).
The relations are revalidated on restore against the root's classes, and
a relations record that carries any record reference is refused.

## What can still fail a boundary

In the society plugin, as before, only an invalid society record: the root,
the pending record, the queue, the ledger, or a stored policy decision; and
now an invalid relations record, which no society operation produces. Every
refusal of the relation intake is a recorded event, a revision at the last
revision included. The relations record binds no references, so no
relation, whatever a packet named, can make the engine refuse another
plugin's write: the owner of a region retires and deletes it as it would if
no edge named it.

## Behaviour that differs from 0.17.0

- A settlement writes the society root at most once, so it journals one
  root record change, summarized "Settled society", where it journaled up
  to four: phase 7 ("Settled aggregate social transitions"), phase 10
  ("Evaluated social mobilization candidates"), phase 12 ("Aggregated
  social state"), and phase 13 ("Materialized actor-relative society
  projections"). A system of another plugin that reads the root in phases 8
  to 13 of a settling boundary now reads the derived views materialized,
  where it read them cleared.
- A refused relation revision is an event; there was no relation primitive.
- The pending record no longer holds transfers settled before the boundary
  that queues the next one.

## Known limits

- The society root is still one record rewritten whole by a settlement that
  changes it: 10 MB and about 20 MB of evidence a day at 4,000 cohorts and
  two targets, half of it the materialized views (aggregates, candidates,
  and projections, which observers with wide scopes make large). Sharding
  the root, or moving the views into records of their own, is the remaining
  L to XL fix.
- Under the default `OutcomeRetention::Keep` the ledger still grows with
  every rebase and lifecycle delta outcome, and transfer outcomes grow under
  both retentions (about 300 bytes a transfer).
- The relations record is rewritten whole by a boundary that applies a
  revision, about 30 to 40 ms and 2 MB of evidence at the 4,096-edge cap
  whatever the number of edges changed; a host revises many relations in
  one packet per period.
- Society does not follow a region's lifecycle: an edge keeps a region its
  owner retired or deleted, and only the host can tell the two apart.
- Edges are never removed, and society gives observers no estimate of a
  relation.

## Deviations from the filed fixes

- G-34 was filed as conditional on a second consumer. The owner asked to
  close every remaining gap, and the contract is generic: class, confessional,
  and ethnic relations are all relations between classification values.
- The edge gains an `id`, a typed `kind` (several relations can hold between
  the same classes), a `revision` with its packet and time, and an optional
  region (absent for wherever the classes live), held as an identifier
  rather than a record reference; `evidence` is a bounded list
  of `EvidenceRef`s. Classes are `CohortClass` classification values, and the
  edges live in their own record rather than in `SocietyState`, for the cost
  and evidence reasons above.
- G-33 is as filed. Its vocabulary mechanism is reused for relation kinds as
  `IntergroupRelations::kinds`.
- The society cost row's rough fix was "shard and archive the ledger (L to
  XL)". This change takes the cheap and safe parts (one write, local checks,
  a bounded pending record, an opt-in ledger retention) and records the
  root's size as a known limit.
