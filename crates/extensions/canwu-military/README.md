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
- **Stepped engagements.** `OpenEngagement` opens a fight of one of six kinds;
  it steps only in a boundary that admits the plugin's step packet, which
  either the wake system (`MilitaryPlugin::with_step_cadence(Some(cadence))`,
  `Daily` by default) or the host's trusted `Step { up_to }` command produces.
  Each step reads the host's latest condition and environment inputs
  (`SupplyInput`), calls the `StepResolver`, splits casualties over
  components by largest remainder, applies the break rule and pursuit, and
  writes the step with every per-step quantity. A fight closes into a
  create-only `engagement_summary`.
- **Resolver.** `MilitaryPlugin<R: StepResolver>`; `ParametricStepResolver`
  is the four-line formula with every coefficient from the rulebook. A host
  may write its own resolver and draw through `StepDraws` (keyed draws on
  `step-contingency`).
- **Sieges and crossings.** `InvestSite` runs investment, emplacement,
  breach, gate, assault, sortie, relief, fall, and lift day by day;
  `OrderCrossing` sends waves against an interceptor (threshold or a keyed
  draw on `crossing-interception`) into a bridgehead. The plugin writes no
  occupation: a fallen site is `site_fell` in the summary.
- **Reports and the signal.** Phase 13 publishes `engagement_report`
  (superseded per side) and `engagement_outcome` (kept) knowledge to the
  holders each side names, within the publication budget, skipping a holder
  the engine would not accept. `with_outcome_signal` sends each closed
  engagement's summary as next-boundary ingress to a host target.

Every keyed draw cites the admitted step packet, so a live, sealed, restored,
and replayed run accept it alike. Every command that changes a record names
the record's `expected_host_revision`, which steps never advance. A refused
command is a recorded rejection (`canwu.military.command_rejected.v1`); a
step the plugin cannot take holds its engagement
(`canwu.military.engagement_held.v1`) until the host revises or abandons it.
The design is `docs/proposals/military-engagements.md`.
