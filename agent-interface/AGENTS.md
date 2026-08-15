# Canwu Agent Interface Instructions

This folder contains Codex skill plugins for working with Canwu. These are agent
tools, not runtime `SimulationPlugin` implementations.

- Put contributor and maintainer skills in
  `plugins/canwu-developer/skills/`.
- Name every contributor or maintainer skill `canwu-developer-*`.
- Put external engine-usage skills in `plugins/canwu-engine/skills/`.
- Engine-usage skills must use `canwu-api` and must not instruct clients to
  mutate `canwu-sim` state directly.
- Actor-facing workflows must use actor-relative reads and
  `available_actions` before `act`.
- Keep skill instructions short and link to repository source-of-truth docs.
- Validate every `SKILL.md`, `agents/openai.yaml`, and plugin manifest after a
  change.
