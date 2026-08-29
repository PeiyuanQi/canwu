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
   requires it.
3. Confirm the root `LICENSE` contains the unmodified Apache License 2.0 and
   `NOTICE` identifies Canwu and Peiyuan Qi without unresolved placeholders.
4. Confirm every first-party crate inherits the workspace `Apache-2.0` SPDX
   expression and packages byte-for-byte copies of the root `LICENSE` and
   `NOTICE`. Confirm separately distributable website and agent-plugin bundles
   carry matching copies too.

## Verify code and packages

Run:

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
