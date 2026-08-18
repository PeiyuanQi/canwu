# Canwu Documentation

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

## Agent access

When the `canwu-engine` agent plugin is installed, invoke
[`$canwu-engine-docs`](../agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/SKILL.md)
to locate and explain the relevant tutorial, design contract, public API source,
or runnable example without loading the entire documentation set.

## Community

- [Branding](community/branding.md)
- [Sponsors](community/sponsors.md)

## Legal and release

- [Third-party license inventory](legal/third-party-licenses.md)
- [Generated third-party notices](legal/third-party-notices.html)
- [Additional third-party notices](legal/third-party-notices-extra.md)

The `cargo-about` configuration and template used to regenerate the HTML notice
live under [`tools/licenses`](../tools/licenses/). The exact command and release
checks are documented in the license inventory and the bundled maintainer
release skill.
