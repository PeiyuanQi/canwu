# Canwu Agent Interface Instructions

This folder contains packaged skills for external users of Canwu. These are
agent tools, not runtime `SimulationPlugin` implementations.

- Repository contributor and maintainer skills belong natively under
  `../.agents/skills/`, with Claude-compatible loaders under
  `../.claude/skills/`. Do not package them here.
- Put general documentation and public engine-usage skills in
  `plugins/canwu-engine/skills/`.
- Put downstream game and historical-simulation development skills in
  `plugins/canwu-developer/skills/`. These help users build products on Canwu;
  they are not workflows for contributing to the Canwu repository.
- Engine-usage and downstream-development skills must use `canwu-api` and must
  not instruct clients to mutate `canwu-sim` state directly.
- Claude-compatible loaders for packaged downstream skills may live under
  `../.claude/skills/`, but the packaged skill remains the source of truth.
- Actor-facing workflows must use actor-relative reads and
  `available_actions` before `act`.
- Keep skill instructions short and link to repository source-of-truth docs.
- Validate every `SKILL.md`, `agents/openai.yaml`, and plugin manifest after a
  change.
