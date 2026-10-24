//! Every expected refusal of the engagement engine is a recorded rejection
//! or a held engagement; the boundary stands.

#![allow(clippy::too_many_lines)]

mod support;

use canwu_api::{
    Canwu, CanwuError, EntityRef, ErrorCode, KnowledgeHistoryView, KnowledgeHolderRef,
    KnowledgeQuery, MAX_KNOWLEDGE_PAGE_SIZE, OrganizationId,
};
use canwu_military::*;
use support::*;

fn engagement_id(value: &str) -> EngagementId {
    EngagementId::new(format!("test:engagement:{value}")).unwrap()
}

fn forces() -> Vec<ForceStateV1> {
    vec![
        force("red-1", 1, &red(), 1, &[("infantry", 3_000, infantry())]),
        force("blue-1", 2, &blue(), 1, &[("infantry", 2_000, infantry())]),
        force("red-2", 1, &red(), 2, &[("infantry", 2_000, infantry())]),
        force("blue-2", 2, &blue(), 2, &[("infantry", 500, infantry())]),
        force("red-3", 1, &red(), 3, &[("infantry", 1_000, infantry())]),
        force("blue-3", 2, &blue(), 1, &[("infantry", 900, infantry())]),
    ]
}

fn run<R: StepResolver>(rulebook: &CombatRulebookV1) -> (Host, MilitaryPlugin<R>) {
    let plugin = MilitaryPlugin::<R>::new();
    let records = scenario_records(rulebook, &forces()).unwrap();
    let canwu = Canwu::new_with_plugins(11, world(records), &[&plugin, &HostPlugin]).unwrap();
    (Host::new(canwu, Drive::Wake), plugin)
}

fn open(
    id: &str,
    site_of: u64,
    attackers: &[&str],
    defenders: &[&str],
    holders: [Vec<KnowledgeHolderRef>; 2],
) -> OpenEngagementV1 {
    OpenEngagementV1 {
        id: engagement_id(id),
        kind: EngagementKind::Field,
        profile: None,
        site: site(site_of),
        fort: None,
        attackers: attackers.iter().map(|value| force_id(value)).collect(),
        defenders: defenders.iter().map(|value| force_id(value)).collect(),
        holders,
        inputs: [condition("field", 700), condition("camp", 700)],
        environment: environment(false),
    }
}

fn open_command(open: OpenEngagementV1) -> MilitaryCommandV1 {
    MilitaryCommandV1::OpenEngagement(Box::new(open))
}

fn count(rejections: &[String], code: &str) -> usize {
    rejections
        .iter()
        .filter(|summary| summary.starts_with(code))
        .count()
}

#[test]
fn refused_commands_are_recorded_and_the_boundary_stands() {
    let rulebook = rulebook(&ParametricStepResolver::IDENTITY, InterceptMode::Threshold);
    let (mut host, _) = run::<ParametricStepResolver>(&rulebook);
    let organization = KnowledgeHolderRef::Entity(EntityRef::Organization(OrganizationId::new(9)));
    let mut wrong_channels = open("channels", 1, &["red-1"], &["blue-1"], [vec![], vec![]]);
    wrong_channels.inputs[0].channels.push(1);
    let mut wrong_posture = open("posture", 1, &["red-1"], &["blue-1"], [vec![], vec![]]);
    wrong_posture.inputs[1].posture = PostureId::new("test:posture:none").unwrap();
    let mut wrong_profile = open("profile", 1, &["red-1"], &["blue-1"], [vec![], vec![]]);
    wrong_profile.profile = Some(ProfileId::new("test:profile:none").unwrap());
    let mut wrong_terrain = open("terrain", 1, &["red-1"], &["blue-1"], [vec![], vec![]]);
    wrong_terrain.environment.terrain = Some(TerrainClassId::new("test:terrain:none").unwrap());
    let too_many_holders = open(
        "holders",
        1,
        &["red-1"],
        &["blue-1"],
        [
            vec![
                person(1),
                person(2),
                person(3),
                government(1),
                government(2),
            ],
            vec![],
        ],
    );
    host.day(
        0,
        &[
            // Refused at admission: a recorded rejected attempt.
            (
                red(),
                open_command(open("empty", 1, &[], &["blue-1"], [vec![], vec![]])),
            ),
            (
                red(),
                open_command(open(
                    "organization",
                    1,
                    &["red-1"],
                    &["blue-1"],
                    [vec![organization], vec![]],
                )),
            ),
            (red(), MilitaryCommandV1::Step { up_to: at_day(0) }),
            (
                red(),
                MilitaryCommandV1::InstallRulebook {
                    rulebook: Box::new(rulebook.clone()),
                },
            ),
            // A host integer past the plugin's bounds is refused before any
            // arithmetic reads it.
            (
                trusted(),
                MilitaryCommandV1::RegisterForce {
                    force: Box::new(force(
                        "huge",
                        1,
                        &red(),
                        1,
                        &[("infantry", MAX_COMPONENT_MEN + 1, infantry())],
                    )),
                },
            ),
            // Refused in phase 7: a rejection event.
            (
                trusted(),
                MilitaryCommandV1::InstallRulebook {
                    rulebook: Box::new(rulebook.clone()),
                },
            ),
            (
                red(),
                MilitaryCommandV1::RetireForce {
                    force: force_id("nobody"),
                    expected_host_revision: 0,
                },
            ),
            (
                red(),
                open_command(open("away", 2, &["red-1"], &["blue-1"], [vec![], vec![]])),
            ),
            (
                blue(),
                open_command(open(
                    "not-mine",
                    1,
                    &["red-1"],
                    &["blue-1"],
                    [vec![], vec![]],
                )),
            ),
            (red(), open_command(wrong_channels)),
            (red(), open_command(wrong_posture)),
            (red(), open_command(wrong_profile)),
            (red(), open_command(wrong_terrain)),
            (red(), open_command(too_many_holders)),
            // A force placed and opened in one boundary composes.
            (
                red(),
                MilitaryCommandV1::PlaceForce {
                    force: force_id("red-3"),
                    expected_host_revision: 0,
                    location: LocationV1::At { site: site(1) },
                },
            ),
            (
                red(),
                open_command(open(
                    "field",
                    1,
                    &["red-1", "red-3"],
                    &["blue-1"],
                    [vec![person(3), person(1)], vec![government(2)]],
                )),
            ),
            // Refusals against the fight opened just before.
            (
                red(),
                MilitaryCommandV1::PlaceForce {
                    force: force_id("red-1"),
                    expected_host_revision: 0,
                    location: LocationV1::At { site: site(4) },
                },
            ),
            (
                blue(),
                MilitaryCommandV1::RetireForce {
                    force: force_id("blue-1"),
                    expected_host_revision: 0,
                },
            ),
            (
                blue(),
                open_command(open("twice", 1, &["blue-3"], &["red-1"], [vec![], vec![]])),
            ),
            (
                red(),
                MilitaryCommandV1::ReviseForce {
                    force: force_id("red-2"),
                    expected_host_revision: 3,
                    components: None,
                    terms: None,
                    controllers: None,
                    clear_morale_shock: false,
                },
            ),
            (
                red(),
                MilitaryCommandV1::InvestSite(Box::new(InvestSiteV1 {
                    id: SiegeId::new("test:siege:keep").unwrap(),
                    site: site(2),
                    profile: SiteProfileV1 {
                        grade: 1,
                        accesses: 2,
                        water_accesses: 0,
                        repair_per_day: 0,
                    },
                    besiegers: vec![force_id("red-2")],
                    defenders: vec![force_id("blue-2")],
                    holders: [vec![], vec![]],
                    inputs: [condition("field", 700), condition("defend", 700)],
                    environment: environment(false),
                    work_bp: 0,
                    supply: SiegeSupplyInputV1 {
                        days_of_stock: 10,
                        ration: RationLevelId::new("test:ration:full").unwrap(),
                        evidence: Vec::new(),
                    },
                    naval: None,
                    storm_on: StormRule::Never,
                })),
            ),
        ],
    );
    let refused = refused_attempts(&host.canwu);
    assert_eq!(
        refused,
        vec![
            ErrorCode::InvalidPayload,
            ErrorCode::InvalidPayload,
            ErrorCode::InvalidAuthority,
            ErrorCode::InvalidAuthority,
            ErrorCode::ValueOutOfRange,
        ],
        "admission refusals"
    );
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 13, "{rejected:#?}");
    assert_eq!(count(&rejected, "EntityNotFound"), 1, "{rejected:#?}");
    assert_eq!(count(&rejected, "InvalidAuthority"), 1, "{rejected:#?}");
    assert_eq!(count(&rejected, "ValueOutOfRange"), 1, "{rejected:#?}");
    assert_eq!(
        count(&rejected, "DomainRecordVersionConflict"),
        1,
        "{rejected:#?}"
    );
    assert_eq!(count(&rejected, "InvalidPayload"), 9, "{rejected:#?}");
    let field = engagement(&host.canwu, &engagement_id("field"));
    assert_eq!(field.attacker.forces.len(), 2);
    assert_eq!(force_state(&host.canwu, "red-3").host_revision, 1);
    let siege = SiegeId::new("test:siege:keep").unwrap();
    assert!(registry(&host.canwu).unwrap().open_sieges.contains(&siege));

    // A holder who dies mid-fight is skipped; the rest still hear.
    host.kill(3);
    // A step in the same instant leaves the host revision a command names.
    let before = field.host_revision;
    host.day(
        1,
        &[(
            red(),
            MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
                siege: siege.clone(),
                expected_host_revision: 0,
                supply: None,
                profile: None,
                work_bp: None,
                naval: None,
                gate_open_steps: None,
                assault: None,
                sortie: false,
                storm_on: None,
            })),
        )],
    );
    assert!(engagement(&host.canwu, &engagement_id("field")).steps_resolved > 0);
    host.now(&[(
        red(),
        MilitaryCommandV1::SupplyInput {
            target: InputTargetV1::Engagement(engagement_id("field")),
            expected_host_revision: before,
            side: Some(Side::Attacker),
            condition: Some(condition("field", 650)),
            environment: None,
        },
    )]);
    assert_eq!(
        rejections(&host.canwu).len(),
        13,
        "{:#?}",
        rejections(&host.canwu)
    );
    let reports = |holder: KnowledgeHolderRef| {
        host.canwu
            .admin_query_knowledge(
                holder,
                &KnowledgeQuery {
                    schemas: vec![engagement_report_schema_id()],
                    view: KnowledgeHistoryView::FullHistory,
                    limit: MAX_KNOWLEDGE_PAGE_SIZE,
                    ..KnowledgeQuery::default()
                },
            )
            .unwrap()
            .records
            .len()
    };
    assert!(reports(person(1)) > 0);
    assert_eq!(reports(person(3)), 0, "a dead holder hears nothing");

    // A sortie: ordered once, then refused before the defenders' cooldown.
    host.day(
        2,
        &[(
            blue(),
            MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
                siege: siege.clone(),
                expected_host_revision: 1,
                supply: None,
                profile: None,
                work_bp: None,
                naval: None,
                gate_open_steps: None,
                assault: None,
                sortie: true,
                storm_on: None,
            })),
        )],
    );
    assert!(force_state(&host.canwu, "blue-2").battle.sortie_ready_at > Some(at_day(3)));
    host.day(
        3,
        &[(
            blue(),
            MilitaryCommandV1::ReviseSiege(Box::new(ReviseSiegeV1 {
                siege,
                expected_host_revision: 2,
                supply: None,
                profile: None,
                work_bp: None,
                naval: None,
                gate_open_steps: None,
                assault: None,
                sortie: true,
                storm_on: None,
            })),
        )],
    );
    let rejected = rejections(&host.canwu);
    assert_eq!(rejected.len(), 14, "{rejected:#?}");
    assert!(rejected[13].contains("cooldown"), "{rejected:#?}");
}

/// A resolver that always fails, before any draw.
#[derive(Clone, Copy, Debug, Default)]
struct FailingResolver;

impl StepResolver for FailingResolver {
    const IDENTITY: ResolverIdentity = ResolverIdentity {
        name: "test.failing",
        version: 1,
    };

    fn resolve(
        &self,
        _input: &StepInputV1<'_>,
        _draws: &mut dyn StepDraws,
    ) -> Result<StepOutcomeV1, CanwuError> {
        Err(CanwuError::new(
            ErrorCode::InvalidPayload,
            "the test resolver fails",
        ))
    }
}

fn held_then_abandoned<R: StepResolver>(rulebook: &CombatRulebookV1, reason: HoldReason) {
    let (mut host, _) = run::<R>(rulebook);
    host.day(
        0,
        &[(
            red(),
            open_command(open("held", 1, &["red-1"], &["blue-1"], [vec![], vec![]])),
        )],
    );
    host.day(1, &[]);
    host.day(2, &[]);
    let held = engagement(&host.canwu, &engagement_id("held"));
    assert_eq!(held.status, EngagementStatus::Held { reason });
    assert_eq!(held.steps_resolved, 0, "nothing is applied");
    assert!(
        host.canwu
            .events()
            .iter()
            .any(|event| event.kind.plugin_identity() == Some((PLUGIN_NAME, ENGAGEMENT_HELD_EVENT)))
    );
    host.day(
        3,
        &[(
            red(),
            MilitaryCommandV1::OrderAction {
                engagement: engagement_id("held"),
                expected_host_revision: held.host_revision,
                side: Side::Attacker,
                action: SideAction::Abandon,
            },
        )],
    );
    let summary = summaries(&host.canwu);
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].outcome, EngagementOutcomeKind::Abandoned);
    assert!(force_state(&host.canwu, "red-1").is_free());
    assert!(force_state(&host.canwu, "blue-1").is_free());
    assert!(rejections(&host.canwu).is_empty());
}

#[test]
fn an_oversized_step_and_a_failing_resolver_hold_until_abandoned() {
    let mut tight = rulebook(&ParametricStepResolver::IDENTITY, InterceptMode::Threshold);
    tight.limits.step_bytes = 64;
    held_then_abandoned::<ParametricStepResolver>(
        &tight.sealed().unwrap(),
        HoldReason::StepOversized,
    );
    held_then_abandoned::<FailingResolver>(
        &rulebook(&FailingResolver::IDENTITY, InterceptMode::Threshold),
        HoldReason::ResolverFailed,
    );
}

#[test]
fn a_rulebook_naming_another_resolver_is_refused_at_activation() {
    let other = rulebook(&FailingResolver::IDENTITY, InterceptMode::Threshold);
    let plugin = military_plugin();
    let records = scenario_records(&other, &forces()).unwrap();
    let error = Canwu::new_with_plugins(3, world(records), &[&plugin, &HostPlugin])
        .err()
        .expect("activation refuses the rulebook");
    assert!(error.message.contains("resolver"), "{error:?}");
}
