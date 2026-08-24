# Releasing Canwu

Canwu publishes its Rust libraries to crates.io and uses GitHub tags and
releases as the immutable source reference for each version. Applications
should depend only on `canwu-api`; the remaining library crates are published
to satisfy its dependency graph. `canwu-debug` is a reference binary and is not
published to crates.io.

## Release identity

All first-party crates use the root `[workspace.package].version` in lockstep.
A release is complete only when these identities agree:

- the version in `Cargo.toml`;
- the release commit;
- the annotated Git tag `v<version>`;
- every published `canwu-*` library crate;
- the GitHub Release attached to that tag.

Do not retag or republish a released version. If any published package is
wrong, fix it in a new SemVer release. Snapshot format and engine SemVer remain
separate contracts as described in [versioning.md](versioning.md).

## Prepare and verify

Work from a clean release commit and run:

```text
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p canwu-debug
cargo doc --locked --workspace --no-deps
cargo +1.88.0 check --locked --workspace --all-targets
```

The workspace declares Rust 1.88 as its minimum supported version. A release
must verify that claim explicitly rather than treating a successful build on
the latest stable toolchain as MSRV proof.

Inspect every source package with `cargo package --locked --list -p <crate>`.
Each package must contain `LICENSE`, `NOTICE`, and `README.md`. Regenerate and
review the third-party notice bundle according to
[legal/third-party-licenses.md](legal/third-party-licenses.md). The notice
bundle is mandatory for compiled debug-client archives, not for library source
packages that contain no debug-client dependency graph.

Before the initial publication of a crate name, confirm that the name is still
available on crates.io. Never put a registry token in a repository file,
command transcript, release artifact, or CI log.

## Publish order

Publish from the tagged commit in dependency order. The human-readable crate
DAG is maintained in [`crates/README.md`](../crates/README.md). Wait until
crates.io can resolve every completed group before continuing:

1. `canwu-core`, `canwu-time`
2. `canwu-decision`, `canwu-event`, `canwu-knowledge`
3. `canwu-routing`, `canwu-sim`
4. `canwu-transport`
5. `canwu-api`

For each package, first run its dry-run and then publish the exact locked
source:

```text
cargo publish --locked --dry-run -p <crate>
cargo publish --locked -p <crate>
```

After the registry exposes the package, verify its version and dependency
metadata before publishing the next group. Finally, create a clean temporary
consumer crate that depends on `canwu-api = "=<version>"`, build it using the
registry only, and run a minimal public-API smoke test.

## GitHub release

Create the annotated tag from the verified release commit and push it without
force. A GitHub Release may be prepared as a draft before registry publication,
but publish the release only after every crates.io package and the external
consumer smoke test pass. Release notes must state the supported public API,
snapshot compatibility implications, major capabilities, known conformance
gaps, and verification performed.

Compiled `canwu-debug` archives are optional. If supplied, build each supported
target separately and include `LICENSE`, `NOTICE`, the third-party license
inventory, generated notices, and additional upstream notices.

## Authentication

The first publication of new crate names requires a short-lived crates.io API
token with the minimum necessary scope. After the crates exist, configure a
trusted publisher for every published crate and prefer GitHub Actions OIDC for
later releases. Remove or revoke bootstrap credentials after the first release.
