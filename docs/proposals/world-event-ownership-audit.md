# World and Event Ownership Audit

Status: accepted architectural direction. This audit does not change the
public API, serialized formats, or current crate dependency graph.

## Decision

Canwu should not broadly consolidate its workspace crates. It should first
remove period- and application-specific model ownership from generic engine
crates.

The governing invariant is:

> Generic engine crates own deterministic simulation contracts. Reference
> integrations own concrete world entities and event payloads such as armies,
> governments, territories, people, letters, and their presentation fields.

Applying that invariant gives two different outcomes:

- `canwu-world` is a compatibility model for the first movement vertical slice.
  After a supported migration moves that model into a reference integration,
  the crate has no irreducible generic engine contract and should be retired.
- `canwu-event` mixes generic engine contracts with legacy payload variants.
  Keep the generic event envelope, provenance, and visibility contracts. Move
  concrete payloads out first; only then decide whether the residual types are
  clearer inside `canwu-sim` or still justify a focused `canwu-event` crate.

The physical placement of the residual event contracts is therefore a later
decision gate, not a pre-approved merge.

## Evidence from the current dependency graph

`canwu-world` has three normal first-party consumers: `canwu-sim`,
`canwu-api`, and `canwu-routing`. The facade re-exports every public world type,
and the runtime persists or projects the same concrete entities. Routing uses
`WorldSnapshot` only through `planning_snapshot_from_world`; its actual solver
already owns a domain-neutral `PlanningSnapshot`. That adapter can move with
the reference integration without merging routing and world ownership.

`canwu-event` has two normal first-party consumers: `canwu-sim` and
`canwu-api`. Its values participate in persisted snapshots, replay,
validation, event projection, plugin descriptors, and causal evidence. This is
why moving it requires a format migration and compatibility plan rather than a
Cargo-only refactor.

The public facade and private runtime remain separate regardless of this audit.
Their high rate of co-change reflects the facade wrapping the runtime; it does
not erase the public/private boundary.

## `canwu-world` public-type classification

| Public type | Current role | Classification | Target action |
| --- | --- | --- | --- |
| `MapPoint` | Two-dimensional display and distance coordinate | Reference-integration content | Move with the example map model; routing keeps its own endpoint abstractions. |
| `Person` | Named person with government, location, roles, and transit | Reference-integration content | Move to the first-party movement/world integration. |
| `PersonTransitState` | Territory-to-territory person journey | Reference-integration content | Move with `Person` and the movement integration. |
| `LetterStatus` | Custody and delivery lifecycle for letters | Reference-integration content | Move to the reference information/logistics integration. |
| `LetterCargo` | Concrete letter body, sender, recipient, custody, and location | Reference-integration content | Move with the letter lifecycle integration. |
| `Government` | Named government and capital territory | Reference-integration content | Move to the example political/world model. |
| `Territory` | Named, controlled map location | Reference-integration content | Move to the example topology/world model. |
| `Route` | Named bidirectional territory edge with terrain and travel minutes | Reference-integration content | Move to the example topology integration; adapt it to `canwu-routing::PlanningSnapshot`. |
| `TransitState` | Territory-to-territory army journey | Reference-integration content | Move with the example army movement model. |
| `Army` | Named military unit with commander, strength, morale, and transit | Reference-integration content | Move to the first-party movement/world integration. |
| `WorldSnapshot` | Detached projection containing exactly the concrete types above | Legacy compatibility projection | Replace with integration-owned projections and typed domain records; retain an adapter until the public migration completes. |
| `WorldDiff` | Changed-ID projection for armies, people, territories, and letters | Legacy compatibility projection | Replace with integration-owned diffs or generic change evidence; retain compatibility until callers migrate. |

No type in this crate is a domain-neutral mutable world store or kernel state
contract. Retiring the crate does not mean removing detached reads; it means
the owning integration defines its own detached read model through supported
public APIs.

## `canwu-event` public-type classification

| Public type | Current role | Classification | Target action |
| --- | --- | --- | --- |
| `CauseRef` | Boundary, command, parent-event, or system provenance | Generic engine contract | Keep semantics and serialized compatibility. Physical crate placement is decided only after decoupling. |
| `EventAudience` | Persisted player-facing visibility policy | Generic engine contract | Keep fail-closed visibility semantics and replay-stable serialization. |
| `SimEvent` | Persisted event envelope with identity, time, affected entities, summary, cause, and correlation | Generic engine contract with legacy coupling | Keep the envelope; replace its mandatory concrete `EventKind` dependency through a versioned migration. |
| `EventKind` | Enum containing movement, arrival, letter, report, knowledge, debug, and plugin variants | Legacy compatibility payload union with one generic extension seam | Move concrete variants to their owning integration or subsystem; replace `Plugin` with a generic namespaced event identity before removing the enum. |

The built-in `EventKind` variants `MoveOrdered`, `ArmyArrived`,
`PersonMoveOrdered`, `PersonArrived`, `LetterDelivered`, and
`ReportDispatched` belong to the reference movement/information integration.
`KnowledgeUpdated` and `KnowledgePublished` belong with the knowledge or
information contract that produces them. `DebugFieldChanged` belongs to the
debug/compatibility surface. `Plugin` demonstrates the required generic seam,
but its string fields alone are not a complete replacement plan.

Before `EventKind` can be retired, the engine needs a canonical namespaced
event identity and any versioned structured metadata required for validation,
projection, and replay. Legacy event records must remain loadable, and their
causal and commitment evidence must retain the same meaning.

## Target boundary

The target ownership is:

1. `canwu-core` keeps stable identity, schema, and deterministic foundation
   contracts. It does not absorb concrete world or event payloads.
2. `canwu-sim` owns the runtime event journal, validation, settlement,
   persistence, and replay behavior.
3. Generic domain extensions own reusable mechanics such as routing,
   transport, knowledge, information, and society contracts.
4. Reference integrations own the concrete first-party world model and the
   event payloads that explain its movement, letters, reports, and presentation.
5. `canwu-api` remains the supported facade and re-exports compatibility types
   only for as long as the migration policy requires.

This is not a proposal for a new `canwu-model` crate. A shared model bucket
would hide the same ownership problem behind a larger name.

## Migration sequence

### 1. Characterize compatibility

Freeze representative JSON snapshots, journals, public facade calls, routing
adapter behavior, and exact replay outcomes. Record which names are guaranteed
by the current compatibility policy.

### 2. Introduce the generic event seam

Add a canonical namespaced event identity and versioned structured event
metadata without removing `EventKind`. Teach validation, projection,
persistence, and replay to handle the new representation. Provide explicit
legacy loading and conversion.

### 3. Extract the reference world integration

Move the current people, government, territory, route, army, and letter model
into a first-party reference integration built only on `canwu-api`. Move
`planning_snapshot_from_world` with that integration so `canwu-routing` accepts
only its own generic planning input.

### 4. Migrate the facade and saved state

Offer integration-owned projections and events alongside the compatibility
surface. Deprecate or remove old re-exports only under the repository's
versioning policy, with snapshot and journal migrations and updated starter
kits.

### 5. Remove empty package boundaries

Retire `canwu-world` after it has no normal consumers. Re-evaluate
`canwu-event` after its legacy payload union is gone: fold the residual generic
contracts into `canwu-sim` only if they have no useful independent consumers;
otherwise keep the now-focused crate.

## Hard gates

Implementation is not complete until all of these are true:

- old supported snapshots and journals load through an explicit versioned
  migration and reproduce their documented replay outcomes;
- commitment, cause, correlation, audience, and affected-entity validation
  remain deterministic and fail closed;
- the public facade follows `docs/versioning.md`, with no silent source or wire
  break;
- `canwu-routing` no longer depends on the concrete example world;
- at least one runnable starter kit demonstrates the extracted integration,
  including save/load, fork, and exact replay;
- a second integration or fixture proves the generic engine contracts do not
  require the first historical world model;
- the dependency DAG, architecture documents, API-delta checks, and persistence
  fixtures all describe the same shipped state.

## Rejected shortcuts

- **Merge `canwu-world` into `canwu-core`:** this would make opinionated
  historical entities foundational.
- **Merge the current `canwu-event` into `canwu-sim`:** this would move legacy
  payload ownership without resolving it and expand the persistence blast
  radius.
- **Create `canwu-model`:** this would consolidate names, not responsibilities.
- **Merge `canwu-api` and `canwu-sim`:** this would erase the supported facade
  and private runtime boundary.
- **Delete compatibility types immediately:** these types are re-exported and
  serialized; Cargo success would not prove source, save, or replay
  compatibility.
