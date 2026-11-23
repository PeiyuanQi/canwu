# canwu-military

The engagement engine: an optional Canwu extension that resolves fights
between typed forces at a host-owned site under a host-installed rulebook and
a step resolver, and writes records and signals the host applies. It owns
forces as combat knows them, stepped engagements, sieges, contested
crossings, their summaries, and holder-relative engagement reports. It owns no
doctrine, unit catalogue, terrain table, weather, control or occupation state,
store, or report text; it is not a universal combat model.

- **Rulebook.** Every number that decides a fight is a parameter of the host's
  versioned, hashed `CombatRulebookV1`: step profiles, per-kind rules,
  forecast terms and bands, the four lines, cohesion, morale, break, pursuit,
  terrain, night, fortification, siege and crossing rules, and the limits of
  every collection the plugin keeps. The scenario installs it
  (`scenario_records`) or a `System` issuer sends `InstallRulebook`;
  activation refuses a rulebook whose hash, limits, or resolver identity do
  not hold.
- **Forces in the host's world.** A force stands at a host `EntityRef`; the
  host registers, revises, and places it (`RegisterForce`, `ReviseForce`,
  `PlaceForce`, `RetireForce`). The plugin owns only its battle state
  (losses, cohesion, the battle's morale shock, posture, engagement, siege,
  and crossing roles) and moves nothing.
- **Stepped engagements.** `OpenEngagement` opens a fight of one of six kinds,
  for its kind's `max_minutes` or a shorter time the host names; it steps
  only in a boundary that admits a step packet, which the wake system
  (`MilitaryPlugin::with_step_cadence(Some(cadence))`, `Daily` by default),
  the host's trusted `Step { up_to }` command, or a timed step the host
  queues for a boundary it already settles (`timed_step_ingress`, which adds
  no boundary) produces. Each step reads the host's latest condition and
  environment inputs (`SupplyInput`), calls the `StepResolver`, splits
  casualties over components by largest remainder, applies the break rule
  and pursuit, and writes the step with every per-step quantity. A fight
  closes into a create-only `engagement_summary`.
- **Engagement groups (since 0.24.0).** When two or more command groups of
  one side meet the same enemy forces at one site without a shared command,
  `OpenGroupEngagements` opens one engagement per `CommandGroupV1`, each
  against the same `shared` forces. Every step splits the shared forces'
  present men over the group's engagements still fighting, in proportion to
  each command group's present men (largest remainder); the engagements
  step interleaved on one step grid; and the shared side's cohesion, battle
  shock, and order-confusion state are one figure, copied to every open
  engagement of the group after each step, so the shared forces' losses and
  cohesion are counted once. A command group whose opening share would be 0
  is refused, and an engagement whose share falls to 0 while the shared
  forces still have men skips its step. A shared-side `SupplyInput` or
  `OrderAction` applies to every open engagement of the group and advances
  each one's `host_revision`; a `ReviseStepProfile` on one, from either
  side's controller, revises every open one and resumes the held ones on
  the group's grid, as a shared-side `SupplyInput` condition also does. The
  shared side takes no joiner. A held engagement of a
  group closes only when the host resumes or abandons it. Siege assaults
  are not grouped.
- **Order confusion by the day (since 0.24.0).** Each source of
  `ConditionInputV1::order_confusion` costs a side the rulebook's
  `cohesion.confusion_per_day` over one rulebook day of fighting
  (`siege.day_minutes`). The plugin charges it after the resolver: a step
  pays for the minutes since the side was last charged
  (`SideStateV1::confusion_paid_to`) and carries the remainder in
  `SideStateV1::confusion_carry`, so the cost per day is the same on a
  half-hour profile and a day profile, and a span is never charged twice. A
  resolver does not charge it.
- **Resolver.** `MilitaryPlugin<R: StepResolver>`; `ParametricStepResolver`
  is the four-line formula with every coefficient from the rulebook. A host
  may write its own resolver and draw through `StepDraws` (keyed draws on
  `step-contingency`).
- **Sieges and crossings.** `InvestSite` runs investment, emplacement,
  breach, gate, assault, sortie, relief, fall, and lift day by day; a second
  column joins with `JoinSiege` and a force leaves with
  `ReviseSiege { release }`. A garrison yields with `SurrenderSiege` (since
  0.24.0): the siege ends `Fell` by `SiegeEndCause::Surrendered`, an assault
  in progress closes `AttackerPrevailed` and a sortie `Abandoned`, and the
  defenders leave with their men, free at the site and not dispersed, for
  the host to take. The closed assault's summary reads as a storm's would,
  so read the siege's `end_cause`. `OrderCrossing`
  sends waves against an
  interceptor (threshold or a keyed draw on `crossing-interception`) into a
  bridgehead, against the far-bank defenders the order or a later
  `ReviseCrossing { defenders }` names. The plugin writes no occupation: a
  fallen site is `site_fell` in the summary.
- **Reports and the signal.** Phase 13 publishes `engagement_report`
  (superseded per side) and `engagement_outcome` (kept) knowledge to the
  holders each side names, within the publication budget, skipping a holder
  the engine would not accept; a fight, siege, or crossing whose sides name
  no holder queues no report. `with_outcome_signal` sends each closed
  engagement's summary as next-boundary ingress to a host target.

Every keyed draw cites the admitted step packet, so a live, sealed, restored,
and replayed run accept it alike. Every command that changes a record names
the record's `expected_host_revision`, which steps never advance. A refused
command is a recorded rejection (`canwu.military.command_rejected.v1`); a
step the plugin cannot take holds its engagement
(`canwu.military.engagement_held.v1`) until the host revises or abandons it.
The design is `docs/proposals/military-engagements.md`, extended in 0.21.0 by
`docs/proposals/military-engagement-revisions.md` and in 0.24.0 by the notes
at its head (engagement groups, surrender, order confusion by the day).
Payloads carry `SCHEMA_VERSION` 3.
