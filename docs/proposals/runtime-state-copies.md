# Runtime State Copies

Status: implemented in 0.18.0. This package closes engine findings E-19 (the
per-command transaction checkpoint) and E-20 (per-boundary copies, `fork`,
and the restore of a large unsealed snapshot) in `canwu-sim`. It changes no
public signature and no persisted format, hash, or replay input: the
snapshot of the world-scale fiscal run below is byte-identical before and
after (661,800,813 bytes).

The release entry will be in [Versioning and Persistence](../versioning.md).
The earlier steps of the same kind are the persistent record store and
version index ([record-store ceiling and cost](record-store-ceiling-and-cost.md)),
the shared knowledge state
([knowledge-ledger cost](knowledge-ledger-cost-and-authentication.md)), and
the command-history index
([command admission history index](command-admission-history-index.md#follow-up-the-per-command-transaction-checkpoint)).

## Problem

The runtime copies its state for rollback and for forks. Up to 0.17.0 those
copies grew with the world and with its history:

- **Every command** captured a `CommandTransactionCheckpoint` that cloned the
  people, letter, army, plugin-component, and scheduled-action maps
  (`BTreeMap`s of owned values). In 0.16.0 an instrumented build put it at
  about 0.15 ms of 0.3 ms per command on the world preset; on the 0.17.0
  world (courts at institution granularity, interval cadence) the whole
  admission boundary of 4,200 commands took 1.6 to 2.0 s.
- **Every boundary** took a stable copy of current state for phases 1 to 9
  (`self.state.current.clone()`) and a `BoundaryTransactionCheckpoint` that
  cloned the entity set, persons, person availability, created persons,
  letters, armies, plugin components, random streams, and the whole
  scheduler (actions, pending and cancelled ingress, transition manifests).
  A scheduled batch and a plugin registration copied the same maps; plugin
  registration cloned the whole runtime state, journals included.
- **`fork()`** cloned all of that plus every retained journal (events,
  commands, attempts, ingress, boundaries, draws), the command-history
  index, and the bound initial scenario: 54 to 60 ms for a run that created
  65,536 records (140 ms when 0.16.0 recorded it), 0.65 to 0.93 s for the
  unsealed world after a burst of every scope. A compact run's seal forks itself for
  rollback, so a seal paid the same copy.
- **Restoring a large unsealed snapshot** was slow: 1.09 GB in 231 s after a
  world-scale burst in 0.16.0; 662 MB in 48 to 51 s for the world below
  (49.8 s in the 0.17.0 baseline).

### Where the restore spent its time

Measured on the 0.18.0 integration branch before this change, on the 662 MB
snapshot of the world-scale fiscal run after a burst of every scope (release
build with temporary timers, one run; restore 49.9 s with the timers, 47.7 s
without):

| Phase | ms |
| --- | ---: |
| Parse the JSON text into a `serde_json::Value` tree | 3,177 |
| Clone that tree | 3,079 |
| Decode the clone into `SimulationSnapshot` | 2,124 |
| Re-encode the snapshot into a second tree (`to_value`) | 3,436 |
| Compare the trees for unknown fields (a path string built per node) | 3,697 |
| Drop the re-encoded tree | 1,939 |
| Snapshot contract | 659 |
| Scenario-state validation (on a copy of entities, world, knowledge, and records) | 2,293 |
| Full snapshot validation | 17,618 |
| &nbsp;&nbsp;of which boundary records (domain-record replay) | 9,522 |
| &nbsp;&nbsp;of which header, records, knowledge, commitment roots | 7,085 |
| Indexes, runtime assembly, settled boundaries, checkpoint hash | 1,968 |
| Plugin registration (2 plugins; each cloned the whole state, 1,349 ms) | 7,103 |

The JSON decode, about 19 s with the drops of both trees, did the document
four times over; inside validation, about 3.1 s of the boundary-record replay
rebuilt a persistent record store of all 68,000 records once per boundary to
check owner-authorized maintenance commits, which none of the three
boundaries had.

## Decision

**Persistent current state and scheduler.** Every collection of
`RuntimeCurrentState` and `RuntimeScheduler` is an `im` structure, as the
record store, the version index, the decision state, and the archived tables
already were. Values that own heap data and are rewritten during a run are
held in an `Arc`, so a tree node copied by a write copies pointers:

| Field | 0.17.0 | 0.18.0 |
| --- | --- | --- |
| `entities` | `BTreeSet<EntityRef>` | `im::OrdSet<EntityRef>` |
| `people` | `BTreeMap<PersonId, Person>` | `im::OrdMap<PersonId, Arc<Person>>` |
| `person_availability` | `BTreeMap<PersonId, PersonAvailability>` | `im::OrdMap<PersonId, PersonAvailability>` |
| `created_persons` | `Vec<CreatedPerson>` | `im::Vector<CreatedPerson>` |
| `letters`, `armies` | `BTreeMap<_, LetterCargo / Army>` | `im::OrdMap<_, Arc<LetterCargo / Army>>` |
| `governments`, `territories`, `routes` | `BTreeMap` | `im::OrdMap` (written only at start) |
| `plugin_components` | `BTreeMap<PluginComponentKey, PluginComponentRecord>` | `im::OrdMap<Arc<PluginComponentKey>, Arc<PluginComponentRecord>>` |
| `random_streams` | `BTreeMap<RandomStreamKey, RandomStreamState>` | `im::OrdMap<RandomStreamKey, RandomStreamState>` |
| scheduler `actions` | `BTreeMap<ScheduleKey, ScheduledAction>` | `im::OrdMap<ScheduleKey, Arc<ScheduledAction>>` |
| scheduler `pending_ingress`, `cancelled_ingress` | `BTreeSet` | `im::OrdSet` |
| scheduler `transition_manifests` | `BTreeMap` | `im::OrdMap` (and the boundary's transition ledger) |

Every transaction checkpoint, the boundary's stable view, and a fork now
share these in O(1); a change copies only the tree paths it changes, and a
change of an `Arc` value goes through `Arc::make_mut`. The command
checkpoint still captures every collection a command may change; the capture
is now constant-size, so no per-command account of written state is needed.

**Shared journals.** The retained journals (`events`, `boundaries`,
`random_draws`, and the storage of the three `RetainedJournal`s) are a
`SharedJournal<T>`: an `Arc<Vec<T>>` that a clone shares and that the first
change of a shared journal copies. An append copies the journal, a
truncation copies only the kept prefix, and a drain from the front (a seal)
copies only the kept suffix; a journal no copy shares changes in place.
Reads still see one contiguous slice, so `events()`, `boundaries()`, and the
other journal accessors keep returning `&[T]`. The command-history index's
two maps are `im::OrdMap`s, and the bound initial scenario is held in an
`Arc`. A fork, a seal's rollback copy, and a plugin registration's rollback
copy are O(1) in the journals.

**Restore.** Four cuts, each of which keeps every answer and refusal:

- The decoder deserializes the snapshot from the parsed tree by reference
  (`T::deserialize(&Value)`) instead of from a clone of it.
- The unknown-field check walks the decoded snapshot's encoding against the
  parsed tree with a serializer that allocates nothing per node
  (`known_fields.rs`), instead of building the re-encoded tree and building a
  path string at every node. Only when the walk cannot pass an input (an
  unknown field, an array of another length, or a case it cannot decide, such
  as a float map key) does the decoder build the re-encoded tree and run the
  old comparison, so a refusal and its message are exactly the old ones.
  This applies to every format 9 JSON loader that decodes through
  `deserialize_current_value`: snapshots, checkpoint journals, continuation
  checkpoints, and replay journals.
- Scenario-state validation reads the snapshot's entities, world, knowledge,
  and records by reference (`ScenarioStateParts`), and the restored runtime
  takes the snapshot's initial scenario instead of cloning it.
- Boundary-record replay builds its persistent record store only when a
  boundary admits an owner-authorized maintenance commit.

With persistent state and shared journals, plugin registration's rollback
copy of the runtime state costs nothing.

## Contract

- **No public change.** No public type, signature, or accessor changes;
  `SimulationView::person` and `army` still return `Option<&Person>` and
  `Option<&Army>`, `person_availabilities` still yields
  `(&PersonId, &PersonAvailability)`, and the journal accessors return
  slices.
- **Same bytes.** Every snapshot, checkpoint, journal, segment, hash,
  commitment root, and replay result is unchanged: the collections iterate
  in the same key order, and the hash inputs serialize the persistent maps
  and vectors through adapters (`AvailabilityTable`, `CreatedPersonList`)
  that emit exactly what the ordered maps and slices emitted. The world-scale
  snapshot after the burst was compared byte for byte.
- **Same refusals.** The decoder refuses exactly the documents it refused
  before, with the same messages; the walk only decides when the full
  comparison is needed. Validation runs the same checks in the same order.
- **Forward iteration only.** No converted collection is iterated in
  reverse, and none uses `get_prev`, `lookup_prev`, or a reverse range. Besides
  full forward iteration, the only reads are keyed lookups, `get_min` (the
  scheduler's next action and the earliest pending ingress, removed by key),
  and forward ranges from a lower bound (`pending_ingress.range(first_ingress..)`
  and `actions.range(first_action..)` in `boundary_window.rs`, read with
  `take_while`).
- **Thread transfer.** Every new structure is `Arc`-based, so `Simulation`,
  `CompactedSimulation`, `Canwu`, and `CompactedCanwu` stay `Send`, and a
  fork moved to another thread shares nothing mutable with the original:
  the first change on either side copies what it changes.

## What can still fail a boundary

Nothing new. The change adds no operation, refusal, or error: it changes how
state is held and copied, not what any operation accepts.

## Costs, before and after

Release builds of the 0.18.0 integration branch (`2b42f05`) before and after
this change, on one Windows x86-64 machine (16 logical CPUs, 32 GB) shared
with other workers' builds and profiles, which loaded it fully at times, so
elapsed figures vary by a quarter or more between runs; pairs were run
alternately and ranges are shown. Allocation counts are exact.

**`fork` and the first boundary after it** (`paged-record-profile`, a run that
created 65,536 records; medians of 31 samples, allocations of 3; reports in
`benchmarks/baselines/2026-10-16-paged-record-0.18.0-*`):

| Case | Before | After | Allocations, before → after |
| --- | ---: | ---: | ---: |
| `fork` | 53.3 ms | 0.025 ms | 786,706 → 156 |
| `fork_first_boundary` (writes one record) | 1.55 ms | 53.5 ms | 331 → 786,864 |
| Idle boundary on a fork | 0.041 ms | 0.053 to 0.065 ms | 176 → 163 |
| Write a 1 KiB record on a fork | 0.11 ms | 0.12 to 0.16 ms | 327 → 314 |

The fork no longer copies the journals. The first boundary on the fork that
appends while the original still holds them copies them instead, so the
pair costs what the fork alone cost before (787,037 allocations before,
787,020 after). The idle and write rows are measured on the second boundary
of a fresh fork, just after that copy, and read 10 to 50 µs higher with fewer
allocations; a probe of 20 consecutive idle boundaries on such a fork
measured 21.0 µs a boundary before and after (20.6 and 21.5 µs on a run of 16
records), so the steady per-boundary cost is unchanged.

**The world-scale fiscal run** (`fiscal-world-profile` world preset: 200
partitions, 50,000 scopes, 4,200 institutions, 4,000 observers, courts at
institution granularity, monthly interval cadence; a probe built from the
profile's own code, three samples of each burst on forks of the run after the
initial publication):

| Measure | Before | After |
| --- | ---: | ---: |
| Admission boundary, 64 scopes | 19.9 to 27.2 ms | 3.6 to 5.2 ms |
| Admission boundary, 1,024 scopes | 229 to 286 ms | 45.5 to 45.8 ms |
| Admission boundary, every scope (4,200 commands) | 1,687 to 2,374 ms | 897 to 1,021 ms |
| `fork` of the unsealed run after the burst | 768 to 926 ms | 0.1 to 0.2 ms |
| Its snapshot | 661,800,813 bytes | identical, byte for byte |
| Restore of that snapshot | 47.7 to 71.4 s (three runs) | 28.7 to 44.8 s (three runs) |
| `fork` of the restored run | 649 to 947 ms | 0.1 to 0.4 ms |

The admission boundary's engine share fell by about 0.2 ms per command, the
cost of the command checkpoint on this world; the settling boundaries were
unchanged within the noise. On the `smoke` preset (2,000 scopes) the
profile's own rows read `fork_ms` 49.1 → 0.16 and the admission boundary of
every scope 120 → 66 ms, and its 31 MB snapshot restores in 2.0 to 5.9 s
before and 1.3 to 1.4 s after.

**Where the restore spends its time now** (the same timers, one run; restore
30.7 s with the timers):

| Phase | Before, ms | After, ms |
| --- | ---: | ---: |
| Parse into a tree | 3,177 | 3,138 |
| Decode (clone and decode before; decode by reference after) | 5,203 | 3,083 |
| Unknown-field check (re-encode, compare, drop before; walk after) | 9,072 | 1,300 |
| Drop of the parsed tree | about 1,900 | 1,734 |
| Snapshot contract | 659 | 596 |
| Scenario-state validation | 2,293 | 1,039 |
| Full snapshot validation | 17,618 | 14,058 |
| &nbsp;&nbsp;of which boundary records | 9,522 | 6,565 |
| Indexes, assembly, settled boundaries, checkpoint hash | 1,968 | 1,563 |
| Plugin registration (2 plugins) | 7,103 | 4,176 |

**Saves of the continuation profile** (`continuation-profile --months 1,4,14
--keyed 0 --samples 3`, two runs each; reports in
`benchmarks/baselines/2026-10-16-continuation-0.18.0-*`): restoring the full
portable save of month 14 (148 MB) took 6.2 to 8.3 s before and 2.9 to 3.6 s
after, and restoring the continuation checkpoint 31 to 48 ms before and 19 to
23 ms after; the bytes of both are unchanged. The gain is the JSON decoder,
which both restores use.

## What remains

- **The deferred copy.** A fork, or a rollback copy, still costs one copy of
  the retained journals when both sides keep them and one appends: the
  appending side pays it at its first append (the `fork_first_boundary` row).
  A host that saves from a fork on a worker thread while play continues pays
  it at the next boundary if the save is still running, as it paid it at the
  fork before. Making the journals persistent trees would remove it, but the
  public accessors return slices; a sealed run (`CompactedCanwu`) keeps only
  the tail since its last seal, so its copy stays small.
- **Restore.** What is left of the 662 MB restore, about 29 s, is mostly
  validation of the evidence itself (14 s): the boundary-record replay
  clones and validates the whole record set at each writing stage (about
  4 s), the header copies and checks the initial scenario (2.3 s), and the
  commitment roots and the knowledge ledger are rebuilt (about 4 s); then the
  JSON parse and the drop of its tree (about 5 s), the decode (3 s), and the
  plugins' own activation checks (`validate_activation`; the fiscal plugin's
  took about 3.5 s here). A sealed run's continuation checkpoint remains the
  save for long campaigns.
- **Hash inputs.** A commitment root of the world, plugin-component, or
  scheduler domain still collects the domain's values into vectors when the
  domain is dirty, as before.

## Tests

- `crates/runtime/canwu-sim/src/runtime/transactions.rs`:
  `rollback_copies_and_forks_share_state_and_journals` checks that the
  command, boundary, and scheduled-batch checkpoints and a fork share the
  current-state collections, the scheduler, the journals, and the initial
  scenario with the live run; that a fork that moves an army and appends
  evidence leaves the original's snapshot as it was and reaches the same
  state as the original advanced alike; and that truncating or draining a
  shared journal leaves the other copy whole.
- `crates/runtime/canwu-sim/src/runtime/known_fields.rs`:
  `walk_agrees_with_the_full_comparison` checks that the field walk agrees
  with the full comparison on a value with structs, internally and externally
  tagged enums, maps with integer keys, arrays, options, and JSON payloads,
  and on every input made from its encoding by adding an unknown field,
  removing a field, lengthening an array, or replacing a node with one of
  another kind; `walk_agrees_with_the_full_comparison_on_a_snapshot` runs the
  same sweep over the snapshot of a run that ordered a movement, advanced
  past the arrival, and settled a boundary (more than 1,000 inputs, under a
  second in a debug build).
- The walk is exact under three conditions, stated in the module
  documentation: the wire types' `Serialize` implementations propagate
  errors, its map-key handling matches `serde_json`'s map-key serializer
  (checked against the locked 1.0.151), and the `raw_value` feature is off. A
  `serde_json` upgrade rechecks the second.

Every existing persistence, tamper, replay, and restore test runs through
the changed paths. Each new test fails with its change reverted.

## Deviations

- E-19 offered persistent maps or a capture of only what a command touches;
  the change uses persistent maps, which make every capture constant-size
  without a per-command account of written state.
- E-20 asked for `fork` to be cheap. `fork` is O(1), but the journal copy is
  deferred, not removed (see "What remains"): the journals stay contiguous
  because the public API returns slices of them.
- The restore cuts are the redundant work (the JSON decode's extra trees and
  copies, two clones, and an unconditional record-store rebuild); the
  validation of the evidence itself is unchanged.
