# World and Event Ownership Audit

Status: implemented. Concrete public event variants have moved out of
`canwu-event` without changing their serialized shape. The world model,
movement behavior, detached projection, and routing adapter now live in
`canwu-reference-world`; the `canwu-world` package is retired.

## Decision

Canwu should not broadly consolidate its workspace crates. It should first
remove period- and application-specific model ownership from generic engine
crates.

The governing invariant is:

> Generic engine crates own deterministic simulation contracts. Reference
> integrations own concrete world entities and event payloads such as armies,
> governments, territories, people, letters, and their presentation fields.

Applying that invariant gives two different outcomes:

- `canwu-world` had no irreducible generic engine contract and is retired.
  `canwu-reference-world` owns the extracted model. Deprecated format-5
  projections remain in the facade/runtime only to load supported saves and
  give existing callers an explicit migration interval.
- `canwu-event` now contains the generic event envelope, provenance, visibility,
  and structured event-record contracts. Its concrete payload variants have
  moved out. Only after independent consumption is known should the residual
  types move into `canwu-sim` or remain in this focused crate.

The physical placement of the residual event contracts is therefore a later
decision gate, not a pre-approved merge.

## Evidence from the current dependency graph

`canwu-world` now has no source package or dependency edges. `canwu-routing`
accepts only its domain-neutral `PlanningSnapshot`; the former
`planning_snapshot_from_world` adapter moved to `canwu-reference-world`.
`canwu-sim` and `canwu-api` retain deprecated format-5 compatibility
projections but do not depend on the extracted integration.

`canwu-event` has two normal first-party consumers: `canwu-sim` and
`canwu-api`. Its values participate in persisted snapshots, replay,
validation, event projection, plugin descriptors, and causal evidence. The
payload extraction therefore preserves the exact flattened event JSON shape;
it is a Rust source-API break, but not a snapshot-format migration.

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
| `SimEvent` | Persisted event envelope with identity, time, affected entities, summary, cause, and correlation | Generic engine contract | Keep the envelope and its serialized compatibility. |
| `EventKind` | Stable `type` label plus flattened structured fields | Generic engine contract | Keep the wire-neutral record. Domain owners define typed payloads outside this crate and use generic encode/decode APIs. |

The former typed variants for movement, arrival, letter, report, knowledge, and
debug behavior are no longer exported by `canwu-event`. Private compatibility
payloads in `canwu-sim` strictly validate the existing tags and fields until
their movement, information, knowledge, or debug owner is extracted. This
intermediate placement removes domain vocabulary from the generic crate without
pretending that the reference integration already exists.

Built-in compatibility tags remain unqualified. Before independently authored
integrations share event types, the engine still needs a canonical cross-
integration namespace and any versioned metadata required for schema evolution.
Plugin events retain their existing `plugin` plus nested `event_type` identity.
Legacy event records remain loadable and keep the same causal and commitment
meaning.

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

### 1. Characterize compatibility - complete for event extraction

Freeze representative JSON snapshots, journals, public facade calls, routing
adapter behavior, and exact replay outcomes. Record which names are guaranteed
by the current compatibility policy.

### 2. Introduce the generic event seam - partially complete

`EventKind` is now a generic record and concrete public variants are removed.
Runtime validation still decodes every compatibility payload into a private
typed structure and fails closed. Existing JSON needs no conversion because its
`type` tag and flattened fields are unchanged. A canonical cross-integration
namespace and payload-version contract remain open work.

### 3. Extract the reference world integration - complete

Move the current people, government, territory, route, army, and letter model
into a first-party reference integration built only on `canwu-api`. Move
`planning_snapshot_from_world` with that integration so `canwu-routing` accepts
only its own generic planning input.

### 4. Migrate the facade and saved state - complete

Offer integration-owned projections and events alongside the compatibility
surface. Deprecate or remove old re-exports only under the repository's
versioning policy, with snapshot and journal migrations and updated starter
kits.

### 5. Remove empty package boundaries - complete for `canwu-world`

`canwu-world` is retired after reaching zero normal consumers. Re-evaluate
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
- **Merge `canwu-event` into `canwu-sim` immediately:** the payload ownership is
  now separated, but a merge would still expand the persistence blast radius
  before independent consumption of the generic contracts is understood.
- **Create `canwu-model`:** this would consolidate names, not responsibilities.
- **Merge `canwu-api` and `canwu-sim`:** this would erase the supported facade
  and private runtime boundary.
- **Delete compatibility types immediately:** these types are re-exported and
  serialized; Cargo success would not prove source, save, or replay
  compatibility.
