# Pending Ingress Read

Status: implemented in 0.19.0. It closes downstream gap G-62: after a seal or
a continuation restore a host cannot list the ingress it queued before and
that is still pending. It changes `canwu-sim` and `canwu-api` only. Same
version only: no compatibility work with 0.18.0 bytes or hashes.

## Problem

A host reads what it has queued from `ingress_log`, which on `CompactedCanwu`
returns only the retained tail. A seal moves the whole ingress journal into
the sealed segment and clears the retained journal, keeping the records still
pending as pinned evidence (the continuation closure); a continuation restore
starts with an empty retained journal and puts the pending records in the
same pinned map. Neither is reachable from the host: `boundaries_between`
counts queued work per kind without the records, and `SimulationView::ingress`
reads one record only inside a boundary system. So after any seal, not only
after a restore, a host that shows its own pending commands ("sent, not yet
applied") or deduplicates its adapters by payload loses them until they apply.
Nothing is lost from the run.

## Decisions

From the owner's decisions of 2026-10-19 (`opus-progress.md` §6):

- **D1.** `pending_ingress()` on the `CanwuRun` trait, so both `Canwu` and
  `CompactedCanwu` have it.
- **D2.** It iterates the queued `IngressRecord`s in queue order, unfiltered.
- **D3.** It is for the trusted host only: never on `SimulationView` or
  `CanwuViewer`.
- **D4.** No `ingress_record(id)`.
- **D5.** It covers a sealed run as well as a continuation-restored one.

## Implementer choices

- **Where.** `Simulation::pending_ingress` (the implementation) and
  `CompactedSimulation::pending_ingress` in `canwu-sim`; the inherent methods
  `Canwu::pending_ingress` and `CompactedCanwu::pending_ingress` and the trait
  method `CanwuRun::pending_ingress` in `canwu-api`.
- **Return type.** The inherent methods return `impl Iterator<Item =
  &IngressRecord>`; the trait returns `Box<dyn Iterator<Item = &IngressRecord>
  + '_>`, as `CanwuRun::domain_records` does, so the trait stays usable as
  `dyn CanwuRun`. An iterator, not a `Vec`, because the engine puts no cap on
  the queue.
- **What it reads.** The scheduler's queue (`RuntimeScheduler::pending_ingress`,
  an `im::OrdSet` of `IngressQueueKey`), walked forward only, each key
  resolved through `RuntimeEvidence::ingress_record`, which returns the
  retained record or the pinned one. Every queued key resolves: the retained
  journal holds the record, or a seal or a continuation restore pinned it (a
  continuation checkpoint that does not pin a pending record is refused). A
  `debug_assert!` checks it in debug builds; a release build skips a key
  that does not resolve rather than fail a read.
- **Queue order** is the admission order: due time, ingress class
  (`Command`, `Communication`, `Acknowledgement`, `Information`, `Decision`,
  `ScheduledSystem`), priority from the highest, issue time, and ID. A record
  still queued at or before `now` (zero-delay work a boundary generated for
  the next boundary at the same instant) is listed.
- **What leaves.** A record leaves when a boundary admits it or a
  cancellation withdraws it. A `PluginCancellation` record is a journal entry,
  never queued, so it is never listed.
- **Trusted-host wording.** The rustdoc of every form says that it lists every
  source with its payload (host commands and packets, decision ingress, other
  plugins' internal packets, boundary-generated work, calendar and
  maintenance requests), must not be shown to an actor as it is, and that a
  host selects its own items, for example those with no `cause`, which the
  boundary ledger counts as `BoundaryIngressOrigin::Host`.
- **The restricted viewer.** The public API delta check
  (`crates/runtime/canwu-sim/tests/api-delta`) gains the feature
  `viewer-ingress-leak`, which calls `pending_ingress` on a `CanwuViewer` and
  must fail to compile, beside the existing snapshot, journal, domain-record,
  and boundary checks.
- `CompactedCanwu::ingress_log`'s rustdoc points to `pending_ingress`.

## Persistence and formats

None. It is a pure `&self` read of current state: no persisted field,
journal, hash, replay input, or format changes, and no `gated!` change (the
continuation gate shadows `&mut self` methods only).

## What can still fail a boundary

Nothing: the read is outside every boundary and cannot fail.

## Public API delta

- `canwu-sim`: `Simulation::pending_ingress`,
  `CompactedSimulation::pending_ingress` (additive).
- `canwu-api`: `Canwu::pending_ingress`, `CompactedCanwu::pending_ingress`
  (additive), and the required trait method `CanwuRun::pending_ingress`. Only
  `canwu-api` implements `CanwuRun` in the workspace; an implementation
  outside it would need the method, as for any 0.x minor release.

## Durable tests

`pending_ingress_lists_the_same_queue_in_every_run` (`canwu-api`,
`tests/continuation_checkpoint.rs`), on the lockstep fixture, which keeps
command-caused, boundary-generated, cross-plugin, and cancelled packets in
flight at every boundary: an unsealed run, a run that seals every five
boundaries, and a run restored from a continuation checkpoint on day 7 (and
sealing every five boundaries after) take the same inputs for 20 days. Every
day, through `&dyn CanwuRun`, all list the same records in the same order;
the list is in queue order; every record the unsealed run lists is in its
`ingress_log`, and a sealed or restored run lists records its own
`ingress_log` no longer holds (asserted to happen); nothing the day's
boundary admitted is listed, and records listed the day before are seen to be
admitted and leave.

Checked inline (not committed) to fail: listing the retained journal in
journal order (the order assertion), listing the queued records in ID order
(the order assertion), and listing only queued records the retained journal
holds, in queue order, which is the 0.18.0 gap (the cross-run comparison, at
the first seal). The api-delta check passes with the new feature (the viewer
call does not compile).

## Documentation delta

For the release documentation pass (canonical page and both website
languages each):

- `docs/architecture.md`, the `CompactedCanwu` paragraph: after "the
  retained-tail journal reads" add "and `pending_ingress`, every record still
  queued, the pinned ones included". After the sealing paragraph's list of
  pinned evidence add: "A host lists the queued records, pinned ones
  included, in queue order with `pending_ingress` (on `Canwu`,
  `CompactedCanwu`, and `CanwuRun`); it is a trusted-host read of every
  source and is never on `SimulationView` or `CanwuViewer`."
- `website/src/content/docs/{,en/}developer/persistence.mdx`, "What a
  continued run can do": add "list the ingress still queued, including what
  was queued before the save, with `pending_ingress` (trusted host only:
  select your own items, for example those with no `cause`, before showing
  anything to a player)".
- `docs/versioning.md`, the release entry: "`pending_ingress()` on `Canwu`,
  `CompactedCanwu`, and `CanwuRun` lists every queued `IngressRecord` in
  queue order, including the records a seal or a continuation restore
  pinned, so a host can list its pending commands after a load. A
  trusted-host read: it holds every source's payloads unfiltered.
  `CanwuRun` gains a required method."
- `docs/terminology.md` and both website term pages: the `run handle
  contract` row (运行句柄契约) adds "the queued ingress" to its list of reads;
  the candidate below if the owner wants a row.
- The engine-user skill's host notes (`agent-interface`, persistence or
  continuation section): one sentence, "After a seal or a continuation
  restore, list your still-pending commands with `pending_ingress()` and keep
  only your own (`cause == None` or your request IDs)".

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| pending ingress | `pending_ingress`, `CanwuRun::pending_ingress` | 待准入输入 | Ingress records queued and not yet admitted by a boundary or withdrawn, in the order boundaries admit them, whether the run retains them or a seal or continuation restore pinned them. Prose may say 排队的输入 as the events page does. |

## Known limits

- No lookup by ID (D4); a host that needs one record filters the iterator,
  `O(queue)`.
- Walking the queue costs one lookup per record in the retained journal or
  the pinned map; a host with a very large queue should stop iterating once
  it has what it needs.
- No filter by source: the host selects its items itself.

## Review record


The code was reviewed with the movement package by one independent reviewer,
and the fix once in a delta review; neither found a blocking issue. The one
finding on this read was optional and accepted: `pending_ingress` dropped a
queued key that did not resolve without any signal, so a `debug_assert!` now
checks that every queued key resolves (release behaviour unchanged). The
reviewer confirmed that the assertion cannot fire in a legitimate state: the
queue and the journals change together, a failed boundary rolls both back,
and a seal or a continuation restore refuses rather than drop a pending
record.
