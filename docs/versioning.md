# Versioning and Persistence

Canwu is pre-1.0. Format 6 is a deliberate clean break: the 0.6 runtime
writes and reads only its current contracts. There is no implicit loader or
runtime migration for format 2, 3, 4, or 5 data. Applications that need old
records must keep the old engine or run an explicit, application-owned export
outside the Canwu runtime.

## Current contract

The workspace version is `0.6.0`. A live `SimulationSnapshot` has:

- snapshot format `6`;
- commitment format `2`;
- state revision format `2`;
- admission cursor format `2`;
- exact replay revision format `2`;
- a declared `RunManifest`, declared run configuration, and canonical
  `initial_scenario`;
- a non-zero `authority_root_seed`, derived independently from the simulation
  random streams.

Typed loading and strict JSON loading reject any other engine or contract
version. Strict JSON loading also rejects unknown fields at every nested
object and rejects a wire value whose canonical re-encoding changes shape.

The runtime no longer contains `migration.rs` or `legacy_v4.rs`. An older
snapshot must not be silently relabeled as format 6, and a legacy replay
journal cannot be promoted to exact replay.

## Self-contained exact replay

`ReplayJournal` is the complete replay boundary. It carries the canonical
initial scenario, declared run identity, run configuration, plugin descriptors,
authority root, commands, attempts, ingress, boundaries, random draws, and
final revision/checkpoint commitments. `replay_from_journal(plugins, journal)`
does not accept a second scenario supplied by the caller; the scenario in the
journal is authoritative and is validated against the manifest.
`replay_from_journal_json` is the strict JSON counterpart and rejects unknown
fields recursively before replay begins.

Executable policy implementations are not replay inputs. Decisions, outcomes,
and evidence already admitted to the journal are replayed as records.

## Durable outbox

Boundary emissions are exposed through `Canwu::outbox_entries()`. Each entry
has a stable `delivery_id` derived from the run manifest, boundary, event, and
emission index. The host application must deliver entries at least once and
deduplicate by `delivery_id`. Exact replay regenerates the same outbox
identity; it does not re-send external effects.

In compact mode, `CompactedCanwu::outbox_entries()` returns entries from the
retained evidence tail. Once evidence is sealed, the caller owns the returned
`EvidenceJournalSegment` and must keep its boundary emissions with the host's
delivery/acknowledgement state; compaction does not acknowledge or deliver an
external effect on the host's behalf.

## Granularity boundary

The engine exposes the domain-neutral `SimulationGranularity` enum:

| Value | Meaning |
| --- | --- |
| `aggregate` | A coarse aggregate or population-scale state. |
| `group` | A bounded social, institutional, military, or organizational group. |
| `actor` | A person or other principal with its own knowledge and authority. |

These are engine simulation levels, not a fixed historical ontology. A host
game may map them to its own terms. In Celestial Mandate, for example,
`aggregate` can map to Population, `group` to Special Group, and `actor` to
Character. That mapping belongs in a CM reference integration or host adapter,
not in Canwu core.

## Reference integrations

Southern Ming and WWII are content and ruleset integrations for Celestial
Mandate. They are not Canwu adapters and are intentionally absent from this
repository. Canwu provides the generic state, authority, boundary, evidence,
granularity, persistence, replay, and outbox contracts that those integrations
consume.

## Source compatibility

The 0.6 change removes caller-supplied replay wrappers from the public facade.
Use the constructor APIs for new runs, `snapshot`/`checkpoint` for persistence,
`replay_from_journal` for exact replay, and `outbox_entries` for host delivery.
Downstream crates must update their code to these contracts; no deprecated
alias is retained before 1.0.
