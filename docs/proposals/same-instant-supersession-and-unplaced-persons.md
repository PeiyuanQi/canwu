# Same-Instant Supersession and Unplaced Persons

Status: implemented in 0.15.0.

Two contracts of `canwu-sim` that applications reach through `canwu-api`:
which records a published knowledge record may supersede or contradict, and
how a publisher finds them (downstream gap G-58), and whether a person created
at a boundary needs a placement in the `WorldSnapshot` projection (downstream
gap G-49).

## Knowledge relations at one instant (G-58)

### Problem

A boundary system publishes holder-relative knowledge in phase 4 or phase 13,
and a record may name earlier records of the same holder in `supersedes` or
`contradicts`. Two places decide which targets are valid, and in 0.14.0 they
used different rules.

**Settlement** validates each `PublishKnowledge` draft against two sets: the
holder's ledger as committed before the boundary, and the records the boundary
has already staged with `SameBoundary` visibility. A phase's records are
staged after all of that phase's systems have run, so the second set holds
only records of an earlier phase. A target must be in one of the two sets and
have the draft's schema kind.

**Snapshot validation** rebuilt the ledger record by record from the boundary
journal. A target had to be in the ledger rebuilt so far, have a lower ID and
the same kind, and, when its learned time equalled the boundary's time, be an
earlier phase-4 `SameBoundary` record of that boundary.

The learned-time test stood in for "published by this boundary", and the two
are not the same. Several boundaries can settle at one instant, for example a
calendar boundary and a follow-on boundary that settles admitted work. The
rules therefore disagreed in two ways:

1. A target learned at the same instant in an earlier boundary was admitted by
   settlement and refused by a load. A plugin that publishes a superseding
   report in two boundaries of one instant wrote a snapshot that the run
   continued past and no load accepted.
2. A phase-4 record that named an earlier phase-4 `SameBoundary` record of its
   own boundary was refused by settlement and accepted by a load. No run
   produces it, so only a tampered snapshot could contain it.

### Decision

There is one rule, in one function that both sites call. A record published
for a holder may relate to a target only if the target

- is a record of the same holder,
- has the same schema kind, and
- is visible at the publisher's cut: it was committed by an earlier boundary,
  whatever its learned time, or it was staged by the same boundary with
  `SameBoundary` visibility in an earlier phase.

Because knowledge is published only in phases 4 and 13, the last case means a
phase-13 record relating to a phase-4 `SameBoundary` record. Settlement passes
the rule its committed ledger and its visible overlay. Snapshot validation now
rebuilds the same two sets instead of comparing times: a boundary's records
join the rebuilt ledger only when the boundary ends, and its `SameBoundary`
records of one phase become visible when the next phase begins.

Two alternatives were rejected. Limiting a plugin to one superseding record
per holder per instant would leave the simulation core writing snapshots it
refuses for every other plugin. Keeping two rules and changing only the
learned-time test would close the gap but keep the second difference and let
the rules drift again.

### Invariants

- A load accepts a relation exactly when settlement admits it.
- Record IDs are allocated in staging order, so every admitted target has a
  lower ID than the record naming it. A relation cannot point at a later
  record, at itself, around a cycle, at another holder's record, at another
  schema kind, or at a record that does not exist.
- A record of the publisher's own phase and a record published with
  `NextBoundary` visibility in the same boundary are not visible at the
  publisher's cut, in a live run or in a load.
- Every load runs the rule through the one snapshot validation: snapshot JSON
  and typed snapshots, checkpoint journals, paged checkpoints, and a compact
  run's sealed segments with its retained tail (`snapshot_with_segments`,
  `replay_journal_with_segments`, and the compact restore calls). Exact replay
  settles each boundary again and so applies the rule at settlement.

### Finding the records to supersede

The rule says which targets are valid; a publisher also has to find them. A
draft names global record IDs, and until now a boundary system could learn
them only from `SimulationView::knowledge_changes_by_correlation` and its
prefix form, which scan retained boundary evidence. After `seal_evidence`
removes the publishing boundary those reads return nothing. A plugin that took
its targets from them then published a second current record beside the one
it meant to supersede, and the sealed run diverged from the unsealed run and
from its own exact replay.

The state-backed read is

```rust
view.knowledge_heads(&holder, &kind, &subjects) -> Result<KnowledgeHeads, CanwuError>
```

It returns the global IDs of the holder's current records of one schema kind:
those that no record of the holder supersedes. Only records that carry every
one of `subjects` are returned, and an empty list selects the whole kind.

- **What it keys on.** Holder, schema kind, and typed subjects are what the
  committed ledger holds for every record. The producing system and the
  producer correlation are recorded only on boundary evidence, so the read
  cannot key on them. Nothing was added to the ledger.
- **Selection.** A record matches when it carries every listed subject, not
  when its subjects equal the list. A publisher that keeps several lines of
  records for one holder therefore gives each line a subject the others lack;
  a line whose subjects are a subset of another line's cannot be selected
  apart from it. A schema kind has one owner plugin, so lines need telling
  apart only between systems or schema versions of one plugin.
- **Visibility.** The read sees the two sets of the relation rule: the ledger
  committed by earlier boundaries, and the records the settling boundary
  published with `SameBoundary` visibility in an earlier phase. Every ID it
  returns is therefore a valid target for a record the reader publishes to the
  same holder with the same kind, and a record that is not visible at the cut
  neither appears nor hides the record it supersedes.
- **Bound.** At most `relations_per_record` IDs are returned (64, the number
  one record may supersede), in the ascending order a draft requires. When
  more records match, the newest are returned and `truncated` is set, so a
  publisher always sees its own latest record. Truncation is not an error:
  the publisher supersedes what was returned, and the older current records
  drain over its later publications. A subject filter longer than the bound
  is refused.
- **Cost.** One call walks the holder's records twice, in place: once to
  collect the IDs that records of the requested kind supersede (a relation
  never crosses kinds), and once, newest first, to select, stopping after
  one match more than the bound. There is no index to persist, roll back, or
  rebuild.
- **Authority.** It needs the declared `canwu.core.knowledge` read, as the
  other knowledge reads do. The IDs are global ledger IDs for systems that
  declared the read; actor-facing reads keep holder-local IDs.

Because the answer comes from committed state and the boundary being settled,
a live run, a run that sealed its evidence, a restored run, and exact replay
return the same IDs. The rustdoc of the two correlation reads now says that
they depend on retained evidence and must not drive a decision that has to
replay exactly.

### Persistence, replay, and format

Nothing persisted changes: snapshot format 8, commitment format 4,
checkpoint/evidence-journal format 4, the paged-checkpoint envelope, plugin
descriptors, and the hash algorithms are as in 0.14.0. A run that never
published a same-instant supersession validates as before. Settlement admits
and refuses the same proposals as before, so replay journals are unaffected.
`knowledge_heads` reads existing state and stores nothing.

### Compatibility

`SimulationView::knowledge_heads` and `KnowledgeHeads` are new; no existing
item changes. `KnowledgeRecordNotFound` and `InvalidKnowledgeRecord` are still
the settlement errors. A load still reports `InvalidSnapshot`; the message now
says that the target is not in its holder's ledger at the publisher's cut, or
that the relation crosses schema kinds.

Not covered: plugins that still take their supersession targets from the
correlation reads. `canwu-fiscal` is such a plugin until it adopts
`knowledge_heads`: after `seal_evidence` its next report supersedes nothing,
and the sealed run fails exact replay.

## Persons without a placement (G-49)

### Problem

`BoundaryDirective::CreatePerson` required its `PersonDraft` to name a
government and a territory that exist in the `WorldSnapshot` projection. An
application that keeps its governments and places in its own domain records
has no such records. To create a person it had to install one inert
government and territory that every draft names and no rule reads.

### Decision

The draft's two placement fields become one optional value,
`PersonDraft::placement: Option<PersonPlacement>`, where a `PersonPlacement`
holds a `government` and a `current_location`. A draft is placed or it is not:
half a placement cannot be built, and it cannot be decoded either, because
both fields of a `PersonPlacement` are required.

`PersonDraft` also rejects unknown fields. A draft written in the 0.14.0
shape, decoded on its own or as a field of another value, therefore fails to
decode instead of silently becoming unplaced. A draft that an application
flattens into its own struct with `#[serde(flatten)]` is not protected: the
old shape decodes there with no placement. An application must not flatten a
`PersonDraft`.

- A **placed** draft behaves as before: both records must exist, and the
  person gets a `Person` record in the projection.
- An **unplaced** draft creates an identity in the entity registry with core
  availability and a creation receipt, and no `Person` record. This is the
  kind of person a scenario already declares when it lists
  `EntityRef::Person` in `Scenario::entities` without a projection record.
  The draft's name and roles are recorded on the creation evidence only; the
  application keeps a person's attributes in its own records.

The creation evidence `BoundaryPersonCreation::person` becomes a
`BoundaryCreatedPerson`: the allocated ID with the draft's name and roles and
its `government` and `current_location`, both present or both absent. It stays
flat so that a placed creation encodes as before. `placement()` returns the
placement and `person_record()` the `Person` record of a placed creation.

Three alternatives were rejected:

- **Two optional fields on the draft.** A draft with only one of them would be
  a valid value that fails its boundary, and a system that derives its draft
  from committed state would propose it again on every retry.
- **A host-declared anchor.** The scenario would name a government and a
  territory for unplaced persons. Every such person would still carry a
  placement that readers cannot tell from a real one, and the projection would
  still need the inert pair.
- **A `Person` record with an optional placement.** Every reader of the
  projection would need a rule for a person with no location: built-in
  movement, letters, scheduled arrivals, and transit validation. Army-arrival
  reports go to every `Person` record and take one random draw each, so
  unplaced records would receive reports about a world they are not part of
  and change the draw sequence. Changing the field types of `Person` would
  also break every scenario that builds one. The ownership direction in
  `architecture.md` points the other way: the projection is a reference and
  runtime API, and availability is keyed by `PersonId` so that it applies to
  persons outside the projection.

### What each reader does for an unplaced person

| Reader | Behaviour |
| --- | --- |
| Entity registry: `entity_exists`, command authority, decision makers, custodians, knowledge subjects, domain-record references | The person exists from the end of the creating boundary and is visible to systems from the next boundary, as any created person. |
| `Canwu::world`, `SimulationView::person` | No `Person` record: the lookup returns `None`. |
| Person availability and custody | Set from the draft and changed by `SetPersonAvailability`. An unavailable person's commands are refused with `IssuerUnavailable` and its open tickets are cancelled. |
| `persons_created_by_correlation`, `BoundaryReceipt::created_persons` | Bind the allocated ID to the creating plugin, system, and correlation, without retained evidence. |
| Holder knowledge and viewers | The person is an eligible holder and subject. Its viewer reads only its own ledger. The compatibility army observations belong to projection persons; none is created for it. |
| Built-in movement, letters, and army-arrival reports | These read the projection. A movement command issued by the person is rejected with `ActorNotFound`, the person cannot be a letter party, and it receives no report and takes no report draw. |
| Extensions such as `canwu-movement`, `canwu-information`, and `canwu-correspondence` | They never read the projection and refer to the person as an `EntityRef` or a holder. |
| `canwu-reference-world` and `canwu-debug` | The reference world keeps its own person records, and the debug client lists those. A person created through the core, placed or not, is not among them. |

No reader guesses a placement, so none can panic on a missing one or expose
ground truth through one.

### Invariants

- The entity registry holds the scenario's identities and every created
  person. The projection's `Person` records are the scenario's and the placed
  creations', and nothing else.
- A placed creation names a government and a territory of the scenario and
  has a `Person` record. An unplaced creation names neither and has none. A
  load refuses any other combination.
- ID allocation, next-boundary visibility, correlation uniqueness, and the
  created-person registry do not depend on placement.

### Persistence, replay, and format

Format versions and hash algorithms are unchanged. A `PersonDraft` is never
persisted, so its new shape reaches no save. A `Person` record is not
touched. A `BoundaryCreatedPerson` of a placed person writes the same
canonical JSON as the `Person` record that 0.14.0 stored in the evidence: the
same keys in the same order, with `Some(value)` written as the value. Boundary
hashes of runs with placed creations are therefore unchanged. An unplaced
creation omits both keys, and strict loading rejects `null` for them.

A snapshot of a run with an unplaced person holds the identity in `entities`,
its `person_availability` entry, its `created_persons` entry, and the person
counter, and has no `world.people` entry for it. Fork, save/load, a sealed
continuation, and exact replay reproduce the same ID and evidence.

The descriptive `person` entry of the base `TypeSchema` is unchanged, because
it is part of every run's identity commitment and describes the projection
record, which did not change.

### Compatibility

0.15.0 is a pre-1.0 minor release. Two public shapes change:

- `PersonDraft::government` and `PersonDraft::current_location` are replaced
  by `PersonDraft::placement: Option<PersonPlacement>`. A placed draft writes
  `Some(PersonPlacement { government, current_location })`. A draft encoded as
  JSON in the 0.14.0 shape no longer decodes, except where an application
  flattened it into its own struct: there it decodes as unplaced, so such an
  application must stop flattening it.
- `BoundaryPersonCreation::person` is a `BoundaryCreatedPerson` instead of a
  `Person`. Code that needs the record calls `person_record()`.

`PersonPlacement` and `BoundaryCreatedPerson` are new and re-exported by
`canwu-api`. `Person`, `WorldSnapshot`, and `Scenario` are unchanged.
