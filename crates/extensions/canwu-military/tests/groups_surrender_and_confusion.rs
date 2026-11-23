//! The 0.24.0 revisions: one engagement per command group against shared
//! forces (G-100), a garrison that surrenders during an assault (G-101), and
//! an order-confusion cost prorated by step length (G-102).

#![allow(clippy::too_many_lines)]

mod support;

use canwu_api::{Canwu, CanwuError, IngressPayload};
use canwu_military::*;
use support::*;

fn engagement_id(value: &str) -> EngagementId {
    EngagementId::new(format!("test:engagement:{value}")).unwrap()
}

fn siege_id() -> SiegeId {
    SiegeId::new("test:siege:keep").unwrap()
}

/// Every side of every step loses 10 men and 7 cohesion, and nothing else:
/// what each engagement does to the shared forces is plain.
#[derive(Clone, Copy, Debug, Default)]
struct FixedResolver;

impl StepResolver for FixedResolver {
    const IDENTITY: ResolverIdentity = ResolverIdentity {
        name: "test.fixed",
        version: 1,
    };

    fn resolve(
        &self,
        input: &StepInputV1<'_>,
        _draws: &mut dyn StepDraws,
    ) -> Result<StepOutcomeV1, CanwuError> {
        let side = |side: &SideStepInputV1<'_>| SideOutcomeV1 {
            lines: LinesV1 {
                men: side.men(),
                engaged: side.men(),
                ..LinesV1::default()
            },
            inflicted_per_mille: 0,
            casualties: 10,
            killed: 0,
            cohesion_after: side.cohesion.saturating_sub(7),
            morale_after: side.morale,
            morale_shock: 0,
        };
        Ok(StepOutcomeV1 {
            attacker: side(&input.attacker),
            defender: side(&input.defender),
            terms: Vec::new(),
        })
    }
}

fn fixed_plugin() -> MilitaryPlugin<FixedResolver> {
    MilitaryPlugin::<FixedResolver>::new().with_outcome_signal(HOST_PLUGIN, OUTCOME_INGRESS)
}

fn fixed_run(plugin: &MilitaryPlugin<FixedResolver>, forces: &[ForceStateV1]) -> Host {
    let rulebook = rulebook(&FixedResolver::IDENTITY, InterceptMode::Threshold);
    let records = scenario_records(&rulebook, forces).unwrap();
    let canwu = Canwu::new_with_plugins(11, world(records), &[plugin, &HostPlugin]).unwrap();
    Host::new(canwu, Drive::Wake)
}

fn steps(canwu: &Canwu, id: &EngagementId) -> Vec<StepV1> {
    canwu
        .typed_domain_record(&engagement_steps_reference(id))
        .unwrap()
        .decode_payload::<EngagementStepsRecord>()
        .unwrap()
        .steps
}

fn signals(canwu: &Canwu) -> Vec<EngagementOutcomeSignalV1> {
    canwu
        .ingress_log()
        .iter()
        .filter_map(|record| match &record.payload {
            IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } if plugin == HOST_PLUGIN && packet_type == OUTCOME_INGRESS => {
                Some(serde_json::from_value(payload.clone()).unwrap())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn two_command_groups_fight_one_enemy_force_as_two_engagements() {
    let plugin = fixed_plugin();
    let forces = [
        force("red-a", 1, &red(), 1, &[("infantry", 2_000, infantry())]),
        force("red-b", 1, &red(), 1, &[("infantry", 1_000, infantry())]),
        force("blue-e", 2, &blue(), 1, &[("infantry", 900, infantry())]),
    ];
    let mut host = fixed_run(&plugin, &forces);
    let group = |value: &str, force: &str| CommandGroupV1 {
        id: engagement_id(value),
        forces: vec![force_id(force)],
        holders: Vec::new(),
        input: condition("field", 900),
    };
    let open = OpenGroupEngagementsV1 {
        kind: EngagementKind::Field,
        profile: None,
        site: site(1),
        fort: None,
        side: Side::Attacker,
        groups: vec![group("front-b", "red-b"), group("front-a", "red-a")],
        shared: vec![force_id("blue-e")],
        shared_holders: Vec::new(),
        shared_input: condition("defend", 700),
        environment: environment(false),
        max_minutes: Some(2 * 1_440),
    };
    host.day(
        0,
        &[(
            red(),
            MilitaryCommandV1::OpenGroupEngagements(Box::new(open)),
        )],
    );
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );
    let (front_a, front_b) = (engagement_id("front-a"), engagement_id("front-b"));
    let membership = EngagementGroupV1 {
        shared: Side::Defender,
        engagements: vec![front_a.clone(), front_b.clone()],
        shared_opened_men: 900,
    };
    for id in [&front_a, &front_b] {
        assert_eq!(
            engagement(&host.canwu, id).group.as_ref(),
            Some(&membership)
        );
    }
    // The shared force opens against both, split 2:1 by the groups' men.
    assert_eq!(
        [
            engagement(&host.canwu, &front_a).defender.opened_men,
            engagement(&host.canwu, &front_b).defender.opened_men,
        ],
        [600, 300]
    );
    assert_eq!(
        force_state(&host.canwu, "blue-e").battle.engaged_in,
        Some(front_a.clone())
    );

    // Day 1: both engagements step on the same day, interleaved. The enemy's
    // men are split afresh before each step (front-b's share after front-a's
    // step: 890 men over 1,990 : 1,000), each loss lands once, and its
    // cohesion is one figure both steps move.
    host.day(1, &[]);
    for id in [&front_a, &front_b] {
        let state = engagement(&host.canwu, id);
        assert_eq!(state.status, EngagementStatus::Open, "{state:#?}");
        assert_eq!((state.steps_resolved, state.last_step_at), (1, at_day(1)));
        assert_eq!(state.defender.losses.total(), 10);
    }
    assert_eq!(steps(&host.canwu, &front_a)[0].defender.lines.men, 600);
    assert_eq!(steps(&host.canwu, &front_b)[0].defender.lines.men, 298);
    let enemy = force_state(&host.canwu, "blue-e");
    assert_eq!((enemy.men(), enemy.losses().total()), (880, 20));
    assert_eq!(enemy.battle.cohesion, Some(800 - 7 - 7));
    assert_eq!(enemy.battle.engaged_in, Some(front_a.clone()));
    for (value, id) in [("red-a", &front_a), ("red-b", &front_b)] {
        let own = force_state(&host.canwu, value);
        assert_eq!(own.battle.cohesion, Some(800 - 7), "{value}");
        assert_eq!(own.battle.engaged_in.as_ref(), Some(id), "{value}");
    }

    // The group restores mid-fight and continues alike.
    let snapshot = host.canwu.snapshot_json().unwrap();
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &[&plugin, &HostPlugin]).unwrap();
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    let mut resumed = Host::new(restored, Drive::Wake);
    resumed.skip_requests(&host);

    // Day 2: both stall at their time limit. The enemy's losses sum once
    // over the two summaries, and each outcome signal carries only its own
    // engagement's share of the enemy's losses that boundary.
    host.day(2, &[]);
    resumed.day(2, &[]);
    assert_eq!(
        resumed.canwu.checkpoint_hash(),
        host.canwu.checkpoint_hash()
    );
    let enemy = force_state(&host.canwu, "blue-e");
    assert_eq!((enemy.men(), enemy.losses().total()), (860, 40));
    assert_eq!(enemy.battle.cohesion, Some(800 - 4 * 7));
    assert!(enemy.is_free(), "{enemy:#?}");
    let closed = summaries(&host.canwu);
    assert_eq!(closed.len(), 2, "{closed:#?}");
    assert_eq!(
        closed
            .iter()
            .map(|summary| summary.defender.losses.total())
            .sum::<u64>(),
        enemy.losses().total()
    );
    for summary in &closed {
        assert_eq!(summary.outcome, EngagementOutcomeKind::Stalled);
        assert_eq!(summary.closed_at, at_day(2));
    }
    let shared_losses: Vec<u64> = signals(&host.canwu)
        .iter()
        .flat_map(|signal| &signal.force_losses)
        .filter(|delta| delta.force == force_id("blue-e"))
        .map(|delta| delta.losses.total())
        .collect();
    assert_eq!(shared_losses, [10, 10]);
    assert!(registry(&host.canwu).unwrap().open_engagements.is_empty());

    let replayed =
        Canwu::replay_from_journal(&[&plugin, &HostPlugin], &host.canwu.replay_journal()).unwrap();
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}

#[test]
fn a_garrison_that_yields_during_an_assault_is_released_to_the_host() {
    let plugin = fixed_plugin();
    let forces = [
        force("red-2", 1, &red(), 2, &[("infantry", 2_000, infantry())]),
        force("blue-2", 2, &blue(), 2, &[("infantry", 800, infantry())]),
    ];
    let mut host = fixed_run(&plugin, &forces);
    let invest = InvestSiteV1 {
        id: siege_id(),
        site: site(2),
        profile: SiteProfileV1 {
            grade: 1,
            accesses: 4,
            water_accesses: 0,
            repair_per_day: 0,
        },
        besiegers: vec![force_id("red-2")],
        defenders: vec![force_id("blue-2")],
        holders: [Vec::new(), Vec::new()],
        inputs: [condition("field", 750), condition("defend", 650)],
        environment: environment(false),
        work_bp: 0,
        supply: SiegeSupplyInputV1 {
            days_of_stock: 30,
            ration: RationLevelId::new("test:ration:full").unwrap(),
            evidence: Vec::new(),
        },
        naval: None,
        storm_on: StormRule::Never,
    };
    let siege = |canwu: &Canwu| {
        canwu
            .typed_domain_record(&siege_reference(&siege_id()))
            .unwrap()
            .decode_payload::<SiegeRecord>()
            .unwrap()
    };
    host.day(
        0,
        &[(red(), MilitaryCommandV1::InvestSite(Box::new(invest)))],
    );
    let storm = MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
        siege: siege_id(),
        expected_host_revision: siege(&host.canwu).host_revision,
        supply: None,
        profile: None,
        work_bp: None,
        naval: None,
        gate_open_steps: None,
        assault: Some(true),
        sortie: false,
        storm_on: None,
        release: Vec::new(),
    }));
    // Day 1 the storm opens the assault; day 2 it fights a step.
    host.day(1, &[(red(), storm)]);
    host.day(2, &[]);
    let assault = siege(&host.canwu).engagement.expect("the assault opened");
    let fight = engagement(&host.canwu, &assault);
    assert_eq!(
        (fight.kind, fight.status, fight.steps_resolved),
        (EngagementKind::Assault, EngagementStatus::Open, 1)
    );
    assert_eq!(force_state(&host.canwu, "blue-2").men(), 790);

    // Day 3 the garrison yields while the assault runs.
    let surrender = MilitaryCommandV1::SurrenderSiege {
        siege: siege_id(),
        expected_host_revision: siege(&host.canwu).host_revision,
    };
    host.day(3, &[(blue(), surrender)]);
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );
    let state = siege(&host.canwu);
    assert_eq!(
        (state.status, state.end_cause, state.ended_at),
        (
            SiegeStatus::Fell,
            Some(SiegeEndCause::Surrendered),
            Some(at_day(3))
        )
    );
    assert_eq!(state.engagement, None);
    let closed = summaries(&host.canwu);
    assert_eq!(closed.len(), 1, "{closed:#?}");
    assert_eq!(closed[0].engagement, assault);
    assert_eq!(
        (closed[0].outcome, closed[0].site_fell, closed[0].closed_at),
        (EngagementOutcomeKind::AttackerPrevailed, true, at_day(3))
    );
    assert!(closed[0].defender.routed.is_empty());
    // The garrison keeps its men, undispersed, free at the site for the host.
    let garrison = force_state(&host.canwu, "blue-2");
    assert!(garrison.is_free(), "{garrison:#?}");
    assert!(garrison.is_active() && garrison.location.is_at(&site(2)));
    assert!(!garrison.battle.dispersed);
    assert_eq!((garrison.men(), garrison.losses().dispersed), (790, 0));
    assert!(force_state(&host.canwu, "red-2").is_free());
    let registry = registry(&host.canwu).unwrap();
    assert!(registry.open_sieges.is_empty() && registry.open_engagements.is_empty());
}

/// One rulebook day of a field fight at `profile`, the attacker suffering
/// `sources` sources of order confusion; the rulebook's lethality is kept low
/// so no side breaks within the day.
fn day_of_fighting(profile: ProfileId, sources: u32) -> EngagementStateV1 {
    let mut rulebook = rulebook(&ParametricStepResolver::IDENTITY, InterceptMode::Threshold);
    rulebook.cohesion = CohesionRuleV1 {
        per_inflicted: 4,
        base: 1,
        confusion_per_day: 5,
        recover_per_day: 10,
    };
    rulebook.lines.inflicted_cap_per_mille = 2;
    let rulebook = rulebook.sealed().unwrap();
    let forces = [
        force("red-1", 1, &red(), 1, &[("infantry", 3_000, infantry())]),
        force("blue-1", 2, &blue(), 1, &[("infantry", 1_500, infantry())]),
    ];
    let plugin = military_plugin();
    let records = scenario_records(&rulebook, &forces).unwrap();
    let canwu = Canwu::new_with_plugins(17, world(records), &[&plugin, &HostPlugin]).unwrap();
    let mut host = Host::new(canwu, Drive::Wake);
    let mut attacker = condition("field", 900);
    attacker.order_confusion = sources;
    let open = OpenEngagementV1 {
        id: engagement_id("field"),
        kind: EngagementKind::Field,
        profile: Some(profile),
        site: site(1),
        fort: None,
        attackers: vec![force_id("red-1")],
        defenders: vec![force_id("blue-1")],
        holders: [Vec::new(), Vec::new()],
        inputs: [attacker, condition("defend", 700)],
        environment: environment(false),
        max_minutes: None,
    };
    host.day(
        0,
        &[(red(), MilitaryCommandV1::OpenEngagement(Box::new(open)))],
    );
    host.day(1, &[]);
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );
    engagement(&host.canwu, &engagement_id("field"))
}

#[test]
fn one_confusion_source_costs_five_cohesion_a_day_at_both_step_lengths() {
    for (profile, steps) in [(half_hour_profile(), 48), (day_profile(), 1)] {
        let none = day_of_fighting(profile.clone(), 0);
        let one = day_of_fighting(profile.clone(), 1);
        for state in [&none, &one] {
            assert_eq!(
                state.status,
                EngagementStatus::Open,
                "{profile}: {state:#?}"
            );
            assert_eq!(state.steps_resolved, steps, "{profile}");
        }
        assert_eq!(
            none.attacker.cohesion - one.attacker.cohesion,
            5,
            "{profile}: one source costs exactly 5 a day"
        );
        assert_eq!(one.attacker.confusion_carry, 0, "{profile}: a whole day");
        assert_eq!(none.defender.cohesion, one.defender.cohesion, "{profile}");
        assert_eq!(
            (none.attacker.losses, none.defender.losses),
            (one.attacker.losses, one.defender.losses),
            "{profile}: the casualties are unchanged"
        );
        assert_eq!(
            (none.attacker.morale_shock, none.defender.morale_shock),
            (one.attacker.morale_shock, one.defender.morale_shock),
            "{profile}"
        );
    }
}

/// An `OpenGroupEngagements` of red's command groups `(engagement, force)`
/// against blue's `shared` forces, field kind, the default (day) profile.
fn open_groups(
    groups: &[(&str, &str)],
    shared: &[&str],
    shared_input: ConditionInputV1,
    max_days: u32,
) -> MilitaryCommandV1 {
    MilitaryCommandV1::OpenGroupEngagements(Box::new(OpenGroupEngagementsV1 {
        kind: EngagementKind::Field,
        profile: None,
        site: site(1),
        fort: None,
        side: Side::Attacker,
        groups: groups
            .iter()
            .map(|(value, force)| CommandGroupV1 {
                id: engagement_id(value),
                forces: vec![force_id(force)],
                holders: Vec::new(),
                input: condition("field", 900),
            })
            .collect(),
        shared: shared.iter().map(|value| force_id(value)).collect(),
        shared_holders: Vec::new(),
        shared_input,
        environment: environment(false),
        max_minutes: Some(max_days * 1_440),
    }))
}

#[test]
fn shared_forces_with_different_cohesion_caps_lose_each_step_once() {
    let plugin = fixed_plugin();
    let mut low = force("blue-y", 2, &blue(), 1, &[("infantry", 500, infantry())]);
    low.terms.cohesion_cap = 400;
    let forces = [
        force("red-a", 1, &red(), 1, &[("infantry", 1_000, infantry())]),
        force("red-b", 1, &red(), 1, &[("infantry", 1_000, infantry())]),
        force("blue-x", 2, &blue(), 1, &[("infantry", 500, infantry())]),
        low,
    ];
    let mut host = fixed_run(&plugin, &forces);
    let open = open_groups(
        &[("front-a", "red-a"), ("front-b", "red-b")],
        &["blue-x", "blue-y"],
        condition("defend", 700),
        3,
    );
    host.day(0, &[(red(), open)]);
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );
    let (front_a, front_b) = (engagement_id("front-a"), engagement_id("front-b"));
    // The shared side opens at the men-weighted mean of 800 and 400.
    for id in [&front_a, &front_b] {
        assert_eq!(engagement(&host.canwu, id).defender.cohesion, 600);
    }

    // Day 1: two steps of 7 each move the one figure 600 to 593 to 586, in
    // both engagements; it is never re-read from the forces' clamped
    // figures (800 and 400 capped), which would drain it further.
    host.day(1, &[]);
    assert_eq!(steps(&host.canwu, &front_a)[0].defender.lines.men, 500);
    // front-b's share after front-a's step: 990 men over 990 : 1,000.
    assert_eq!(steps(&host.canwu, &front_b)[0].defender.lines.men, 497);
    for id in [&front_a, &front_b] {
        let state = engagement(&host.canwu, id);
        assert_eq!(state.status, EngagementStatus::Open, "{state:#?}");
        assert_eq!(state.defender.cohesion, 600 - 7 - 7, "{id}");
    }
    assert_eq!(
        force_state(&host.canwu, "blue-x").battle.cohesion,
        Some(586)
    );
    assert_eq!(
        force_state(&host.canwu, "blue-y").battle.cohesion,
        Some(400)
    );

    // Day 2: two more steps, 14 more, still one figure.
    host.day(2, &[]);
    for id in [&front_a, &front_b] {
        assert_eq!(engagement(&host.canwu, id).defender.cohesion, 600 - 4 * 7);
    }
}

#[test]
fn one_command_group_closing_early_leaves_the_other_fighting_all_shared_men() {
    let plugin = fixed_plugin();
    let forces = [
        force("red-a", 1, &red(), 1, &[("infantry", 2_000, infantry())]),
        force("red-b", 1, &red(), 1, &[("infantry", 1_000, infantry())]),
        force("blue-e", 2, &blue(), 1, &[("infantry", 900, infantry())]),
    ];
    let mut host = fixed_run(&plugin, &forces);
    let open = open_groups(
        &[("front-a", "red-a"), ("front-b", "red-b")],
        &["blue-e"],
        condition("defend", 700),
        4,
    );
    host.day(0, &[(red(), open)]);
    host.day(1, &[]);
    let (front_a, front_b) = (engagement_id("front-a"), engagement_id("front-b"));
    assert_eq!(force_state(&host.canwu, "blue-e").men(), 880);

    // Day 2: the command of red-b abandons its own fight before the step.
    let abandon = MilitaryCommandV1::OrderAction {
        engagement: front_b.clone(),
        expected_host_revision: engagement(&host.canwu, &front_b).host_revision,
        side: Side::Attacker,
        action: SideAction::Abandon,
    };
    host.day(2, &[(red(), abandon)]);
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );
    let closed = summaries(&host.canwu);
    assert_eq!(closed.len(), 1, "{closed:#?}");
    assert_eq!(
        (
            &closed[0].engagement,
            closed[0].outcome,
            closed[0].closed_at
        ),
        (&front_b, EngagementOutcomeKind::Abandoned, at_day(2))
    );
    assert!(force_state(&host.canwu, "red-b").is_free());

    // front-a fights on alone, against every shared man, the shared force
    // still engaged in it.
    let state = engagement(&host.canwu, &front_a);
    assert_eq!(
        (state.status, state.steps_resolved),
        (EngagementStatus::Open, 2)
    );
    assert_eq!(steps(&host.canwu, &front_a)[0].defender.lines.men, 880);
    let enemy = force_state(&host.canwu, "blue-e");
    assert_eq!(enemy.men(), 870);
    assert_eq!(enemy.battle.engaged_in, Some(front_a.clone()));
    assert_eq!(enemy.battle.cohesion, Some(800 - 3 * 7));
    assert_eq!(state.defender.cohesion, 800 - 3 * 7);

    // Day 3 likewise.
    host.day(3, &[]);
    assert_eq!(steps(&host.canwu, &front_a)[0].defender.lines.men, 870);
    assert_eq!(force_state(&host.canwu, "blue-e").men(), 860);
}

#[test]
fn a_command_group_whose_share_would_round_to_nothing_is_refused() {
    let plugin = fixed_plugin();
    let forces = [
        force("red-a", 1, &red(), 1, &[("infantry", 10_000, infantry())]),
        force("red-b", 1, &red(), 1, &[("infantry", 100, infantry())]),
        force("blue-e", 2, &blue(), 1, &[("infantry", 50, infantry())]),
    ];
    let mut host = fixed_run(&plugin, &forces);
    // 50 men over 10,000 : 100 split [50, 0]: front-b would fight no one and
    // its first step would close as a win over forces front-a still fights.
    let open = open_groups(
        &[("front-a", "red-a"), ("front-b", "red-b")],
        &["blue-e"],
        condition("defend", 700),
        2,
    );
    host.day(0, &[(red(), open)]);
    let refused = rejections(&host.canwu);
    assert_eq!(refused.len(), 1, "{refused:#?}");
    assert!(refused[0].contains("no share"), "{refused:#?}");
    assert!(registry(&host.canwu).unwrap().open_engagements.is_empty());
    for value in ["red-a", "red-b", "blue-e"] {
        assert!(force_state(&host.canwu, value).is_free(), "{value}");
    }
    host.day(1, &[]);
    assert!(summaries(&host.canwu).is_empty());
}

#[test]
fn a_shared_side_pays_its_order_confusion_once_a_day_not_once_per_group() {
    let plugin = fixed_plugin();
    let forces = [
        force("red-a", 1, &red(), 1, &[("infantry", 2_000, infantry())]),
        force("red-b", 1, &red(), 1, &[("infantry", 1_000, infantry())]),
        force("blue-e", 2, &blue(), 1, &[("infantry", 900, infantry())]),
    ];
    let mut host = fixed_run(&plugin, &forces);
    let mut shared_input = condition("defend", 700);
    shared_input.order_confusion = 1;
    let open = open_groups(
        &[("front-a", "red-a"), ("front-b", "red-b")],
        &["blue-e"],
        shared_input,
        3,
    );
    host.day(0, &[(red(), open)]);
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );
    let (front_a, front_b) = (engagement_id("front-a"), engagement_id("front-b"));
    // Each day: two steps of 7, and one source at 50 a day paid once (by
    // front-a, the first engagement due), not once per engagement.
    for (day, cohesion) in [(1_u32, 800 - 7 - 50 - 7), (2, 800 - 2 * (7 + 50 + 7))] {
        host.day(i64::from(day), &[]);
        for id in [&front_a, &front_b] {
            let state = engagement(&host.canwu, id);
            assert_eq!(state.status, EngagementStatus::Open, "{state:#?}");
            assert_eq!(state.defender.cohesion, cohesion, "day {day}: {id}");
            assert_eq!(state.defender.confusion_carry, 0, "day {day}: {id}");
            // The groups' own sides owe no confusion.
            assert_eq!(state.attacker.cohesion, 800 - 7 * day, "{id}");
        }
        assert_eq!(
            force_state(&host.canwu, "blue-e").battle.cohesion,
            Some(cohesion),
            "day {day}"
        );
    }
}
