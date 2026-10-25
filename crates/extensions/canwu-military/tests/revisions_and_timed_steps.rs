//! The 0.21.0 revisions of a running operation: a timed step a host queues
//! for the boundary it settles anyway, a report queue left alone when no
//! side names a holder, a fight shorter than its kind's rule, a siege that
//! takes a second column and releases its defenders, and a crossing whose
//! defending bank is revised after the order.

#![allow(clippy::too_many_lines)]

mod support;

use canwu_api::{
    Canwu, CommandEnvelope, CommandRequest, CommandRequestId, Issuer, PersonId, SimDuration,
    SystemCadence,
};
use canwu_military::*;
use support::*;

fn engagement_id(value: &str) -> EngagementId {
    EngagementId::new(format!("test:engagement:{value}")).unwrap()
}

fn siege_id() -> SiegeId {
    SiegeId::new("test:siege:keep").unwrap()
}

fn crossing_id() -> CrossingId {
    CrossingId::new("test:crossing:ford").unwrap()
}

fn forces() -> Vec<ForceStateV1> {
    vec![
        force("red-1", 1, &red(), 1, &[("infantry", 3_000, infantry())]),
        force("blue-1", 2, &blue(), 1, &[("infantry", 1_500, infantry())]),
        force("red-2", 1, &red(), 2, &[("infantry", 2_000, infantry())]),
        force("blue-2", 2, &blue(), 2, &[("infantry", 800, infantry())]),
        force("blue-4", 2, &blue(), 2, &[("infantry", 400, infantry())]),
        force("red-4", 1, &red(), 1, &[("infantry", 2_000, infantry())]),
        force("red-3", 1, &red(), 3, &[("infantry", 2_000, infantry())]),
        force("blue-5", 2, &blue(), 5, &[("infantry", 900, infantry())]),
        force("blue-7", 2, &blue(), 2, &[("infantry", 300, infantry())]),
        force("green-6", 2, &green(), 4, &[("infantry", 500, infantry())]),
    ]
}

/// A third controller, of a force on blue's side.
fn green() -> Issuer {
    Issuer::Actor(PersonId::new(3))
}

fn run(plugin: &MilitaryPlugin) -> Canwu {
    let rulebook = rulebook(&ParametricStepResolver::IDENTITY, InterceptMode::Threshold);
    let records = scenario_records(&rulebook, &forces()).unwrap();
    Canwu::new_with_plugins(13, world(records), &[plugin, &HostPlugin]).unwrap()
}

fn open(
    id: &str,
    kind: EngagementKind,
    at: u64,
    attacker: &str,
    defender: &str,
) -> OpenEngagementV1 {
    let mut environment = environment(false);
    if kind == EngagementKind::Assault {
        environment.terrain = Some(TerrainClassId::new("test:terrain:walls").unwrap());
    }
    OpenEngagementV1 {
        id: engagement_id(id),
        kind,
        profile: None,
        site: site(at),
        fort: (kind == EngagementKind::Assault).then_some((2, false)),
        attackers: vec![force_id(attacker)],
        defenders: vec![force_id(defender)],
        holders: [Vec::new(), Vec::new()],
        inputs: [condition("field", 900), condition("defend", 700)],
        environment,
        max_minutes: None,
    }
}

fn invest(defenders: &[&str], accesses: u32) -> MilitaryCommandV1 {
    MilitaryCommandV1::InvestSite(Box::new(InvestSiteV1 {
        id: siege_id(),
        site: site(2),
        profile: SiteProfileV1 {
            grade: 1,
            accesses,
            water_accesses: 0,
            repair_per_day: 0,
        },
        besiegers: vec![force_id("red-2")],
        defenders: defenders.iter().map(|value| force_id(value)).collect(),
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
    }))
}

fn siege(canwu: &Canwu) -> SiegeStateV1 {
    canwu
        .typed_domain_record(&siege_reference(&siege_id()))
        .unwrap()
        .decode_payload::<SiegeRecord>()
        .unwrap()
}

fn crossing(canwu: &Canwu) -> CrossingStateV1 {
    canwu
        .typed_domain_record(&crossing_reference(&crossing_id()))
        .unwrap()
        .decode_payload::<CrossingRecord>()
        .unwrap()
}

fn revise(expected_host_revision: u64) -> ReviseSiegeV1 {
    ReviseSiegeV1 {
        siege: siege_id(),
        expected_host_revision,
        supply: None,
        profile: None,
        work_bp: None,
        naval: None,
        gate_open_steps: None,
        assault: None,
        sortie: false,
        storm_on: None,
        release: Vec::new(),
    }
}

fn revise_siege(expected_host_revision: u64, release: &[&str]) -> MilitaryCommandV1 {
    MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
        release: release.iter().map(|value| force_id(value)).collect(),
        ..revise(expected_host_revision)
    }))
}

fn place(value: &str, at: u64, canwu: &Canwu) -> MilitaryCommandV1 {
    MilitaryCommandV1::PlaceForce {
        force: force_id(value),
        expected_host_revision: force_state(canwu, value).host_revision,
        location: LocationV1::At { site: site(at) },
    }
}

/// A host that steps once a day: each day's daily calendar boundary admits
/// the timed step it queued the day before. Returns the run and how many
/// boundaries each day settled.
fn stepped_daily(plugin: &MilitaryPlugin, days: i64) -> (Host, Vec<usize>) {
    let mut host = Host::new(run(plugin), Drive::Wake);
    host.now(&[
        (
            red(),
            MilitaryCommandV1::OpenEngagement(Box::new(open(
                "field",
                EngagementKind::Field,
                1,
                "red-1",
                "blue-1",
            ))),
        ),
        (red(), invest(&["blue-2"], 8)),
    ]);
    host.canwu
        .enqueue_plugin_ingress(timed_step_ingress(at_day(1)))
        .unwrap();
    let mut boundaries = Vec::new();
    for day in 1..=days {
        let at = at_day(day);
        host.canwu
            .schedule_calendar_boundary(at, vec![SystemCadence::Daily])
            .unwrap();
        let before = host
            .canwu
            .typed_domain_record(&registry_reference())
            .unwrap()
            .version;
        let now = host.canwu.time();
        let receipts = host
            .canwu
            .advance_canonical(at.checked_sub(now).unwrap_or(SimDuration::ZERO))
            .unwrap();
        boundaries.push(
            receipts
                .iter()
                .filter(|receipt| receipt.settled_at == at)
                .count(),
        );
        let registry = host
            .canwu
            .typed_domain_record(&registry_reference())
            .unwrap();
        // No side names a holder: the step queues no report, so phase 13
        // leaves the registry alone and it is written at most once.
        assert!(
            registry.version <= before + 1,
            "day {day}: the registry was written {} times",
            registry.version - before
        );
        assert!(
            registry
                .decode_payload::<RegistryRecord>()
                .unwrap()
                .report_queue
                .is_empty()
        );
        host.canwu
            .enqueue_plugin_ingress(timed_step_ingress(at_day(day + 1)))
            .unwrap();
    }
    (host, boundaries)
}

#[test]
fn a_host_stepping_once_a_day_settles_one_boundary_a_day() {
    const DAYS: i64 = 4;
    let manual = military_plugin().with_step_cadence(None);
    let (mut host, boundaries) = stepped_daily(&manual, DAYS);
    assert_eq!(boundaries, [1, 1, 1, 1], "one boundary a day");
    // Every day's work stepped in that one boundary.
    assert_eq!(siege(&host.canwu).next_day_at, at_day(DAYS + 1));
    let field = engagement(&host.canwu, &engagement_id("field"));
    assert!(field.steps_resolved > 0, "{field:#?}");
    assert!(field.is_closed() || field.last_step_at == at_day(DAYS));
    assert!(
        rejections(&host.canwu).is_empty(),
        "{:#?}",
        rejections(&host.canwu)
    );

    // The default wake cadence sees the admitted timed step and schedules
    // nothing more: the same boundaries and the same records.
    let woken = military_plugin();
    let (woken_host, woken_boundaries) = stepped_daily(&woken, DAYS);
    assert_eq!(woken_boundaries, boundaries);
    assert_eq!(
        plugin_payloads(&woken_host.canwu),
        plugin_payloads(&host.canwu)
    );

    // A command queued as ingress for the timed step's instant is admitted
    // with it but applies in the next cut, after the step: the siege day
    // seals four of the old eight accesses, then the profile changes.
    let day = at_day(DAYS + 1);
    let profile = MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
        profile: Some(SiteProfileV1 {
            grade: 1,
            accesses: 4,
            water_accesses: 0,
            repair_per_day: 0,
        }),
        ..revise(siege(&host.canwu).host_revision)
    }));
    let revision = host.canwu.revision();
    host.canwu
        .enqueue_command(
            day,
            0,
            CommandRequest::new(
                CommandRequestId::new(1_000),
                revision,
                CommandEnvelope::new(red(), military_command(&profile).unwrap()).at_time(day),
            ),
        )
        .unwrap();
    host.canwu
        .schedule_calendar_boundary(day, vec![SystemCadence::Daily])
        .unwrap();
    let now = host.canwu.time();
    let receipts = host
        .canwu
        .advance_canonical(day.checked_sub(now).unwrap())
        .unwrap();
    assert_eq!(
        receipts
            .iter()
            .filter(|receipt| receipt.settled_at == day)
            .count(),
        2,
        "the command's apply packet takes the next cut"
    );
    let state = siege(&host.canwu);
    assert_eq!(state.next_day_at, at_day(DAYS + 2), "the day stepped");
    assert_eq!(
        (state.investment_bp, state.profile.accesses),
        (5_000, 4),
        "the step ran before the command applied"
    );
    assert!(rejections(&host.canwu).is_empty());

    // The queued steps and the command replay exactly.
    let replayed =
        Canwu::replay_from_journal(&[&manual, &HostPlugin], &host.canwu.replay_journal()).unwrap();
    assert_eq!(replayed.checkpoint_hash(), host.canwu.checkpoint_hash());
}

#[test]
fn a_fight_opened_for_a_day_stalls_after_it() {
    let plugin = military_plugin();
    let mut host = Host::new(run(&plugin), Drive::Wake);
    let mut gate = open("gate", EngagementKind::Assault, 1, "red-1", "blue-1");
    gate.max_minutes = Some(1_440);
    let mut zero = open("zero", EngagementKind::Assault, 1, "red-4", "blue-1");
    zero.max_minutes = Some(0);
    let mut long = open("long", EngagementKind::Assault, 1, "red-4", "blue-1");
    long.max_minutes = Some(6 * 1_440);
    host.day(
        0,
        &[
            (red(), MilitaryCommandV1::OpenEngagement(Box::new(gate))),
            (red(), MilitaryCommandV1::OpenEngagement(Box::new(zero))),
            (red(), MilitaryCommandV1::OpenEngagement(Box::new(long))),
            (
                red(),
                MilitaryCommandV1::OpenEngagement(Box::new(open(
                    "wall",
                    EngagementKind::Assault,
                    2,
                    "red-2",
                    "blue-2",
                ))),
            ),
        ],
    );
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 2, "{rejected:#?}");
    assert!(
        rejected
            .iter()
            .all(|summary| summary.starts_with("ValueOutOfRange")),
        "{rejected:#?}"
    );
    host.day(1, &[]);
    // The one-day assault stalls after its day; the wall assault runs on
    // under its kind's five days.
    let gate = summaries(&host.canwu);
    assert_eq!(gate.len(), 1, "{gate:#?}");
    assert_eq!(gate[0].engagement, engagement_id("gate"));
    assert_eq!(gate[0].outcome, EngagementOutcomeKind::Stalled);
    assert_eq!((gate[0].steps, gate[0].closed_at), (1, at_day(1)));
    assert_eq!(
        engagement(&host.canwu, &engagement_id("gate")).max_minutes,
        1_440
    );
    let wall = engagement(&host.canwu, &engagement_id("wall"));
    assert_eq!(wall.status, EngagementStatus::Open);
    assert_eq!((wall.steps_resolved, wall.max_minutes), (1, 5 * 1_440));
}

#[test]
fn a_siege_takes_a_second_column_and_releases_its_defenders() {
    let plugin = military_plugin();
    let mut host = Host::new(run(&plugin), Drive::Wake);
    host.day(0, &[(red(), invest(&["blue-2", "blue-4"], 8))]);
    // The second column arrives and joins the besiegers before the first
    // siege day; a force elsewhere cannot join, nor can an issuer join a
    // force it does not control.
    let join = |expected_host_revision, side, forces: &[&str]| MilitaryCommandV1::JoinSiege {
        siege: siege_id(),
        expected_host_revision,
        side,
        forces: forces.iter().map(|value| force_id(value)).collect(),
    };
    let arrival = place("red-4", 2, &host.canwu);
    host.day(
        1,
        &[
            (red(), arrival),
            (red(), join(0, Side::Attacker, &["red-4"])),
            (red(), join(1, Side::Attacker, &["red-3"])),
            (red(), join(1, Side::Defender, &["blue-7"])),
        ],
    );
    let state = siege(&host.canwu);
    assert_eq!(state.besiegers, vec![force_id("red-2"), force_id("red-4")]);
    assert_eq!(
        state.investment_bp, 10_000,
        "both columns' men seal the place"
    );
    assert_eq!(
        force_state(&host.canwu, "red-4").battle.siege_role,
        Some(SiegeRoleV1::Besieging { siege: siege_id() })
    );
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 2, "{rejected:#?}");
    assert!(
        rejected[0].contains("not free at the site"),
        "{rejected:#?}"
    );
    assert!(rejected[1].starts_with("InvalidAuthority"), "{rejected:#?}");

    // A defender marches off: released, then placed in the same boundary.
    // Releasing every besieger is refused (LiftSiege ends a siege), and so is
    // a release by an issuer who does not control the force. The garrison
    // orders a sortie.
    let revision = state.host_revision;
    let march = MilitaryCommandV1::PlaceForce {
        force: force_id("blue-4"),
        expected_host_revision: force_state(&host.canwu, "blue-4").host_revision,
        location: LocationV1::At { site: site(5) },
    };
    host.day(
        2,
        &[
            (blue(), revise_siege(revision, &["blue-4"])),
            (blue(), march),
            (red(), revise_siege(revision + 1, &["red-2", "red-4"])),
            (blue(), revise_siege(revision + 1, &["red-2"])),
            (
                blue(),
                MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
                    sortie: true,
                    ..revise(revision + 1)
                })),
            ),
        ],
    );
    let state = siege(&host.canwu);
    assert_eq!(state.defenders, vec![force_id("blue-2")]);
    assert_eq!(state.besiegers.len(), 2);
    let blue_4 = force_state(&host.canwu, "blue-4");
    assert!(blue_4.is_free() && blue_4.location.is_at(&site(5)));
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 4, "{rejected:#?}");
    assert!(rejected[2].contains("without a besieger"), "{rejected:#?}");
    assert!(rejected[3].starts_with("InvalidAuthority"), "{rejected:#?}");
    let sortie = state.engagement.clone().expect("the sortie opened");

    // A defender joins while the sortie is out: it enters the siege and the
    // sortie's attacking side. A force in the fight cannot leave the siege.
    host.day(
        3,
        &[
            (
                blue(),
                join(state.host_revision, Side::Defender, &["blue-7"]),
            ),
            (blue(), revise_siege(state.host_revision + 1, &["blue-2"])),
        ],
    );
    let state = siege(&host.canwu);
    assert_eq!(
        state.defenders,
        vec![force_id("blue-2"), force_id("blue-7")]
    );
    let fight = engagement(&host.canwu, &sortie);
    assert_eq!(fight.kind, EngagementKind::Sortie);
    assert_eq!(
        fight.attacker.forces,
        vec![force_id("blue-2"), force_id("blue-7")]
    );
    assert_eq!(fight.host_revision, 1);
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 5, "{rejected:#?}");
    assert!(rejected[4].contains("in a fight"), "{rejected:#?}");
    assert!(fight.is_closed(), "the one-day sortie closed: {fight:#?}");

    // The last defenders leave: the siege day finds no defender and the
    // place falls `Abandoned`, releasing the besiegers. A force of the
    // besiegers' owner cannot take the emptied defenders' side.
    let arrival = place("red-1", 2, &host.canwu);
    host.day(
        4,
        &[
            (
                blue(),
                revise_siege(state.host_revision, &["blue-2", "blue-7"]),
            ),
            (red(), arrival),
            (
                red(),
                join(state.host_revision + 1, Side::Defender, &["red-1"]),
            ),
        ],
    );
    let state = siege(&host.canwu);
    assert_eq!(
        (state.status, state.end_cause),
        (SiegeStatus::Fell, Some(SiegeEndCause::Abandoned))
    );
    assert!(state.defenders.is_empty());
    for value in ["blue-2", "blue-7", "red-2", "red-4"] {
        let force = force_state(&host.canwu, value);
        assert!(force.is_free(), "{value}");
        assert_eq!(force.losses().dispersed, 0, "{value} was not dispersed");
    }
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 6, "{rejected:#?}");
    assert!(
        rejected[5].starts_with("InvalidPayload") && rejected[5].contains("its own owner"),
        "{rejected:#?}"
    );

    // Nor can one owner's forces besiege each other.
    let MilitaryCommandV1::InvestSite(mut own) = invest(&["red-4"], 8) else {
        unreachable!("invest builds an InvestSite");
    };
    own.id = SiegeId::new("test:siege:own").unwrap();
    host.day(5, &[(red(), MilitaryCommandV1::InvestSite(own))]);
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 7, "{rejected:#?}");
    assert!(
        rejected[6].starts_with("InvalidPayload") && rejected[6].contains("one owner"),
        "{rejected:#?}"
    );
}

#[test]
fn a_force_that_reaches_the_far_bank_resists_the_landing() {
    let plugin = military_plugin();
    let mut host = Host::new(run(&plugin), Drive::Wake);
    let defenders = |expected_host_revision, forces: &[&str]| MilitaryCommandV1::ReviseCrossing {
        crossing: crossing_id(),
        expected_host_revision,
        interceptor: None,
        night: None,
        defenders: Some(forces.iter().map(|value| force_id(value)).collect()),
    };
    let arrival = place("blue-5", 4, &host.canwu);
    let commands: Vec<(Issuer, MilitaryCommandV1)> = vec![
        (
            red(),
            MilitaryCommandV1::OrderCrossing(Box::new(OrderCrossingV1 {
                id: crossing_id(),
                force: force_id("red-3"),
                from: site(3),
                to: site(4),
                night: false,
                defenders: Vec::new(),
                holders: [Vec::new(), Vec::new()],
                inputs: [condition("march", 800), condition("defend", 700)],
                environment: environment(false),
                interceptor: None,
            })),
        ),
        // The bank's own controller names the force that arrived there; a
        // second controller adds its own force but cannot drop the first's.
        (blue(), arrival),
        (blue(), defenders(0, &["blue-5"])),
        (green(), defenders(1, &["blue-5", "green-6"])),
        (green(), defenders(2, &["green-6"])),
        (red(), defenders(2, &["blue-1"])),
    ];
    host.day(0, &commands);
    assert_eq!(
        crossing(&host.canwu).defenders,
        vec![force_id("blue-5"), force_id("green-6")]
    );
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 2, "{rejected:#?}");
    assert!(rejected[0].starts_with("InvalidAuthority"), "{rejected:#?}");
    assert!(rejected[1].contains("not at the far bank"), "{rejected:#?}");

    host.day(1, &[]);
    let state = crossing(&host.canwu);
    let wave = state.last_wave.expect("the first wave crossed");
    assert!(!wave.intercepted);
    let fight = engagement(
        &host.canwu,
        &wave.engagement.expect("the landing is resisted"),
    );
    assert_eq!(fight.kind, EngagementKind::BridgeheadFight);
    assert_eq!(
        fight.defender.forces,
        vec![force_id("blue-5"), force_id("green-6")]
    );
}
