---
name: canwu-developer-release
description: Prepare and verify Canwu releases, packages, tags, and release artifacts. Use for SemVer decisions, cross-platform release checks, Cargo packaging, custom-license integrity, commercial attribution, third-party dependency audits, notice generation, or any request to publish or distribute Canwu source crates or compiled binaries.
---

# Release Canwu

Work from the Canwu repository root. Read `AGENTS.md`, `CONTRIBUTING.md`,
`docs/versioning.md`, `LICENSE`, `BRANDING.md`, and
`THIRD_PARTY_LICENSES.md` before changing release state.

## Prepare

1. Inspect `git status`, the intended artifacts, and the current workspace
   version. Treat unrelated changes as user-owned.
2. Choose the SemVer change from `docs/versioning.md`. Do not bump the version
   unless the user authorized a release and the intended compatibility change
   requires it.
3. Treat `TBD` copyright-holder, Licensor, governing-law, or court fields in
   `LICENSE` as blockers for a formal public release.
4. Confirm every first-party crate inherits the same workspace version and
   `license-file`.

## Verify code and packages

Run:

```text
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p canwu-debug
```

Use `cargo package --allow-dirty --no-verify --list -p <crate>` to inspect each
source package. A source crate normally references dependencies instead of
bundling them. A compiled binary contains dependency code and assets and needs
the complete notice bundle.

## Verify license integrity

- Keep the title `Canwu License 1.0` and describe it as source-available, not
  OSI-approved open source.
- Preserve the Product Family aggregation and progressive marginal brackets.
  At $50 million Product Revenue, the royalty must remain exactly $325,000.
- Preserve the commercial Canwu logo and acknowledgement requirement from
  `LICENSE` and `BRANDING.md`.
- Confirm Third-Party Materials are excluded from the Canwu license grant and
  remain under their upstream licenses.
- Never apply Canwu royalties, branding, or attribution to a third-party
  component used independently of Canwu.
- Sponsorship must not create downstream acknowledgement or licensing duties.

## Generate third-party notices

Use the pinned tool:

```text
cargo install cargo-about --version 0.8.4 --locked
cargo about generate --workspace --all-features --locked about.hbs --output-file THIRD_PARTY_NOTICES.html
```

Review the generated diff. Also review `THIRD_PARTY_NOTICES_EXTRA.md`; the
generator does not collect every upstream NOTICE file.

Check that the output includes:

- Apache, MIT, BSD, Unicode, zlib, ISC, Boost, and other selected license text;
- `epaint_default_fonts` plus SIL Open Font License and Ubuntu Font Licence;
- the emoji font MIT notice and Hack/Bitstream Vera notice from
  `THIRD_PARTY_NOTICES_EXTRA.md`; and
- every external crate in the release dependency graph.

`cargo-about` 0.8.4 emits warnings for Canwu's own crates because Cargo exposes
the custom workspace `license-file` without an SPDX `license` expression. These
first-party warnings are known. Do not dismiss warnings, missing text, or
unaccepted licenses for external packages.

## Avoid known release pitfalls

- `Cargo.lock` lists packages but does not contain their full license or
  copyright notices.
- `THIRD_PARTY_LICENSES.md` is an inventory, not a substitute for the generated
  and additional notice files.
- An SPDX `OR` allows a license choice; `AND` requires every listed license.
- The debug client's `default_fonts` feature embeds font assets with separate
  OFL and Ubuntu Font Licence terms.
- Package-level SPDX metadata does not expose every embedded asset license. The
  Hack font also carries Bitstream Vera terms and the emoji font has a named MIT
  copyright notice.
- Windows, macOS, and Linux select different target dependencies. Review the
  complete target set in `about.toml`.
- Upstream NOTICE files such as `cfg_aliases/NOTICES.md` require separate
  preservation.
- Regenerating notices online can change harvested text even when `Cargo.lock`
  does not change. Inspect the diff instead of accepting it mechanically.
- Do not publish a compiled archive containing only the Canwu `LICENSE`.

## Assemble and report

Include `LICENSE`, `BRANDING.md`, required logo assets,
`THIRD_PARTY_LICENSES.md`, `THIRD_PARTY_NOTICES.html`, and
`THIRD_PARTY_NOTICES_EXTRA.md` with compiled releases. Report the version,
targets, verification commands, notice changes, unresolved legal fields, and
the exact files included. Do not tag, publish, upload, or push unless the user
authorized that external state change.
