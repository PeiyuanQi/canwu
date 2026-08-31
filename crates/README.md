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
    resource["canwu-resource"]
    production["canwu-production"]
    ming_fiscal["canwu-ming-fiscal"]
    economy_content["canwu-economy-reference-content"]
    ming_reference["canwu-ming-fiscal-reference"]
    force_supply["canwu-force-supply-reference"]
    economy_reference["canwu-economy-reference"]
    debug["canwu-debug"]

    subgraph tools["Tools"]
        debug
    end
    subgraph integrations["Reference integrations"]
        reference_world
        ming_reference
        force_supply
        economy_reference
    end
    subgraph reference_content["Reference content"]
        ming_fiscal
        economy_content
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
        resource
        production
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
    api --> resource
    api --> production
    resource --> production
    technology --> production
    fiscal --> ming_fiscal
    api --> economy_content
    resource --> economy_content
    production --> economy_content
    technology --> economy_content
    api --> ming_reference
    fiscal --> ming_reference
    ming_fiscal --> ming_reference
    reference_world --> ming_reference
    api --> force_supply
    resource --> force_supply
    economy_content --> force_supply
    api --> economy_reference
    resource --> economy_reference
    production --> economy_reference
    economy_content --> economy_reference
    force_supply --> economy_reference
    routing --> economy_reference
    transport --> economy_reference
    technology --> economy_reference
    reference_world --> economy_reference
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
| `extensions/` | `canwu-information`, `canwu-correspondence`, `canwu-society`, `canwu-culture`, `canwu-law`, `canwu-technology`, `canwu-history-research`, `canwu-fiscal`, `canwu-resource`, `canwu-production` | Domain implementations built on the public API; resource owns conserved quantities and fulfillment, production consumes resource and technology evidence, and fiscal procedure remains independent from resource balances and physical transfers | Published except for milestone-stage crates awaiting their release tag |
| `reference-content/` | `canwu-ming-fiscal`, `canwu-economy-reference-content` | Versioned, source-cited historical definitions and explicit synthetic fixtures compiled by generic extensions | Published |
| `integrations/` | `canwu-reference-world`, `canwu-ming-fiscal-reference`, `canwu-force-supply-reference`, `canwu-economy-reference` | Replaceable example worlds, adapters, scenario composition, force-supply consumers, and runnable starters | Not published |
| `tools/` | `canwu-debug` | Reference clients and maintainer tools | Not published |

## Registry order

Publish the lockstep version only from its tagged release commit, waiting for
each completed group to become resolvable before continuing:

1. `canwu-core`, `canwu-time`
2. `canwu-decision`, `canwu-event`, `canwu-knowledge`
3. `canwu-routing`, `canwu-sim`
4. `canwu-transport`
5. `canwu-api`
6. `canwu-information`, `canwu-society`, `canwu-technology`, `canwu-fiscal`, `canwu-resource`
7. `canwu-culture`, `canwu-law`, `canwu-correspondence`, `canwu-history-research`, `canwu-production`, `canwu-ming-fiscal`
8. `canwu-economy-reference-content`

See [the architecture](../docs/architecture.md), [versioning](../docs/versioning.md),
and [the release procedure](../docs/releasing.md) for the behavioral and
distribution contracts behind this graph.
