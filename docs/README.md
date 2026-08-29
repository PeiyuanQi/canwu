# Canwu Documentation

> **Start with a runnable example.** Pick the guide closest to your goal and
> look up the underlying contracts as you need them. If you use a coding agent,
> you can ask it to find the relevant example and public API while it implements
> the first working version. With the `canwu-engine` plugin installed, it can
> invoke
> [`$canwu-engine-docs`](../agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/SKILL.md).

This directory keeps project documentation grouped by purpose so the repository
root remains limited to files expected by Cargo, GitHub, licensing tools, and
coding agents.

## Engine contracts

- [Chinese-English terminology](terminology.md)
- [Architecture](architecture.md)
- [Culture and legal institutional systems](architecture-culture-law.md)
- [World and event ownership audit](proposals/world-event-ownership-audit.md)
- [End-state design](end-state.md)
- [Reusable-engine conformance](engine-conformance.md)
- [Versioning and compatibility](versioning.md)

## Integration guides

- [Continuous-time / proportional-time game loop](continuous-game-loop.md)

## Simulation domain extensions

Canwu calls an optional domain-specific module built on the public engine
contracts a **domain extension** (**模拟领域扩展**). `canwu-society` is the
current published **social diffusion simulation module**
(**社会传播模拟模块**) built at that layer.

- [Social diffusion simulation module design](proposals/social-belief-framework.md)
- [Social framework implementation checklist](proposals/social-belief-framework-todo.md)
- [Culture authoring SDK and lifecycle design](proposals/culture-authoring-sdk-and-lifecycle.md)
- [Legal institutionalization framework](proposals/legal-institutionalization-framework.md)
- [Legal institutionalization consensus review](proposals/legal-institutionalization-review.md)

## Runnable cases, reference content, and starter kits

The engine also maintains a growing first-party collection for developers
who need a complete starting point rather than isolated API examples. Reference
content packs provide reusable, versioned domain data; reference integrations
map that data to small public-API world and economy models; starter kits compose
both into runnable vertical slices. They remain downstream of the engine and
are designed to be replaced or extended by games such as Celestial Mandate.

The architecture document defines the ownership and persistence boundary for
these packages. Once reference content and integrations are composed into a
runnable vertical slice, the documentation presents that slice as a case.

- [Governance transition case](governance-transition-case.md)
- [Ming fiscal case](ming-fiscal-case.md)

## Agent access

When the `canwu-engine` agent plugin is installed, invoke
[`$canwu-engine-docs`](../agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/SKILL.md)
to locate and explain the relevant tutorial, design contract, public API source,
or runnable example without loading the entire documentation set. Agents should
consult the [terminology reference](terminology.md) before introducing or
translating a public Canwu term.

## Community

- [Branding](community/branding.md)
- [Sponsors](community/sponsors.md)

## Legal and release

- [Release procedure](releasing.md)
- [Third-party license inventory](legal/third-party-licenses.md)
- [Generated third-party notices](legal/third-party-notices.html)
- [Additional third-party notices](legal/third-party-notices-extra.md)

The `cargo-about` configuration and template used to regenerate the HTML notice
live under [`tools/licenses`](../tools/licenses/). The exact command and release
checks are documented in the license inventory and the bundled maintainer
release skill.
