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
configuration, and source identities. Additive format-4 run-policy fields use
explicit `CompatibilityV1` or `LegacyUnspecified` provenance when older data did
not record the six run-policy dimensions. Earlier format-4 snapshots with a
custom run-configuration artifact hydrate as `ManifestOnlyV1`: their exact
manifest identity and replay remain valid, but the engine does not invent
policy dimensions that were never serialized. Pre-policy format-4 replay
journals hydrate the same provenance from their manifest and default the absent
attempt journal to empty, so command-only journals remain readable. The default
compatibility artifact cannot be relabeled as `ManifestOnlyV1`. Declared
configurations persist those dimensions plus typed live/frozen-replay
command-attempt evidence,
including accepted and expected-rejected outcomes, idempotency keys, revision
and simulation-time guards, authority context, and synchronous emitted-event
IDs. Legacy-direct command records are restricted to compatibility provenance;
declared runs require tracked attempt evidence. Legacy format 2 and 3 inputs are
rejected if they attempt to smuggle these newer fields into a historical shape.

Format 4 also permits additive canonical-ingress descriptors, records, counters,
and boundary admission/generation evidence. Empty ingress fields and the default
next-ingress counter are omitted, preserving the serialized and hashed shape of
earlier record-free format-4 state. Persisted queue order is due time, class,
descending priority, issue time, then ingress ID; every record also carries the
boundary-count cut after which it can be admitted. Boundary-generated packets
name their producing boundary and appear in that boundary's ordered
`generated_ingress` evidence with the producer plugin, system, phase, and
visibility/commit-stage input. Both admission and generation evidence are bound
by the chained boundary hash. A zero-delay generated packet is deliberately
assigned to the next admission cut, which may create a second boundary at the
same timestamp. Loading validates descriptor ownership and schemas, stable
entity identities at the issue and producer-proposal cuts, canonical admission
order, cause and generation provenance, pending-work timeliness, and counter
continuity. Command-attempt order, live-request provenance, and admitted calendar
cadences are reconstructed from the queue. Generated delays must fit the signed
simulation-duration domain. Exact replay re-enqueues external records but
requires plugin systems to reproduce boundary-generated records. Declared read-only runs reject newly authored live
plugin ingress. Format 2 and 3 inputs reject all of these fields rather than
interpreting canonical-ingress semantics under a legacy identity.

Format 4 also has a separately versioned authoritative-revision sub-contract.
Revision format 1 persists a monotonic value that advances exactly once for each
accepted command, persisted expected rejection, or completed settlement
boundary. Failed transactions, exact retries, request-ID collisions, bare clock
movement, queued but unadmitted ingress, and plugin setup do not advance it;
expected simulation time remains the independent guard for clock and scheduled
work. The value is reconstructible as tracked command attempts plus boundaries,
or as legacy-direct commands plus boundaries when no attempt journal exists.
Checkpoint domain `canwu.checkpoint.v3` binds the revision format and value in
addition to deterministic state, the boundary-chain head, and applicable run
identity. The boundary-state hash deliberately remains revision-neutral.

An earlier format-4 snapshot defaults to revision format 0. Loading first
verifies its legacy checkpoint, translates command-attempt revisions and
expected-revision guards without changing their stale/current relationship,
refreshes the boundary-head state commitment and chained boundary hashes when
needed, derives the final revision from committed evidence, and emits a current
checkpoint. A format-0 replay journal does not carry a final revision commitment
and is therefore not reinterpreted as exact replay. Its snapshot can migrate and
continue, but retains migration-only replay provenance because snapshot-only
migration cannot reconstruct every historical boundary state commitment. Saves
created under revision format 1 export current exact-replay journals normally.

Format 4 also permits additive admission-cursor format 1. Snapshots persist the
number of attempt, accepted-command, and event records consumed by completed
boundaries. Runtime settlement uses those monotonic counts to read only the new
journal tails. Loading still walks boundary evidence once to prove the global
causal prefix and requires the persisted counts to match exactly; gaps,
duplicates, backward cursors, and counts beyond the journals are rejected.
Older format-4 snapshots default to cursor format 0 and derive current counts
from their validated boundary lists. The cursors are redundant derived metadata
and are deliberately excluded from authoritative state and boundary hashes, so
this optimization does not reinterpret existing simulation-result commitments.

Format 4 also permits commitment format 1. Current snapshots persist
domain-separated canonical roots for world, knowledge, plugin components,
generic records, scheduler state, commands and attempts, events, ingress,
random state and draws, the boundary chain, authoritative run/plugin identity,
and runtime control counters. Each unordered collection is sorted by stable
identity before hashing. Checkpoint domain `canwu.checkpoint.v4` binds those
roots, the exact run-manifest hash, the commitment format, and the authoritative
revision contract. A format-0 snapshot is never interpreted under the new
semantics: loading first recomputes and verifies its checkpoint-v3 full-state
hash, then derives roots and emits checkpoint v4. Any present format-1 root is
recomputed and compared independently before the outer checkpoint is accepted.
Replay journals persist their commitment format; format-0 journals reproduce
checkpoint v3 exactly, while current journals reproduce checkpoint v4.

Boundary state commitments have an independent in-field contract. Historical
64-character hashes are legacy format 0 and retain their original full-state
meaning. New boundaries use `v1:<64-character hash>`, derived from commitment
format 1 roots after authoritative mutation and before the new boundary record
enters the chain. The roots include current world, knowledge, plugin state,
generic records, scheduler, command/attempt, event, ingress, random, identity,
control, and the prior boundary-chain head. Existing hashes are never
reinterpreted or rewritten. A loaded legacy chain may append tagged v1 records,
and exact replay chooses the contract recorded on each boundary. Unknown tags
are rejected. When a snapshot is at a boundary head and the record carries a
state commitment, loading recomputes that commitment from validated state and
requires an exact match.

Checkpoint-journal format 1 is an additive persistence envelope separate from
snapshot format 4. Its current-state checkpoint contains a current snapshot
shell with empty event, command, command-attempt, ingress, boundary, and random-
draw arrays plus the full evidence cursor and existing commitment roots.
Contiguous evidence segments reconstruct those arrays before normal snapshot
validation. Segment gaps, duplicates, non-advancing ranges, false end cursors,
checkpoint-side evidence duplication, unsupported envelope formats, and any
record tampering are rejected. Existing flat snapshots are neither relabeled
nor migrated into this envelope implicitly; they remain readable through the
existing snapshot APIs. Checkpoint-journal format 1 requires the current
snapshot format and has no legacy interpretation.

Authoritative state and boundary hashes normalize the run-policy artifact: the
actual command/effect journal remains authoritative, while run purpose,
controller, seat, observation, interaction, and trace policy do not alter
simulation-result identity. The recomputed checkpoint uses a versioned
save-container commitment that also binds the exact full run-manifest hash.
Thus observation/trace-only variants have identical authoritative and RNG
results but distinct save identity. Exact `ReplayJournal` replay verifies engine
and format versions, root seed, run and plugin manifests, run configuration,
plugin-registration lifecycle state, accepted commands, command attempts,
boundaries, final time, and final checkpoint hash, including command-only,
rejection-only, and registration-closure-only runs. Each report dispatch must retain exactly one
causally linked core random draw, and authoritative scheduling rejects
unrepresentable time instead of saturating. Checked hour/day construction and
checked time/duration arithmetic are available for data-dependent values;
convenience constructors and operators never clamp. New runs require declared
manifests; format 2 and 3 checkpoints without plugins migrate with explicit
legacy provenance. They may continue, but exact replay returns
`legacy_replay_unavailable`. Legacy snapshots containing executable plugin
descriptors are rejected because their handler semantic identities cannot be
recovered safely.

Format 4 also permits additive application-defined domain record schemas,
ordered record stores, and boundary `record_changes`. These fields are omitted
when empty, preserving the serialized and hashed shape of earlier format-4
state. A record change carries its plugin/system owner, operation, previous and
current versioned values, visibility, and summary; its boundary emission points
back to the exact change index. Loading validates schemas and live references,
reverse-reconstructs the initial record store, reapplies lifecycle bundles in
canonical commit-stage order, rejects successor cycles, validates entity-bearing
evidence at both its proposal-visible and committed historical cuts, and compares
the result with the persisted store.
Format 2 and 3 inputs reject these fields rather than interpreting new lifecycle
semantics under a legacy identity. Format-4 snapshots with declared domain
record schemas also retain the canonical initial scenario and verify it against
the scenario artifact in the run manifest. Record history reconstructed in
reverse must equal that bound genesis, so a rehashed snapshot cannot relabel
created records as initial state. Record-free format-4 snapshots omit this
additive field and retain their prior serialized and hashed shape. A pristine,
registration-open declared snapshot can reconstruct and manifest-validate that
genesis before activating record schemas; execution-closed or migrated-legacy
snapshots cannot gain that capability without an explicit migration.

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
