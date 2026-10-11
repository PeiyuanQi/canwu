# Boundary Window

Status: implemented in 0.14.0. It replaces the day read of 0.13.3
(`SimulationView::boundaries_on_day` and `DayBoundaries`).

## Problem

A boundary system that carries work into a later boundary (a packet re-sent
to a later time, or a resolution that settles in a follow-on boundary at the
same instant) may have to keep a period of simulated time within a boundary
budget. To do that it must count the boundaries other sources open in that
period, not only its own. Retained boundary evidence cannot answer the
question, because a seal removes it, so the engine needs a replay-stable read
(downstream gap G-60).

0.13.3 answered it for one fixed window: the 1,440-minute day containing the
view time, with settled boundaries kept only for the latest boundary's day.
That window suits a game that budgets per day from midnight, but not one that
budgets per turn, per week, or per day starting at another hour, and it made
"day" the core's only unit of boundary history.

## Decision

The read takes a half-open window of simulated time:

```rust
view.boundaries_between(from, to) -> Result<BoundaryWindow, CanwuError>
```

`BoundaryWindow { from, to, settled, current, queued }` lists the boundaries
settled in the window, the boundary being settled when the reader is a
boundary system and its time is in the window, and each distinct due time
still queued in the window. Each `BoundarySummary` carries the boundary's id
(once it has one), time, cadences, and its ingress counted by class, kind, and
origin, without payloads. Hosts read the same ledger through
`Canwu::boundaries_between` and `CompactedCanwu::boundaries_between`.

**Retention is declared by the reader.** A boundary system declares how far
back before the boundary time it reads settled boundaries, as
`BoundarySystemContract::boundary_schedule_horizon`, the way it declares its
reads, writes, and random streams. The run keeps settled boundaries for the
longest horizon any registered system declares, at most
`MAX_BOUNDARY_SCHEDULE_HORIZON` (366 days), but a system may look back only as
far as its own horizon: a window that starts further back from the boundary
time than the reader's horizon returns `EvidenceUnavailable`, whatever other
plugins declare, so a plugin never depends on another plugin's horizon. Hosts
read back to the run's horizon. A non-zero horizon requires the
`canwu.core.boundary_schedule` read, and a run in which no system declares one
keeps only the boundaries at the latest boundary's instant.

A host-chosen retention (a run-configuration value) was considered and
rejected: the code that reads the history knows how far back it needs, and a
host could otherwise configure a window too short for the systems it loads,
which would surface only as a runtime error inside a boundary. Because the
horizon lives in the plugin descriptor, it is part of the run's persisted
identity, so a restored or replayed run keeps exactly the same ledger.

## Invariants

- The settled ledger is a derived index of the boundary and ingress journals
  and the persisted plugin descriptors. It is updated at boundary commit,
  rolled back with a failed boundary, kept across a seal, and rebuilt from the
  full journal on restore and by exact replay. Snapshot, commitment,
  checkpoint, and paged-checkpoint formats are unchanged.
- Only phased boundary systems may declare the read. It names other plugins'
  queued packet types, so plugin commands and synchronous event-driven systems
  are refused at registration, as they are for `canwu.core.ingress`. Hosts must
  not pass the result to player, agent, observer, or remote clients.
- The core keeps no calendar. `SimTime::day` remains a plain count of
  1,440-minute days from time zero for applications that want day windows;
  any other period is the application's to compute.
- The queued list is a lower bound: work a boundary proposes is queued when it
  commits, and work not yet queued or a boundary the host settles directly is
  not listed. Legacy scheduled actions are counted only for queued boundaries.

## Compatibility

0.14.0 is a pre-1.0 minor release: `BoundarySystemContract` gains a public
field, and `boundaries_on_day` and `DayBoundaries` are removed. A system that
read the day of 0.13.3 declares `boundary_schedule_horizon` and asks for the
day's window, for example
`boundaries_between(SimTime::from_minutes(day * SimTime::MINUTES_PER_DAY), ...)`.
