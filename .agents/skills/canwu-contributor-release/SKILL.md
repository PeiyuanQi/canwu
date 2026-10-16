---
name: canwu-contributor-release
description: Prepare and verify Canwu releases, packages, tags, and release artifacts. Use for SemVer decisions, cross-platform release checks, Cargo packaging, Apache-2.0 license integrity, NOTICE preservation, third-party dependency audits, notice generation, or any request to publish or distribute Canwu source crates or compiled binaries.
---

# Release Canwu As A Contributor

Work from the Canwu repository root. Read `AGENTS.md`, `CONTRIBUTING.md`,
`docs/versioning.md`, `LICENSE`, `NOTICE`, `docs/community/branding.md`, and
`docs/legal/third-party-licenses.md` before changing release state.

## Prepare

1. Inspect `git status`, the intended artifacts, and the current workspace
   version. Treat unrelated changes as user-owned.
2. Choose the SemVer change from `docs/versioning.md`. Do not bump the version
   unless the user authorized a release and the intended compatibility change
   requires it. When you bump it, move every `=X.Y.Z` first-party pin with
   the workspace version: the `[workspace.dependencies]` pins in the root
   `Cargo.toml` and the pin in `crates/extensions/canwu-production/Cargo.toml`,
   then run `cargo update --workspace --offline` in the root, in
   `crates/runtime/canwu-sim/tests/api-delta`, and in
   `benchmarks/performance-harness` so each lockfile follows. Canwu crates
   depend only on the exact same version of each other.
3. Confirm the root `LICENSE` contains the unmodified Apache License 2.0 and
   `NOTICE` identifies Canwu and Peiyuan Qi without unresolved placeholders.
4. Confirm every first-party crate inherits the workspace `Apache-2.0` SPDX
   expression and packages byte-for-byte copies of the root `LICENSE` and
   `NOTICE`. Confirm separately distributable website and agent-plugin bundles
   carry matching copies too.

## Verify code and packages

Test in proportion to the change. While iterating, run only the tests of the
crates you changed and of crates that depend on what you changed; do not run
unrelated suites, the performance harness, the website build, or long scale
probes, and do not add tests for parts the change does not touch. Run the full
verification list once before a change set is committed or integrated; in
multi-agent work the integrating agent runs it once for the integrated whole.
For the release itself, run the full list once:

```text
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p canwu-debug
```

Use `cargo package --allow-dirty --no-verify --list -p <crate>` to inspect each
source package. Every first-party source package must contain `LICENSE` and
`NOTICE`. A compiled binary also contains dependency code and assets and needs
the complete third-party notice bundle.

## Run the pre-release tests

CI runs only the four checks of the full list. Once, on the release commit
before it is tagged, also run the release tests that CI does not run. Run them
in a POSIX shell (Git Bash on Windows, or the default shell on macOS and
Linux):

```text
RUSTDOCFLAGS=-Dwarnings cargo doc --locked --workspace --no-deps
cargo run --locked -p canwu-reference-world --example starter
cargo run --locked -p canwu-api --example phased_boundary
cargo run --locked -p canwu-api --example plugin
cargo run --locked -p canwu-api --example decision_ticket
cargo +1.88.0 check --locked --workspace --all-targets
cargo fmt --manifest-path benchmarks/performance-harness/Cargo.toml -- --check
cargo clippy --locked --manifest-path benchmarks/performance-harness/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --locked --manifest-path benchmarks/performance-harness/Cargo.toml
cargo test --locked --manifest-path benchmarks/performance-harness/Cargo.toml --features allocation-counting
cargo test --release --locked --workspace -- --ignored
cargo clippy --locked --workspace --all-targets --features canwu-api/continuation-gate -- -D warnings
cargo test --locked --workspace --features canwu-api/continuation-gate
```

- `RUSTDOCFLAGS=-Dwarnings` makes any rustdoc warning fail the doc build.
- `+1.88.0` proves the `rust-version` that the workspace declares; install it
  with `rustup toolchain install 1.88.0 --profile minimal`, and move it
  whenever `rust-version` moves.
- `--ignored` runs the `#[ignore]` release-only scale tests, which take minutes
  in a debug build: the 65,537-action fiscal boundary and the two law scale
  probes. With the release build it takes about 15 minutes, most of it the
  million-candidate law probe.
- The continuation gate, the `continuation-gate` feature of `canwu-api`, is a
  release test that CI and applications never enable. It shadows every
  `Canwu` in every workspace test with a sealed run, restores the shadow from
  continuation checkpoints, replays the run exactly, and panics on any
  divergence. It proves state equality (checkpoint hashes and the
  continuation round trip) with the run's own plugin registry, not that the
  public restore and replay paths accept the checkpoint or the journal. Lint
  it with the feature on as well, since CI never builds it. A passing run is
  fully green: the one test that declares evidence after its seal,
  `a_packet_first_declared_after_its_seal_is_not_ready`, expects the gate's
  panic. It takes about 40 minutes.

Any failure blocks the release. Fix it in the release change set, then rerun
the failing command and every check above that the fix affects.

## Verify license integrity

- Keep the root license text identical to the standard Apache License 2.0.
- Keep Cargo metadata, package contents, READMEs, contributor terms, plugin
  manifests, website copy, and release documentation aligned on `Apache-2.0`.
- Do not add royalties, revenue thresholds, field-of-use limits, mandatory
  product displays, or other custom restrictions to the Apache license.
- Treat Canwu logo use as optional and separate from the copyright license.
  `docs/community/branding.md` controls truthful, non-endorsing use of project
  marks.
- Confirm third-party materials remain under their upstream licenses and are
  not presented as first-party Apache-2.0 material.
- Preserve the inbound-equals-outbound Apache-2.0 contribution terms.

## Generate third-party notices

Use the pinned tool:

```text
cargo install cargo-about --version 0.8.4 --locked
cargo about generate --workspace --all-features --locked --config tools/licenses/about.toml tools/licenses/about.hbs --output-file docs/legal/third-party-notices.html
```

Review the generated diff. Also review
`docs/legal/third-party-notices-extra.md`; the generator does not collect every
upstream NOTICE file.

Check that the output includes:

- Apache, MIT, BSD, Unicode, ISC, Boost, font, and other selected license text;
- `epaint_default_fonts` plus SIL Open Font License and Ubuntu Font Licence;
- the emoji font MIT notice and Hack/Bitstream Vera notice from
  `docs/legal/third-party-notices-extra.md`; and
- every external crate in the release dependency graph.

Warnings, missing text, or unaccepted licenses for any package must be resolved
before release. First-party Canwu crates have explicit Apache-2.0 metadata and
must not produce missing-license warnings.

## Avoid known release pitfalls

- `Cargo.lock` lists packages but does not contain their full license or
  copyright notices.
- `docs/legal/third-party-licenses.md` is an inventory, not a substitute for the
  generated and additional notice files.
- An SPDX `OR` allows a license choice; `AND` requires every listed license.
- The debug client's `default_fonts` feature embeds font assets with separate
  OFL and Ubuntu Font Licence terms.
- Package-level SPDX metadata does not expose every embedded asset license. The
  Hack font also carries Bitstream Vera terms and the emoji font has a named MIT
  copyright notice.
- Windows, macOS, and Linux select different target dependencies. Review the
  complete target set in `tools/licenses/about.toml`.
- Upstream NOTICE files such as `cfg_aliases/NOTICES.md` require separate
  preservation.
- Regenerating notices online can change harvested text even when `Cargo.lock`
  does not change. Inspect the diff instead of accepting it mechanically.
- Do not publish a source package without its Apache `LICENSE` and `NOTICE`.
- Do not publish a compiled archive without the third-party notice bundle.

## Assemble and report

Include `LICENSE`, `NOTICE`, `docs/legal/third-party-licenses.md`,
`docs/legal/third-party-notices.html`, and
`docs/legal/third-party-notices-extra.md` with compiled releases. Include
`docs/community/branding.md` and logo assets when the release uses those marks.
Report the version, targets, verification commands, notice changes, package
contents, and exact files included. Do not tag, publish, upload, or push unless
the user authorized that external state change.
