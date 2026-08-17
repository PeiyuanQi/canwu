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
- External mutations must enter through validated commands or canonical ingress.
  Do not expose a mutable reference to live simulation state.
- Agent-facing reads must be actor-relative and must not leak ground truth.
- Preserve Windows, macOS, and Linux support. Avoid assumptions about path
  separators, shells, case sensitivity, line endings, or platform-only APIs.

## Agent Development Flow

1. Read this file and every nearer `AGENTS.md` before editing.
2. Inspect `git status` and preserve user-owned or concurrent changes. Use a
   worktree for unrelated work; stay in the current checkout when the task
   explicitly depends on its uncommitted state.
3. Name the invariant being changed and use the change-surface map below to
   inspect every affected public, persistence, replay, documentation, and test
   surface before implementation.
4. Make the smallest coherent change. Keep behavior changes separate from large
   file moves, generated-file refreshes, licensing work, and unrelated cleanup.
5. Treat tests as durable evidence rather than a development process. Admit a
   committed test only when it is necessary, reusable, non-trivial, and very
   likely to catch a plausible future regression. Run narrower one-off checks
   inline without committing them, then run the scoped checks and full
   repository verification listed below.
6. For public API, persistence, replay, migration, authority, determinism, or
   performance changes, obtain an independent review before committing. Resolve
   every blocking finding and re-run the affected checks.
7. Keep implementation and verification uncommitted until the requested change
   set is complete. When explicitly asked to commit or push, follow the commit
   economy rules below, stage explicit paths only, use a conventional commit,
   push without force, and report remaining uncommitted work separately.

<details>
<summary><strong>Project hierarchy and change-surface map</strong></summary>

### Repository hierarchy

- `crates/canwu-core`: stable IDs, deterministic RNG, and schema metadata shared
  across packages.
- `crates/canwu-time`: deterministic simulation time and duration arithmetic.
- `crates/canwu-event`: causal event types and evidence references.
- `crates/canwu-world`: authoritative world entities and immutable snapshots.
- `crates/canwu-knowledge`: actor-relative knowledge and observation state.
- `crates/canwu-sim`: authoritative state, ingress, settlement, scheduling,
  plugins, records, persistence, validation, hashing, migration, and replay.
- `crates/canwu-api`: the supported public facade and re-export boundary.
- `crates/canwu-debug`: reference client; it may depend on `canwu-api` only.
- `docs`: architecture, end-state, versioning, and compatibility contracts.
- `benchmarks`: deterministic non-CI performance harnesses and recorded
  baselines; measurement tooling, not authoritative runtime code.
- `agent-interface`: Codex plugin packages and skills. Follow its nested
  `AGENTS.md`; these are development/user tools, not runtime simulation plugins.
- `website` and `assets`: community site and project media, outside the
  authoritative simulation runtime.
- `.github`: CI workflows, the pull-request template, and repository automation.

### Change-surface map

| If changing | Inspect and usually update |
| --- | --- |
| Stable IDs, generic references, or schemas | `canwu-core`; owning world/event/record types; `canwu-api` re-exports; serialization and migration tests |
| World entity shape or lifecycle | `canwu-world`; `canwu-knowledge`; simulation validation/persistence/replay; public queries; debug projections |
| Actor-relative knowledge or visibility | `canwu-knowledge`; `SimulationView`; `canwu-api`; information-flow and replay tests |
| Commands, authority, run policy, or ingress | `canwu-sim` policy/ingress/validation; core request IDs; `canwu-api`; snapshot/replay/versioning docs; stale/idempotency/rollback tests |
| Settlement phases, reservations, or scheduling | `canwu-sim` boundary/scheduling code; architecture docs; API-only examples; atomicity, ordering, and exact-replay tests |
| Random algorithms, streams, or draws | `canwu-core` deterministic generator; `canwu-sim` random ownership, journals, persistence, hashing, validation, migration, and replay; statistical-boundary and tamper tests |
| Runtime plugin, component, or record contracts | registrar/descriptors/semantic hashes; ownership checks; snapshot rehydration; `canwu-api`; plugin fixtures and examples |
| Snapshot fields, journals, hashes, or format versions | persistence, hashing, validation, migration, replay, and checkpoint code together; `docs/versioning.md`; old-format and per-domain tamper tests |
| Performance or scaling behavior | `benchmarks`; affected runtime paths; deterministic workload counts; allocation, elapsed-time, snapshot-size, and growth evidence before and after |
| Public API behavior or types | `canwu-api`; crate re-exports; public examples; rustdoc; debug client; compatibility notes |
| Agent skills or plugin packaging | `agent-interface/AGENTS.md`; affected `SKILL.md`; `.codex-plugin/plugin.json`; any `agents/openai.yaml`; plugin validation |
| Dependencies, licensing, or release metadata | workspace and crate manifests; `Cargo.lock`; notices/license inventory; contribution/release docs; packaged plugin notices |
| Website or branding | `website`; `assets`; `BRANDING.md`; community-facing README links; site-specific checks |
| Community README or onboarding structure | Keep `README.md` and `README.zh-CN.md` equivalent in claims, sections, commands, links, and folded development flow; update related website/docs links when needed |

If a change crosses several rows, treat that as one architectural milestone and
verify the whole dependency path. Keep application-specific rules and entity
types outside Canwu core; downstream packages depend on Canwu's generic public
contracts, never the reverse.

</details>

## Git Workflow

- Prefer git worktrees for parallel or unrelated agent work so multiple agents
  can develop concurrently without colliding. `.worktrees/` is ignored.
- Treat existing uncommitted changes as user-owned unless told otherwise.
- Minimize commit count. Do not create a commit per agent, turn, phase, file,
  test pass, or review iteration, and do not use checkpoint, progress, WIP,
  test-fix, or review-fix commits as agent state.
- Unless the user requests a different boundary, keep a normal task uncommitted
  through implementation and verification. When a commit is requested, produce
  one coherent final commit for the requested change set.
- If the same task already has an unpublished agent-owned commit, amend it
  instead of appending another commit. If temporary local fixups already exist,
  squash them before handoff. Never rewrite commits that predate the task, are
  user-owned, or have been pushed or shared unless explicitly authorized.
- Use multiple commits only when parts genuinely require independent review,
  rollback, or release. In multi-agent work, the integrating agent owns the
  final commit; delegated workers leave changes uncommitted unless assigned an
  explicit commit boundary.
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
- Tests are evidence, not process. The project uses no test-driven-development
  requirement, test quota, or coverage target.
- Admit a committed test only when it is necessary for a durable contract,
  reusable across implementations, very likely to fail under a plausible
  future change, and non-trivial. Non-trivial tests exercise a multi-step
  invariant, public contract, persistence/replay boundary, or failure recovery
  path that format, lint, compile, or a simple accessor assertion cannot prove.
- Run useful verification that falls below that bar once inline and leave it out
  of the committed test suite.

## Verification

- Format: `cargo fmt --all -- --check`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Test: `cargo test --workspace`
- Build debug client: `cargo check -p canwu-debug`
- Cross-platform CI: `.github/workflows/ci.yml` runs on Windows, macOS, and Linux.
- Public examples when APIs or behavior change:
  `cargo run -p canwu-api --example move_army`,
  `cargo run -p canwu-api --example phased_boundary`, and
  `cargo run -p canwu-api --example plugin`.
- Rust documentation when public types or docs change:
  `cargo doc --workspace --no-deps`.
- Standalone performance harness when its workload or reporting changes:
  `cargo fmt --manifest-path benchmarks/performance-harness/Cargo.toml -- --check`,
  `cargo clippy --manifest-path benchmarks/performance-harness/Cargo.toml --all-targets --all-features -- -D warnings`,
  `cargo test --manifest-path benchmarks/performance-harness/Cargo.toml`, and
  `cargo test --manifest-path benchmarks/performance-harness/Cargo.toml --features allocation-counting`.
