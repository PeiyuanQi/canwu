//! The engagement engine on the reference world: an army marches with the
//! reference world's movement plugin and the host places its force on
//! arrival; a field fight is stepped by the daily wake cadence and promoted
//! to half-hour steps; a place is invested, breached, and stormed; a river
//! crossing meets an interceptor; and the host receives each closed fight's
//! outcome signal and reads a government's reports.

use canwu_api::{
    Canwu, CanwuError, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, IngressClass,
    IngressPayload, Issuer, KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeQuery,
    MAX_KNOWLEDGE_PAGE_SIZE, PayloadSchema, PluginIngressDescriptor, PluginRegistrar, SimDuration,
    SimTime, SimulationPlugin, SystemCadence,
};
use canwu_military::{
    CrossingId, CrossingRecord, EngagementId, EngagementKind, EngagementRecord, EngagementReportV1,
    EngagementSummaryRecord, ForceRecord, InputTargetV1, InterceptorInputV1, InvestSiteV1,
    LocationV1, MilitaryCommandV1, MilitaryPlugin, OpenEngagementV1, OrderCrossingV1,
    ParametricStepResolver, RationLevelId, Side, SiegeId, SiegeRecord, SiegeSupplyInputV1,
    SiteProfileV1, StormRule, crossing_reference, engagement_outcome_schema_id,
    engagement_reference, force_reference, military_command, siege_reference,
};
use canwu_military_reference::{
    condition, demo_engagement_scenario, environment, force_id, reference_host, rival_government,
};
use canwu_military_reference_content::half_hour_profile;
use canwu_reference_world::{MovementCommand, ReferenceWorldPlugin, order_movement, snapshot};

const STARTER_HOST: &str = "starter-host";
const OUTCOME_INGRESS: &str = "outcome_v1";
const DAY: i64 = 1_440;

/// The host's tiny plugin: the target of the outcome signal.
struct StarterHost;

impl SimulationPlugin for StarterHost {
    fn name(&self) -> &'static str {
        STARTER_HOST
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000a1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: OUTCOME_INGRESS.to_owned(),
            description: "Receive one closed engagement's outcome".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })
    }
}

/// Enqueues commands at the run's time; each advances the revision by one.
fn enqueue(
    canwu: &mut Canwu,
    next: &mut u64,
    commands: Vec<CommandEnvelope>,
) -> Result<(), Box<dyn std::error::Error>> {
    let at = canwu.time();
    let base = canwu.revision();
    for (index, envelope) in commands.into_iter().enumerate() {
        canwu.enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(*next),
                base + u64::try_from(index)?,
                envelope.at_time(at),
            ),
        )?;
        *next += 1;
    }
    Ok(())
}

fn military(issuer: &Issuer, command: &MilitaryCommandV1) -> Result<CommandEnvelope, CanwuError> {
    Ok(CommandEnvelope::new(
        issuer.clone(),
        military_command(command)?,
    ))
}

fn field() -> EngagementId {
    EngagementId::new("canwu:military:engagement:eastern-field").expect("valid id")
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (scenario, ids) = demo_engagement_scenario()?;
    let world = ReferenceWorldPlugin;
    let engine = MilitaryPlugin::<ParametricStepResolver>::new()
        .with_outcome_signal(STARTER_HOST, OUTCOME_INGRESS);
    let plugins: [&dyn SimulationPlugin; 3] = [&world, &engine, &StarterHost];
    let mut canwu = Canwu::new_with_plugins(35, scenario, &plugins)?;
    let host = reference_host();
    let yun = Issuer::Actor(ids.commander);
    let ours = KnowledgeHolderRef::Entity(EntityRef::Government(ids.government));
    let theirs = KnowledgeHolderRef::Entity(EntityRef::Government(rival_government()));
    let place = |territory| EntityRef::Territory(territory);
    let siege = SiegeId::new("canwu:military:siege:westford")?;
    let crossing = CrossingId::new("canwu:military:crossing:capital-ford")?;
    let mut next = 1;

    // Day 0: the army marches east; the siege of Westford and the crossing begin.
    enqueue(
        &mut canwu,
        &mut next,
        vec![
            order_movement(
                yun.clone(),
                &MovementCommand {
                    subject: EntityRef::Army(ids.army),
                    destination: ids.eastern_territory,
                    cargo: Vec::new(),
                },
            )?,
            military(
                &yun,
                &MilitaryCommandV1::InvestSite(Box::new(InvestSiteV1 {
                    id: siege.clone(),
                    site: place(ids.western_territory),
                    profile: SiteProfileV1 {
                        grade: 2,
                        accesses: 4,
                        water_accesses: 0,
                        repair_per_day: 50,
                    },
                    besiegers: vec![force_id("yun-siege")],
                    defenders: vec![force_id("rival-garrison")],
                    holders: [vec![ours.clone()], vec![theirs.clone()]],
                    inputs: [condition("field", 750), condition("defend", 650)],
                    environment: environment(false),
                    work_bp: 10_000,
                    supply: SiegeSupplyInputV1 {
                        days_of_stock: 30,
                        ration: RationLevelId::new("canwu:military:ration:full")?,
                        evidence: Vec::new(),
                    },
                    naval: None,
                    storm_on: StormRule::BreachOrGate,
                })),
            )?,
            military(
                &yun,
                &MilitaryCommandV1::OrderCrossing(Box::new(OrderCrossingV1 {
                    id: crossing.clone(),
                    force: force_id("yun-river"),
                    from: place(ids.central_territory),
                    to: place(ids.western_territory),
                    night: false,
                    defenders: Vec::new(),
                    holders: [vec![ours.clone()], vec![theirs.clone()]],
                    inputs: [condition("march", 700), condition("defend", 700)],
                    environment: environment(false),
                    interceptor: Some(InterceptorInputV1 {
                        force: force_id("rival-fleet"),
                        control_bp: 9_000,
                        scouts: 10,
                        evidence: Vec::new(),
                    }),
                })),
            )?,
        ],
    )?;
    canwu.advance_canonical(SimDuration::ZERO)?;

    for day in 1..=10 {
        let at = SimTime::from_minutes(day * DAY);
        let now = canwu.time();
        canwu.advance_canonical(at.checked_sub(now).unwrap_or(SimDuration::ZERO))?;
        canwu.schedule_calendar_boundary(at, vec![SystemCadence::Daily])?;
        let mut commands = Vec::new();
        let field_state = canwu
            .typed_domain_record(&engagement_reference(&field()))
            .map(canwu_api::DomainRecord::decode_payload::<EngagementRecord>)
            .transpose()?;
        let field_open = field_state.as_ref().is_some_and(|state| !state.is_closed());
        match day {
            1 => {
                // The army arrived with the reference world's movement; the
                // host places its force there and opens the fight.
                let army = snapshot(&canwu)?
                    .army(ids.army)
                    .map(|army| army.location)
                    .ok_or("the reference army exists")?;
                assert_eq!(army, ids.eastern_territory, "the army arrived");
                let mirror = canwu
                    .typed_domain_record(&force_reference(&force_id("yun-field")))
                    .ok_or("the field force exists")?
                    .decode_payload::<ForceRecord>()?;
                commands.push(military(
                    &yun,
                    &MilitaryCommandV1::PlaceForce {
                        force: force_id("yun-field"),
                        expected_host_revision: mirror.host_revision,
                        location: LocationV1::At { site: place(army) },
                    },
                )?);
                commands.push(military(
                    &yun,
                    &MilitaryCommandV1::OpenEngagement(Box::new(OpenEngagementV1 {
                        id: field(),
                        kind: EngagementKind::Field,
                        profile: None,
                        site: place(ids.eastern_territory),
                        fort: None,
                        attackers: vec![force_id("yun-field")],
                        defenders: vec![force_id("rival-field")],
                        holders: [
                            vec![ours.clone(), KnowledgeHolderRef::Person(ids.commander)],
                            vec![theirs.clone()],
                        ],
                        inputs: [condition("field", 720), condition("camp", 680)],
                        environment: environment(false),
                    })),
                )?);
            }
            2..=4 if field_open => {
                let revision = field_state.as_ref().map_or(0, |state| state.host_revision);
                for (offset, (issuer, side, posture, morale)) in [
                    (yun.clone(), Side::Attacker, "field", 720 - 10 * day),
                    (host.clone(), Side::Defender, "camp", 680 - 20 * day),
                ]
                .into_iter()
                .enumerate()
                {
                    commands.push(military(
                        &issuer,
                        &MilitaryCommandV1::SupplyInput {
                            target: InputTargetV1::Engagement(field()),
                            expected_host_revision: revision + u64::try_from(offset)?,
                            side: Some(side),
                            condition: Some(condition(posture, u16::try_from(morale)?)),
                            environment: None,
                        },
                    )?);
                }
                if day == 3 {
                    commands.push(military(
                        &host,
                        &MilitaryCommandV1::ReviseStepProfile {
                            engagement: field(),
                            expected_host_revision: revision + 2,
                            profile: half_hour_profile(),
                        },
                    )?);
                }
            }
            _ => {}
        }
        if day == 2 {
            // The river force waits for night after its first wave was caught.
            let state = canwu
                .typed_domain_record(&crossing_reference(&crossing))
                .ok_or("the crossing exists")?
                .decode_payload::<CrossingRecord>()?;
            commands.push(military(
                &yun,
                &MilitaryCommandV1::ReviseCrossing {
                    crossing: crossing.clone(),
                    expected_host_revision: state.host_revision,
                    interceptor: None,
                    night: Some(true),
                },
            )?);
        }
        enqueue(&mut canwu, &mut next, commands)?;
        canwu.advance_canonical(SimDuration::ZERO)?;
    }

    let mut summaries: Vec<_> = canwu
        .domain_records()
        .filter(|record| {
            record
                .reference
                .kind
                .matches_type::<EngagementSummaryRecord>()
        })
        .map(canwu_api::DomainRecord::decode_payload::<EngagementSummaryRecord>)
        .collect::<Result<_, _>>()?;
    summaries.sort_by_key(|summary| summary.closed_at);
    for summary in &summaries {
        println!(
            "engagement={} kind={:?} outcome={:?} steps={} site_fell={} losses={}/{}",
            summary.engagement,
            summary.kind,
            summary.outcome,
            summary.steps,
            summary.site_fell,
            summary.attacker.losses.total(),
            summary.defender.losses.total()
        );
    }
    let siege_state = canwu
        .typed_domain_record(&siege_reference(&siege))
        .ok_or("the siege exists")?
        .decode_payload::<SiegeRecord>()?;
    let crossing_state = canwu
        .typed_domain_record(&crossing_reference(&crossing))
        .ok_or("the crossing exists")?
        .decode_payload::<CrossingRecord>()?;
    println!(
        "siege={:?} cause={:?} crossing={:?} landed={}",
        siege_state.status, siege_state.end_cause, crossing_state.status, crossing_state.landed
    );
    let reports = canwu.admin_query_knowledge(
        ours,
        &KnowledgeQuery {
            schemas: vec![engagement_outcome_schema_id()],
            view: KnowledgeHistoryView::FullHistory,
            limit: MAX_KNOWLEDGE_PAGE_SIZE,
            ..KnowledgeQuery::default()
        },
    )?;
    for record in reports.records {
        let report: EngagementReportV1 = serde_json::from_value(record.payload)?;
        println!(
            "report subject={:?} side={:?} closing={} engagement={:?}",
            report.subject,
            report.side,
            report.closing,
            report
                .engagement
                .map(|part| (part.outcome, part.own_losses.total()))
        );
    }
    let signals = canwu
        .ingress_log()
        .iter()
        .filter(|record| {
            matches!(&record.payload, IngressPayload::Plugin { plugin, .. } if plugin == STARTER_HOST)
        })
        .count();
    assert_eq!(
        signals,
        summaries.len(),
        "every closed fight signalled the host"
    );
    assert!(summaries.iter().any(|summary| summary.site_fell));

    let saved = canwu.snapshot_json()?;
    let restored = Canwu::from_snapshot_json_with_plugins(&saved, &plugins)?;
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())?;
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    println!(
        "military_starter=complete engagements={} signals={signals} checkpoint={}",
        summaries.len(),
        canwu.checkpoint_hash()
    );
    Ok(())
}
