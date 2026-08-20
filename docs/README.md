# Canwu Documentation

> **Recommended route: send in an agent first.** Tell your coding agent,
> “I want to build `[your goal]` with Canwu. Find the right contract, runnable
> example, and public API, then teach me while implementing it.” If the
> `canwu-engine` plugin is installed, have it invoke
> [`$canwu-engine-docs`](../agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/SKILL.md).
> Reading every page by hand is still legal; we just assumed your scroll wheel
> had other plans.

This directory keeps project documentation grouped by purpose so the repository
root remains limited to files expected by Cargo, GitHub, licensing tools, and
coding agents.

## Engine contracts

- [Architecture](architecture.md)
- [End-state design](end-state.md)
- [Reusable-engine conformance](engine-conformance.md)
- [Versioning and compatibility](versioning.md)

## Integration guides

- [Continuous-time / proportional-time game loop](continuous-game-loop.md)

## Experimental extensions

- [Social belief and institutional diffusion framework](proposals/social-belief-framework.md)
- [Social framework implementation checklist](proposals/social-belief-framework-todo.md)

## Agent access

When the `canwu-engine` agent plugin is installed, invoke
[`$canwu-engine-docs`](../agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/SKILL.md)
to locate and explain the relevant tutorial, design contract, public API source,
or runnable example without loading the entire documentation set.

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
