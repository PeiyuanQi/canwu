# Versioning and Compatibility

Canwu uses [Semantic Versioning 2.0.0](https://semver.org/). The canonical
project version is `[workspace.package].version` in the root `Cargo.toml`, and
all first-party crates use that version in lockstep.

## SemVer policy

- `MAJOR`: incompatible changes to supported public APIs after `1.0.0`.
- `MINOR`: backward-compatible functionality. Before `1.0.0`, a minor release
  may contain an intentional breaking API change, which must be documented.
- `PATCH`: backward-compatible fixes, documentation, and internal improvements.
- Pre-release identifiers such as `0.3.0-alpha.1` are used for unstable release
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
Format-4 state and boundary commitments include the producing engine version,
so this runtime rejects a format-4 snapshot from any other engine version until
an explicit migration rewrites the commitments. Format 2 and 3 migration records
the source engine version, then emits a current-engine format-4 checkpoint.

Executable plugin handlers are never serialized. Snapshots retain their plugin
descriptors and author-declared package versions and semantic hashes, block
authoritative continuation while required handlers are inactive, and accept
rehydration only when registration recreates that exact identity and contract.
Plugin command journals must use plugin-aware replay.

Snapshot format 4 replaces the single global RNG with owned, versioned random
streams and a draw journal containing producer, purpose, cause, correlation,
position, bound, and result. Every successful phased boundary records a
deterministic state hash and a chained boundary hash. Snapshots also persist a
hashed run manifest for scenario, rules, content, localization contracts, run
configuration, and source identities. A recomputed checkpoint hash binds the
complete current deterministic state to the boundary-chain head. Exact
`ReplayJournal` replay verifies engine and format versions, root seed, run and
plugin manifests, the plugin-registration lifecycle state, commands,
boundaries, final time, and final checkpoint hash, including command-only and
registration-closure-only runs. Each report dispatch must retain exactly one
causally linked core random draw, and authoritative scheduling rejects
unrepresentable time instead of saturating. Checked hour/day construction and
checked time/duration arithmetic are available for data-dependent values;
convenience constructors and operators never clamp. New runs require declared
manifests; format 2 and 3 checkpoints without plugins migrate with explicit
legacy provenance. They may continue, but exact replay returns
`legacy_replay_unavailable`. Legacy snapshots containing executable plugin
descriptors are rejected because their handler semantic identities cannot be
recovered safely.

Snapshot format 3 adds canonical phased-boundary records, exact plugin/system
emission provenance, command and event admission, reservation offers, requests,
allocations, committed component changes, boundary causes, and the next boundary
counter. Loading recomputes allocation evidence and validates each boundary
change and emission against its serialized plugin contract. The engine performs
an explicit format 2 to format 3 migration when no phased-boundary state is
present. Boundary-aware replay regenerates and compares complete boundary
records rather than silently replaying only the command subset of a run.

Snapshot format 2 introduced namespaced plugin component records,
deterministic typed state keys, machine-validated command payload schemas,
declared read/write ownership, the plugin-registration lifecycle flag,
actor-known army names, initial simulation time, and deterministic plugin
system/action contracts. Component records use typed
`(plugin, state, entity, component)` identity. Format 1 remains intentionally
rejected; no released save depends on that initial development-only format.

Every supported load validates canonical ordering, references, causes,
transit/queue and report-delivery coherence, registration lifecycle, run and
plugin manifests, causally linked random evidence, descriptors, ownership,
boundary hashes, the current checkpoint commitment, and counter continuity
before constructing runtime maps.

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
