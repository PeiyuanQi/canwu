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
- [Uncertainty, random decisions, and external selectors](uncertainty-resolution.md)
- [Culture and legal institutional systems](architecture-culture-law.md)
- [World and event ownership audit](proposals/world-event-ownership-audit.md)
- [Boundary window](proposals/boundary-window.md)
- [Same-instant supersession and unplaced persons](proposals/same-instant-supersession-and-unplaced-persons.md)
- [Keyed-draw index, draw ID, and boundary profile](proposals/keyed-draw-index-and-settlement-profile.md)
- [Knowledge ledger cost and authentication](proposals/knowledge-ledger-cost-and-authentication.md)
- [Record store ceiling and cost](proposals/record-store-ceiling-and-cost.md)
- [Continuation checkpoint](proposals/continuation-checkpoint.md)
- [Command admission history index](proposals/command-admission-history-index.md)
- [Reference archive and replay profile](proposals/reference-archive-and-replay-profile.md)
- [Request outcome retention](proposals/request-outcome-retention.md)
- [Engine load and budget guards](proposals/engine-load-and-budget-guards.md)
- [Runtime state copies](proposals/runtime-state-copies.md)
- [Paged restore budget and seal cost](proposals/paged-restore-budget-and-seal-cost.md)
- [Pending ingress read](proposals/pending-ingress-read.md)
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
- [Society settlement cadence and runtime revision](proposals/society-cadence-and-runtime-revision.md)
- [Culture authoring SDK and lifecycle design](proposals/culture-authoring-sdk-and-lifecycle.md)
- [Legal institutionalization framework](proposals/legal-institutionalization-framework.md)
- [Legal storage sharding, COW, delta persistence, and cold archive](proposals/legal-storage-sharding-compaction.md)
- [Legal institutionalization consensus review](proposals/legal-institutionalization-review.md)
- [Military domain extension design](proposals/military-domain-design.md)
- [Resource archive lookup and volume](proposals/resource-archive-lookup-and-volume.md)
- [Single-step completion lease and runtime protected-floor install](proposals/resource-single-step-lease-and-floor-install.md)
- [Fiscal throughput and authority](proposals/fiscal-throughput-and-authority.md)
- [Fiscal and society limits](proposals/fiscal-and-society-limits.md)
- [World-scale fiscal state](proposals/world-scale-fiscal.md)
- [Fiscal observers at scale](proposals/fiscal-observers-at-scale.md)
- [Resource refusals without stalls](proposals/resource-refusals-without-stalls.md)
- [Production archive volume](proposals/production-archive-volume.md)
- [Law, information, and military correctness](proposals/law-information-military-correctness.md)
- [Recurring traversal, edge conditions, and the compiled routing index](proposals/recurring-traversal.md)
- [Society intergroup relations and settlement cost](proposals/society-intergroup-relations.md)
- [Law: aggregate caseload, plan records at scale, and bounded bookkeeping](proposals/law-aggregate-caseload.md)
- [Fiscal archive and due-instant cost](proposals/fiscal-archive-and-due-instant-cost.md)
- [Resource series scale](proposals/resource-series-scale.md)
- [Reference archive receipts and operation room](proposals/reference-archive-receipts.md)
- [Production at series scale](proposals/production-series-scale.md)
- [Movement leg delay and current incident evidence](proposals/movement-delay-leg.md)
- [Resource transit, account place, and external inflow](proposals/resource-transit-place-and-inflow.md)
- [Downstream grand-strategy gap set](proposals/downstream-grand-strategy-gap-set.md)

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
