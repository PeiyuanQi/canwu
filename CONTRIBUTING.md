# Contributing to Canwu

Thank you for contributing to Canwu. Canwu is open-source software under the
[Apache License 2.0](LICENSE). Review the license and the contribution terms
below before submitting work.

## Development expectations

- Read `AGENTS.md`, `docs/architecture.md`, and `docs/end-state.md` before
  changing architectural boundaries.
- Keep the simulation headless and preserve the validated command boundary.
- Preserve Windows, macOS, and Linux support.
- Keep changes focused and document user-visible or architectural behavior.
- Minimize commit count: keep work uncommitted while iterating, avoid
  checkpoint, WIP, per-agent, test-fix, and review-fix commits, and normally
  submit one coherent commit per reviewable change. Split only for parts that
  genuinely need independent review, rollback, or release; amend unpublished
  task commits instead of stacking more and never rewrite shared history.
- Treat tests as durable evidence. Commit a test only when it is necessary,
  reusable, very likely to fail under a plausible future change, and
  non-trivial: it exercises a multi-step invariant, public contract,
  persistence/replay boundary, or failure-recovery path beyond format, lint,
  compile, or a simple accessor assertion. Run narrower one-off verification
  inline; the project uses no TDD requirement, test quota, or coverage target.
- Identify third-party code, data, assets, or generated material and its license
  in the pull request. Do not submit material that cannot be distributed under
  the project and contributor terms.
- When adding or updating a dependency, update `Cargo.lock` and review
  `docs/legal/third-party-licenses.md` so the dependency inventory stays
  accurate.

## Local development

Install a Rust toolchain that meets the `rust-version` requirement in the root
`Cargo.toml`. Cargo will download the Rust dependencies.

Run the small headless example:

```text
cargo run -p canwu-api --example move_army
```

Run the reference debug client:

```text
cargo run -p canwu-debug
```

Before requesting review, run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use a Git worktree under `.worktrees/` when working on unrelated changes at the
same time. Agents working in this repository must also follow `AGENTS.md`.

## Agent skills

Contributor and maintainer skills live in
`agent-interface/plugins/canwu-developer/skills/`. Their names must begin with
`canwu-developer-`. Use
`canwu-developer-release` for release preparation, package inspection,
third-party notices, and cross-platform release verification.

Engine-user skills live separately in
`agent-interface/plugins/canwu-engine/skills/` so client guidance does not get
mixed with repository-maintainer workflows.

## Contribution licensing

Canwu uses an inbound-equals-outbound contribution model. Unless You explicitly
state otherwise in writing, any Contribution intentionally submitted for
inclusion in Canwu is licensed under the Apache License 2.0 without additional
terms or conditions, consistent with Section 5 of that license.

You retain ownership of Your Contribution. By submitting it, You represent that:

- You have the legal authority to submit the Contribution under Apache-2.0;
- the Contribution is Your original work, or all third-party material is
  clearly identified and may be distributed under compatible terms;
- if an employer or another party may own relevant rights, You have obtained
  its permission; and
- to Your knowledge, the Contribution does not violate another party's
  copyright, patent, trade secret, privacy, or other legal rights.

Contributions are provided without warranties or an obligation to provide
support. The project is not required to accept or use a Contribution.

## Recording agreement

Every external pull request must affirm the contribution licensing terms using
the checkbox in the pull-request template:

> I have read the contribution licensing terms in CONTRIBUTING.md, and I have
> authority to submit this Contribution under the Apache License 2.0.

Maintainers should not merge an external Contribution without that recorded
affirmation. Material clearly marked in writing as "Not a Contribution" is not
submitted for inclusion under these terms.
