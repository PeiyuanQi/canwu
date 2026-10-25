//! The engagement engine's lifecycle: a field fight stepped and promoted, a
//! siege relieved in vain then breached, stormed, and fallen, a contested
//! crossing, holder-relative reports, the outcome signal, the two step
//! drivers, save and restore mid-siege, exact replay, and keyed draws.

#![allow(clippy::too_many_lines)]

mod support;

use canwu_api::{
    Canwu, IngressPayload, Issuer, KnowledgeHistoryView, KnowledgeQuery, MAX_KNOWLEDGE_PAGE_SIZE,
    RandomDrawId,
};
use canwu_military::*;
use support::*;

fn engagement_id(value: &str) -> EngagementId {
    EngagementId::new(format!("test:engagement:{value}")).unwrap()
}

fn siege_id() -> SiegeId {
    SiegeId::new("test:siege:fort").unwrap()
}

fn crossing_id() -> CrossingId {
    CrossingId::new("test:crossing:ford").unwrap()
}

fn forces() -> Vec<ForceStateV1> {
    vec![
        force(
            "red-1",
            1,
            &red(),
            1,
            &[("infantry", 3_000, infantry()), ("cavalry", 500, cavalry())],
        ),
        force("blue-1", 2, &blue(), 1, &[("infantry", 2_000, infantry())]),
        force(
            "red-2",
            1,
            &red(),
            2,
            &[("infantry", 4_000, infantry()), ("guns", 200, guns())],
        ),
        force("blue-2", 2, &blue(), 2, &[("infantry", 800, infantry())]),
        force("blue-3", 2, &blue(), 5, &[("infantry", 600, infantry())]),
        force("red-3", 1, &red(), 3, &[("infantry", 2_000, infantry())]),
        force("blue-fleet", 2, &blue(), 3, &[("vessels", 300, vessels())]),
    ]
}

/// The run of the campaign with resolver `R`.
fn start<R: StepResolver>(drive: Drive, mode: InterceptMode) -> (Host, MilitaryPlugin<R>) {
    let plugin = plugin::<R>(drive);
    let records = scenario_records(&rulebook(&R::IDENTITY, mode), &forces()).unwrap();
    let canwu = Canwu::new_with_plugins(7, world(records), &[&plugin, &HostPlugin]).unwrap();
    (Host::new(canwu, drive), plugin)
}

fn plugin<R: StepResolver>(drive: Drive) -> MilitaryPlugin<R> {
    let plugin = MilitaryPlugin::<R>::new().with_outcome_signal(HOST_PLUGIN, OUTCOME_INGRESS);
    match drive {
        Drive::Wake => plugin,
        Drive::Step => plugin.with_step_cadence(None),
    }
}

fn revision_of_engagement(host: &Host, value: &str) -> u64 {
    engagement(&host.canwu, &engagement_id(value)).host_revision
}

fn supply(host: &Host, value: &str, side: Side, offset: u64, morale: u16) -> MilitaryCommandV1 {
    MilitaryCommandV1::SupplyInput {
        target: InputTargetV1::Engagement(engagement_id(value)),
        expected_host_revision: revision_of_engagement(host, value) + offset,
        side: Some(side),
        condition: Some(condition(
            if side == Side::Attacker {
                "field"
            } else {
                "camp"
            },
            morale,
        )),
        environment: None,
    }
}

/// The host's commands of day `day`.
fn script(host: &mut Host, day: i64) {
    let commands: Vec<(Issuer, MilitaryCommandV1)> = match day {
        0 => vec![
            (
                red(),
                MilitaryCommandV1::OpenEngagement(Box::new(OpenEngagementV1 {
                    id: engagement_id("field"),
                    kind: EngagementKind::Field,
                    profile: None,
                    site: site(1),
                    fort: None,
                    attackers: vec![force_id("red-1")],
                    defenders: vec![force_id("blue-1")],
                    holders: [vec![person(1), force_holder("red-1")], vec![government(2)]],
                    inputs: [condition("field", 700), condition("camp", 700)],
                    environment: environment(false),
                    max_minutes: None,
                })),
            ),
            (
                red(),
                MilitaryCommandV1::OrderCrossing(Box::new(OrderCrossingV1 {
                    id: crossing_id(),
                    force: force_id("red-3"),
                    from: site(3),
                    to: site(4),
                    night: false,
                    defenders: Vec::new(),
                    holders: [vec![person(1)], vec![government(2)]],
                    inputs: [condition("march", 700), condition("defend", 700)],
                    environment: environment(false),
                    interceptor: Some(InterceptorInputV1 {
                        force: force_id("blue-fleet"),
                        control_bp: 9_000,
                        scouts: 10,
                        evidence: Vec::new(),
                    }),
                })),
            ),
        ],
        1 => vec![
            (red(), supply(host, "field", Side::Attacker, 0, 690)),
            (blue(), supply(host, "field", Side::Defender, 1, 660)),
        ],
        2 => {
            let blue_1 = force_state(&host.canwu, "blue-1");
            let mut components = std::collections::BTreeMap::new();
            for (component, state) in &blue_1.components {
                components.insert(
                    component.clone(),
                    ComponentRevisionV1 {
                        present: state.present + 300,
                        values: state.values.clone(),
                    },
                );
            }
            let crossing = host
                .canwu
                .typed_domain_record(&crossing_reference(&crossing_id()))
                .unwrap()
                .decode_payload::<CrossingRecord>()
                .unwrap();
            vec![
                (
                    blue(),
                    MilitaryCommandV1::ReviseForce {
                        force: force_id("blue-1"),
                        expected_host_revision: blue_1.host_revision,
                        components: Some(components),
                        terms: None,
                        controllers: None,
                        clear_morale_shock: false,
                    },
                ),
                (
                    trusted(),
                    MilitaryCommandV1::ReviseStepProfile {
                        engagement: engagement_id("field"),
                        expected_host_revision: revision_of_engagement(host, "field"),
                        profile: half_hour_profile(),
                    },
                ),
                (
                    red(),
                    MilitaryCommandV1::ReviseCrossing {
                        crossing: crossing_id(),
                        expected_host_revision: crossing.host_revision,
                        interceptor: None,
                        night: Some(true),
                        defenders: None,
                    },
                ),
            ]
        }
        3 => vec![(
            red(),
            MilitaryCommandV1::InvestSite(Box::new(InvestSiteV1 {
                id: siege_id(),
                site: site(2),
                profile: SiteProfileV1 {
                    grade: 2,
                    accesses: 4,
                    water_accesses: 0,
                    repair_per_day: 50,
                },
                besiegers: vec![force_id("red-2")],
                defenders: vec![force_id("blue-2")],
                holders: [vec![person(1)], vec![government(2), force_holder("blue-2")]],
                inputs: [condition("field", 750), condition("defend", 650)],
                environment: environment(false),
                work_bp: 10_000,
                supply: SiegeSupplyInputV1 {
                    days_of_stock: 40,
                    ration: RationLevelId::new("test:ration:full").unwrap(),
                    evidence: Vec::new(),
                },
                naval: None,
                storm_on: StormRule::BreachOrGate,
            })),
        )],
        5 => {
            let relief = force_state(&host.canwu, "blue-3");
            vec![
                (
                    blue(),
                    MilitaryCommandV1::PlaceForce {
                        force: force_id("blue-3"),
                        expected_host_revision: relief.host_revision,
                        location: LocationV1::At { site: site(2) },
                    },
                ),
                (
                    blue(),
                    MilitaryCommandV1::OpenEngagement(Box::new(OpenEngagementV1 {
                        id: engagement_id("relief"),
                        kind: EngagementKind::Relief,
                        profile: None,
                        site: site(2),
                        fort: None,
                        attackers: vec![force_id("blue-3")],
                        defenders: vec![force_id("red-2")],
                        holders: [vec![government(2)], vec![person(1)]],
                        inputs: [condition("field", 600), condition("camp", 750)],
                        environment: environment(false),
                        max_minutes: None,
                    })),
                ),
            ]
        }
        _ => Vec::new(),
    };
    host.day(day, &commands);
}

const LAST_DAY: i64 = 14;
const SAVE_DAY: i64 = 6;

fn campaign<R: StepResolver>(drive: Drive, mode: InterceptMode) -> (Host, MilitaryPlugin<R>) {
    let (mut host, plugin) = start::<R>(drive, mode);
    for day in 0..=LAST_DAY {
        script(&mut host, day);
    }
    (host, plugin)
}

fn outcome_reports(
    canwu: &Canwu,
    holder: canwu_api::KnowledgeHolderRef,
) -> Vec<EngagementReportV1> {
    canwu
        .admin_query_knowledge(
            holder,
            &KnowledgeQuery {
                schemas: vec![engagement_outcome_schema_id()],
                view: KnowledgeHistoryView::FullHistory,
                limit: MAX_KNOWLEDGE_PAGE_SIZE,
                ..KnowledgeQuery::default()
            },
        )
        .unwrap()
        .records
        .into_iter()
        .map(|record| serde_json::from_value(record.payload).unwrap())
        .collect()
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
fn engagements_sieges_and_crossings_run_to_their_records() {
    let (host, plugin) = campaign::<ParametricStepResolver>(Drive::Wake, InterceptMode::Threshold);
    let canwu = &host.canwu;
    let summaries = summaries(canwu);
    let summary = |value: &str| {
        summaries
            .iter()
            .find(|summary| summary.engagement == engagement_id(value))
            .unwrap_or_else(|| panic!("no summary of {value}: {summaries:#?}"))
    };

    // The field fight stepped on the day profile, was promoted, and closed.
    let field = summary("field");
    assert_eq!(field.kind, EngagementKind::Field);
    assert!(field.steps > 2, "{field:#?}");
    assert_ne!(field.outcome, EngagementOutcomeKind::Abandoned);
    let field_state = engagement(canwu, &engagement_id("field"));
    assert_eq!(field_state.profile, half_hour_profile());
    assert!(field_state.attacker.losses.total() > 0 && field_state.defender.losses.total() > 0);
    // The reinforcement arrived mid-fight; the losses stayed the plugin's.
    let blue_1 = force_state(canwu, "blue-1");
    assert_eq!(
        blue_1.losses(),
        field_state.defender.losses,
        "a force's component losses are the fight's"
    );
    assert!(blue_1.is_free() && blue_1.battle.cohesion.is_some());

    // The relief was fought first and repulsed; then the breach opened and
    // the storm took the site, dispersing the defenders.
    let relief = summary("relief");
    assert_eq!(relief.kind, EngagementKind::Relief);
    assert_eq!(relief.outcome, EngagementOutcomeKind::Repulsed);
    let assault = summaries
        .iter()
        .find(|summary| summary.kind == EngagementKind::Assault)
        .expect("the siege opened an assault");
    assert!(
        assault.site_fell && assault.opened_at >= relief.closed_at,
        "{assault:#?}"
    );
    assert_eq!(assault.siege, Some(siege_id()));
    let siege = canwu
        .typed_domain_record(&siege_reference(&siege_id()))
        .unwrap()
        .decode_payload::<SiegeRecord>()
        .unwrap();
    assert_eq!(siege.status, SiegeStatus::Fell);
    assert_eq!(siege.end_cause, Some(SiegeEndCause::Breach));
    assert!(siege.breach_open && siege.investment_bp == 10_000);
    let blue_2 = force_state(canwu, "blue-2");
    assert!(blue_2.battle.dispersed && blue_2.men() == 0 && blue_2.losses().dispersed > 0);
    assert!(force_state(canwu, "red-2").is_free());

    // The first wave was intercepted into a ferry fight; the night waves
    // landed every man.
    let ferry = summaries
        .iter()
        .find(|summary| summary.kind == EngagementKind::FerryFight)
        .expect("the first wave was intercepted");
    assert_eq!(ferry.crossing, Some(crossing_id()));
    let crossing = canwu
        .typed_domain_record(&crossing_reference(&crossing_id()))
        .unwrap()
        .decode_payload::<CrossingRecord>()
        .unwrap();
    assert_eq!(crossing.status, CrossingStatus::Complete);
    let last = crossing.last_wave.as_ref().unwrap();
    assert!(!last.intercepted && crossing.waves_sent >= 2);
    let red_3 = force_state(canwu, "red-3");
    assert_eq!(crossing.landed, red_3.men());
    assert!(red_3.battle.crossing.is_none());

    // Holder-relative reports: a person, a government, and a force.
    for holder in [person(1), government(2), force_holder("red-1")] {
        let reports = outcome_reports(canwu, holder.clone());
        assert!(
            reports.iter().any(|report| {
                report.subject == ReportSubjectV1::Engagement(engagement_id("field"))
                    && report.closing
                    && report
                        .engagement
                        .as_ref()
                        .is_some_and(|part| part.outcome.is_some())
            }),
            "{holder:?} holds no outcome of the field fight: {reports:#?}"
        );
    }
    let blue_view = outcome_reports(canwu, government(2));
    let report = blue_view
        .iter()
        .find(|report| report.subject == ReportSubjectV1::Engagement(engagement_id("field")))
        .unwrap();
    assert_eq!(report.side, Side::Defender);
    assert_eq!(
        report.engagement.as_ref().unwrap().own_losses,
        field_state.defender.losses
    );
    assert!(blue_view.iter().any(|report| report.siege.is_some()));

    // Every closed engagement signalled its target, a boundary later.
    let signalled = signals(canwu);
    assert_eq!(signalled.len(), summaries.len());
    assert!(
        signalled
            .iter()
            .all(|signal| summaries.contains(&signal.summary))
    );

    // The registry holds no open work; closed fights leave small records.
    let registry = registry(canwu).unwrap();
    assert!(registry.open_engagements.is_empty() && registry.open_sieges.is_empty());
    assert!(registry.open_crossings.is_empty());
    assert!(rejections(canwu).is_empty(), "{:#?}", rejections(canwu));

    // The `Step` driver gives the same records.
    let (stepped, _) = campaign::<ParametricStepResolver>(Drive::Step, InterceptMode::Threshold);
    assert_eq!(plugin_payloads(&stepped.canwu), plugin_payloads(canwu));

    // Exact replay reproduces the run.
    let replayed =
        Canwu::replay_from_journal(&[&plugin, &HostPlugin], &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
}

#[test]
fn a_run_saved_mid_siege_restores_and_continues_alike() {
    let (mut host, plugin) = start::<ParametricStepResolver>(Drive::Wake, InterceptMode::Threshold);
    for day in 0..=SAVE_DAY {
        script(&mut host, day);
    }
    assert!(
        registry(&host.canwu)
            .unwrap()
            .open_sieges
            .contains(&siege_id())
    );
    let snapshot = host.canwu.snapshot_json().unwrap();
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &[&plugin, &HostPlugin]).unwrap();
    assert_eq!(restored.checkpoint_hash(), host.canwu.checkpoint_hash());
    assert_eq!(plugin_payloads(&restored), plugin_payloads(&host.canwu));
    let mut resumed = Host::new(restored, Drive::Wake);
    resumed.skip_requests(&host);
    for day in SAVE_DAY + 1..=LAST_DAY {
        script(&mut host, day);
        script(&mut resumed, day);
    }
    assert_eq!(
        resumed.canwu.checkpoint_hash(),
        host.canwu.checkpoint_hash()
    );
}

/// The parametric step with one contingency draw a side, which moves the
/// casualties by up to a tenth.
#[derive(Clone, Copy, Debug, Default)]
struct DrawingResolver;

impl StepResolver for DrawingResolver {
    const IDENTITY: ResolverIdentity = ResolverIdentity {
        name: "test.drawing",
        version: 1,
    };

    fn resolve(
        &self,
        input: &StepInputV1<'_>,
        draws: &mut dyn StepDraws,
    ) -> Result<StepOutcomeV1, canwu_api::CanwuError> {
        let mut outcome = ParametricStepResolver.resolve(input, draws)?;
        let (attacker, _) = draws.draw(Side::Attacker, 0, 1, 21)?;
        let (defender, _) = draws.draw(Side::Defender, 0, 1, 21)?;
        outcome.attacker.casualties = outcome.attacker.casualties * (90 + attacker) / 100;
        outcome.defender.casualties = outcome.defender.casualties * (90 + defender) / 100;
        outcome.attacker.killed = outcome.attacker.killed.min(outcome.attacker.casualties);
        outcome.defender.killed = outcome.defender.killed.min(outcome.defender.casualties);
        Ok(outcome)
    }
}

#[test]
fn keyed_draws_reproduce_on_a_rerun_and_in_replay() {
    let (first, plugin) = campaign::<DrawingResolver>(Drive::Wake, InterceptMode::KeyedDraw);
    let (second, _) = campaign::<DrawingResolver>(Drive::Wake, InterceptMode::KeyedDraw);
    assert_eq!(
        first.canwu.checkpoint_hash(),
        second.canwu.checkpoint_hash()
    );
    assert_eq!(
        plugin_payloads(&first.canwu),
        plugin_payloads(&second.canwu)
    );
    let drawn: Vec<RandomDrawId> = first
        .canwu
        .random_draws()
        .iter()
        .map(|draw| draw.id)
        .collect();
    assert!(drawn.len() > 4, "the resolver and the crossing drew");
    let crossing = first
        .canwu
        .typed_domain_record(&crossing_reference(&crossing_id()))
        .unwrap()
        .decode_payload::<CrossingRecord>()
        .unwrap();
    assert!(crossing.last_wave.unwrap().draw.is_some());
    let replayed =
        Canwu::replay_from_journal(&[&plugin, &HostPlugin], &first.canwu.replay_journal()).unwrap();
    assert_eq!(replayed.checkpoint_hash(), first.canwu.checkpoint_hash());
}
