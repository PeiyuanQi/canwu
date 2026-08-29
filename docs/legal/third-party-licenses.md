# Third-Party Licenses

Canwu uses third-party Rust crates. Those crates are not covered by Canwu's
Apache-2.0 license. Each dependency remains under the license chosen by its own
authors.

This file is a readable inventory and release checklist. It does not replace
the copyright notices, license files, or attribution requirements supplied by
the dependency authors.

The generated [third-party notices](third-party-notices.html) contain the full
license texts and package attribution discovered by `cargo-about`.
[Additional third-party notices](third-party-notices-extra.md) preserve upstream
notices that the generator does not collect.

## Current dependency set

The inventory was reviewed against the locked dependency graph on August 28,
2026. `cargo metadata --locked --format-version 1` reports 316 external
packages, and every package in that graph declares license metadata.

The complete versioned package list is recorded in `Cargo.lock`. The external
packages and bundled assets use one or more of these license families:

- MIT
- Apache License 2.0, sometimes with the LLVM exception
- BSD Zero Clause, BSD 2-Clause, and BSD 3-Clause
- Boost Software License 1.0
- Creative Commons Zero 1.0
- ISC
- Unicode License 3.0
- The Unlicense
- zlib License
- SIL Open Font License 1.1
- Ubuntu Font License 1.0
- Bitstream Vera Font License
- LGPL 2.1-or-later as one optional choice for `r-efi`

An `OR` in dependency metadata means the upstream package offers a choice of
licenses. An `AND` means all listed terms apply to the relevant code or assets.

## Direct dependencies

| Package | Locked version | Declared license |
| --- | ---: | --- |
| `blake3` | 1.8.6 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| `eframe` | 0.32.3 | MIT OR Apache-2.0 |
| `image` | 0.25.10 | MIT OR Apache-2.0 |
| `semver` | 1.0.28 | MIT OR Apache-2.0 |
| `serde` | 1.0.229 | MIT OR Apache-2.0 |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 |

The first-party `canwu-*` crates are not third-party dependencies. They use the
root [Apache License 2.0](../../LICENSE).

## Bundled fonts

The debug client enables the `default_fonts` feature in `eframe`. Its locked
`epaint_default_fonts` 0.32.3 package declares:

```text
(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0
```

Binary releases of the debug client must preserve the applicable font license
and attribution material as well as the Rust crate notices. Package-level SPDX
metadata does not expose every embedded font notice: the emoji font has its own
MIT copyright notice, and Hack includes Source Foundry, DejaVu, and Bitstream
Vera terms. Those notices are preserved in
`docs/legal/third-party-notices-extra.md`.

## Release requirements

Before publishing a source archive or compiled binary, release maintainers
must:

1. run `cargo metadata --locked --format-version 1` and review the licenses for
   the dependency graph used by that release;
2. install the pinned generator with
   `cargo install cargo-about --version 0.8.4 --locked`;
3. regenerate the notice file with
   `cargo about generate --workspace --all-features --locked --config
   tools/licenses/about.toml tools/licenses/about.hbs --output-file
   docs/legal/third-party-notices.html`;
4. update this file when `Cargo.lock` or enabled features change the inventory;
5. preserve additional upstream NOTICE files in
   `docs/legal/third-party-notices-extra.md`; and
6. include `LICENSE`, `NOTICE`, this inventory, and both third-party notice
   files in every compiled release package; include branding assets only when
   the release itself uses them.

Platform-specific dependencies differ across Windows, macOS, and Linux. A
release for one platform only needs notices for material included in that
release, but the repository inventory should continue to cover the complete
locked cross-platform graph.

The first-party Canwu crates inherit the workspace's `Apache-2.0` SPDX license
expression and package copies of the root `LICENSE` and `NOTICE` files.
Warnings or failures for an external package are not expected and must be
resolved before release.
