# Agent Instructions

## Compatibility

- `CLAUDE.md` is the Claude Code bridge to this instruction file.

## Project Context

- Canwu is a headless historical simulation engine. Rendering, production UI,
  audio, and animation belong in external clients.
- `docs/architecture.md` and `docs/end-state.md` are the architectural source of
  truth. Keep them aligned with public APIs and dependency boundaries.
- Follow `docs/versioning.md`. The root `[workspace.package].version` is the
  canonical SemVer version and all first-party crates version in lockstep.
- External mutations must enter through validated commands. Do not expose a
  mutable reference to live simulation state.
- Agent-facing reads must be actor-relative and must not leak ground truth.
- Preserve Windows, macOS, and Linux support. Avoid assumptions about path
  separators, shells, case sensitivity, line endings, or platform-only APIs.

## Git Workflow

- Prefer git worktrees for parallel or unrelated agent work so multiple agents
  can develop concurrently without colliding. `.worktrees/` is ignored.
- Treat existing uncommitted changes as user-owned unless told otherwise.
- Prefer rebase-based conflict resolution unless a task requires a merge.

## Coding Rules

- Keep deterministic state in ordered collections and give scheduled work an
  explicit sequence number.
- Favor typed IDs and references over ownership trees.
- Keep period-specific mechanics in plugins or systems; the core model stays
  generic.
- Use integer or fixed-unit simulation values where floating-point behavior
  could affect determinism.
- The debug client depends on `canwu-api`, not on simulation internals.
- Add committed tests only for architectural invariants that are reusable and
  plausibly regress under future changes.

## Verification

- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Test: `cargo test --workspace`
- Build debug client: `cargo check -p canwu-debug`
- Cross-platform CI: `.github/workflows/ci.yml` runs on Windows, macOS, and Linux.
