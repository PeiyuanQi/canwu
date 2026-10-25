# Military Engagement Revisions: Reports, Durations, Siege Sides, Crossing Defenders, and Timed Steps

Status: implemented in 0.21.0. Six changes to `canwu-military` that close
downstream gaps G-85 to G-90, found when the downstream moved its sieges,
fights, and crossings onto the 0.20.0 engagement engine. They extend
[the engagement engine](military-engagements.md) and change nothing else:
`canwu-sim`, `canwu-api`, and the reference crates' behaviour are untouched
(the reference starter and content only build the new fields). Same version
only: no compatibility work with 0.20.0 records, hashes, or descriptors.

"Today" means 0.20.0 as published.

## Contents

1. [The report queue (G-85)](#the-report-queue-g-85)
2. [A fight shorter than its kind (G-86)](#a-fight-shorter-than-its-kind-g-86)
3. [Joining a siege (G-87)](#joining-a-siege-g-87)
4. [Revising a crossing's defenders (G-88)](#revising-a-crossings-defenders-g-88)
5. [Releasing forces from a siege (G-89)](#releasing-forces-from-a-siege-g-89)
6. [The timed step (G-90)](#the-timed-step-g-90)
7. [Public API delta](#public-api-delta)
8. [Persistence and replay](#persistence-and-replay)
9. [Refusals and what can still fail a boundary](#refusals-and-what-can-still-fail-a-boundary)
10. [Durable tests](#durable-tests)
11. [Known limits](#known-limits)
12. [Upgrading from 0.20.0](#upgrading-from-0200)
13. [Documentation delta](#documentation-delta)
14. [Terminology candidates](#terminology-candidates)

## The report queue (G-85)

### Problem

`settle.rs` `queue_report` pushes a `ReportItemV1` onto
`CombatRegistryV1::report_queue` for every engagement, siege, and crossing
that steps or closes, and `report.rs` `publish_engagement_reports` (phase 13)
drains the queue and rewrites the registry, whatever the subject's holders.
An item whose sides name no holder publishes nothing and is dropped. So a
host that names no report holder (it publishes its own reports from the
records) pays two registry writes on every stepping boundary: the queue
filled in phase 7 and emptied in phase 13 (downstream: about 0.24 KB a
write, two a siege day).

### Invariant

A stepping boundary writes the registry only for what changed in it; a
subject that no holder can hear queues no report.

### Design

`queue_report` takes the subject's holders and queues nothing when neither
side lists one. Holders are fixed when a fight, siege, or crossing opens (a
fight a siege or crossing opens inherits theirs) and no command revises
them, so a subject either always or never queues. A host without holders
now writes the registry once in a stepping boundary when its own fields move
(`earliest_due`, the open sets, the retire queue, `steps_to_empty`,
`next_sequence`) and not at all otherwise; phase 13 finds the queue empty
and proposes nothing. A subject that lists holders behaves as in 0.20.0,
including the item dropped in phase 13 when every holder it lists is
refused at publish (a holder who died).

Rejected: moving the queue out of the registry into a record of its own.
It adds a record kind and still costs two writes a stepping boundary for a
host with holders, and it does not help a host without them more than the
rule above.

## A fight shorter than its kind (G-86)

### Problem

`CombatRulebookV1::kinds` holds one `KindRuleV1` per `EngagementKind` with
one `max_minutes`, and `settle.rs` closes an engagement `Stalled` once
`minutes_between(opened_at, at) >= max_minutes`. So `Assault` has one
duration: it cannot be both a multi-day wall assault and the one-day gate
assault a host opens against a closed gate (the downstream runs its gate
assault as a `Field` fight meanwhile).

### Invariant

A fight's duration is fixed when it opens and never exceeds its kind's rule.

### Design

- `OpenEngagementV1.max_minutes: Option<u32>`: `None` takes the kind rule's
  `max_minutes`; a value must be from 1 to the kind rule's, else the command
  is a recorded `ValueOutOfRange` rejection.
- `EngagementStateV1.max_minutes: u32` holds the fight's duration as
  resolved at the opening (the host's value or the kind's), and the step
  reads it instead of the kind rule. A fight the plugin opens (an assault, a
  sortie, a ferry or bridgehead fight) takes its kind's.
- Activation refuses an active engagement whose `max_minutes` is 0 or above
  its kind's.

Rejected: a second assault kind. A kind adds an enum variant, a rule row, and
posture cells to every host's rulebook and a row to the siege's outcome
table for what is one duration; the field lets a host shorten any kind's
fight (a one-day relief, a skirmish) the same way.

## Joining a siege (G-87)

### Problem

`command.rs` `invest_site` requires every besieger and defender to be free
at the site, and a besieged force is not free, so a second investment of an
invested place is refused; `ReviseSiegeV1` revises the supply, profile, work,
fleet, gate, storm, and sortie, never the sides; and `JoinEngagement` has no
siege counterpart. A siege cannot take a second column.

### Invariant

A force is on a siege's side list exactly while its `battle.siege_role`
names that siege; every force on a side of the siege's assault or sortie is
on the matching side of the siege (the besiegers are an assault's attackers
and a sortie's defenders); a siege's two sides have different owners.

### Design

`MilitaryCommandV1::JoinSiege { siege, expected_host_revision, side, forces }`,
as `JoinEngagement`: `Attacker` joins the besiegers, `Defender` the
defenders. Each joining force must be registered, active, free, and at the
siege's site, controlled by the issuer (or the issuer trusted), of the side's
owner (of one owner among themselves when the side is empty), not of the
opposite side's owner (so a release that empties the defenders cannot let
the besiegers' own forces in as defenders; `InvestSite` now refuses
besiegers and defenders of one owner as well), and not in the siege
already; the side stays within `limits.forces_per_side`. A joining force
takes the siege role and the siege's `host_revision` advances. The siege
reads the joined forces from its next siege day that runs (in the same
boundary when one is due there): investment, the guns, the fights it opens.

While the siege's assault or sortie is in progress (`siege.engagement` is
some) the joining forces also enter that fight on the matching side, as
`JoinEngagement` would (`join_side`: recovery credited, the side's posture,
cohesion and shock recomposed, `opened_men` raised), within the fight side's
`forces_per_side` too, and the fight's `host_revision` advances, since a
host command changed it. Without this, a column that arrives during a
multi-day wall assault could not join for the whole assault: a force in a
siege is not free, so `JoinEngagement` refuses it, and a force in a fight is
not free, so `JoinSiege` would refuse it. A relief, which is not the siege's
fight, is not joined; a joining besieger stands in the siege beside it.

## Revising a crossing's defenders (G-88)

### Problem

`ReviseCrossing { crossing, expected_host_revision, interceptor, night }`
revises the interceptor and the night, never the defenders, so a crossing's
defending bank is the one named when `OrderCrossing` was read: a force that
reaches the far bank afterwards does not resist the landing.

### Design

`ReviseCrossing` gains `defenders: Option<Vec<ForceId>>`, which replaces the
list and is checked as `OrderCrossing` checks its own (one helper now serves
both): within `limits.forces_per_side`, no force named twice, not the
crossing force, each registered, active, and at the far bank. Authority: a
trusted issuer or the crossing force's controller (who named the defenders
on the order) may name any list; otherwise the issuer must control a force
on the old or new list (a bank's own commander names the force that arrived
there) and every force the revision removes that is still registered, so
one bank commander cannot drop another's defenders. The
next landed wave's bridgehead fight takes the free defenders at the far bank
from the revised list; a fight already in progress keeps its sides (a free
defender may `JoinEngagement` it).

## Releasing forces from a siege (G-89)

### Problem

`PlaceForce` refuses a force that is not `is_free()`, so a besieged defender
cannot leave its siege: a garrison force that marches off still stands in the
siege, while a besieger that leaves can only lift it (`LiftSiege`).

### Invariant

As for G-87, and: an open siege has at least one besieger.

### Design

`ReviseSiegeV1.release: Vec<ForceId>` releases forces from either side: each
leaves its side's list and its siege role and is free to be placed, in the
same boundary (`PlaceForce` after the release in one admission order
composes, as the commands apply against the staged state). Each released
force must be in the siege, controlled by the issuer (or the issuer trusted),
and in no fight. A release that would leave no besieger is refused:
`LiftSiege` ends a siege. A release that leaves no defender is allowed: the
next siege day finds no defender at the site and the siege falls `Abandoned`
(0.20.0's rule for a siege whose defenders are gone), and the released
forces are not dispersed (they are no longer its defenders). The release is
applied after the revision's authority checks and before its sortie check,
so a sortie is checked against the defenders that remain.

Rejected: `PlaceForce` admitting a defender, which leaves the siege's
defenders. It would change the siege record from a force command, moving
the siege's `host_revision` under a command that does not name it, and would
give a besieger no way out short of `LiftSiege`. The release keeps every
change to a siege's sides on a siege command that names the siege's
`expected_host_revision`, and covers both sides (a second column that joined
with `JoinSiege` can leave again).

## The timed step (G-90)

### Problem

`Step { up_to }` is a trusted command: "steps the work due by `up_to` in the
next boundary at once, after the apply packets admitted with it". A command
is admitted in a boundary and its packet in the next cut at the same
instant, so a host whose day's orders are known only after its daily
boundary (the downstream's day-open batch) pays a command boundary and the
cut that applies it on every day with due work; on a siege, fight, or
crossing day it settles 3–4 boundaries where a quiet day settles 1.
`with_step_cadence(Some(cadence))` instead schedules a step packet due at
once in each boundary of that cadence with work due, one extra boundary at
the same instant.

### Invariant

The step packet's semantics are unchanged: a boundary that admits one
applies the apply packets it admitted, in admission order, then steps the
work due by its own time, and its draws cite the first step packet it
admitted.

### Design

- A public plugin ingress, `military_timed_step_v1`
  (`MILITARY_TIMED_STEP_INGRESS`, class `ScheduledSystem`, payload `Null`),
  that the host queues on its run handle due at a named time:
  `canwu.enqueue_plugin_ingress(timed_step_ingress(due_at))`. The boundary at
  that instant admits it with the host's other ingress (the daily calendar
  boundary it settles anyway) and the settle system treats it as a step
  packet with `up_to` at the boundary's time. A host that steps once a day
  queues the next day's timed step after each daily boundary and adds no
  boundary of its own.
- The wake system schedules nothing in a boundary that admitted a step
  packet of either kind (its contract now reads `core_ingress`), so a host
  may keep the default `Daily` wake cadence and queue timed steps without
  paying the wake's extra boundary.
- Its draws cite the timed step's ingress record, which exists in every run
  (live, sealed, restored, replayed) like the internal packet; exact replay
  re-queues it from the replay journal. It carries no issuer: only a run
  handle (`Canwu`, `CompactedCanwu`, `CanwuRun`) can queue plugin ingress,
  never a `CanwuViewer`, so its authority is the trusted host's, as the
  `Step` command's trusted issuer is. The host may withdraw it before it is
  due with `cancel_plugin_ingress`.
- What it does not change: every military command is admitted in one
  boundary and applied by its apply packet in a later cut. The engine takes
  a boundary's due ingress before it admits any command, and a command's
  `EnqueuePluginIngress` is eligible only after the current boundary
  (`canwu-sim` `settlement.rs`, `append_ingress(.., after_current_boundary:
  true)`). So the commands the timed step's boundary admits, whether the
  host queued them as ingress due at the same instant or proposes them
  during that boundary (from its own `Daily` system, say), apply in the next
  cut, after the step. The timed step removes the step's own boundaries,
  not a command's. To step after a day's orders, the host either sends the
  `Step` command with them (their apply packets and its step packet land in
  the cut the orders cost anyway, applies first) or has them admitted before
  the timed step's instant; otherwise they step from the next timed step.

Rejected: a delayed `Step { up_to, due_at }` command. The command still
needs a boundary to be admitted in, so a day with no other order would pay
it; and letting the settle system apply commands admitted in its own
boundary (skipping the apply packet) would change the admit-then-apply
contract every command relies on for a gap that is about the step.

## Public API delta

- New: `MilitaryCommandV1::JoinSiege { siege, expected_host_revision, side,
  forces }`; `timed_step_ingress(due_at) -> PluginIngressRequest`;
  `MILITARY_TIMED_STEP_INGRESS`.
- Changed types (code that builds them with a struct literal adds the
  field): `OpenEngagementV1.max_minutes: Option<u32>`,
  `EngagementStateV1.max_minutes: u32`, `ReviseSiegeV1.release:
  Vec<ForceId>`, `MilitaryCommandV1::ReviseCrossing.defenders:
  Option<Vec<ForceId>>`. `SCHEMA_VERSION` is 2.
- Behaviour: a subject whose sides name no holder queues no report; the wake
  system skips a boundary that admitted a step packet; `JoinSiege` enters the
  siege's assault or sortie in progress; `InvestSite` refuses besiegers and
  defenders of one owner.
- `canwu-api` is unchanged: the extension's public API is its crate root.

## Persistence and replay

| Item | What changed |
| --- | --- |
| Every plugin payload | `schema_version` 2 (`SCHEMA_VERSION`), because the engagement record changed shape; the plugin keeps one schema version for all its payloads, and a 0.20.0 record does not load |
| `canwu.military:engagement` | `max_minutes: u32`; activation refuses an active engagement whose `max_minutes` is 0 or above its kind rule's, or whose kind has no rule (0.20.0 stepped such a record with an unbounded duration; unreachable, since a valid rulebook has a rule for every kind) |
| `canwu.military:siege`, `canwu.military:crossing` | shapes unchanged; activation now also checks the side lists and the crossing's defenders against `forces_per_side`, since commands grow them |
| `canwu.military:registry` | shape unchanged; the report queue holds items only for subjects with a holder |
| Command payloads in the journals (`military_command_v1`, `military_apply_v1`) | `OpenEngagementV1.max_minutes`, `ReviseSiegeV1.release`, `ReviseCrossing.defenders`, and the `JoinSiege` variant |
| Plugin descriptor | version `0.21.0` (semantic hash of `canwu-military plugin semantic hash 0.21.0: engagements <resolver>@<version>`); the public ingress `military_timed_step_v1`; the wake system's reads gain `core_ingress` |
| Knowledge schemas | unchanged (`engagement_report` and `engagement_outcome` v1, same bodies and hashes) |

The timed step is an ordinary host ingress record: the snapshot, the
journals, the replay journal, and a continuation checkpoint's pinned ingress
carry it while it is pending, and exact replay re-queues it. A fork with a
different timed step diverges at its first different boundary. No boundary
decision reads retained evidence or a host record version; the new checks
read the current versions of the plugin's own records and the
`expected_host_revision` a command names.

## Refusals and what can still fail a boundary

Recorded rejections (`canwu.military.command_rejected.v1`, the boundary
stands), added to 0.20.0's: a `max_minutes` of 0 or above the kind rule's
(`ValueOutOfRange`); a joining force that is not free at the site, already
in the siege, of another owner than its side or of the opposite side's
owner, or past the siege's or its open fight's side limit, or a join of a
force the issuer does not control (`InvalidAuthority`); an `InvestSite`
whose besiegers and defenders have one owner; a release of a force not in
the siege, in a fight, or not the issuer's (`InvalidAuthority`), or a
release that would leave no besieger; a revised defender that is not at the
far bank, is the crossing force, or passes the side limit, or a revision by
an issuer who controls neither the crossing force nor a defender, or who
would remove a defender it does not control (`InvalidAuthority`). A force
named twice in a join, a release, or a
revised defender list is refused at admission (`InvalidPayload`, a rejected
command attempt).

The timed step cannot be refused in a boundary: the engine refuses a
non-null payload when the host enqueues it (`InvalidPayload` from
`enqueue_plugin_ingress`), and a timed step with no rulebook installed or no
work due steps nothing.

What can still fail a boundary is unchanged from 0.20.0: a plugin-owned
record that fails to decode, validate, or encode; a record store version
overflow; a host resolver that panics; an outcome-signal target no plugin
registered.

## Durable tests

`crates/extensions/canwu-military/tests/revisions_and_timed_steps.rs`:

- `a_host_stepping_once_a_day_settles_one_boundary_a_day` (G-90, G-85): a
  field fight and a siege with no report holder, a daily calendar boundary
  and a timed step queued the day before each day; every day settles one
  boundary at its instant, the work of the day steps in it, the registry is
  written at most once and its queue stays empty; the default `Daily` wake
  cadence gives the same boundaries and the same records; a command queued
  as ingress for the timed step's instant is admitted with it and applies in
  the next cut, after the step (the siege day seals four of the old eight
  accesses, then the profile changes); exact replay equals the run.
- `a_fight_opened_for_a_day_stalls_after_it` (G-86): a one-day assault
  closes `Stalled` after one step while an assault on the kind's five days
  runs on; 0 and six days are recorded `ValueOutOfRange` rejections.
- `a_siege_takes_a_second_column_and_releases_its_defenders` (G-87, G-89):
  a second column placed and joined the day of the first siege day seals
  every access with the first; a force elsewhere cannot join, nor can an
  issuer join a force it does not control; a defender is released and placed
  in one boundary; releasing every besieger, or a force the issuer does not
  control, is refused; a defender that joins during the garrison's sortie
  enters the siege and the sortie's attacking side, and a force in the sortie
  cannot be released; releasing the last defenders makes the place fall
  `Abandoned` with nobody dispersed and every force free.
- `a_force_that_reaches_the_far_bank_resists_the_landing` (G-88): a force
  placed at the far bank after the order is named by its own controller, a
  second controller adds its own force but cannot drop the first's, and both
  fight the first landed wave's bridgehead fight; a defender elsewhere is
  refused.

Checked inline (not committed) to fail: queueing a report for a subject
without holders (the registry is written twice a day) and a wake system that
schedules beside an admitted timed step (two boundaries a day); a revised
defender list without the removal check lets the second controller drop the
first's force. The 0.20.0
lifecycle and refusal tests pass with the new fields set to their defaults.

## Known limits

- **A day's orders still cost their boundaries.** A command admitted by the
  timed step's boundary (queued as ingress for its instant or proposed
  during it) applies in the next cut, after the step, so a host whose orders
  depend on its daily boundary's own results still pays the command
  boundary and the cut that applies them on a day it sends orders; the
  timed step removes only the step's boundaries. Those orders step from the
  next timed step, a day after a `Step` sent with them would.
- **Report holders are fixed at the opening.** A fight, siege, or crossing
  opened without report holders can never gain them (no command revises a
  subject's holders), so it never queues a report.
- **Joined and released forces** change the siege from its next siege day
  (investment is not recomputed at once); a besieger that joins during a
  relief stands beside it, not in it.
- **A fight's duration is fixed at its opening**, and a fight the plugin
  opens takes its kind's; no command revises it.
- **Revised crossing defenders** do not enter a fight already in progress.
- The 0.20.0 military limits (E-51) stand.

## Upgrading from 0.20.0

Code that builds `OpenEngagementV1` adds `max_minutes: None` (the kind's
duration, as before), `ReviseSiegeV1` adds `release: Vec::new()`, and
`MilitaryCommandV1::ReviseCrossing` adds `defenders: None`; payloads use
`SCHEMA_VERSION` (now 2), so a host that writes the constant needs no other
change. A 0.20.0 save does not load (every military record carries schema
version 1). A host record that embeds a 0.20.0 `MilitaryCommandV1` does not
decode if it holds a `ReviseSiege` (`release` is required; no serde default
is kept, by the same-version policy); an embedded `OpenEngagement` or
`ReviseCrossing` decodes with `max_minutes` or `defenders` as `None` (serde
reads a missing `Option` field as `None`). A host that names no report holder now pays at most one registry
write a stepping boundary instead of two, with no change on its side. To open a
fight shorter than its kind (a one-day gate assault), set `max_minutes`. To
add a column to an invested place, send `JoinSiege` (not a second
`InvestSite`); during the siege's assault or sortie the column also enters
that fight. An `InvestSite` whose besiegers and defenders have one owner is
now refused. To let a besieged
force march off, release it with `ReviseSiege { release }` and place it in
the same admission order; lift a siege whose last besieger leaves with
`LiftSiege`. To make a force that reached the far bank resist the landing,
send `ReviseCrossing { defenders: Some(..) }`. To step once a day without a
boundary of the step's own, queue
`timed_step_ingress(<the next daily boundary's time>)` on the run handle
after each daily boundary instead of sending `Step` (the default wake
cadence may stay; it skips a boundary that admitted the timed step).
Commands that boundary admits, whether queued as ingress for the same
instant or proposed during it by the host's own systems, apply in later cuts
after the step; on a day whose orders must apply before the step, send
`Step` with them instead (and withdraw that day's timed step with
`cancel_plugin_ingress` before it is due).

## Documentation delta

In this change: the crate README; the `canwu-military` paragraph of
`docs/architecture.md` (the timed step beside the wake system and the
`Step` command); the status line of
[`military-engagements.md`](military-engagements.md); the website military
architecture page and the military tutorial in both languages (the step
drivers, the siege sides, the crossing defenders, the fight's duration, the
starter's `OpenEngagementV1`).

For the release documentation pass: `docs/versioning.md` (the release entry,
its "Upgrading from 0.20.0" and "Known limits"); `docs/terminology.md` and
both website term pages (the candidates below); the engine-usage skill's
military note (the timed step, `JoinSiege`, `release`, the crossing
defenders, `max_minutes`); `opus-progress.md` (delete rows G-85 to G-90).

## Terminology candidates

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| timed step | `timed_step_ingress`, `MILITARY_TIMED_STEP_INGRESS` | 定时战斗步 | A step the host queues as plugin ingress due at a named time; the boundary at that time steps the work due, so a host that steps once a day adds no boundary. Commands admitted in that boundary apply after it. |
| siege join | `JoinSiege` | 加入围城 | A command that adds free forces at the site to a siege's besiegers or defenders, and to the siege's assault or sortie in progress. |
| siege release | `ReviseSiegeV1::release` | 撤出围城 | The part of a siege revision that takes forces off a siege's side, free to be placed. |
| engagement time limit | `OpenEngagementV1::max_minutes`, `EngagementStateV1::max_minutes` | 交战时限 | How long a fight runs before it closes `Stalled` without a break: its kind rule's, or the shorter one the host named when it opened the fight. |
