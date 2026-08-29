# Canwu crates

This directory is organized by dependency layer. The folder names help people
navigate the repository; published Cargo package names remain unchanged.
Applications should depend on `canwu-api`, not on the implementation crates
below it. Published domain extensions remain optional and are not part of the
supported public API surface.

## Dependency graph

Arrows point from a dependency to the crate that consumes it. The graph shows
direct first-party dependencies from the Cargo manifests.

```mermaid
flowchart BT
    core["canwu-core"]
    time["canwu-time"]

    decision["canwu-decision"]
    event["canwu-event"]
    knowledge["canwu-knowledge"]
    reference_world["canwu-reference-world"]

    routing["canwu-routing"]
    transport["canwu-transport"]
    sim["canwu-sim"]
    api["canwu-api"]

    information["canwu-information"]
    correspondence["canwu-correspondence"]
    society["canwu-society"]
    culture["canwu-culture"]
    law["canwu-law"]
    technology["canwu-technology"]
    history["canwu-history-research"]
    fiscal["canwu-fiscal"]
    ming_fiscal["canwu-ming-fiscal"]
    ming_reference["canwu-ming-fiscal-reference"]
    debug["canwu-debug"]

    subgraph tools["Tools"]
        debug
    end
    subgraph integrations["Reference integrations"]
        reference_world
        ming_reference
    end
    subgraph reference_content["Reference content"]
        ming_fiscal
    end
    subgraph extensions["Published extensions"]
        information
        correspondence
        society
        culture
        law
        technology
        history
        fiscal
    end
    subgraph PublicApi["Public API"]
        api
    end
    subgraph runtime["Runtime"]
        sim
    end
    subgraph mechanisms["Reusable mechanisms"]
        routing
        transport
    end
    subgraph model["Model contracts"]
        decision
        event
        knowledge
    end
    subgraph foundation["Foundation"]
        core
        time
    end

    core --> decision
    time --> decision
    core --> event
    time --> event
    core --> knowledge
    time --> knowledge

    core --> routing
    time --> routing
    core --> transport
    time --> transport
    routing --> transport

    core --> sim
    time --> sim
    decision --> sim
    event --> sim
    knowledge --> sim

    core --> api
    time --> api
    decision --> api
    event --> api
    knowledge --> api
    routing --> api
    transport --> api
    sim --> api

    api --> information
    api --> correspondence
    information --> correspondence
    api --> society
    society --> culture
    api --> culture
    api --> law
    api --> technology
    api --> history
    technology --> history
    api --> fiscal
    fiscal --> ming_fiscal
    api --> ming_reference
    fiscal --> ming_reference
    ming_fiscal --> ming_reference
    reference_world --> ming_reference
    api --> debug
    api --> reference_world
    reference_world --> debug
```

## Layers

| Layer | Crates | Responsibility | Registry policy |
| --- | --- | --- | --- |
| `foundation/` | `canwu-core`, `canwu-time` | Stable identifiers, schemas, deterministic random primitives, and simulation time | Published |
| `model/` | `canwu-decision`, `canwu-event`, `canwu-knowledge` | Serializable model and policy contracts | Published |
| `mechanisms/` | `canwu-routing`, `canwu-transport` | Reusable planning and transport execution | Published |
| `runtime/` | `canwu-sim` | Authoritative state, commands, settlement, persistence, replay, and plugins | Published as an implementation dependency |
| `api/` | `canwu-api` | Supported application-facing Rust API | Published and recommended for applications |
| `extensions/` | `canwu-information`, `canwu-correspondence`, `canwu-society`, `canwu-culture`, `canwu-law`, `canwu-technology`, `canwu-history-research`, `canwu-fiscal` | Domain implementations built on the public API; culture remains downstream from society, law consumes admitted social evidence through controller-mediated procedure, historical research remains downstream from technology, and fiscal procedure remains independent from resource balances and physical transfers | Published except for milestone-stage crates awaiting their release tag |
| `reference-content/` | `canwu-ming-fiscal` | Versioned, source-cited historical definitions compiled by generic extensions | Published |
| `integrations/` | `canwu-reference-world`, `canwu-ming-fiscal-reference` | Replaceable example worlds, adapters, scenario composition, and runnable starters | Not published |
| `tools/` | `canwu-debug` | Reference clients and maintainer tools | Not published |

## Registry order

Publish the lockstep version only from its tagged release commit, waiting for
each completed group to become resolvable before continuing:

1. `canwu-core`, `canwu-time`
2. `canwu-decision`, `canwu-event`, `canwu-knowledge`
3. `canwu-routing`, `canwu-sim`
4. `canwu-transport`
5. `canwu-api`
6. `canwu-information`, `canwu-society`, `canwu-technology`, `canwu-fiscal`
7. `canwu-culture`, `canwu-law`, `canwu-correspondence`, `canwu-history-research`, `canwu-ming-fiscal`

See [the architecture](../docs/architecture.md), [versioning](../docs/versioning.md),
and [the release procedure](../docs/releasing.md) for the behavioral and
distribution contracts behind this graph.
