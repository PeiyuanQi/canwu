# Military Engagements: a Generic Combat Contract

Status: implemented in 0.20.0 (package A: `canwu-military`; package B: the
reference crates and the documentation), after one engine review and one
host-side review (2026-10-24); the implementation corrected the details listed
in [Implementation notes](#implementation-notes-0200), and the sections below
state the implemented contract. It closes downstream
gap G-38 (the `canwu-military` shape contradicts the downstream's combat
design, so the downstream built its own combat on `canwu-api` alone) by
replacing the extension's model with a generic engagement contract: the
mechanics in Canwu, the historical content and policy in the host. It changes
`canwu-military`, `canwu-military-reference-content`, and
`canwu-military-reference`; `canwu-sim`, `canwu-api`, `canwu-movement`,
`canwu-resource`, and `canwu-knowledge` are untouched. Same version only: no
compatibility work with 0.19.0 records, hashes, or the 0.19.0 crate's public
API. It supersedes `military-domain-design.md`.

"Today" means 0.19.0 as it is on `main`.

## Contents

1. [Problem](#problem)
2. [Decision and invariants](#decision-and-invariants)
3. [Ownership](#ownership)
4. [Causal model](#causal-model)
5. [Records](#records)
6. [Commands and authority](#commands-and-authority)
7. [Systems and the step packet](#systems-and-the-step-packet)
8. [The step: resolver trait and parametric resolver](#the-step-resolver-trait-and-parametric-resolver)
9. [Composition of force and side state](#composition-of-force-and-side-state)
10. [Sieges](#sieges)
11. [Contested crossings](#contested-crossings)
12. [Reports](#reports)
13. [Signals to the host](#signals-to-the-host)
14. [Determinism](#determinism)
15. [Persistence, continuation, and replay](#persistence-continuation-and-replay)
16. [Refusals and what can still fail a boundary](#refusals-and-what-can-still-fail-a-boundary)
17. [Budgets and scale](#budgets-and-scale)
18. [What is removed and what the reference crates become](#what-is-removed-and-what-the-reference-crates-become)
19. [Adoption by a host with its own combat](#adoption-by-a-host-with-its-own-combat)
20. [Implementation plan](#implementation-plan)
21. [Documentation and terminology delta](#documentation-and-terminology-delta)
22. [Known limits](#known-limits)
23. [Open questions](#open-questions)
24. [Review record](#review-record)
25. [Implementation notes (0.20.0)](#implementation-notes-0200)

## Problem

Downstream gap G-38 records that the 0.19.0 `canwu-military` shape
contradicts the downstream's combat design, so the downstream built its own
combat (an engagement with a traced forecast, daily steps on a parametric
formula, sieges, and a contested crossing) on `canwu-api` directly and adopts
nothing from the crate. Each point below is verified in the 0.19.0 source
(`crates/extensions/canwu-military/src/{model,plugin}.rs`).

1. **Combat is one fixed attrition formula.** `resolve_combat_round`
   (`plugin.rs` 1703–1867) takes `defender_strength / 20 + sample % 8` and
   `attacker_strength / 18 + (999 − sample) % 8` men a round, morale
   `−loss / 2`, fatigue `+35`, and closes at annihilation or `round >= 3`
   with one of five `CombatResult` values. The ruleset's `tactics` and
   `terrain_modifiers` exist (`model.rs` 122–124) but the round never reads
   them; `OperationState.terrain` and `.weather` are written `String::new()`
   (`plugin.rs` 891–892, 947–948). A host cannot supply casualty, cohesion,
   or break rules, a step cadence, or an environment.
2. **Victory creates control.** An attacker victory auto-creates an
   `OccupationState` with `administrative_reach`, `fiscal_capacity`,
   `legitimacy`, `collaboration`, `resistance`, and an `IntegrationStage`
   ladder (`plugin.rs` 1780–1817), which a daily plugin tick then climbs to
   `Intergenerational` (`advance_occupation_state`, 1882–1955). Control and
   occupation are the host's authority domain; a combat extension that owns
   them is a second authoritative truth.
3. **A private world.** Forces stand on `MilitaryNodeId` entries of a
   `MilitaryCatalog` (`model.rs` 219–233, 252), not on an `EntityRef` the
   host's world or `canwu-movement` knows; a march is a one-minute
   plugin-scheduled tick that teleports the force (`plugin.rs` 894–896,
   1562–1607). A host must mirror every place into the catalog and keep two
   location truths.
4. **One random stream for everything** (`military-operation`, `plugin.rs`
   162–164); `RandomEnvelope.ruleset_hash` is the literal
   `"synthetic-runtime-v1"` (1838).
5. **Knowledge is four fixed facts** (`KnowledgeFact`, `model.rs` 463–483)
   and a report system that hard-codes a JSON body for every force's
   commander (`plugin.rs` 1976–1984), scanning up to 256 force records each
   boundary.
6. **Authority is a person.** Every command needs `Issuer::Actor` and the
   force's `commander: PersonId` (`plugin.rs` 256–261, 357–373); a host whose
   issuers are institutions or a declared run cannot command a force.
7. **Self-scheduled cadence and unbounded ledger.** The plugin schedules its
   own ticks (`SchedulePluginIngress` one minute or one day ahead, 912–918,
   1000–1006, 1678–1684) and keeps every command outcome for the life of the
   run in `MilitaryLedger.outcomes` (`model.rs` 507–511); each create scans
   `domain_records_of_kind(…, MAX_RECORDS)` (`plugin.rs` 587–590).
8. **Scope drift.** Recruitment, training, reconnaissance, ambush
   preparation, special operations, occupation policy, and a
   provider-outcome saga duplicate what the host's institutions,
   `canwu-knowledge`, `canwu-movement`, and `canwu-resource` already own.

The downstream's re-evaluation criteria for the gap are (a) a resolver or
rule-driven step hook returning an outcome bundle, (b) forces bound to the
host's entities with movement outside the crate, (c) no control dimensions,
(d) one stream per purpose, (e) host-declared knowledge schemas, and (f) a
second independent host. This proposal meets (a)–(e) in the contract and
leaves (f) to the downstream's adoption.

## Decision and invariants

`canwu-military` becomes the **engagement engine**: a stateful, multi-step
process that resolves fights between typed forces at a host-owned site under
host-supplied rules and inputs, and writes records and signals the host
applies. It owns no doctrine, unit catalogue, terrain table, weather, control
state, store, or report text.

Invariants:

- **I1. Mechanics in Canwu, content in the host.** Every number that is a
  historical or gameplay choice (strength terms, band cuts, fortification
  grades, terrain rows, casualty split, break thresholds, posture
  modifiers, step profiles, per-kind rules, limits) is a parameter of a
  versioned, hashed `CombatRulebookV1` the host installs; the plugin
  validates it at activation and reads it at every step. The plugin ships
  no default values that decide a fight.
- **I2. An engagement is explicit and stepped.** A fight opens on a command,
  steps only in a boundary that admits the plugin's step packet, writes its
  steps with every per-step quantity, and closes into a summary. Nothing
  resolves in one hidden round.
- **I3. Outcomes are records and signals, never state of another domain.**
  The plugin writes only its own record kinds (`canwu.military:*`); a site
  that falls, a force that routs, a bridgehead that lands are facts in its
  records and an optional next-boundary signal. It never creates
  occupation, control, population, or resource state.
- **I4. Forces live in the host's world.** A force's location is a host
  `EntityRef`; the host places it (on arrival from `canwu-movement` or its
  own movement) and the plugin only checks co-location. The plugin keeps no
  node catalogue and moves nothing.
- **I5. Determinism and replay.** One random stream per purpose; every keyed
  draw cites the admitted step packet (replay-stable in a live, sealed,
  restored, and replayed run); integer arithmetic with one floor division
  per line; ordered collections with explicit tie-break keys; no boundary
  decision reads retained evidence or a host record version.
- **I6. Refusals are recorded rejections.** A stale revision, a wrong issuer,
  an absent force, an over-limit, a resolver mismatch, or an oversized step
  is a rejection event or a held engagement; the boundary stands.
- **I7. Bounded and proportional.** Every collection the plugin keeps has a
  cap in the rulebook's limits; the live record set is the open work plus
  bounded summaries; a boundary's cost is proportional to the open
  engagements and the forces engaged in them, never to the forces
  registered; a boundary without a step packet does no stepping work.
- **I8. Holder-relative reports.** Knowledge leaves only through
  `PublishKnowledge` under the plugin's declared report schemas, to holders
  the host names per side and the engine accepts, within the publication
  budget; no ground truth reaches an actor.

## Ownership

| Concern | Owner | Military boundary |
| --- | --- | --- |
| Generic identity, time, random draws, records, ingress, authority envelope, atomic commit, persistence, replay | `canwu-sim` behind `canwu-api` | the plugin registers descriptors and handlers; it never mutates runtime internals |
| A force as combat knows it: identity, owner, controllers, location, components with present and lost men, side terms, battle state (losses, cohesion, morale shock, posture, engagement role) | `canwu-military` (`force` record) | the host mirrors location, composition, and terms in by command and reads battle state back; each field has one writer (§9 states the composition) |
| Everything else about a force (institution, pay, autonomy, readiness derivation, doctrine, commander, supply stores) | host | enters the engagement as `ConditionInputV1` values the host supplies per engagement-day, each citing the host record version it was derived from (cited, never read) |
| The engagement process: open, forecast, steps, break, pursuit, close, summary | `canwu-military` | on the installed rulebook and a `StepResolver` |
| The step formula | `StepResolver` trait; the crate's `ParametricStepResolver` is the default whose every coefficient is a rulebook parameter | a host may supply its own resolver type; the plugin owns the state machine around it and performs its draws |
| Siege: investment, emplacement, breach progress, assault, sortie, relief, fall, lift | `canwu-military` (`siege` record) | site profile (grade, accesses, repair), supply (days of stock, ration level), naval control, and a gate opened by the host's own rule (betrayal, incident) are host inputs |
| Contested crossing: waves, interception, bridgehead | `canwu-military` (`crossing` record) | wave size and interception rule are rulebook parameters; the interceptor's control and scouts are host inputs |
| Rulebook content: terrain rows, fortification grades, band cuts, posture modifiers, per-kind rules, step profiles, casualty and break parameters | host content; the reference content pack ships one explicitly synthetic rulebook | immutable, versioned, hashed; never read unverified during a boundary |
| Unit profiles, doctrine, commanders' traits, wall grades, weather, terrain classes of places | host | compiled by the host into component values, side terms, site profiles, and environment inputs |
| Contact, detection, sightings, hostility | host and `canwu-knowledge` | the host opens engagements; the plugin checks co-location |
| Army movement | `canwu-movement` (or the host) | arrival → host `PlaceForce`; the plugin reads no movement record |
| Supply and losses of goods | `canwu-resource` through the host | the plugin reports men lost; the host debits stores (`RecordLoss`, `ExternalOutflow`) and feeds `days_of_stock` back |
| Orders and AI | `canwu-decision` through the host | a ticket's option becomes a military command; the plugin owns no ticket |
| Control, occupation, legitimacy, population, casualties as people | host (authority, society, population domains) | from the `site_fell`, routed, and loss facts in the summary and signal |
| Player verbs, previews, report text | host | reads the plugin's records and knowledge; the pure helpers `forecast`, `intercept_bp`, `breach_rate`, `largest_remainder` are public for previews |

## Causal model

```text
authored definitions      host content: rulebook parameters, component values, site profiles, holders
  -> compiled plan        CombatRulebookV1 installed and validated (rules_hash, resolver identity, limits)
  -> admitted input       military_command_v1 -> military_apply_v1 packets; military_step_v1 { up_to }
                          from the optional wake system or the trusted Step command
  -> ordered phases       7: settle-engagements-v1 applies the boundary's apply packets, then, when a step
                             packet is admitted, steps every open engagement, siege, and crossing due by up_to
                          13: publish-engagement-reports-v1 drains the registry's report queue to holders
  -> atomic state and     force, engagement, engagement_steps, siege, crossing, registry (all versioned);
     evidence             engagement_summary (create-only) at close; one rejection event per refused command
  -> actor views          engagement_report / engagement_outcome knowledge per holder; the host's own facts
                          from the records
```

In a boundary that admits a step packet the settle system: (1) applies the
boundary's apply packets in admission order, each against the host revision
it names; (2) drains the registry's retire queue; (3) for every open
engagement whose `next_step_at <= min(up_to, context.at)`, in id order,
resolves `min(elapsed / step_minutes, max_steps_per_boundary)` steps:
assembles `StepInputV1` from the sides (components present, the latest
condition inputs and their age, the environment input, the kind rule, the
site state), calls the resolver with a draw accessor, applies the outcome
(casualties by component by largest remainder in component-id order,
cohesion, morale, posture), evaluates the break rule and pursuit, appends the
`StepV1` to this boundary's steps record, and closes the engagement when a
side breaks, the attacker abandons, or the kind's `max_minutes` elapse without
a break; (4) advances sieges and crossings due by `up_to`; (5) rewrites each
engaged force once; (6) at close writes the summary, queues the steps record
to be rewritten empty at the next stepping boundary (it keeps the closing
boundary's steps until then), queues the closing report, and emits the
signal.

## Records

Namespace `canwu.military`; every payload carries `schema_version: u32` (1 in
0.20.0) and is validated on create, update, activation, and restore. Ids are
namespaced strings (`validate_id` kept). Every versioned record carries a
`host_revision: u64` that only host commands advance; steps advance the
record store's version and leave `host_revision`, so a host command names
`expected_host_revision` and is not made stale by a step settled in the
same instant.

| Record (kind) | Class · policy | Payload (one per) | Written by | Lifecycle |
| --- | --- | --- | --- | --- |
| `rulebook` | Record · CreateOnly | `CombatRulebookV1` (one per `RulebookId`; one per run) | scenario or `InstallRulebook` | lives for the run |
| `registry` | Record · Versioned | `CombatRegistryV1 { rulebook, force_count, open_engagements: BTreeSet<EngagementId>, open_sieges, open_crossings, earliest_due: Option<SimTime>, retire_queue: Vec<RetireItemV1 { at, record }> /* ascending */, steps_to_empty: BTreeSet<EngagementId>, report_queue: Vec<ReportItemV1>, next_sequence: u64 }` (one, id `root`) | open/close/register, every stepping boundary (`earliest_due`), phase 13 (queue drain) | rewritten only when its fields change |
| `force` | Entity · Versioned · `holder_policy: Allowed` | `ForceStateV1` | `RegisterForce`, `ReviseForce`, `PlaceForce`, `RetireForce`, the step | `RetireForce` sets `status: Retired`; the record is retired without successor at the next stepping boundary (one mutation per record a boundary) |
| `engagement` | Record · Versioned | `EngagementStateV1` | `OpenEngagement`, `JoinEngagement`, `SupplyInput`, `OrderAction`, `ReviseStepProfile`, the step | `Retired` (no successor) from the retire queue `retire_after_minutes` after close |
| `engagement_steps` | Record · Versioned | `EngagementStepsV1` (one per engagement, id = engagement id, created empty at the opening; rewritten each stepping boundary with that boundary's steps only — the history is the sealable journal) | the step; rewritten **empty** at the first stepping boundary after the close (`registry.steps_to_empty`) | retired with its engagement |
| `engagement_summary` | Record · CreateOnly | `EngagementSummaryV1` | close | kept (the durable record) |
| `siege` | Record · Versioned | `SiegeStateV1` | `InvestSite`, `ReviseSiege`, `SupplyInput`, `LiftSiege`, the step | `Retired` from the retire queue after end |
| `crossing` | Record · Versioned | `CrossingStateV1` | `OrderCrossing`, `ReviseCrossing`, `SupplyInput`, `CancelCrossing`, the step | `Retired` from the retire queue when complete or cancelled |

Retired records keep their payload in the store (the engine's lifecycle),
the kernel validates every owned record at each load, retired ones included
(shape, schema, references, successors), and the archive reachability
participant marks plugin archive objects only — this plugin has none in
0.20.0, so nothing removes a closed fight's records. The plugin therefore
keeps what a closed fight leaves small: the steps record is rewritten empty at
the first stepping boundary after the close (its steps are in the journal; a
record takes one mutation a boundary, so the closing boundary's steps are
written first) and the engagement record
holds its final state, so a closed fight leaves ≈ 2 KB of owned records
(engagement ≈ 1.5 KB, empty steps ≈ 0.1 KB, summary ≈ 0.6 KB), never the up
to 48 KB of a day's steps. `validate_activation` decodes and validates the
registry, the rulebook (hash, resolver identity, limits), and every `Active`
record against the limits; for retired records it checks the envelope only,
and it checks no host record version (the G-77 class).

Type sketches (fields that decide behaviour; the implementer names the rest):

```rust
pub struct CombatRulebookV1 {
    pub id: RulebookId, pub schema_version: u32, pub rules_hash: String,   // canonical_hash of the parameters
    pub resolver: ResolverIdentityV1 { name: String, version: u32 },       // the owned form of R::IDENTITY (a const holds `&'static str`)
    pub trusted_issuers: Vec<Issuer>,      // <= limits.trusted_issuers; may issue any command
    pub step_profiles: BTreeMap<ProfileId, StepProfileV1 { step_minutes: u32, step_scale_bp: u32, max_steps_per_boundary: u32 }>,
    pub default_profile: ProfileId,
    pub kinds: BTreeMap<EngagementKind, KindRuleV1 { frontage_bp: u32, max_minutes: u32 /* a one-day fight: siege.day_minutes, whatever the profile's step */, terrain: Option<TerrainClassId>,
        fort: FortUse /* None | Site | SiteHalved */, posture_bp: Vec<PostureModifierV1>, posture_steps: u32 }>,
    pub condition_channels: Vec<ChannelId>,// host-declared names; values arrive in ConditionInputV1 in this order
    pub band_cuts: Vec<BandCutV1>,         // value >= cut -> band index (conditions, loss and strength bands)
    pub forecast: ForecastRulesV1 {
        terms: Vec<StrengthTermV1 { id: TermId, source: TermSource, divisor: u32, cap: i64 }>,
        manpower_bands: Vec<(u64, i64)>, terrain_fit: Vec<TerrainFitRowV1 { terrain, trait_: TraitId, delta: i64 }>,
        environment_terms: Vec<EnvironmentTermV1 { ground: Option<GroundClassId>, ranged_reliability_below_bp: Option<u32>,
            trait_: Option<TraitId>, ranged_share_from_per_mille: Option<u16>, delta: i64 }>,
        ratio_bands: Vec<RatioBandV1 { min_ratio_bp: i64, fortified: Option<bool>, band: ForecastBandId }>,
    },
    pub lines: LineRulesV1 { fire_weight: u32, shock_weight: u32, melee_weight: u32, defense_offset: u32,
        inflicted_cap_per_mille: u32, killed_numerator: u32, killed_denominator: u32,
        supply_fire: SupplyFireRuleV1 { below_per_mille: u16, bp: u32 },
        leadership: LeadershipRuleV1 { base_bp: u32, command_weight: u32, pick_weight: u32, min_bp: u32, max_bp: u32 } },
    pub cohesion: CohesionRuleV1 { per_inflicted: u32, base: u32, confusion: u32, recover_per_day: u32 },
    pub morale: MoraleRuleV1 { sensitivity_default: u32, credibility_divisor: u32, shock_recover_per_day: u32 },
    pub break_rule: BreakRuleV1 { rout_cohesion_below: u32, rout_morale_margin: u32, walled_holds_from_grade: u8 },
    pub pursuit: PursuitRuleV1 { captured_per_mille_at_full: u32, wounded_left_per_mille: u32 },
    pub terrain: BTreeMap<TerrainClassId, TerrainRowV1 { frontage_bp, melee_bp, shock_bp, defense_bp, charge_window_bp, pursuit_bp }>,
    pub night: NightFactorsV1 { fire_bp, shock_bp, melee_bp, pursuit_bp },
    pub fortification: Vec<FortGradeV1 { grade: u8, fort_bp: u32, breach_threshold: u32 }>,
    pub postures: Vec<PostureId>,
    pub siege: SiegeRulesV1 { day_minutes: u32, men_per_access_sealed: u32, emplacement_days: BTreeMap<GroundClassId, u16>,
        sortie_cooldown_minutes: u32, water_access_control_bp: u32, assault_terrain: TerrainClassId, breach_terrain: TerrainClassId,
        sortie_terrain: TerrainClassId },
    pub crossing: CrossingRulesV1 { wave_minutes: u32, wave_men: u64, hard_river_wave_bp: u32, bridgehead_secure_men: u64,
        terrain: TerrainClassId, intercept: InterceptRuleV1 { min_control_bp: u32, control_weight: u32, scout_weight: u32,
            concealment: BTreeMap<ConcealmentClassId, u32>, night_concealment: u32, threshold_bp: u32, mode: InterceptMode /* Threshold | KeyedDraw */ } },
    pub limits: CombatLimitsV1,           // below
}
pub struct PostureModifierV1 { pub attacker: PostureId, pub defender: PostureId, pub fort: FortState /* Any | Closed | Open */, pub bp: u32 }
    // a Vec searched in order, first match wins (a map keyed by a tuple does not serialize to a JSON object)

pub struct ForceStateV1 {
    pub id: ForceId, pub owner: EntityRef, pub controllers: Vec<Issuer>, pub host_revision: u64,
    pub location: LocationV1 { At { site: EntityRef } | OnRoute { route: EntityRef, from: EntityRef, to: EntityRef, progress_per_mille: u16 } },
    pub components: BTreeMap<ComponentId, ComponentStateV1 { present: u64, values: ComponentValuesV1, losses: LossTallyV1 }>,
    pub terms: SideTermsV1 { cohesion_cap: u32, morale_break: u32, morale_sensitivity: u32,
        battle_command: u32, assessment: u32, credibility: u32, traits: BTreeSet<TraitId> },
    pub battle: BattleStateV1 { cohesion: Option<u32>, morale_shock: u32, last_fought_at: Option<SimTime>, recovered_to: Option<SimTime>,
        posture: Option<PostureId>, engaged_in: Option<EngagementId>, siege_role: Option<SiegeRoleV1>, crossing: Option<CrossingId>,
        sortie_ready_at: Option<SimTime>, dispersed: bool },
    pub status: ForceStatus /* Active | Retired */,
}
pub struct ComponentValuesV1 { pub fire: u32, pub melee: u32, pub shock: u32, pub defense: u32,
    pub training_bp: u32, pub equipment_bp: u32, pub breach_per_piece: u32, pub crew_per_piece_x10: u32,
    pub ranged: bool, pub mounted: bool, pub vessel: bool }

pub struct ConditionInputV1 {            // per side; the latest stands until the next SupplyInput
    pub channels: Vec<u16>,              // per mille, in rulebook.condition_channels order (count must match)
    pub supply_per_mille: u16, pub morale_per_mille: u16, pub readiness_per_mille: u16, pub fatigue_per_mille: u16,
    pub posture: PostureId, pub surprise_bp: u32, pub order_confusion: u32, pub defense_halved: bool,
    pub evidence: Vec<DomainRecordVersionRef>,   // <= limits.evidence_refs_per_input; cited on the step, never read
}
pub struct EnvironmentInputV1 { pub terrain: Option<TerrainClassId> /* None: the kind rule's */, pub ground: GroundClassId,
    pub concealment: ConcealmentClassId, pub night: bool, pub ranged_reliability_bp: u32, pub river: RiverBandId,
    pub evidence: Vec<DomainRecordVersionRef> }

pub struct EngagementStateV1 {
    pub id: EngagementId, pub host_revision: u64, pub kind: EngagementKind /* Field | Assault | Sortie | Relief | FerryFight | BridgeheadFight */,
    pub profile: ProfileId, pub site: EntityRef, pub fort: Option<(u8, bool)> /* grade, breach or gate open */,
    pub siege: Option<SiegeId>, pub crossing: Option<CrossingId>,
    pub attacker: SideStateV1, pub defender: SideStateV1, pub environment: Option<(EnvironmentInputV1, SimTime)>,
    pub forecast: ForecastV1 { attacker_terms: Vec<(TermId, i64)>, defender_terms: Vec<(TermId, i64)>, ratio_bp: i64, band: ForecastBandId },
    pub opened_at: SimTime, pub last_step_at: SimTime, pub next_step_at: SimTime, pub steps_resolved: u32,
    pub status: EngagementStatus /* Open | Held { reason: HoldReason } | Closed */,
    pub outcome: Option<EngagementOutcomeV1 { kind: AttackerPrevailed | Repulsed | Stalled | Abandoned, closed_at: SimTime, site_fell: bool }>,
}
pub struct SideStateV1 { pub forces: Vec<ForceId>, pub holders: Vec<KnowledgeHolderRef>,
    pub input: Option<(ConditionInputV1, SimTime)>, pub morale_shock: u32, pub cohesion: u32, pub posture_after: Continuation, pub losses: LossTallyV1 }

pub struct EngagementStepsV1 { pub engagement: EngagementId, pub boundary_at: SimTime, pub steps: Vec<StepV1> }   // this boundary's steps
pub struct StepV1 { pub index: u32, pub at: SimTime, pub attacker: SideStepV1, pub defender: SideStepV1,
    pub attacker_channels: Vec<u16>, pub defender_channels: Vec<u16>, pub environment_digest: String,
    pub terms: Vec<TermId>, pub draws: Vec<RandomDrawId>, pub resolver: ResolverIdentityV1 }
pub struct SideStepV1 { pub lines: LinesV1 { men, engaged, fire, shock, melee, defense, leadership_bp }, pub inflicted_per_mille: u32,
    pub killed: u64, pub wounded: u64, pub captured: u64, pub dispersed: u64, pub cohesion_after: u32, pub morale_after: u32,
    pub morale_shock: u32, pub posture_after: Continuation /* Continue | Withdraw | Rout */, pub condition_age_steps: u32 }

pub struct SiegeStateV1 { pub id: SiegeId, pub host_revision: u64, pub site: EntityRef,
    pub profile: SiteProfileV1 { grade: u8, accesses: u32, water_accesses: u32, repair_per_day: u32 },
    pub besiegers: Vec<ForceId>, pub defenders: Vec<ForceId>, pub holders: [Vec<KnowledgeHolderRef>; 2],
    pub inputs: [Option<(ConditionInputV1, SimTime)>; 2], pub environment: Option<(EnvironmentInputV1, SimTime)>,
    pub naval: Option<InterceptorInputV1>, pub investment_bp: u32, pub pieces: u64,
    pub emplacement_from: Option<SimTime>, pub bombardment_from: Option<SimTime>, pub work_bp: u32,
    pub breach_progress: u32, pub breach_threshold: u32, pub breach_open: bool, pub gate_open_until: Option<SimTime>,
    pub supply: SiegeSupplyInputV1 { days_of_stock: u64, ration: RationLevelId, evidence: Vec<DomainRecordVersionRef> },
    pub storm_on: StormRule /* Never | BreachOrGate */, pub assault_ordered: bool, pub sortie_ordered: bool,
    pub engagement: Option<EngagementId>, pub started_at: SimTime, pub next_day_at: SimTime,
    pub status: SiegeStatus /* Open | Fell | Lifted */, pub end_cause: Option<SiegeEndCause /* Breach | GateOpened | Storm | Abandoned | Relieved | Withdrawn */> }

pub struct CrossingStateV1 { pub id: CrossingId, pub host_revision: u64, pub force: ForceId, pub from: EntityRef, pub to: EntityRef, pub night: bool,
    pub defenders: Vec<ForceId>, pub holders: [Vec<KnowledgeHolderRef>; 2], pub inputs: [Option<(ConditionInputV1, SimTime)>; 2],
    pub environment: Option<(EnvironmentInputV1, SimTime)>,
    pub interceptor: Option<InterceptorInputV1 { force: ForceId, control_bp: u32, scouts: u32, evidence: Vec<DomainRecordVersionRef> }>,
    pub waves_sent: u32, pub landed: u64, pub bridgehead_secure: bool, pub next_wave_at: SimTime,
    pub last_wave: Option<WaveV1 { index, men, intercepted, intercept_bp: Option<i64>, draw: Option<RandomDrawId>, engagement: Option<EngagementId> }>,
    pub status: CrossingStatus /* Open | Complete | Cancelled */ }

pub struct CombatLimitsV1 { pub forces: u32 /* 4_096 */, pub components_per_force: u32 /* 64 */, pub forces_per_side: u32 /* 16 */,
    pub controllers_per_force: u32 /* 8 */, pub trusted_issuers: u32 /* 8 */, pub holders_per_side: u32 /* 8 */,
    pub open_engagements: u32 /* 8 */, pub open_sieges: u32 /* 8 */, pub open_crossings: u32 /* 4 */,
    pub step_bytes: u32 /* 1_024, per step */, pub draws_per_step: u32 /* 64 */, pub retire_after_minutes: u64 /* 43_200 */,
    pub evidence_refs_per_input: u32 /* 8 */, pub report_queue: u32 /* 64 */ }
```

The values in comments are the reference rulebook's; the plugin refuses a
rulebook whose limits exceed its hard ceilings (forces 16,384; steps per
boundary 96; step bytes 4,096; draws per step 256) at activation.

## Commands and authority

One action `military_command_v1` with payload `MilitaryCommandV1`; the
handler admits it (static checks, holder acceptance) and enqueues the
internal `military_apply_v1` packet, admitted at the next cut and applied in
phase 7 against the staged state. Every command that changes a record names
that record's `expected_host_revision`; a mismatch is a recorded
`DomainRecordVersionConflict` rejection. The plugin keeps no outcome ledger:
the kernel's request outcome retention is the record of attempts, and a
resend of an applied command is refused by its stale host revision.

| Command | Changes | Authority |
| --- | --- | --- |
| `InstallRulebook { rulebook }` | creates `rulebook`; sets `registry.rulebook` (refused when one is set) | scenario installation, or an `Issuer::System(_)` issuer (the trusted issuers live in the rulebook, so they cannot gate its installation) |
| `RegisterForce { force: ForceStateV1 }` (Active, no battle state, `host_revision` 0) | creates `force`; `registry.force_count` | trusted issuer, or any of `force.controllers` |
| `ReviseForce { force, expected_host_revision, components?, terms?, controllers?, clear_morale_shock: bool }` | composition, values, terms, controllers; losses kept; allowed while engaged and applied before the step of the same boundary | controller |
| `PlaceForce { force, expected_host_revision, location }` | location; refused while `engaged_in` is some, besieging, or crossing | controller |
| `RetireForce { force, expected_host_revision }` | status Retired (the record retires at the next stepping boundary); refused while engaged, besieging, or crossing | controller |
| `OpenEngagement { id, kind, profile?, site, fort: Option<(u8, bool)>, attackers, defenders, holders: [Vec; 2], inputs: [ConditionInputV1; 2], environment }` | creates `engagement`; every named force `At(site)`, not engaged, one owner per side; `kind` any of the six (an `Assault` with `fort` stands without a siege); forecast computed | controller of every attacker; defenders need none |
| `JoinEngagement { engagement, expected_host_revision, side, forces }` | adds co-located forces to a side | controller of the joining forces |
| `SupplyInput { target: Engagement(id) \| Siege(id) \| Crossing(id), expected_host_revision, side: Option<Side>, condition: Option<ConditionInputV1>, environment: Option<EnvironmentInputV1> }` | the target's latest per-side or environment input; a siege or crossing hands its latest inputs to every engagement it opens | condition: a controller on that side or trusted; environment: trusted or either side's controller |
| `OrderAction { engagement, expected_host_revision, side, action: Hold \| Withdraw \| Abandon }` | `Hold`/`Withdraw` set the side's posture for the next step; the attacker's `Abandon` closes the engagement `Abandoned` in the applying boundary (a `Held` engagement included), freeing its forces, and a defender's `Abandon` is a `Withdraw` (on a `Held` engagement it only sets `withdraw_ordered`, which takes effect when the engagement steps again) | controller on that side |
| `ReviseStepProfile { engagement, expected_host_revision, profile }` | the step profile from the next step (`next_step_at = last_step_at + new step_minutes`) — promotion and demotion | trusted or either side's controller |
| `InvestSite { id, site, profile, besiegers, defenders, holders, inputs, environment, work_bp, supply, naval?, storm_on }` | creates `siege`; forces at site; defenders may be empty (`Abandoned` at the first siege day) | controller of every besieger |
| `ReviseSiege { siege, expected_host_revision, supply?, profile?, work_bp?, naval?: InterceptorRevisionV1 { Set(input) \| Withdraw }, gate_open_steps?: u32, assault: Option<bool>, sortie: bool, storm_on? }` | host inputs; `gate_open_steps` opens a gate for n steps (the host's betrayal or incident rule); `assault` orders or stands down the storm; `sortie` orders one when `sortie_ready_at` has passed | supply, profile, gate, naval, storm_on: trusted or the side's controller; `assault`: besiegers' controller; `sortie`: defenders' controller |
| `LiftSiege { siege, expected_host_revision }` | status Lifted, cause Withdrawn | besiegers' controller |
| `OrderCrossing { id, force, from, to, night, defenders, holders, inputs, environment, interceptor? }` | creates `crossing`; force `At(from)` | controller |
| `ReviseCrossing { crossing, expected_host_revision, interceptor?: InterceptorRevisionV1 { Set(input) \| Withdraw }, night? }` | interceptor input (`Withdraw` removes it; JSON cannot tell `None` from `Some(None)`) | interceptor: trusted or the old or new interceptor's controller; night: the crossing force's controller |
| `CancelCrossing { crossing, expected_host_revision }` | status Cancelled | controller |
| `Step { up_to: SimTime }` | the handler enqueues a `military_step_v1 { up_to }` packet (no apply packet); refused when `up_to` is before the admitting boundary | trusted issuer |

Authority: `Issuer` is matched by equality against `force.controllers`
(`Actor`, `Human`, `Ai`, `Institution`, `System`, …), so an institutional
issuer or a declared run's adapter can command a force;
`rulebook.trusted_issuers` may issue any command. Holders named on
`OpenEngagement`, `InvestSite`, and `OrderCrossing` are checked at admission
with `knowledge_holder_accepts_publication` and refused when the engine would
not accept them. Co-location is checked against the staged force records, so
a `PlaceForce` and an `OpenEngagement` in one boundary compose in admission
order.

## Systems and the step packet

| System | Phase · cadence | Reads | Writes | Draws | Emits |
| --- | --- | --- | --- | --- | --- |
| `wake-engagements-on-cadence-v1` (registered only with `with_step_cadence(Some(cadence))`; `EventDriven` refused at registration) | 7 · the host's cadence | `registry` | nothing | — | schedules `military_step_v1 { up_to: context.at }` due at once when `registry.earliest_due <= context.at` and `view.can_schedule_ingress()`; no early exit on admitted ingress (a calendar boundary carries calendar ingress, so fiscal's early exit would never wake on a calendar day) |
| `settle-engagements-v1` (name carries the signal target, as fiscal's report system carries its share; the publication reservation is registered through the registrar and needs no name, since `knowledge_publication_budget()` already accounts for the plugin's own reservation) | 7 · `EventDriven` | every plugin kind, `core_ingress` | every plugin kind (the `rulebook` by an applied `InstallRulebook`); `engagement_summary` create-only | `crossing-interception`, `step-contingency` | `canwu.military.command_rejected.v1`, `.engagement_opened.v1`, `.engagement_closed.v1`, `.engagement_held.v1`, `.siege_ended.v1`, `.crossing_wave.v1`, `.report_dropped.v1`; the optional signal packet |
| `publish-engagement-reports-v1` | 13 · `EventDriven` | `registry`, `engagement`, `engagement_steps`, `engagement_summary`, `siege`, `crossing`, `core_person_availability` (the summary's version is read through `current_domain_record_version`, which needs the declared read) | `registry` (drains `report_queue`); knowledge `engagement_report` and `engagement_outcome` (`SameBoundary`) | — | — |

**One mechanism for stepping.** An `EventDriven` system runs in every
boundary that admits any ingress — commands and calendar boundaries
included — and a zero-delay plugin packet is admitted only at the next cut.
So the settle system, on every run, (a) applies the `military_apply_v1`
packets this boundary admitted, in admission order, and (b) steps **only
when this boundary admitted a `military_step_v1 { up_to }` packet**, stepping
the work due by `min(up_to, context.at)`; a boundary with neither does
nothing after reading `core_ingress`. The step packet has two producers of
one shape:

- the **wake system**, when the host chooses `MilitaryPlugin::with_step_cadence(Some(cadence))`:
  in a boundary of that cadence it schedules the packet whenever
  `registry.earliest_due` has arrived, so the next boundary at the same
  instant (which `advance_canonical` and `step_canonical` settle) steps —
  one extra boundary at the same instant on days with due work, none on
  quiet days;
- the **`Step { up_to }` command** from a trusted issuer, when the host
  prefers to drive stepping itself (`with_step_cadence(None)`): the host
  enqueues its `SupplyInput`s and the `Step` in one admission order; their
  packets land in the same next cut, apply packets first, so the inputs
  apply and the step runs in the boundary the host's own batch already
  causes, and the step adds no boundary of its own.

Both producers give the draws the same evidence (the admitted step packet),
the same ordering (commands, then steps in id order), and the same
`host_revision` semantics (a step never makes a host command stale). When a
wake packet and a `Step` land in one cut, the draws cite the **first admitted**
step packet and the boundary steps to the **larger** `up_to`; the second
packet adds nothing (its work is already due). The settle system applies
commands before it steps even when both packets arrive together, and
`earliest_due` is kept on the registry (the minimum
`next_step_at`, `next_day_at`, `next_wave_at`, and retire time of the open
work), rewritten only when it changes. Steps elapsed are
`floor((min(up_to, at) − last_step_at) / step_minutes)`, capped by the
profile's `max_steps_per_boundary`; the remainder carries. A command boundary
mid-day that also carries a `Step` resolves the elapsed steps up to it, so an
order applies from the next step. The cadence, the signal target, and the
reservation are in the plugin descriptor (system contracts, the settle
system's name, and the registered reservation), so a run restores and replays
only with the same.

## The step: resolver trait and parametric resolver

Boundary-system and command handlers are bare `fn` pointers and the kernel
borrows the plugin only while it registers, so the resolver is a **type
parameter**, not a value: `MilitaryPlugin<R: StepResolver + Default +
'static>` registers `settle::<R>`, which constructs `R::default()` per call;
`R::IDENTITY` is a const `ResolverIdentity { name: &'static str, version: u32 }`
(a const cannot hold a `String`; the records keep the owned
`ResolverIdentityV1`); the plugin's `semantic_hash` is a `String` built at
construction from `R::IDENTITY` (returned as `&str`); `validate_activation`
refuses a rulebook whose `resolver` is not `R::IDENTITY`.

```rust
pub struct ResolverIdentity { pub name: &'static str, pub version: u32 }   // the const form; ResolverIdentityV1 is its owned twin
pub trait StepResolver: Default + Send + Sync + 'static {
    const IDENTITY: ResolverIdentity;
    /// Deterministic in its input and its draws. It receives no view and no
    /// state; the plugin performs every draw it asks for. It must validate
    /// its input before its first draw: an error after a draw is applied as
    /// an empty step (both sides unchanged, posture Continue, term
    /// `resolver_failed`) and the engagement is held, so a drawn address is
    /// never re-drawn with other evidence.
    fn resolve(&self, input: &StepInputV1<'_>, draws: &mut dyn StepDraws) -> Result<StepOutcomeV1, CanwuError>;
}
pub trait StepDraws {
    /// A keyed draw on `step-contingency` with the step packet as evidence,
    /// operation id `contingency:<engagement>:<step index>`, target the
    /// engagement record version, slot `side × 32_768 + participant × 256 +
    /// purpose`; each slot at most once per step, at most limits.draws_per_step.
    fn draw(&mut self, side: Side, participant: u16, purpose: u8, upper_exclusive: u64) -> Result<(u64, RandomDrawId), CanwuError>;
}
pub struct StepInputV1<'a> { pub rulebook: &'a CombatRulebookV1, pub kind: EngagementKind, pub kind_rule: &'a KindRuleV1,
    pub profile: &'a StepProfileV1, pub step_index: u32, pub attacker: SideStepInputV1<'a>, pub defender: SideStepInputV1<'a>,
    pub environment: &'a EnvironmentInputV1, pub terrain: &'a TerrainRowV1, pub fort: Option<(u8, bool)> }
pub struct SideStepInputV1<'a> { pub components: Vec<(&'a ForceId, &'a ComponentId, u64, &'a ComponentValuesV1)>,
    pub terms: SideTermsV1 /* men-weighted over the side's forces */, pub condition: &'a ConditionInputV1, pub condition_age_steps: u32,
    pub cohesion: u32, pub morale: u32, pub attacking: bool }
pub struct StepOutcomeV1 { pub attacker: SideOutcomeV1, pub defender: SideOutcomeV1, pub terms: Vec<TermId> }
pub struct SideOutcomeV1 { pub lines: LinesV1, pub inflicted_per_mille: u32, pub casualties: u64, pub killed: u64,
    pub cohesion_after: u32, pub morale_after: u32, pub morale_shock: u32 }
```

The plugin owns everything around the call: it clamps the outcome to the
rulebook's ranges (term `resolver_clamped`), splits casualties over the
side's components by largest remainder in (force id, component id) order
(killed by the rulebook's split, the rest wounded), evaluates the break rule
(`morale_after < morale_break` → `Withdraw`; `cohesion_after <
rout_cohesion_below` or `morale_after + rout_morale_margin < morale_break` →
`Rout`; a defender behind `fort` of grade ≥ `walled_holds_from_grade` with no
breach or gate open never breaks, neither withdrawing nor routing),
applies pursuit on a rout
(captured `floor(engaged × pursuit_bp × captured_per_mille_at_full / 10^7)`
with `pursuit_bp = terrain.pursuit_bp × night.pursuit_bp / 10^4` on a night
step, wounded left `wounded_today × wounded_left_per_mille / 1000`), appends
the step, and decides continuation: a side `Withdraw` or `Rout` closes the
engagement (both in one step → the attacker's break is read first,
`Repulsed`); the kind rule's `max_minutes` elapsed since the engagement opened
without a break closes `Stalled` (a one-day fight sets `max_minutes` to
`siege.day_minutes`, which is one step on a day profile and 48 on a half-hour
one; a wall assault or relief sets more).

`ParametricStepResolver` (identity `canwu.parametric`, 1) is the four-line
formula with every coefficient from the rulebook, per side: `men = Σ
present`; `engaged = floor(men × frontage_bp / 10^4)` (the kind rule's
frontage, else the terrain row's); `engaged_bp = floor(engaged × 10^4 /
max(1, men))`; `fire_base = floor(Σ present × fire × training_bp × engaged_bp
/ 10^11)`; `shock_base`, `melee_base`, `defense_base = floor(Σ present ×
value × engaged_bp / 10^7)` (training as basis points, so a 0–100 training
host multiplies by 100; these divisors keep a small component's base exact
to the man); `fire = floor(fire_base ×
ranged_reliability_bp × supply_fire_bp × night_fire_bp / 10^12)` with
`supply_fire_bp = lines.supply_fire.bp` when `supply_per_mille <
below_per_mille`, else 10^4, and night factors 10^4 on a day step; `shock =
floor(shock_base × terrain.shock_bp × terrain.charge_window_bp ×
night_shock_bp / 10^12)`; `melee = floor(melee_base × terrain.melee_bp ×
night_melee_bp / 10^8)`; `defense = floor(defense_base × fort_bp ×
terrain.defense_bp / 10^8)` for a defender (`fort_bp` from `fort` per the
kind rule's `FortUse`, halved when breach or gate open or `SiteHalved`;
an attacker's defense is its `defense_base`), halved again when
`defense_halved`; `leadership_bp = clamp(base_bp + battle_command ×
command_weight + pick × pick_weight, min_bp, max_bp)` with `pick` the
attacker's `battle_command` and the defender's `assessment`; `attack = fire
× fire_weight + shock × shock_weight + melee × melee_weight`;
`enemy_defense_term = (enemy_defense + defense_offset) × 10^13` (an `i128`
intermediate; `defense_offset` is added to the enemy's defense line in the
lines' own units, so a host whose formula adds 200 to its defense line sets
200, and with these divisors and `step_scale_bp` 230_400 for 48 compressed
half-hour steps at 48 % the parametric step reproduces such a host's day). The **per-step rate** a side inflicts is the exact fraction
`rate = min(cap, attack × leadership_bp × surprise_bp × posture_bp /
enemy_defense_term)` per mille of the enemy's engaged men, where `cap` is
`inflicted_cap_per_mille` and applies to the unscaled rate; `surprise_bp` (0
means none, 10^4) and `posture_bp` (the first matching `PostureModifierV1`
of the kind rule, scaling the attacker's rate only; else 10^4) apply for
`posture_steps` steps. Every per-step quantity then scales the rate by the
profile's `step_scale_bp` once, in its own single division (a day profile of
48 compressed half-hour steps at 48 % sets `step_scale_bp` 230_400; the
half-hour profile sets 10_000): `inflicted_per_mille = min(1000, floor(rate
× step_scale_bp / 10^4))`; casualties `floor(own_engaged ×
enemy_inflicted_per_mille / 1000)`, killed by `killed_numerator /
killed_denominator`; cohesion loss `floor((rate × per_inflicted + base +
order_confusion × confusion) × step_scale_bp / 10^4)`; morale shock
`max(0, floor(rate × sensitivity × step_scale_bp / 10^5) −
floor(floor(credibility / credibility_divisor) × step_scale_bp / 10^4))` (whole
quanta of credibility, each scaled by the step, so credibility below one
divisor relieves nothing). Every line is an ordered
`i128` product with one floor division; the parametric resolver draws
nothing.

## Composition of force and side state

One writer per field, and one rule for every derived value:

- **Morale.** The host owns morale; the plugin owns the battle's shock.
  `side.morale = condition.morale_per_mille − side.morale_shock` (floor 0),
  where `side.morale_shock` accumulates over the engagement and is copied to
  each force's `battle.morale_shock` at every step. `morale_shock` decays by
  `morale.shock_recover_per_day` for each full day the force did not fight,
  applied lazily when the force is next read (open, join, forecast), and the
  host may clear it after folding it into its own morale (`ReviseForce {
  clear_morale_shock }`).
- **Cohesion.** The plugin owns it once a battle has moved it:
  `force.cohesion = battle.cohesion.unwrap_or(terms.cohesion_cap)`, plus
  `cohesion.recover_per_day` for each full day the force did not fight, up to
  the cap, applied lazily when the force is next read; `side.cohesion` is the
  men-weighted floor mean over the side's forces; after a step each force
  takes `min(side.cohesion_after, own cap)`.
- **Idle days.** Recovery counts only days without a step: `battle.last_fought_at`
  is set at every step the force fights, `recovered_to` at every lazy credit;
  a credit covers the full days from `max(last_fought_at, recovered_to)` to
  now, so a day the force fought never recovers, and the host's own rule (a
  point a day the force did not fight) composes exactly.
- **Side terms** are the men-weighted floor mean of the forces' terms;
  `traits` is their intersection (a trait a side has only when every force
  has it), so `mounted_only` holds only for an all-mounted side.
- **Composition revisions run mid-engagement.** `ReviseForce` while engaged
  is applied before the step of the same boundary: `present` changes take
  effect from that step; `losses` are the plugin's and are kept.
- **Location** is the host's; `PlaceForce` is refused while engaged,
  besieging, or crossing, and a force leaves a fight only through
  `OrderAction { Abandon }`, a break, or the fight's close.
- **Fall and dispersal.** When a siege ends `Fell`, every defender still at
  the site has its remaining `present` moved to `dispersed` losses and
  `battle.dispersed = true`; the host decides what a dispersed force becomes.

## Sieges

A siege is a day-grained process (`siege.day_minutes`) over a site the host
describes (`SiteProfileV1`); its engagements step on the engagement's
profile. Each siege day:

1. **Investment** `sealed = min(accesses, floor(besieger_men /
   men_per_access_sealed))` plus `water_accesses` when `naval` is some with
   `control_bp >= water_access_control_bp`; `investment_bp = floor(sealed ×
   10^4 / (accesses + water_accesses))`.
2. **Emplacement and breach**: `pieces = Σ floor(present × 10 /
   crew_per_piece_x10)` over components with `breach_per_piece > 0`; the
   first day pieces are present sets `emplacement_from`; bombardment starts
   after `emplacement_days[environment.ground]`; then `breach_progress +=
   max(0, floor(Σ pieces × breach_per_piece × work_bp / 10^4) −
   repair_per_day)` (`breach_rate` is the public helper); `breach_open` at
   the grade's `breach_threshold`.
3. **Supply** is the host's (`SupplyInput`/`ReviseSiege.supply`): recorded,
   reported, decided on by nothing in the plugin.
4. **Gate**: `gate_open_until` from `gate_open_steps` halves `fort_bp` for
   those steps and makes `GateOpened` the end cause when the defenders break
   while it is open.
5. **Assault**: when `assault_ordered`, or `storm_on == BreachOrGate` and the
   breach or a gate opens this day (read in the same step, not a day late),
   an `Assault` engagement opens at the site with the kind rule's frontage
   and terrain (`breach_terrain` when the breach is open, else
   `assault_terrain`), `fort = (grade, open)`, the siege's latest per-side
   and environment inputs, and the siege's holders; it runs until a side
   breaks or its `max_minutes`. A **sortie** (`sortie` ordered and
   `sortie_ready_at` passed) opens a `Sortie` engagement (defenders attacking,
   `sortie_terrain`); `sortie_ready_at = now + sortie_cooldown_minutes`. A
   **relief** is an `OpenEngagement { kind: Relief }` or `JoinEngagement` by
   co-located forces against the besiegers, fought before an assault that
   day. A relief that breaks the besiegers, or a sortie that routs them,
   lifts the siege (`Relieved`); a sortie that prevails without routing
   them breaks the siege lines, so the guns re-emplace from the close
   (`bombardment_from` cleared, `emplacement_from` = the close); a repulsed
   sortie or relief changes nothing on the siege.
6. **End**: defenders break at the wall → `Fell` with `Breach`, `GateOpened`,
   or `Storm`, and the remaining defenders disperse (§9); no defender at the
   site → `Abandoned`; besiegers routed at the wall, or broken by a relief →
   `Lifted` with `Relieved`;
   `LiftSiege` → `Withdrawn`. The siege's engagement summary and the signal
   carry `site_fell`; the plugin writes no occupation.

## Contested crossings

One wave per `crossing.wave_minutes` from the first step after the order (a
wave due while the crossing's fight is open, or before an environment input
is set, waits with `next_wave_at` held, and is due from the fight's close or
the input's time):
`men = min(remaining, wave_men)`, `× hard_river_wave_bp` on `river == Hard`,
none on `Closed`. With an interceptor whose `control_bp >= min_control_bp`:
`intercept_bp = floor(control_bp × control_weight / 10^4) + scouts ×
scout_weight − concealment[environment.concealment] − (night ?
night_concealment : 0)` (`intercept_bp` is the public helper). `Threshold`
mode intercepts when `intercept_bp > threshold_bp`; `KeyedDraw` mode draws on
`crossing-interception` (evidence the step packet, operation
`interception:<crossing>:<wave>`, target the crossing record version, slot
the wave, bound 10^4) and intercepts when `intercept_bp > draw`. The draw is
the wave's **last refusable step**: every check (crossing open, force at
`from`, interceptor valid) runs before it, and nothing after it can refuse,
so a staging restore never leaves a drawn address to be re-drawn with other
evidence. An intercepted wave opens a `FerryFight` (the interceptor as
attacker against the wave, the crossing force capped at the wave's men, so
the kind's posture cells scale the interceptor; the kind rule's frontage and
the crossing terrain, the crossing's inputs and holders) and turns back; a landed wave adds to
`landed`; the bridgehead fights the host-named `defenders` at `to` as a
`BridgeheadFight` with `defense_halved` until `landed >= bridgehead_secure_men`.
The crossing completes when the force's men have landed or the force is
gone; the host moves the force (`PlaceForce` to `to`) on completion.

## Reports

Phase 7 queues one `ReportItemV1 { subject: Engagement | Siege | Crossing,
closing: bool, boundary_at }` per stepped or closed subject on
`registry.report_queue`. Progress items are capped at `limits.report_queue`
(when full, the oldest progress item is dropped with
`canwu.military.report_dropped.v1`); closing items have headroom above the
cap, and the whole queue is held to `report_queue + open_engagements +
open_sieges + open_crossings`: a closing item that would pass that bound
drops the oldest progress item first, and the oldest closing item only when
no progress item is left. Activation validates the queue against the same
bound, so a save taken with a full queue always loads. Phase 13
drains the queue in order while `knowledge_publication_budget()` has room
for a whole item, and rewrites the registry with what remains (a declared
write); an item that does not fit waits for the next boundary with
admitted ingress (and the engagement's close is not deferred by it: the
closing item carries the summary's data).

Two schemas, because retention is per schema: `canwu.military:engagement_report`
v1 (`RetireWhenSuperseded`; each side's next progress report supersedes the
last) and `canwu.military:engagement_outcome` v1 (`Keep`; the closing report,
one per side per engagement). Bodies are typed: `EngagementReportV1 {
subject, kind, forecast_band, own_losses: LossTallyV1, enemy_losses_band,
own_channels: Vec<BandIndex>, enemy_strength_band, posture, steps, siege:
Option<SiegeReportV1 { investment_band, breach_band, breach_open,
days_of_stock_band }>, crossing: Option<CrossingReportV1>, as_of }` — own
figures exact, the enemy's as bands from `band_cuts`. One
`KnowledgePublicationGroup` per side to its `holders`. Holders are checked
with `knowledge_holder_accepts_publication` at admission (refused) and again
at publish (skipped), so a person who died, an organization, or a retired
entity record never fails the boundary; a `force` record is itself an
eligible holder (`Entity` class, `holder_policy: Allowed`), so a host may
address a report to the force. The report's `origin.evidence` cites only
versions created this boundary (the `engagement_steps` or `engagement_summary`
version, through `current_domain_record_version`), never the host inputs'
`evidence`, which may be missing after a seal. The plugin reserves no
publication budget by default; `MilitaryPlugin::with_publication_reservation(..)`
registers one through `PluginRegistrar::reserve_knowledge_publication`, which
the descriptor records and `knowledge_publication_budget()` already credits
to the plugin's own publisher, so no system name carries it.

## Signals to the host

At close the plugin creates `engagement_summary` (`EngagementSummaryV1 {
engagement, kind, site, opened_at, closed_at, steps, outcome, site_fell,
attacker: SideSummaryV1 { forces, losses, routed: Vec<ForceId> }, defender,
siege: Option<SiegeId>, crossing: Option<CrossingId> }`) and emits
`canwu.military.engagement_closed.v1` with the forces and site as `affected`.
A host that wants a packet registers
`MilitaryPlugin::with_outcome_signal(target_plugin, packet_type)` (encoded in
the settle system's name and declared as a `plugin_ingress_targets` entry):
the settle system then enqueues `EngagementOutcomeSignalV1` (the summary plus
each force's `LossTallyV1` delta of the boundary) as next-boundary plugin
ingress to that target. The target must be an ingress the named plugin
registered; the engine finds an undeclared target only when the first signal
is scheduled and fails that boundary, so the starter registers one and the
lifecycle test exercises it (§16 lists it).

## Determinism

- **Draws.** Two streams, each declared by the settle system:
  `crossing-interception` and `step-contingency`. Every draw is keyed with
  the admitted step packet as evidence (`EvidenceRef::Ingress`), which exists
  in every run, so a live, sealed, restored, and replayed run accept it
  alike; an address already drawn returns its value. The `RandomDrawId`s are
  stored on the wave and the step. When two step packets are admitted in one
  cut, every draw cites the first admitted one. A draw is never followed by a
  refusal that could restore staging (the crossing's last refusable step; the
  resolver's error-after-draw rule), so an address is never re-drawn with
  other evidence. Keyed draws stay in the keyed-draw index for the run
  (engine finding G-71); a crossing of ten waves and a fight of a few
  hundred contingency draws are small against the index's cost.
- **Arithmetic.** Every line is an ordered `i128` product with one floor
  division; a line that needs two names an intermediate (`engaged_bp`,
  `*_base`, `enemy_defense_term`). Allocation of casualties over components
  is largest remainder in (force id, component id) order. Per-mille and
  basis-point integers throughout.
- **Order.** Commands in admission order; engagements, sieges, crossings in
  id order; sides attacker then defender; components in id order; holders
  in `KnowledgeHolderRef` order; the break rule reads the attacker first.
- **No retained-evidence or host-version decisions.** The plugin reads the
  current versions of its own records and the `host_revision` a command
  names; the `evidence` fields on inputs are cited on the step and never
  read or checked.

## Persistence, continuation, and replay

| Item | 0.19.0 | Proposed |
| --- | --- | --- |
| Record kinds | `catalog`, `force`, `operation`, `combat`, `occupation`, `knowledge`, `provider_outcome`, `ledger` | `rulebook` (create-only), `registry`, `force`, `engagement`, `engagement_steps`, `siege`, `crossing` (versioned), `engagement_summary` (create-only); all schema version 1 of the new shapes; the old payloads do not load, by the same-version policy |
| Knowledge schemas | `canwu.military:military_report` v1, payload `Any` | `canwu.military:engagement_report` v1 (`RetireWhenSuperseded`) and `canwu.military:engagement_outcome` v1 (`Keep`), typed payloads, new `schema_hash`es |
| Random streams | `military-operation` | `crossing-interception`, `step-contingency` |
| Ingress | `military_command_v1` (internal), `military_provider_ack_v1` | `military_apply_v1` and `military_step_v1` (internal); the acknowledgement ingress is removed |
| Plugin descriptor | `0.1.0`, hash `f07c44f9…3810` | `0.20.0`; `semantic_hash` = BLAKE3 of `canwu-military plugin semantic hash 0.20.0: engagements <R::IDENTITY.name>@<version>` (a `String` held by the plugin) |
| Host choices | plugin-scheduled ticks | the wake system's cadence (its contract), the signal target (the settle system's name), the publication reservation (the registrar): descriptor-recorded, restored and replayed only with the same |
| Scenario | `MilitaryCatalog` record `root` | a `rulebook` record and `force` records installed by the scenario (`record_from` kept), or by command |

Continuation: every record a boundary reads is a current record; the steps
record carries one boundary's steps and the journal the history; the
registry names the open work, the retire queue, and the report queue, so a
paged restore touches the open engagements only. Exact replay re-runs the
same commands, step packets, and draws; a fork with different inputs
diverges at its first different command. `validate_activation` checks
shapes, limits, the rulebook hash, and the resolver identity; it recomputes
no step digest (the inputs are not retained) and reads no host record
version.

## Refusals and what can still fail a boundary

Recorded rejections (the boundary stands; `canwu.military.command_rejected.v1`
with the code and a bounded message): unknown or retired force, a force not
at the site, a force already engaged, a stale `expected_host_revision`, an
issuer that is no controller and not trusted, a side with no forces, a
holder the engine would not accept, more forces, components, holders, draws,
or open work than the limits, a rulebook not installed or a second
`InstallRulebook`, an input whose channel count differs from the rulebook's,
an unknown posture, profile, terrain, or kind id, a `Step` with `up_to`
before the boundary, a sortie before `sortie_ready_at`, a `PlaceForce` or
`RetireForce` of an engaged force, and a host integer past the plugin's
bounds (`ValueOutOfRange`, at admission and again where phase 7 applies it):
`present` above 10^9 a component, a component value or side term above
10^5, a basis-point input above 10^5, and in the rulebook a step scale above
10^7, a weight above 10^3, a basis-point parameter above 10^5, or a forecast
cap, delta, or band value above 10^6 in magnitude. With those bounds every
product of the step fits an `i128`, and the arithmetic saturates rather
than overflows past them.

Held, not failed (`canwu.military.engagement_held.v1`; the engagement steps
no further until `SupplyInput`, `ReviseStepProfile`, or `OrderAction {
Abandon }`): a step whose encoded `StepV1` exceeds `step_bytes`, a resolver
error (before a draw: nothing applied; after a draw: an empty step applied,
§8), more draws than `draws_per_step`.

What can still fail a boundary: a plugin-owned record that fails to decode,
validate, or encode (a corrupted or hand-edited record); a record store
version overflow; a host resolver that panics (host code); and an outcome
signal whose target ingress no plugin registered, found when the first
engagement closes (documented on `with_outcome_signal`, exercised by the
lifecycle test with a registered target and, inline, with an unregistered
one). Nothing in this design turns a refusal into a failed boundary.

## Budgets and scale

Cardinalities (reference limits): forces ≤ 4,096 registered (never scanned;
the registry counts them); open engagements ≤ 8, sieges ≤ 8, crossings ≤ 4;
forces per side ≤ 16 and components per force ≤ 64, so a side may hold up
to 1,024 components (the reference starter and the downstream's known need
are two to four forces of a dozen components a side); holders per side ≤ 8;
draws per step ≤ 64.

Hot path per boundary: a boundary without a step packet reads `core_ingress`
and exits. A stepping boundary reads the rulebook (≈ 8–12 KB, decoded once),
the open engagements, their forces, and the inputs; a step is integer
arithmetic over the side's components (microseconds at a dozen,
sub-millisecond at a thousand). Cost is `O(open engagements × steps ×
components engaged)`.

Bytes per engagement-day (journal keeps previous and current versions; a
force record is ≈ 0.5 KB at eight components and ≈ 10–16 KB at sixty-four):
engagement record ≈ 1.5 KB × 2; the steps record ≈ `step_bytes` × steps × 2;
engaged force rewrites × 2 each; two report bodies ≈ 0.4 KB. At the
downstream's shape (one step a day, two to four small forces) ≈ 7–10 KB per
engagement-day; at 48 steps ≈ 100 KB (the steps record and its previous
version dominate). At the reference limits the worst boundary is several
megabytes (8 engagements × 32 forces × 16 KB × 2 alone is 8 MB), so the
limits are a host's choice, not a claim: a host sizes `forces_per_side`,
`components_per_force`, and the step profile to its own evidence line.

Benchmark matrix (package A measures, inline, before integration; the
numbers are reported in the hand-back, not claimed here): dimensions open
engagements {1, 2, 8} × forces per side {2, 16} × components per force {8,
64} × steps per boundary {1, 48}; measured per boundary: settle time,
allocations, record bytes written (current and previous), the steps record
size, restore time of a save taken mid-fight, and exact replay equality.

Growth: the owned record set grows by ≈ 2 KB per closed fight (the retired
engagement, its emptied steps record, its summary) and by one record per
force; nothing removes them in 0.20.0, and the kernel validates every owned
record at each load, retired ones included, so a load's cost grows linearly
with the fights ever closed — about 200 KB and three hundred records after a
hundred fights, a few milliseconds of validation. The plugin's own
`validate_activation` decodes active records and the registry only. A
campaign of thousands of fights is a host that needs the record store's
archive, which is a later proposal, not this one.

## What is removed and what the reference crates become

Removed from `canwu-military` (with the reason):

- `MilitaryCatalog`, `MilitaryNodeId`, `MilitaryNodeProfile`, `CommanderProfile`
  — a private world and commander axes; the host's entities and content.
- `OperationState`, `OrderMarch`, `PlanOperation`, `ExecuteSpecialOperation`,
  `Recon`, `PrepareAmbush`, `AdvanceTick` — marching is `canwu-movement`,
  reconnaissance and ambush are the host's detection on `canwu-knowledge`,
  special operations are host content; the self-scheduled tick goes with the
  step packet.
- `CombatState`, `CombatResult`, `RandomEnvelope`, `resolve_combat_round` —
  the fixed formula and the five-value result.
- `OccupationState`, `IntegrationStage`, `EstablishOccupation`,
  `SetOccupationPolicy`, `MilitaryAdministrationAction`,
  `advance_occupation_state` — control dimensions and an integration ladder
  belong to the host's authority domain.
- `Recruit`, `TrainAndEquip`, `SubunitState`, `RecruitmentProfileV1`,
  `BranchProfileV1`, `TacticProfileV1`, `TerrainModifierV1`,
  `CombatProfileV1`, `OccupationProfileV1`, `MilitaryRulesetV1` — institution
  rules and the old ruleset; the rulebook replaces the parts that are combat
  parameters.
- `MilitaryKnowledge`, `KnowledgeFact`, the `military_report` schema — the
  typed report schemas replace them.
- `ProviderOutcome`, `PendingMilitaryEffect`, `MilitaryLedger`,
  `MilitaryOutcome`, `military_provider_ack_v1`, `enqueue_provider_outcome`
  — the saga duplicated `canwu-resource`'s lease and acknowledgement
  contract, and the ledger grew for the life of the run.
- `MilitaryOperationKey` — the kernel's request identity and
  `expected_host_revision` are the idempotency record.
- `tests/lifecycle.rs` — it tests the removed model.

Kept in spirit: the namespaced id types (`ForceId`, …), `record_from` for
scenario installation, the admit-then-apply split with staged reads
(`Staging`), recorded rejections, and the admitted-packet draw evidence.

`canwu-military-reference-content` becomes one function,
`synthetic_rulebook_v1() -> CombatRulebookV1` (identity
`canwu:military:rulebook:synthetic:v1`, explicitly synthetic: six terrain
rows, five fortification grades, five band cuts, a `Field`/`Camp`/`March`/
`Defend` posture list with a few modifiers, two step profiles (`day`,
`half-hour`), the six kind rules, the reference limits, resolver
`canwu.parametric` 1), with `validate` and the hash helper;
`riverine_preindustrial` and `industrial_front` are deleted (their branch,
recruitment, and occupation profiles have no reader).
`canwu-military-reference` becomes the runnable starter on the reference
world through `canwu-api` only: `demo_engagement_scenario()` installs the
rulebook and two forces at a reference place; the `military_starter` example
registers a tiny outcome-signal target plugin, opens a `Field` engagement,
supplies inputs, steps three days by the wake cadence, invests the place,
breaches and storms it, orders a crossing against an interceptor, promotes
one fight to the half-hour profile, and prints the summaries and a
government holder's reports; it composes `canwu-reference-world`'s movement
plugin to show an arrival followed by `PlaceForce`.
`canwu-force-supply-reference` is not military combat and is unchanged by
this proposal; `military-domain-design.md` is marked superseded by this
proposal.

## Adoption by a host with its own combat

A host that already resolves fights in its own plugin adopts the engine in
this order, each step mechanical:

1. **Compile its content into a rulebook**: its terrain matrix, night
   factors, fortification grades, band cuts, posture modifiers, casualty
   split, break thresholds, and step lengths become `CombatRulebookV1`
   parameters; its per-kind frontages and one-day fights become
   `kinds[kind].frontage_bp` and `max_minutes`; its daily step with a compressed
   scale and its high-fidelity step become two `step_profiles`. If its
   formula is the four-line shape, it configures `ParametricStepResolver`;
   otherwise it implements `StepResolver` once, drawing through `StepDraws`.
2. **Mirror forces**: `RegisterForce` with its components' values and side
   terms derived from its own unit profiles, institutions, and commanders;
   `ReviseForce` on muster, desertion, or wounded returns; `PlaceForce` on
   each arrival it reads from `canwu-movement` or its own movement; `ReviseForce
   { clear_morale_shock }` after its own morale review folds the shock.
3. **Open fights from its own contact rules**: `OpenEngagement`,
   `InvestSite`, `OrderCrossing` with the holders its report model names and
   the first inputs; thereafter one `SupplyInput` per engaged side and one
   per environment each day, derived from its own supply, pay, command, and
   weather records (cited as `evidence`), in the same batch as a `Step` when
   it drives stepping itself, or before the wake cadence's boundary otherwise.
4. **Decide what the engine does not**: betrayal and incidents (`ReviseSiege
   { gate_open_steps }`), doctrine (`ReviseSiege { assault }`, `storm_on`),
   ration policy (`SupplyInput`/`ReviseSiege.supply`), naval control and
   patrols (`naval`, `interceptor`), promotion (`ReviseStepProfile`).
5. **Apply the outcomes**: read `engagement_summary`, `siege`, and `crossing`
   one boundary old (or take the outcome signal), and write its own
   occupation, control, population, store, and report facts from them;
   publish its own fidelity-graded knowledge where the engine's banded
   reports are not enough.

What stays in the host by design: detection and sightings, hostility,
orders and their delivery, doctrine, pay and autonomy, supply stores, control
and occupation, player text. A host that keeps a parallel force record keeps
the fields the engine does not own and reads battle state back from the
engine's `force` record; no field is written by both.

## Implementation plan

Two worker packages, integrated together (the workspace does not build
between them: the reference crates compile against package A's new API).

**Package A — `canwu-military` core.** Edit boundary:
`crates/extensions/canwu-military/**` only (`Cargo.toml` adds nothing new;
`canwu-api` is unchanged — the extension's public API is its crate root, as
for every extension). Work: delete `model.rs`, `plugin.rs`, and
`tests/lifecycle.rs`; add `ids.rs` (id types, `validate_id`, `record_from`),
`rulebook.rs` (`CombatRulebookV1`, `CombatLimitsV1`, validation,
`rules_hash`), `force.rs`, `engagement.rs`, `siege.rs`, `crossing.rs`,
`resolver.rs` (`StepResolver`, `StepDraws`, `ParametricStepResolver`, the
public helpers `forecast`, `intercept_bp`, `breach_rate`,
`largest_remainder`), `report.rs`, `plugin.rs` (`MilitaryPlugin<R>`,
registration, admission, `Staging`, settle, wake, publish, signal, the
system-name encodings); README rewritten. Durable tests (two):
`tests/engagement_lifecycle.rs` — rulebook and two forces installed, a
`Field` engagement stepped three days by the wake cadence with changing
inputs and a `ReviseForce` mid-fight, a siege invested, emplaced, breached,
stormed on `BreachOrGate`, and fallen with a relief fought first and the
defenders dispersed, a crossing whose first wave is intercepted (`Threshold`)
and whose second lands, one fight promoted to the half-hour profile, reports
to a person and a government holder and to a force, the outcome signal
received by a registered target, a `Step`-driven variant with
`with_step_cadence(None)` giving the same records, the run saved mid-siege
and restored with equal state and checkpoint hash, exact replay equal, and
the `KeyedDraw` variant with a contingency-drawing test resolver reproducing
on a re-run; `tests/refusals_hold_the_boundary.rs` — every rejection of §16
recorded with the boundary standing, a force placed and opened in one
boundary, a stale host revision after a step in the same instant **not**
raised, an oversized step and a failing resolver holding the engagement and
`Abandon` freeing its forces, an unacceptable holder refused at admission and
a holder who dies mid-fight skipped, and a rulebook naming another resolver
refused at activation. The benchmark matrix of §17 runs inline. Checks:
`cargo test -p canwu-military`, `cargo fmt --all -- --check`, `cargo clippy
-p canwu-military --all-targets -- -D warnings`.

**Package B — reference crates and documentation.** Edit boundary:
`crates/reference-content/canwu-military-reference-content/**`,
`crates/integrations/canwu-military-reference/**`, `docs/architecture.md`
(the extension paragraph), `docs/end-state.md` (the military sentences),
`docs/terminology.md` (the `canwu-military` row and the new rows below),
`docs/proposals/military-domain-design.md` (status line: superseded),
`crates/README.md` (the `extensions/` row), `README.md` and `README.zh-CN.md`
(the `canwu-military` bullet), `website/src/content/docs/{,en/}tutorials/military-domain.mdx`
(rewritten on the starter), `website/src/content/docs/{,en/}architecture/{military,systems,index}.mdx`,
`website/src/content/docs/{,en/}developer/{extensions,index}.mdx`,
`website/src/content/docs/{,en/}reference/terminology.mdx`, and
`agent-interface/plugins/canwu-engine/skills/**` where a note names
`canwu-military` commands. Work: `synthetic_rulebook_v1`, the starter
scenario and example, the documentation mirrors. Checks: `cargo test -p
canwu-military-reference-content -p canwu-military-reference`, `cargo run -p
canwu-military-reference --example military_starter`, fmt, clippy on the two
crates. Not in either package: `docs/versioning.md`, the root version, the
website build and deploy, `opus-progress.md` (the orchestrator's release
commit).

## Documentation and terminology delta

For the release commit (canonical page and both website languages each):

- `docs/architecture.md`: the `canwu-military` sentence becomes "the
  engagement engine: typed forces in the host's world, stepped engagements
  on a host-installed rulebook and a `StepResolver`, sieges and contested
  crossings, holder-relative engagement reports, and an outcome signal; it
  owns no doctrine, catalogue, control, or store". The line "`canwu-military`
  cites the admitted packet of every keyed draw" stays true.
- `docs/end-state.md`: the military paragraph gains "Since 0.20.0 the
  military extension is an engagement engine whose every deciding number is
  a host rulebook parameter; it writes no occupation or control."
- `docs/terminology.md` row `canwu-military`: "The optional engagement
  engine: forces, stepped engagements, sieges, crossings, and engagement
  reports on a host-installed rulebook; it is not a universal combat model
  and owns no control or occupation."
- `docs/versioning.md`, 0.20.0 entry, "Upgrading from 0.19.0": *`canwu-military`
  is rewritten. The catalog, operation, combat, occupation, knowledge,
  provider-outcome, and ledger records, the `military-operation` stream, the
  `military_report` schema, the acknowledgement ingress, and the commands
  `OrderMarch`, `PlanOperation`, `Recruit`, `TrainAndEquip`, `Recon`,
  `PrepareAmbush`, `ExecuteSpecialOperation`, `EstablishOccupation`,
  `SetOccupationPolicy`, and `MilitaryAdministrationAction` are gone. A host
  installs a `CombatRulebookV1` (scenario or `InstallRulebook` from a
  `System` issuer), constructs `MilitaryPlugin::<R>::new()` with
  `ParametricStepResolver` or its own `StepResolver` type, chooses
  `with_step_cadence(Some(cadence))` or drives stepping with the trusted
  `Step { up_to }` command, registers forces with `RegisterForce`, places them
  on arrival with `PlaceForce`, opens engagements, sieges, and crossings by
  command, supplies condition and environment inputs per engagement-day, and
  applies the `engagement_summary` or the optional outcome signal. Commands
  name a `host_revision` that steps never advance. Issuers are matched
  against a force's `controllers` (any `Issuer` kind) or the rulebook's
  `trusted_issuers`. No 0.19.0 military record loads.*
- Terminology candidates (the host-side review accepted these; the owner
  confirms):

| English | Code | 中文 (proposed) | Meaning |
| --- | --- | --- | --- |
| engagement | `EngagementStateV1`, `OpenEngagement` | 交战 | A stepped fight between two sides at a host site, opened by command and closed into a summary. |
| engagement step | `StepV1`, `EngagementStepsV1` | 战斗步 | One resolution of the step formula; the per-step evidence. |
| combat rulebook | `CombatRulebookV1`, `InstallRulebook` | 战斗规则册 | The host's versioned, hashed parameters every step reads. |
| step resolver | `StepResolver`, `ParametricStepResolver` | 战斗步结算器 | The deterministic function from step input and draws to step outcome; the crate's parametric one or the host's. |
| siege | `SiegeStateV1`, `InvestSite` | 围城 | Investment, emplacement, breach, assault, sortie, relief, and fall of a host site. |
| contested crossing | `CrossingStateV1`, `OrderCrossing` | 渡河作战 | Waves against an interceptor into a bridgehead. |
| condition input | `ConditionInputV1`, `SupplyInput` | 战斗条件输入 | The host-supplied per-side values a step reads, cited to the host records they came from. |
| pre-battle forecast | `ForecastV1`, `ForecastBandId` | 战前预判 | The opening ratio's band from the rulebook's cuts. |

## Known limits

Ships in 0.20.0: forces, the stepped engagement with the parametric resolver
and the resolver trait with draws, step profiles and promotion, per-kind
rules, the forecast, break and pursuit rules, sieges (investment,
emplacement, breach, gate, assault, sortie, relief, fall, lift), contested
crossings (waves, threshold or keyed interception, ferry and bridgehead
fights), engagement and outcome reports, the outcome signal, the wake
cadence and the `Step` command, the two tests, the synthetic rulebook, and
the starter.

Recorded as limits (not in 0.20.0):

- **No failure-mode model.** The resolver may draw and record contingency
  terms, but the plugin models no risk source or probability band; a host
  that wants bounded contingency builds it in its resolver from the inputs
  it supplies.
- **Two sides.** An engagement has one attacker and one defender side; a
  third party fights as a separate engagement at the same site, in id order.
- **No automatic contact.** Hostility, detection, and surprise are the
  host's; the plugin only checks co-location.
- **Vessels are components.** A fleet fight uses the same four lines with
  `vessel` components; no boarding, wind, or current channel exists beyond
  what the host folds into component values and `ranged_reliability_bp`.
- **One wave size.** Waves are `wave_men` scaled on a hard river; hull
  capacity is the host's, and a crossing whose hulls change mid-way is
  cancelled and re-ordered. A per-wave size on `ReviseCrossing` is the
  natural follow-up.
- **Reports are bands.** The enemy's figures are bands from `band_cuts`; a
  host that wants fidelity classes (exact, band, rumour) publishes its own
  knowledge from the records.
- **Retired records stay in the store** and are validated at every load (the
  engine's lifecycle; the archive reachability participant marks plugin
  archive objects only, of which this plugin has none); a closed fight leaves
  ≈ 2 KB for the run's life, and the plugin bounds its active set, not the
  store.
- **Keyed draws stay indexed** for the run (engine finding G-71).
- **No second independent host yet.** Criterion (f) of the gap is met only
  when a host other than the reference starter adopts the crate.

## Open questions

Taken from the host-side review (its answers, which this revision adopts):
the thin force record with the composition rules of §9; the resolver trait
with the parametric default and the draw accessor; the wake pattern as the
default but optional beside the `Step` command; the admitted step packet as
draw evidence; the summary always and the signal optional; the Chinese terms
of §21 with 战前预判; deleting the two synthetic profiles and keeping
`canwu-force-supply-reference`.

Still the owner's:

1. **Whether the plugin owns a force record at all** (the thin record of §9),
   now that both reviews accept it with the composition rules — the
   alternative (sides by reference, the host applying deltas) is recorded as
   the rejected shape.
2. **The default step cadence** of the reference starter (`Daily` with the
   wake system, or none with `Step`), which is what the tutorial teaches.
3. **Whether `Step` should also be accepted from a side's controller** (a
   player stepping their own fight) or stay trusted-only as designed.
4. **The Chinese terms** of §21 for the public term sheet.

## Review record

Engine review (not ready → fixed): (1) resolver reachability → type
parameter `MilitaryPlugin<R>`, `R::IDENTITY`, name-encoded signal target; (2) event-driven stepping → step only on an admitted `military_step_v1`
packet, wake without early exit after `can_schedule_ingress`, `earliest_due`
on the registry, `host_revision` not advanced by steps; (3) step-record
lifecycle → one versioned steps record per engagement, summary durable,
retire without successor from a registry queue, `validate_activation` on
active records only; (4) reports → two schemas, `knowledge_holder_accepts_publication`
at admission and publish, a registry report queue drained in phase 13 with
declared writes, origin citing this boundary's versions only; (5) gaps →
siege and crossing inputs handed to the fights they open, `Held` closable by
`Abandon`, `InstallRulebook` by scenario or `System` issuer, posture
modifiers as a list, every named field defined, the contradictions resolved
(per-step bytes, per-kind `max_steps`, one rulebook with step profiles, a
sortie that breaks the besiegers lifts the siege). Non-blocking: the draw as
the wave's last refusable step and the resolver's error-after-draw rule;
components and bytes restated with the benchmark matrix; no `input_digest`
at activation; the signal-target failure documented and tested; packages
integrated together, `tests/lifecycle.rs` deleted, the documentation scope
widened, `military-domain-design.md` superseded.

Host-side review (fits after fixes → fixed): (1) `SupplyInput { target }`
and inputs on sieges and crossings, per-kind terrain; (2) `kinds` with
`frontage_bp`, `max_steps`, posture modifiers, and `fort`, an `Assault` with
`fort` standing without a siege; (3) the composition rules of §9 and
`ReviseForce` while engaged; (4) `step_profiles`, `OpenEngagement.profile`,
`ReviseStepProfile`; (5) `StepDraws` on `step-contingency`. Non-blocking:
the `Step` command reconciled with the engine's fix in §7; posture modifiers
keyed with the fortification state; `storm_on`; `naval` on the siege; the
supply-fire threshold; dispersal on `Fell`; the public helpers; 7–10 KB per
engagement-day at the host's shape.

Delta review of the revision (engine: three small blocking points; host:
fits): the publisher declares `engagement_summary`; the steps record is
rewritten empty at close and the growth and load claims restated (no archive
removes retired records; ≈ 2 KB per closed fight, validated at every load);
`R::IDENTITY` holds `&'static str` with `ResolverIdentityV1` as the owned
form in records. Non-blocking taken: closing report items have headroom
above `report_queue`, checked so at activation; two step packets in one cut
cite the first admitted and step to the larger `up_to`; the publication share
left the settle system's name; `kinds[*].max_minutes` replaces `max_steps`;
cohesion and shock recover only on full days the force did not fight.

## Implementation notes (0.20.0)

The implementation keeps the reviewed contract and corrected these details
where the engine's source or the arithmetic proved the text wrong; the
sections above are updated to match.

- **One scaling per quantity.** The reviewed formula scaled
  `inflicted_per_mille` by `step_scale_bp` and then scaled the cohesion and
  morale lines by it again, which made them quadratic in the step scale. The
  resolver now computes the unscaled per-step rate as an exact fraction
  (capped by `inflicted_cap_per_mille`) and every per-step quantity —
  inflicted, cohesion loss, morale shock, and the credibility relief — scales
  it once (§8). `cohesion.confusion` multiplies `order_confusion` (the host
  passes how many sources of confusion a side suffers).
- **Walls hold.** A defender behind a closed fortification of grade
  `walled_holds_from_grade` or more never breaks (neither withdraws nor
  routs), as the host's model has it; the text said "never routs from
  morale".
- **The steps record at close.** The engine takes one mutation per record per
  boundary, so the closing boundary's steps are written as usual and the
  record is rewritten empty at the next stepping boundary
  (`registry.steps_to_empty`, counted in `earliest_due`). `RetireForce` sets
  the force's `status` at once and retires the record at the next stepping
  boundary for the same reason; the retire queue is a sorted
  `Vec<RetireItemV1 { at, record }>`.
- **Settle writes the rulebook kind**: an applied `InstallRulebook` creates
  the rulebook record in phase 7.
- **Revisions that withdraw** use `InterceptorRevisionV1 { Set(input) |
  Withdraw }` on `ReviseSiege.naval` and `ReviseCrossing.interceptor`: a JSON
  payload cannot tell `None` from `Some(None)`. `LocationV1::At` is the
  struct variant `At { site }` (the enum is internally tagged). `RiverBand`
  is an enum (`Open | Hard | Closed`).
- **Admission and application.** Admission checks the payload, the sides
  (non-empty, disjoint, a holder on one side only), the holders
  (`knowledge_holder_accepts_publication`), the host entities a command
  names (the engine refuses a command naming a missing entity), and the
  authority of `Step` and `InstallRulebook`; every other check runs in
  phase 7 against the staged state and is a recorded rejection whose summary
  begins with the error code. Siege and crossing ids are at most 160 bytes,
  so the ids of the fights they open (`<id>/<kind>-<n>`) fit.
- **Forecast.** `TermSource` is `Manpower | Readiness | Supply | Morale |
  Cohesion | Channel { index } | Training | Equipment | Command | TerrainFit
  | Environment`; numeric terms are `min(cap, floor(value / divisor))`, the
  terrain and environment terms clamped sums. `TerrainFitRowV1` gains
  `defending: Option<bool>`; `ForecastV1` records both totals. `band_cuts`
  are strictly descending per-mille cuts; a report's enemy figures are bands
  of per-mille shares of the enemy's men at the opening.
- **Fights the plugin opens.** A ferry fight is the interceptor attacking
  the wave (the crossing force capped at the wave's men,
  `SideStateV1.men_cap`), so the kind's posture cells scale the interceptor;
  a bridgehead fight is the landed men against the free defenders at `to`.
  An assault repulsed with the besiegers routed, a sortie that routs them,
  or a relief that breaks them lifts the siege (`Relieved`); a sortie that
  prevails without a rout re-emplaces the guns; a siege whose defenders are
  gone falls (`Abandoned`), one whose besiegers are gone lifts
  (`Withdrawn`). A held engagement resumes from the time of the
  `SupplyInput` or `ReviseStepProfile` that frees it. A defender's `Abandon`
  is a `Withdraw`.
- **Review fixes.** Host integers are bounded at admission and in
  `validate` (§16) and the step's arithmetic saturates; the base lines use
  the finer divisors 10^11 and 10^7 and the defense term 10^13, so a small
  component keeps its precision and a host's own offset and day scale map
  directly; a wave held by an open fight or a missing environment waits
  instead of losing its slot.
- **Report bodies.** One typed `EngagementReportV1 { subject, side, closing,
  engagement?, siege?, crossing?, as_of }` serves both schemas; the
  defenders' report carries their exact `own_days_of_stock` instead of a band.
- **Measured** (release build, one stepping day boundary, the benchmark
  matrix of §17 with 200 men a component): 1 engagement × 2 forces a side × 8
  components × 1 step settles in about 1 ms with 21 KB of plugin records;
  8 × 16 × 64 × 48 steps in about 190 ms with 5 MB (force records dominate)
  and a 1.9 s restore; a 48-step steps record is 23–33 KB and one step
  encodes to about 0.95 KB. Restore and exact replay were equal in every
  cell. An outcome-signal target no plugin registered fails the first
  closing boundary, as §16 states.
