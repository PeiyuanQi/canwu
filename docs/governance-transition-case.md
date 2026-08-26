# Governance transition case

This case demonstrates how an application can model a central-government
relief order with Canwu's public contracts. The historical labels are example
content, not engine vocabulary: a host may replace the central relief office,
treasury, county, and actors with any institutions in its own period model.

The runnable source is
[`governance_transition.rs`](../crates/api/canwu-api/examples/governance_transition.rs).
It uses three simulation plugins:

- a central plugin that owns the order manifest and the read-only audit;
- a treasury plugin that owns the treasury disposition;
- a county plugin that owns the county disposition.

## Transition protocol

The case follows one deterministic transaction protocol:

1. A canonical ingress asks the central plugin to issue order
   `relief-order-1646`.
2. The central `publish-order` boundary system creates a manifest containing
   the expected owner, system, version, disposition, and post-state hash for
   both participants. It schedules one zero-delay plugin ingress for the
   treasury and one for the county. Canwu admits those generated inputs at the
   next boundary.
3. The treasury and county `prepare-*` systems consume only their own admitted
   ingress, validate the manifest, and stage their owner-scoped domain record
   with `SameBoundary` visibility.
4. The central `audit-order` system is read-only. It checks both committed
   records against the manifest and fails the transaction if either participant
   is missing or mismatched.
5. Snapshot restore and exact replay reproduce the same final state without
   rerunning an external choice.

The important property is not the number of offices. It is the explicit
ownership and commit contract: each institution owns its state, the central
manifest states what must be produced, and one transaction either commits the
complete transition or rolls it back.

## Domain mapping

The example deliberately keeps period-specific semantics outside Canwu:

| Historical concept | Example representation | Engine contract |
| --- | --- | --- |
| central relief office | `case-relief-central` simulation plugin | plugin-owned domain records and canonical ingress |
| treasury | `case-relief-treasury` simulation plugin | owner-scoped domain record and event-driven boundary system |
| county grain office | `case-relief-county` simulation plugin | owner-scoped domain record and event-driven boundary system |
| imperial or ministerial order | `ReliefOrder` manifest | typed domain record with expected post-state hashes |
| local execution report | `ReliefAction` record | typed entity record with version and owner |
| officials, households, and social groups | host-defined actors and domain records | actor-relative reads, authority, knowledge, and decisions supplied by the host |

Canwu does not assume that a bureaucracy is a tree, that one person makes the
final decision, or that every office needs an independently simulated person.
Those are scenario choices. A host can model a single controller, delegated
authority, competing offices, or a larger hierarchy while using the same
ingress, boundary, ownership, visibility, persistence, and replay contracts.

## Run it

```shell
cargo run -p canwu-api --example governance_transition
```

The example prints the order ID, both 600-unit dispositions, and an
`exact_replay=ok` confirmation. It is intentionally a public-API example, so
it can be copied into a host application without depending on Canwu runtime
internals.
