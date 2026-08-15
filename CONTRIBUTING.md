# Contributing to Canwu

Thank you for contributing to Canwu. Canwu is source-available under the
[Canwu License 1.0](LICENSE), not an OSI-approved open-source license. Review
the license and the contributor terms below before submitting work.

## Development expectations

- Read `AGENTS.md`, `docs/architecture.md`, and `docs/end-state.md` before
  changing architectural boundaries.
- Keep the simulation headless and preserve the validated command boundary.
- Preserve Windows, macOS, and Linux support.
- Keep changes focused and document user-visible or architectural behavior.
- Identify third-party code, data, assets, or generated material and its license
  in the pull request. Do not submit material that cannot be distributed under
  the project and contributor terms.
- When adding or updating a dependency, update `Cargo.lock` and review
  `THIRD_PARTY_LICENSES.md` so the dependency inventory stays accurate.

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

## Contributor License Grant

These terms preserve the Project Owner's ability to distribute Canwu under the
Canwu License 1.0, maintain its commercial-success royalty model, and offer
separate commercial agreements. They do not transfer ownership of a
Contribution to the Project Owner.

### Definitions

"Contribution" means any original work of authorship, modification, patch,
documentation, test, design, or other material that You intentionally submit
for inclusion in Canwu. It does not include material that You clearly mark in
writing as "Not a Contribution."

"Submit" means providing a Contribution to the project through a pull request,
commit, patch, issue attachment, electronic communication, or another channel
managed by the project for the purpose of discussing or improving Canwu.

"Project Owner" means the Licensor identified in `LICENSE`, including a lawful
successor that owns or administers the relevant project rights.

### Ownership

You retain ownership of Your Contribution. Except for the licenses expressly
granted below, no ownership right is transferred.

### Copyright license

By submitting a Contribution, You grant the Project Owner a perpetual,
worldwide, non-exclusive, royalty-free, fully paid-up, irrevocable,
transferable, and sublicensable license under all copyrights and similar rights
You control in the Contribution to:

- use, reproduce, modify, and prepare derivative works of the Contribution;
- publicly display, publicly perform, and make the Contribution available;
- distribute, license, sell, offer for sale, import, and otherwise exploit the
  Contribution, alone or as part of Canwu; and
- exercise these rights under the Canwu License 1.0 or later versions, and
  under separate commercial, proprietary, enterprise, or other custom license
  terms.

The Project Owner may collect and retain fees or royalties from those licenses
without an obligation to account to You, unless a separate written agreement
signed by You and the Project Owner states otherwise.

To the extent permitted by applicable law, You waive and agree not to assert
moral rights or similar rights in the Contribution to the extent necessary for
the Project Owner and its sublicensees to exercise the licenses granted here.

### Patent license

You grant the Project Owner and its sublicensees a perpetual, worldwide,
non-exclusive, royalty-free, fully paid-up, irrevocable except as stated below,
transferable, and sublicensable license under patent claims You control that
are necessarily infringed by Your Contribution alone or by its combination
with Canwu as submitted, to make, have made, use, offer for sale, sell, import,
and otherwise transfer the Contribution and Canwu.

If You initiate patent litigation alleging that Canwu or a Contribution
infringes a patent, the patent licenses granted by You under these contributor
terms terminate as of the date the litigation is filed for the work that is the
subject of that litigation.

### Your assurances

You represent that:

- You have the legal authority to submit the Contribution and grant these
  licenses;
- the Contribution is Your original work, or You have clearly identified all
  third-party material and have sufficient rights to submit it under these
  terms;
- if an employer or another party may own rights in the Contribution, You have
  obtained its permission to submit the Contribution and grant these licenses;
  and
- to Your knowledge, the Contribution does not violate a third party's
  copyright, patent, trade secret, privacy, or other legal rights.

Contributions are provided without warranties or an obligation to provide
support. The Project Owner is not required to accept or use any Contribution.

## Recording agreement

Every external pull request must affirm the contributor grant using the
checkbox in the pull-request template. Contributions submitted through another
channel require an equivalent written statement:

> I have read and agree to the Contributor License Grant in CONTRIBUTING.md,
> and I have authority to submit this Contribution under those terms.

Maintainers should not merge an external Contribution without a recorded
affirmation. Changes to these contributor terms apply prospectively; the terms
accepted when a Contribution was submitted continue to govern that
Contribution.
