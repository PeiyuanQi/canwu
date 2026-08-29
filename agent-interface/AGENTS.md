# Canwu Agent Interface Instructions

This folder contains Codex skill plugins for working with Canwu. These are agent
tools, not runtime `SimulationPlugin` implementations.

- The existing contributor and maintainer plugin is
  `plugins/canwu-developer/skills/`; keep that package path and its existing
  `canwu-developer-release` name for compatibility.
- Name new contributor skills `canwu-contributor-*` so they are not confused
  with developers who build games or historical simulations on Canwu.
- Put external engine-usage skills in `plugins/canwu-engine/skills/`.
- Engine-usage skills must use `canwu-api` and must not instruct clients to
  mutate `canwu-sim` state directly.
- Actor-facing workflows must use actor-relative reads and
  `available_actions` before `act`.
- Keep skill instructions short and link to repository source-of-truth docs.
- Validate every `SKILL.md`, `agents/openai.yaml`, and plugin manifest after a
  change.
- `canwu-contributor-design` is additionally exposed through the repository-local
  `.agents/` and `.claude/` skill loaders; keep those loaders pointed at the
  packaged source. This contributor skill is for changing Canwu itself, not for
  developers building games or historical simulations on top of Canwu.
