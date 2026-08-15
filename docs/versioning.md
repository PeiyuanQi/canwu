# Versioning and Compatibility

Canwu uses [Semantic Versioning 2.0.0](https://semver.org/). The canonical
project version is `[workspace.package].version` in the root `Cargo.toml`, and
all first-party crates use that version in lockstep.

## SemVer policy

- `MAJOR`: incompatible changes to supported public APIs after `1.0.0`.
- `MINOR`: backward-compatible functionality. Before `1.0.0`, a minor release
  may contain an intentional breaking API change, which must be documented.
- `PATCH`: backward-compatible fixes, documentation, and internal improvements.
- Pre-release identifiers such as `0.2.0-alpha.1` are used for unstable release
  candidates when needed.

The SemVer compatibility surface includes exported Rust API types and behavior,
serialized command/query/event contracts that are documented as public, and the
semantic-agent operation shapes. Internal crate implementation details are not
part of the compatibility guarantee.

## Snapshot format

Engine SemVer and snapshot format versioning are separate. Every snapshot stores
the producing engine version and an integer snapshot format version. Patch and
minor releases may continue to read an older snapshot format. A format change
increments the format number and should provide a migration path when practical.

Executable plugin handlers are never serialized. Snapshots retain their plugin
descriptors, block authoritative continuation while required handlers are
inactive, and accept rehydration only when registration recreates the exact
stored descriptor. Plugin command journals must use plugin-aware replay.

Snapshot format 2 adds namespaced plugin component records plus deterministic,
typed state keys, machine-validated command payload schemas, declared
read/write ownership, the plugin-registration lifecycle flag, actor-known army
names, initial simulation time, and deterministic plugin system/action
contracts. Component records use typed `(plugin, state, entity, component)`
identity. Load validates canonical ordering, references, causes, transit/queue
and report-delivery coherence, registration lifecycle, descriptors, ownership,
and counter continuity before constructing runtime maps. Format 1 is
intentionally rejected; no released save depends on that initial
development-only format.

## Supported operating systems

Canwu supports Windows, macOS, and Linux:

- Headless engine crates avoid operating-system-specific APIs.
- The reference debug client uses `eframe` with the OpenGL `glow` backend.
- Linux enables both Wayland and X11 window backends.
- CI builds, lints, and tests the workspace on all three operating systems.

Platform-specific integrations must remain in adapters or narrowly scoped
modules. New code should use `std::path` and portable Rust APIs rather than
assuming path separators, shell syntax, filesystem case sensitivity, or a
particular newline convention.
