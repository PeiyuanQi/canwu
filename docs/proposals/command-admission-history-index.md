# Command Admission History Index

Status: implemented in 0.16.0.

Admitting a command did work in proportion to the run's unsealed command
history: four admission checks rescanned the retained commands, command
attempts, or ingress on every call. `canwu-sim` now answers them from a
derived index of the retained command history, and the three journals it is
derived from can change only through the methods that keep it in step.

The durable test is
`crates/runtime/canwu-sim/src/runtime/tests/ingress.rs`:
`the_command_history_index_answers_as_the_retained_journals_do`. Besides it,
every index answer checks itself against the scan it replaces in a debug
build, so every test that queues or admits a command, seals, restores, or
rolls back exercises the index.

## Problem

The world-scale fiscal profile measured that engine admission grows with run
history: about 0.4 ms per command in a fresh 50,000-scope world, much more
after a burst of 50,000 assessments. The growth came from these checks, all
linear in the retained (unsealed) journals:

- `runtime_has_unqueued_command_history` (`validation.rs`), asked by
  `ensure_canonical_ingress_can_start` before every canonical ingress append:
  once when the host queues a command, and once inside admission for each
  plugin packet a command's directives queue (a fiscal batch command queues
  its settlement packet). Each call collected every command request in the
  retained ingress into a set, then scanned every retained command and
  attempt.
- `ensure_command_ingress_family` scanned every retained command for a
  legacy-direct one, once per command.
- `cached_command_outcome` searched the retained attempts for the request,
  once per command.
- `command_request_id_is_in_use`, asked when the host queues a command or a
  decision that carries one, and
  `command_request_id_is_unique_for_admitted_decision` scanned the retained
  attempts and ingress.

The retained journals shrink only at a seal, so a host that seals rarely paid
more for every command as the run went on.

## Decision

- `RuntimeEvidence` holds the retained commands, command attempts, and
  ingress as `RetainedJournal<T>`: every module of the runtime reads one as a
  slice, but only the command-history methods in `state.rs` change it:
  `push_ingress`, `push_command`, and `push_command_attempt` (appends),
  `truncate_command_history` (the rollback of a failed command, rejection,
  ingress append, or boundary transaction), `seal_command_history` (a seal),
  and `restore_command_history` (the snapshot restore, which the
  checkpoint-and-journal, segment, and paged restores reach, and the
  continuation-checkpoint restore). A direct push, truncation, or replacement
  outside those methods does not compile.
- Those methods keep `CommandHistoryIndex` in step: for each command request,
  the number of retained command and decision ingress records that carry it
  and the position of its first retained attempt; the position of the first
  retained legacy-direct command; and the position of the first retained
  attempt whose request is queued neither as retained ingress nor as ingress
  a seal archived.
- The four checks read the index: a flag, or a lookup in a map keyed by
  request. In a debug build each answer is compared with the original scan
  (`debug_assert_eq!`); a release build does not scan.

## Invariants

- **Derived.** The index is a function of the three retained journals and the
  archived request tables (`archived_ingress_requests`,
  `archived_decision_command_requests`). It is never part of a snapshot, a
  checkpoint, a continuation checkpoint, a journal segment, a hash, or a
  commitment. A fork copies it with the evidence, and a failed transaction
  restores it with the journals it truncates.
- **Exact.** Every answer equals the scan it replaces, in every state the
  methods can reach, not only in those the canonical flows reach. An append
  that queues the request of an unqueued attempt, or a truncation that
  removes the ingress of a retained attempt, recomputes the first unqueued
  attempt. Canonical ingress is refused while such an attempt is retained,
  and a transaction truncates the attempts it appended with its ingress, so
  neither happens in a canonical run, but the index does not rely on it.
- **Replay-neutral.** No check changes its answer, so no admission, refusal,
  error, event, record, or hash changes, and snapshots are byte-identical.
- **Seal-independent.** A seal extends the archived request tables first,
  then drops the sealed prefixes and rebuilds the index from what it retains,
  so an attempt whose ingress the seal archived stays queued, as the scan
  says. A restore rebuilds the index after it has placed the archived tables.
  The archived tables change nowhere else.
- The index uses `std` maps, read by key; no `im` ordered map is iterated.

## Measurements

The harness was a copy of the `fiscal-world-profile` binary of the
world-scale fiscal change, on its `world` preset (200 partitions, 50,000
scopes, 4,200 institutions, one fiscal batch command per institution, so
4,200 commands per admission boundary), built in release mode against this
release with and without the index. It times the host queuing the commands
and the boundary that admits them, on a fork, and reports the engine's share
of the boundary (the boundary minus the fiscal handlers, from the boundary
profile). "After one burst" admits on a run that has already assessed every
scope: 4,200 commands, 121,353 events, and 65 boundaries retained. Each build
ran twice, in the order before, after, after, before, on one machine that was
also running other builds; the ranges are the two runs.

| Admission boundary | Before: engine ms per command | After |
| --- | ---: | ---: |
| Close every book, fresh world | 0.386 to 0.412 | 0.287 to 0.290 |
| Assess every scope, fresh world | 0.412 to 0.444 | 0.312 |
| Close every book after one burst | 0.707 to 0.713 | 0.310 to 0.339 |

After the burst, the whole admission boundary of the close went from 3,316 to
3,340 ms down to 1,613 to 1,764 ms, and the host's queuing of its 4,200
commands from 1,569 to 1,789 ms down to 414 to 456 ms (on the fresh world,
356 to 372 ms down to 260 to 264 ms). An instrumented build put the
unqueued-history check alone at 50 µs a call on the fresh world and 226 µs a
call after the burst, with 8,400 calls for 4,200 commands (half of them when
the host queues); with the index the 8,400 calls take 0.2 ms.

## What can still fail a boundary

Nothing new. The index answers the questions the scans answered, with the
same answers, and adds no error, refusal, or boundary failure. An index that
diverged from its journals would be a runtime bug; the debug cross-checks
report it in any test that reaches it, and the journals can change only
through the methods that maintain it.

## Follow-up: the per-command transaction checkpoint

With the index, the engine's share of admission is about 0.3 ms per command
on the world preset, and it no longer grows with history. In the
instrumented build about half of it was
`CommandTransactionCheckpoint::capture` (`transactions.rs`), which clones the
people, letter, army, plugin component, and scheduled action maps for every
command so that a failed command can be rolled back: 626 to 638 ms of
1,226 ms for 4,200 commands, about 0.15 ms per command. It grows with the
world's size, not its history. Persistent maps for those collections, as the
archived tables already use, or a capture of only the collections a command
can change, would remove most of it.
