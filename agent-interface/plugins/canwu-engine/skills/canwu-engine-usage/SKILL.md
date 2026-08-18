---
name: canwu-engine-usage
description: Use Canwu through its public Rust and semantic agent APIs. Use when building a game client, renderer, research tool, Python or web binding, AI agent, debug consumer, or experiment that observes a Canwu world, queries actor-relative knowledge, discovers legal actions, submits validated commands, advances time, explains events, or compares snapshots and forks.
---

# Use Canwu Engine

Work through `canwu-api`. Read `README.md`, `docs/architecture.md`,
`docs/end-state.md`, and the public types in `crates/canwu-api` as needed.
Use the sibling
[`canwu-engine-docs`](../canwu-engine-docs/SKILL.md) skill when the primary task
is to find, compare, summarize, or explain Canwu documentation.

## Choose the interface

- Use typed commands, queries, events, snapshots, forks, and diffs for Rust
  programs and research tools.
- Use `observe`, `inspect`, `query_as`, `available_actions`, `act`, `explain`,
  `wait`, and `describe_capabilities` for actor-facing or AI-agent workflows.
- Use omniscient reads only for explicitly authorized research or debug tools.

## Follow the boundary

1. Never mutate live world state directly or reach into `canwu-sim` from a
   client.
2. Discover legal actions with `available_actions(actor)` before acting.
3. Turn an intended action into `SemanticAction` or a validated
   `CommandEnvelope`; handle validation errors instead of changing fields.
4. Advance simulation time explicitly. Do not couple time to rendering frames.
5. Consume emitted events and causal references to update external views.

Runtime plugins are also implemented through `canwu-api`; do not add a direct
client dependency on `canwu-sim`. Declare every core collection read with the
matching `StateKey::core_*` constructor, and declare every plugin-state read or
write with its owned `StateKey`. Handlers are stateless function pointers;
store deterministic plugin state in serialized components.

## Protect partial knowledge

- Use `observe(actor, ...)`, `inspect(actor, ...)`, or `query_as(actor, ...)`
  for an actor's view.
- Do not silently replace missing actor knowledge with `world()` ground truth.
- Show estimates, confidence, source, observation time, and information age when
  available.
- Keep agent context small. Start with a summary, then inspect details only when
  needed.

## Start with the vertical slice

Run:

```text
cargo run -p canwu-api --example move_army
```

The example demonstrates a validated army move, scheduled travel, attributable
arrival events, an immediate commander update, and a delayed report to another
person.

Run `cargo run -p canwu-api --example plugin` for an issuer-aware plugin command
with a machine-validated payload and declared core/component access.

## Integrate external clients

- Renderers consume snapshots, geometry, positions, relationships, and events;
  they decide whether to draw sprites, SVG, 3D scenes, tables, or text.
- Save with `snapshot` or `snapshot_json`; restore with
  `from_snapshot_json` for read-only inspection. If the snapshot contains
  plugins, use `from_snapshot_json_with_plugins` before advancing or submitting
  commands; continuation remains blocked until every exact stored plugin
  descriptor is rehydrated.
- Use `fork` and `diff` for counterfactual comparisons.
- Use `replay_with_plugins` for journals containing plugin commands. Executable
  handlers are not serialized, and plain replay intentionally fails rather than
  silently omitting plugin behavior.
- Treat `Issuer` as an assertion made by a trusted in-process host. Authenticate
  external callers in the adapter before constructing a command envelope.

Keep the external client replaceable. If the public API cannot express a
needed read or command, improve `canwu-api` instead of bypassing it.
