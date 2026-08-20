# Canwu documentation map

Use this file only to choose sources. Read the selected source before answering.
Resolve local paths from the Canwu repository root, not from this skill folder.

## Entry points

| Need | Local source | Official fallback |
| --- | --- | --- |
| English project overview and quick start | `README.md` | `https://github.com/PeiyuanQi/canwu/blob/main/README.md` |
| Chinese project overview and quick start | `README.zh-CN.md` | `https://github.com/PeiyuanQi/canwu/blob/main/README.zh-CN.md` |
| Repository documentation index | `docs/README.md` | `https://github.com/PeiyuanQi/canwu/blob/main/docs/README.md` |
| Chinese tutorial index | `website/src/content/docs/tutorials/index.mdx` | `https://canwu.org/tutorials/` |
| English tutorial index | `website/src/content/docs/en/tutorials/index.mdx` | `https://canwu.org/en/tutorials/` |

## Tutorials and examples

| Question | Tutorial source | Detailed or executable source |
| --- | --- | --- |
| First successful action, explicit time advancement, or actor-relative knowledge | Chinese: `website/src/content/docs/tutorials/move-army.mdx`; English: `website/src/content/docs/en/tutorials/move-army.mdx` | `crates/canwu-api/examples/move_army.rs` |
| Pause, speed controls, FPS independence, wall/simulation/presentation clocks, or interpolation | Chinese: `website/src/content/docs/tutorials/continuous-game-loop.mdx`; English: `website/src/content/docs/en/tutorials/continuous-game-loop.mdx` | `docs/continuous-game-loop.md` and `crates/canwu-api/examples/continuous_game_loop.rs` |
| Schema-validated plugin commands, issuer checks, or declared state access | Chinese: `website/src/content/docs/tutorials/command-plugin.mdx`; English: `website/src/content/docs/en/tutorials/command-plugin.mdx` | `crates/canwu-api/examples/plugin.rs` |
| Daily or turn boundaries, supply and demand, reservation, allocation, or multi-system settlement | Chinese: `website/src/content/docs/tutorials/phased-boundary.mdx`; English: `website/src/content/docs/en/tutorials/phased-boundary.mdx` | `crates/canwu-api/examples/phased_boundary.rs` |
| DecisionTicket, controller binding, dynamic options, utility evaluation, Human/External/LLM policy boundaries, decision traces, or a neighboring warlord's military-aid request | Chinese: `website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx`; English: `website/src/content/docs/en/tutorials/cases/warlord-aid-decision.mdx` | `crates/canwu-api/examples/decision_ticket.rs` and `crates/canwu-api/tests/decision_framework.rs` |

When no checkout is available, follow links from the corresponding official
tutorial index or open the example under
`https://github.com/PeiyuanQi/canwu/tree/main/crates/canwu-api/examples`.

## Design contracts

| Topic | Primary source | Useful sections or companion source |
| --- | --- | --- |
| Headless boundary, dependency direction, world/time/events, public interfaces, knowledge, plugins, settlement, renderer integration | `docs/architecture.md` | Match the section heading to the user's term; inspect `crates/canwu-api` for exact APIs |
| Intended product surfaces, historical state, causality, decisions/controllers, persistence, counterfactuals, geography, plugins, debug client | `docs/end-state.md` | Use for design intent and destination, then confirm implemented behavior in current sources |
| Named engine requirements and acceptable evidence | `docs/engine-conformance.md` | Requirements E01-E16 and the conformance evidence section |
| SemVer, snapshot formats, migrations, hashing, replay, and supported platforms | `docs/versioning.md` | Use for compatibility claims; do not infer compatibility from examples |

Official design fallbacks:

- `https://github.com/PeiyuanQi/canwu/blob/main/docs/architecture.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/end-state.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/engine-conformance.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/versioning.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/continuous-game-loop.md`

## Exact API questions

Start at `crates/canwu-api/src/lib.rs` and search for the named public type or
method. Use the matching example to show composition. Do not use
`crates/canwu-sim` as client documentation, and do not infer a public guarantee
from a private implementation detail.
