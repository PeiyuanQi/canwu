//! A small synthetic world, rulebook, and host plugin for the engagement
//! engine's contract tests. Every value here is a test fixture, not content.

#![allow(dead_code, clippy::missing_panics_doc, clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope, CommandRequest, CommandRequestId,
    CustodyState, DomainRecord, EntityRef, ErrorCode, GovernmentId, IngressClass, IngressPayload,
    Issuer, KnowledgeHolderRef, LifeState, PayloadSchema, PersonAvailability, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, TerritoryId,
};
use canwu_military::*;
use std::collections::{BTreeMap, BTreeSet};

pub const HOST_PLUGIN: &str = "test-host";
pub const OUTCOME_INGRESS: &str = "outcome_v1";
pub const LIFE_INGRESS: &str = "life_v1";
pub const DAY: i64 = 1_440;

pub fn force_id(value: &str) -> ForceId {
    ForceId::new(format!("test:force:{value}")).unwrap()
}

pub fn red() -> Issuer {
    Issuer::Actor(PersonId::new(1))
}

pub fn blue() -> Issuer {
    Issuer::Actor(PersonId::new(2))
}

pub fn trusted() -> Issuer {
    Issuer::System(HOST_PLUGIN.to_owned())
}

pub fn site(territory: u64) -> EntityRef {
    EntityRef::Territory(TerritoryId::new(territory))
}

pub fn person(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(id))
}

pub fn government(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(id)))
}

pub fn force_holder(value: &str) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Domain(
        force_reference(&force_id(value)).into_untyped(),
    ))
}

pub fn at_day(day: i64) -> SimTime {
    SimTime::from_minutes(day * DAY)
}

fn terrain(value: &str) -> TerrainClassId {
    TerrainClassId::new(format!("test:terrain:{value}")).unwrap()
}

pub fn posture(value: &str) -> PostureId {
    PostureId::new(format!("test:posture:{value}")).unwrap()
}

pub fn ground(value: &str) -> GroundClassId {
    GroundClassId::new(format!("test:ground:{value}")).unwrap()
}

pub fn concealment(value: &str) -> ConcealmentClassId {
    ConcealmentClassId::new(format!("test:concealment:{value}")).unwrap()
}

pub fn day_profile() -> ProfileId {
    ProfileId::new("test:profile:day").unwrap()
}

pub fn half_hour_profile() -> ProfileId {
    ProfileId::new("test:profile:half-hour").unwrap()
}

/// A synthetic rulebook for resolver `identity`, intercepting by `mode`.
pub fn rulebook(identity: &ResolverIdentity, mode: InterceptMode) -> CombatRulebookV1 {
    let term = |value: &str| TermId::new(format!("test:term:{value}")).unwrap();
    let band = |value: &str| ForecastBandId::new(format!("test:band:{value}")).unwrap();
    let row =
        |frontage_bp, melee_bp, shock_bp, defense_bp, charge_window_bp, pursuit_bp| TerrainRowV1 {
            frontage_bp,
            melee_bp,
            shock_bp,
            defense_bp,
            charge_window_bp,
            pursuit_bp,
        };
    let kind = |frontage_bp, days: u32, terrain: Option<TerrainClassId>, fort| KindRuleV1 {
        frontage_bp,
        max_minutes: days * 1_440,
        terrain,
        fort,
        posture_bp: vec![PostureModifierV1 {
            attacker: posture("field"),
            defender: posture("camp"),
            fort: FortState::Any,
            bp: 12_000,
        }],
        posture_steps: 1,
    };
    CombatRulebookV1 {
        id: RulebookId::new("test:rulebook:synthetic").unwrap(),
        schema_version: SCHEMA_VERSION,
        rules_hash: String::new(),
        resolver: identity.owned(),
        trusted_issuers: vec![trusted()],
        step_profiles: BTreeMap::from([
            (
                day_profile(),
                StepProfileV1 {
                    step_minutes: 1_440,
                    step_scale_bp: 230_400,
                    max_steps_per_boundary: 4,
                },
            ),
            (
                half_hour_profile(),
                StepProfileV1 {
                    step_minutes: 30,
                    step_scale_bp: 10_000,
                    max_steps_per_boundary: 48,
                },
            ),
        ]),
        default_profile: day_profile(),
        kinds: BTreeMap::from([
            (
                EngagementKind::Field,
                kind(None, 10, Some(terrain("open")), FortUse::None),
            ),
            (
                EngagementKind::Assault,
                kind(Some(4_000), 5, None, FortUse::Site),
            ),
            (
                EngagementKind::Sortie,
                kind(Some(3_000), 1, None, FortUse::None),
            ),
            (
                EngagementKind::Relief,
                kind(None, 5, Some(terrain("open")), FortUse::None),
            ),
            (
                EngagementKind::FerryFight,
                kind(Some(2_000), 1, None, FortUse::None),
            ),
            (
                EngagementKind::BridgeheadFight,
                kind(None, 1, None, FortUse::None),
            ),
        ]),
        condition_channels: vec![
            ChannelId::new("test:channel:information").unwrap(),
            ChannelId::new("test:channel:terrain-fit").unwrap(),
        ],
        band_cuts: vec![800, 600, 400, 200],
        forecast: ForecastRulesV1 {
            terms: vec![
                StrengthTermV1 {
                    id: term("manpower"),
                    source: TermSource::Manpower,
                    divisor: 1,
                    cap: 30,
                },
                StrengthTermV1 {
                    id: term("readiness"),
                    source: TermSource::Readiness,
                    divisor: 50,
                    cap: 20,
                },
                StrengthTermV1 {
                    id: term("morale"),
                    source: TermSource::Morale,
                    divisor: 100,
                    cap: 10,
                },
                StrengthTermV1 {
                    id: term("cohesion"),
                    source: TermSource::Cohesion,
                    divisor: 100,
                    cap: 10,
                },
                StrengthTermV1 {
                    id: term("information"),
                    source: TermSource::Channel { index: 0 },
                    divisor: 100,
                    cap: 10,
                },
                StrengthTermV1 {
                    id: term("training"),
                    source: TermSource::Training,
                    divisor: 1_000,
                    cap: 10,
                },
                StrengthTermV1 {
                    id: term("command"),
                    source: TermSource::Command,
                    divisor: 100,
                    cap: 10,
                },
                StrengthTermV1 {
                    id: term("terrain-fit"),
                    source: TermSource::TerrainFit,
                    divisor: 1,
                    cap: 10,
                },
                StrengthTermV1 {
                    id: term("environment"),
                    source: TermSource::Environment,
                    divisor: 1,
                    cap: 5,
                },
            ],
            manpower_bands: vec![
                ManpowerBandV1 {
                    from_men: 0,
                    value: 5,
                },
                ManpowerBandV1 {
                    from_men: 1_000,
                    value: 14,
                },
                ManpowerBandV1 {
                    from_men: 3_000,
                    value: 22,
                },
            ],
            terrain_fit: vec![TerrainFitRowV1 {
                terrain: terrain("walls"),
                trait_: None,
                defending: Some(true),
                delta: 5,
            }],
            environment_terms: vec![EnvironmentTermV1 {
                ground: Some(ground("mud")),
                ranged_reliability_below_bp: None,
                trait_: None,
                ranged_share_from_per_mille: None,
                delta: -2,
            }],
            ratio_bands: vec![
                RatioBandV1 {
                    min_ratio_bp: 15_000,
                    fortified: None,
                    band: band("advance"),
                },
                RatioBandV1 {
                    min_ratio_bp: 11_000,
                    fortified: Some(true),
                    band: band("hold"),
                },
                RatioBandV1 {
                    min_ratio_bp: 11_000,
                    fortified: None,
                    band: band("advance"),
                },
                RatioBandV1 {
                    min_ratio_bp: 9_000,
                    fortified: None,
                    band: band("stall"),
                },
                RatioBandV1 {
                    min_ratio_bp: 0,
                    fortified: None,
                    band: band("withdraw"),
                },
            ],
        },
        lines: LineRulesV1 {
            fire_weight: 3,
            shock_weight: 4,
            melee_weight: 3,
            defense_offset: 200,
            inflicted_cap_per_mille: 30,
            killed_numerator: 1,
            killed_denominator: 3,
            supply_fire: SupplyFireRuleV1 {
                below_per_mille: 500,
                bp: 7_000,
            },
            leadership: LeadershipRuleV1 {
                base_bp: 8_000,
                command_weight: 20,
                pick_weight: 10,
                min_bp: 6_000,
                max_bp: 12_000,
            },
        },
        cohesion: CohesionRuleV1 {
            per_inflicted: 4,
            base: 1,
            confusion: 50,
            recover_per_day: 10,
        },
        morale: MoraleRuleV1 {
            sensitivity_default: 10,
            credibility_divisor: 40,
            shock_recover_per_day: 20,
        },
        break_rule: BreakRuleV1 {
            rout_cohesion_below: 200,
            rout_morale_margin: 150,
            walled_holds_from_grade: 2,
        },
        pursuit: PursuitRuleV1 {
            captured_per_mille_at_full: 300,
            wounded_left_per_mille: 500,
        },
        terrain: BTreeMap::from([
            (
                terrain("open"),
                row(3_000, 10_000, 12_000, 10_000, 10_000, 10_000),
            ),
            (
                terrain("walls"),
                row(2_000, 9_000, 3_000, 12_000, 5_000, 5_000),
            ),
            (
                terrain("breach"),
                row(3_000, 10_000, 5_000, 10_000, 5_000, 8_000),
            ),
            (
                terrain("river"),
                row(2_000, 8_000, 2_000, 10_000, 3_000, 6_000),
            ),
        ]),
        night: NightFactorsV1 {
            fire_bp: 5_000,
            shock_bp: 3_000,
            melee_bp: 7_000,
            pursuit_bp: 3_000,
        },
        fortification: vec![
            FortGradeV1 {
                grade: 0,
                fort_bp: 10_000,
                breach_threshold: 100,
            },
            FortGradeV1 {
                grade: 1,
                fort_bp: 15_000,
                breach_threshold: 300,
            },
            FortGradeV1 {
                grade: 2,
                fort_bp: 20_000,
                breach_threshold: 600,
            },
        ],
        postures: vec![
            posture("field"),
            posture("camp"),
            posture("march"),
            posture("defend"),
        ],
        siege: SiegeRulesV1 {
            day_minutes: 1_440,
            men_per_access_sealed: 500,
            emplacement_days: BTreeMap::from([(ground("firm"), 1), (ground("mud"), 2)]),
            sortie_cooldown_minutes: 2 * 1_440,
            water_access_control_bp: 6_000,
            assault_terrain: terrain("walls"),
            breach_terrain: terrain("breach"),
            sortie_terrain: terrain("open"),
        },
        crossing: CrossingRulesV1 {
            wave_minutes: 1_440,
            wave_men: 1_000,
            hard_river_wave_bp: 5_000,
            bridgehead_secure_men: 1_500,
            terrain: terrain("river"),
            intercept: InterceptRuleV1 {
                min_control_bp: 4_000,
                control_weight: 10_000,
                scout_weight: 200,
                concealment: BTreeMap::from([
                    (concealment("clear"), 0),
                    (concealment("fog"), 5_000),
                ]),
                night_concealment: 20_000,
                threshold_bp: 5_000,
                mode,
            },
        },
        limits: CombatLimitsV1 {
            forces: 64,
            components_per_force: 16,
            forces_per_side: 4,
            controllers_per_force: 4,
            trusted_issuers: 4,
            holders_per_side: 4,
            open_engagements: 8,
            open_sieges: 4,
            open_crossings: 4,
            step_bytes: 2_048,
            draws_per_step: 16,
            retire_after_minutes: 5 * 1_440,
            evidence_refs_per_input: 4,
            report_queue: 16,
        },
    }
    .sealed()
    .unwrap()
}

pub fn infantry() -> ComponentValuesV1 {
    ComponentValuesV1 {
        fire: 400,
        melee: 1_000,
        defense: 800,
        training_bp: 6_000,
        equipment_bp: 5_000,
        ranged: true,
        ..ComponentValuesV1::default()
    }
}

pub fn cavalry() -> ComponentValuesV1 {
    ComponentValuesV1 {
        melee: 800,
        shock: 1_500,
        defense: 400,
        training_bp: 6_000,
        equipment_bp: 5_000,
        mounted: true,
        ..ComponentValuesV1::default()
    }
}

pub fn guns() -> ComponentValuesV1 {
    ComponentValuesV1 {
        fire: 200,
        defense: 100,
        training_bp: 5_000,
        breach_per_piece: 10,
        crew_per_piece_x10: 100,
        ranged: true,
        ..ComponentValuesV1::default()
    }
}

pub fn vessels() -> ComponentValuesV1 {
    ComponentValuesV1 {
        fire: 600,
        melee: 600,
        defense: 900,
        training_bp: 7_000,
        vessel: true,
        ranged: true,
        ..ComponentValuesV1::default()
    }
}

pub fn force(
    value: &str,
    owner: u64,
    controller: &Issuer,
    territory: u64,
    components: &[(&str, u64, ComponentValuesV1)],
) -> ForceStateV1 {
    ForceStateV1 {
        schema_version: SCHEMA_VERSION,
        id: force_id(value),
        owner: EntityRef::Government(GovernmentId::new(owner)),
        controllers: vec![controller.clone()],
        host_revision: 0,
        location: LocationV1::At {
            site: site(territory),
        },
        components: components
            .iter()
            .map(|(name, present, values)| {
                (
                    ComponentId::new(format!("test:component:{name}")).unwrap(),
                    ComponentStateV1 {
                        present: *present,
                        values: values.clone(),
                        losses: LossTallyV1::default(),
                    },
                )
            })
            .collect(),
        terms: SideTermsV1 {
            cohesion_cap: 800,
            morale_break: 400,
            morale_sensitivity: 10,
            battle_command: 100,
            assessment: 100,
            credibility: 20,
            traits: BTreeSet::new(),
        },
        battle: BattleStateV1::default(),
        status: ForceStatus::Active,
    }
}

pub fn condition(posture_name: &str, morale: u16) -> ConditionInputV1 {
    ConditionInputV1 {
        channels: vec![600, 500],
        supply_per_mille: 900,
        morale_per_mille: morale,
        readiness_per_mille: 800,
        fatigue_per_mille: 100,
        posture: posture(posture_name),
        surprise_bp: 0,
        order_confusion: 0,
        defense_halved: false,
        evidence: Vec::new(),
    }
}

pub fn environment(night: bool) -> EnvironmentInputV1 {
    EnvironmentInputV1 {
        terrain: None,
        ground: ground("firm"),
        concealment: concealment("clear"),
        night,
        ranged_reliability_bp: 10_000,
        river: RiverBand::Open,
        evidence: Vec::new(),
    }
}

/// The test world: persons 1–3, governments 1–2, territories 1–5.
pub fn world(records: Vec<DomainRecord>) -> Scenario {
    let mut entities = vec![
        EntityRef::Person(PersonId::new(1)),
        EntityRef::Person(PersonId::new(2)),
        EntityRef::Person(PersonId::new(3)),
        EntityRef::Government(GovernmentId::new(1)),
        EntityRef::Government(GovernmentId::new(2)),
    ];
    entities.extend((1..=5).map(site));
    Scenario::new(SimTime::EPOCH, entities).with_domain_records(records)
}

/// The host: the outcome-signal target and a switch that kills a person.
pub struct HostPlugin;

impl SimulationPlugin for HostPlugin {
    fn name(&self) -> &str {
        HOST_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000d7"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for (name, description) in [
            (OUTCOME_INGRESS, "Receive one closed engagement's outcome"),
            (LIFE_INGRESS, "Kill one person"),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let mut life = BoundarySystemContract::new(
            "host-life-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        life.reads = vec![StateKey::core_ingress()];
        life.writes = vec![StateKey::core_person_availability()];
        life.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(life, host_life)
    }
}

fn host_life(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == HOST_PLUGIN
            && packet_type == LIFE_INGRESS
        {
            directives.push(BoundaryDirective::SetPersonAvailability {
                person: PersonId::new(payload["person"].as_u64().unwrap_or(0)),
                availability: PersonAvailability::new(
                    LifeState::Dead,
                    CustodyState::Free,
                    view.time(),
                ),
                summary: "Kill one person".to_owned(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Who produces the step packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Drive {
    /// The wake system on daily calendar boundaries.
    Wake,
    /// The host's `Step` command, sent when work is due.
    Step,
}

/// A run and the host's request counter.
pub struct Host {
    pub canwu: Canwu,
    pub drive: Drive,
    next_request: u64,
}

impl Host {
    pub fn new(canwu: Canwu, drive: Drive) -> Self {
        Self {
            canwu,
            drive,
            next_request: 1,
        }
    }

    /// Continues another host's request numbering (a restored run).
    pub fn skip_requests(&mut self, other: &Self) {
        self.next_request = other.next_request;
    }

    /// Enqueues commands at the run's current time, in this order: each
    /// admitted command advances the run's revision by one.
    pub fn enqueue(&mut self, commands: &[(Issuer, MilitaryCommandV1)]) {
        let at = self.canwu.time();
        let base = self.canwu.revision();
        for (index, (issuer, command)) in commands.iter().enumerate() {
            let request = self.next_request;
            self.next_request += 1;
            self.canwu
                .enqueue_command(
                    at,
                    0,
                    CommandRequest::new(
                        CommandRequestId::new(request),
                        base + index as u64,
                        CommandEnvelope::new(issuer.clone(), military_command(command).unwrap())
                            .at_time(at),
                    ),
                )
                .unwrap();
        }
    }

    /// Settles every boundary up to `at`.
    pub fn settle_to(&mut self, at: SimTime) {
        let now = self.canwu.time();
        self.canwu
            .advance_canonical(at.checked_sub(now).unwrap_or(SimDuration::ZERO))
            .unwrap();
    }

    /// Opens day `day` with the host's commands: in `Wake` drive a daily
    /// calendar boundary wakes the engine; in `Step` drive the host sends
    /// `Step` after its commands whenever the registry has work due.
    pub fn day(&mut self, day: i64, commands: &[(Issuer, MilitaryCommandV1)]) {
        let at = at_day(day);
        self.settle_to(at);
        let due = registry(&self.canwu).and_then(|registry| registry.earliest_due);
        match self.drive {
            Drive::Wake => {
                self.canwu
                    .schedule_calendar_boundary(at, vec![SystemCadence::Daily])
                    .unwrap();
                self.enqueue(commands);
            }
            Drive::Step => {
                let mut commands = commands.to_vec();
                if due.is_some_and(|due| due <= at) {
                    commands.push((trusted(), MilitaryCommandV1::Step { up_to: at }));
                }
                self.enqueue(&commands);
            }
        }
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }

    /// Commands settled at the current time, outside the day cadence.
    pub fn now(&mut self, commands: &[(Issuer, MilitaryCommandV1)]) {
        self.enqueue(commands);
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }

    pub fn kill(&mut self, person: u64) {
        let at = self.canwu.time();
        self.canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                HOST_PLUGIN,
                LIFE_INGRESS,
                at,
                serde_json::json!({ "person": person }),
            ))
            .unwrap();
        self.canwu.advance_canonical(SimDuration::ZERO).unwrap();
    }
}

pub fn registry(canwu: &Canwu) -> Option<CombatRegistryV1> {
    canwu
        .typed_domain_record(&registry_reference())
        .map(|record| record.decode_payload::<RegistryRecord>().unwrap())
}

pub fn force_state(canwu: &Canwu, value: &str) -> ForceStateV1 {
    canwu
        .typed_domain_record(&force_reference(&force_id(value)))
        .unwrap()
        .decode_payload::<ForceRecord>()
        .unwrap()
}

pub fn engagement(canwu: &Canwu, engagement: &EngagementId) -> EngagementStateV1 {
    canwu
        .typed_domain_record(&engagement_reference(engagement))
        .unwrap()
        .decode_payload::<EngagementRecord>()
        .unwrap()
}

pub fn summaries(canwu: &Canwu) -> Vec<EngagementSummaryV1> {
    canwu
        .domain_records()
        .filter(|record| {
            record
                .reference
                .kind
                .matches_type::<EngagementSummaryRecord>()
        })
        .map(|record| record.decode_payload::<EngagementSummaryRecord>().unwrap())
        .collect()
}

/// The summaries of rejected commands, in order.
pub fn rejections(canwu: &Canwu) -> Vec<String> {
    canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, MILITARY_REJECTION_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect()
}

/// The codes of rejected command attempts (admission), in order.
pub fn refused_attempts(canwu: &Canwu) -> Vec<ErrorCode> {
    canwu
        .command_attempts()
        .iter()
        .filter_map(|attempt| match &attempt.outcome {
            CommandAttemptOutcome::Rejected { error } => Some(error.code.clone()),
            CommandAttemptOutcome::Accepted { .. } => None,
        })
        .collect()
}

/// Every plugin record's payload, by reference.
pub fn plugin_payloads(canwu: &Canwu) -> BTreeMap<String, serde_json::Value> {
    canwu
        .domain_records()
        .filter(|record| record.owner == PLUGIN_NAME)
        .map(|record| (record.reference.to_string(), record.payload.clone()))
        .collect()
}
