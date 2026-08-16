# 参伍引擎 Canwu Engine

[English](README.md) | [简体中文](README.zh-CN.md)

Website: [canwu.org](https://canwu.org)

<img src="assets/branding/canwu-logo-en.png" alt="Canwu historical simulation engine logo" width="320">

Canwu is a headless historical simulation engine written in Rust. It simulates
a historical world, advances time in a repeatable way, accepts validated
commands, and records events and their causes. It can also represent what each
person knows instead of giving every actor access to the true world state.

Canwu does not render graphics, play audio, or provide a production user
interface. Games, research tools, Python programs, web clients, and AI agents
use Canwu through its public APIs.

Canwu is built for simulations that need more than a mutable game-state object.
It provides deterministic time, validated authority-aware commands, atomic
settlement, actor-relative knowledge, typed extension points, causal evidence,
save/load validation, and exact replay. The engine remains domain-neutral:
applications define their own rules and content through public contracts rather
than adding application-specific types to the kernel.

The project is under active development. The public examples are small on
purpose: they make the engine's guarantees easy to inspect, test, and reuse in
larger games, research environments, and agent-driven simulations.

## Quick start

Install a Rust toolchain compatible with the `rust-version` in `Cargo.toml`,
then run the headless movement example:

```text
cargo run -p canwu-api --example move_army
```

For a phased, API-only plugin example:

```text
cargo run -p canwu-api --example phased_boundary
```

## How the repository fits together

- `canwu-core`: stable IDs, repeatable random numbers, and schema metadata
- `canwu-time`: historical time that is independent of rendering speed
- `canwu-event`: stored events and links between causes and effects
- `canwu-world`: historical entities and read-only world snapshots
- `canwu-knowledge`: what each actor knows and when they learned it
- `canwu-sim`: private simulation state, commands, scheduling, and plugins
- `canwu-api`: public APIs for programs, agents, explanations, and debugging
- `canwu-debug`: a small reference client built only on the public API

The `docs` directory contains the architectural contracts. `agent-interface`
contains skills for engine users and repository maintainers; these are tooling,
not runtime simulation plugins. The `website` and `assets` directories contain
the community site and project media.

Read [the architecture](docs/architecture.md) and
[end-state design](docs/end-state.md) before changing boundaries. Release and
compatibility rules are defined in [versioning](docs/versioning.md).

## Version and platforms

Canwu uses Semantic Versioning, with all workspace crates currently at `0.4.0`
and released in lockstep. The canonical version is in the root `Cargo.toml`.

The supported operating systems are Windows, macOS, and Linux. The simulation
crates are headless and platform-neutral; the reference debug client uses
OpenGL through `eframe`, with Wayland and X11 enabled on Linux. The CI matrix
checks all three operating systems.

## Development

Contributions, bug reports, examples, documentation improvements, and careful
architecture discussions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md)
for local setup, development commands, project rules, and the Contributor
License Grant. External contributors must accept that grant when opening a pull
request. Coding agents must also follow [AGENTS.md](AGENTS.md) and any nearer
instructions.

<details>
<summary><strong>Development flow</strong></summary>

1. Read `AGENTS.md`, `docs/architecture.md`, `docs/end-state.md`, and any nearer
   repository instructions for the area being changed.
2. Inspect `git status`. Preserve existing work and use a worktree for unrelated
   parallel changes.
3. State the invariant, identify every affected surface, and make the smallest
   coherent implementation. Keep semantic changes separate from large file
   moves or generated-file refreshes.
4. Add durable tests and run:

   ```text
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo check -p canwu-debug
   ```

5. Run affected public examples and `cargo doc --workspace --no-deps` when APIs
   or documentation change.
6. Obtain independent review for architectural, persistence, replay, authority,
   determinism, or performance work. Commit only coherent, passing milestones.

The detailed project hierarchy and change-surface map live in
[AGENTS.md](AGENTS.md).

</details>

## Agent skills

Agent-facing integrations live under [`agent-interface`](agent-interface/).
External users can use the
[`canwu-engine-usage`](agent-interface/plugins/canwu-engine/skills/canwu-engine-usage/SKILL.md)
skill. Contributors and maintainers use skills under
[`canwu-developer`](agent-interface/plugins/canwu-developer/skills/); the release
workflow is
[`canwu-developer-release`](agent-interface/plugins/canwu-developer/skills/canwu-developer-release/SKILL.md).

## Minimal API example

```rust
use canwu_api::{Canwu, Command, CommandEnvelope, Issuer, SimDuration};

let mut canwu = Canwu::demo(35)?;
let ids = Canwu::demo_ids();

canwu.submit(CommandEnvelope::new(
    Issuer::Actor(ids.commander),
    Command::MoveArmy {
        army: ids.army,
        destination: ids.eastern_territory,
    },
))?;
let events = canwu.advance(SimDuration::days(1))?;
# Ok::<(), canwu_api::CanwuError>(())
```

See `crates/canwu-api/examples/phased_boundary.rs` for an API-only plugin that
offers and claims a conserved resource, consumes its declared allocation, and
commits attributable boundary evidence.

## License

This project is source-available under the Canwu License 1.0. Personal,
community, research, educational, nonprofit, and commercial use is
royalty-free while a Product Family remains at or below $10 million in revenue
during its applicable 12-month measurement period. Progressive marginal
royalties apply only to revenue above that threshold. Commercial products must
display an [official Canwu logo](BRANDING.md) and include a Canwu
acknowledgement in the product or its user-facing materials. Proprietary
products are permitted, and independent downstream code does not need to be
disclosed or open-sourced. See [LICENSE](LICENSE) for the authoritative terms.
Third-party dependencies remain under their own licenses; see
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
