//! Downstream gap set §27 (gap G-35): the culture boundary system settles
//! exposure inside the engine, hands its society delta to the society owner,
//! and emits cultural signals as next-boundary ingress.

use canwu_api::{
    BoundaryId, BoundaryPhase, BoundaryProposal, BoundaryRequest, BoundarySystemContract, Canwu,
    CanwuError, DomainRecordLifecycle, DomainRecordVersionRef, DomainRecordVersionSource,
    ErrorCode, EventKind, EvidenceRef, IngressPayload, PluginIngressRequest, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin, StateKey, StateVisibility, SystemCadence,
};
use canwu_culture::{
    CULTURAL_SIGNAL_INGRESS, CULTURE_EXPOSURE_INGRESS, CULTURE_EXPOSURE_REJECTED_EVENT,
    CULTURE_LIFECYCLE_SYSTEM, CulturalEffectBinding, CulturalSignalBatch, CultureBoundaryPlugin,
    CultureCohortDefinition, CultureDefinition, CultureExposureSignalBatch, CultureLifecycle,
    CultureRuntime, EffectPersistence, RetirementPolicy, TransitionSpec, compile_culture,
    culture_definition_record, culture_definition_reference, install_into_society,
    load_culture_runtime,
};
use canwu_society::{
    SOCIETY_LIFECYCLE_DELTA_INGRESS, SocietyCohortExchangeLedgerRecord, SocietyIngressStatus,
    SocietyPlugin, SocietyState, load_society_state, society_cohort_exchange_ledger_reference,
    validate_society_runtime,
};

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g35_culture_boundary_system() {
    let definition = definition();
    let plan = compile_culture(&definition).expect("compile culture plan");
    let plugins: [&dyn SimulationPlugin; 2] = [&SocietyPlugin, &CultureBoundaryPlugin];
    let mut canwu =
        Canwu::new_with_plugins(7, scenario(&definition), &plugins).expect("culture simulation");

    // The society plugin stays the only owner of its state: a second
    // plugin declaring the same write is rejected at registration, and the
    // host cannot author the internal lifecycle-delta packet.
    let with_rogue: [&dyn SimulationPlugin; 3] =
        [&SocietyPlugin, &CultureBoundaryPlugin, &RogueSocietyWriter];
    let error = Canwu::new_with_plugins(7, scenario(&definition), &with_rogue)
        .err()
        .expect("a second society writer must be rejected");
    assert_eq!(error.code, ErrorCode::DuplicateStateOwner);
    assert!(
        canwu
            .plugin_descriptors()
            .filter(|descriptor| descriptor.name == "canwu-culture")
            .flat_map(|descriptor| &descriptor.boundary_systems)
            .all(|system| !system.writes.contains(&society_state_key()))
    );
    assert_eq!(
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canwu-society",
                SOCIETY_LIFECYCLE_DELTA_INGRESS,
                canwu.time(),
                serde_json::json!({}),
            ))
            .expect_err("the lifecycle delta is internal ingress")
            .code,
        ErrorCode::InvalidAuthority
    );

    // One exposure for the current generation and one citing a generation
    // that does not exist are admitted and queued.
    let evidence = EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
        record: culture_definition_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    });
    for generation in [1, 2] {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canwu-culture",
                CULTURE_EXPOSURE_INGRESS,
                canwu.time(),
                serde_json::to_value(CultureExposureSignalBatch {
                    target_id: "equality".to_owned(),
                    target_generation: generation,
                    cohort_scope: vec!["town".to_owned()],
                    fidelity_per_mille: 800,
                    evidence: vec![evidence.clone()],
                    earliest_boundary: BoundaryId::new(0),
                })
                .expect("exposure payload"),
            ))
            .expect("queue an exposure batch");
    }
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit exposure");
    daily(&mut canwu, 1);

    // First Monthly boundary: the plugin consumes the exposure, keeps the
    // exposed target active, rejects the stale generation, and emits the
    // compiled effect as next-boundary ingress.
    let first_month = monthly(&mut canwu, 30);
    let runtime = load_culture_runtime(&canwu, &plan)
        .expect("load culture runtime")
        .expect("culture state");
    assert_eq!(runtime.state().boundary_index(), 1);
    assert_eq!(
        runtime.target("equality").expect("equality").last_work_at,
        Some(day(30))
    );
    assert_eq!(
        canwu
            .events()
            .iter()
            .filter(|event| {
                event.kind == EventKind::plugin("canwu-culture", CULTURE_EXPOSURE_REJECTED_EVENT)
            })
            .count(),
        1
    );
    let generated = canwu
        .ingress_log()
        .iter()
        .filter_map(|record| match &record.payload {
            IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } if plugin == "canwu-culture" && packet_type == CULTURAL_SIGNAL_INGRESS => {
                Some((record.id, payload.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 1);
    let signal_ingress = generated[0].0;
    let batch: CulturalSignalBatch =
        serde_json::from_value(generated[0].1.clone()).expect("signal batch payload");
    assert_eq!(batch.plan_hash, plan.content_hash());
    assert_eq!(batch.signals[0].effect_id, "equality-pressure");
    assert_eq!(batch.signals[0].persistence, EffectPersistence::Level);
    assert!(batch.signals[0].strength_per_mille > 0);
    assert!(
        canwu
            .boundaries()
            .iter()
            .find(|boundary| boundary.id == first_month)
            .expect("first monthly boundary")
            .generated_ingress
            .iter()
            .any(|generation| {
                generation.ingress == signal_ingress
                    && generation.plugin == "canwu-culture"
                    && generation.system == CULTURE_LIFECYCLE_SYSTEM
            })
    );
    assert!(
        canwu
            .boundaries()
            .iter()
            .all(|boundary| !boundary.admitted_ingress.contains(&signal_ingress)),
        "a signal batch is not visible in the boundary that emits it"
    );
    // Generated ingress is due at the emitting instant, so the next
    // canonical boundary (at the same time) admits it.
    canwu
        .step_canonical()
        .expect("admit generated ingress")
        .expect("a canonical boundary is due");
    assert!(
        canwu
            .boundaries()
            .last()
            .expect("next boundary")
            .admitted_ingress
            .contains(&signal_ingress),
        "the signal batch is admitted at the next boundary"
    );
    daily(&mut canwu, 31);

    // Second Monthly boundary: the quiet target becomes dormant. Culture
    // persists its own state but only hands society a delta packet.
    monthly(&mut canwu, 60);
    let runtime = load_culture_runtime(&canwu, &plan)
        .expect("load culture runtime")
        .expect("culture state");
    assert_eq!(
        runtime.target("liberty").expect("liberty").state,
        CultureLifecycle::Dormant
    );
    assert_eq!(
        runtime.target("equality").expect("equality").state,
        CultureLifecycle::Active
    );
    let liberty_rules = |state: &SocietyState| {
        state
            .transition_rules
            .values()
            .filter(|rule| rule.target_id == "liberty")
            .count()
    };
    assert_eq!(
        liberty_rules(&load_society_state(&canwu).expect("society")),
        1
    );
    canwu
        .step_canonical()
        .expect("admit the society delta")
        .expect("a canonical boundary is due");
    assert_eq!(
        liberty_rules(&load_society_state(&canwu).expect("society")),
        1,
        "the admitted delta is queued for the next Daily society settlement"
    );
    // Restore re-derives the queued delta from the journal as an
    // engine-generated packet.
    let queued = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&canwu.snapshot()).expect("encode snapshot"),
        &plugins,
    )
    .expect("restore with a queued delta");
    validate_society_runtime(&queued).expect("the queued delta matches its admitted packet");
    daily(&mut canwu, 61);
    let society = load_society_state(&canwu).expect("society after delta");
    assert_eq!(liberty_rules(&society), 0);
    assert!(
        society
            .transition_rules
            .values()
            .any(|rule| rule.target_id == "equality")
    );
    let ledger = canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("society ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    assert_eq!(ledger.lifecycle_deltas.len(), 1);
    assert!(
        society
            .distributions
            .values()
            .any(|distribution| distribution.target_id == "liberty"),
        "dormancy keeps the target's distributions"
    );
    assert!(
        ledger
            .lifecycle_deltas
            .values()
            .all(|outcome| outcome.status == SocietyIngressStatus::Applied)
    );
    // Every society record change in the run was made by the society plugin.
    for boundary in canwu.boundaries() {
        for change in &boundary.record_changes {
            if change.current.reference.kind.namespace == "canwu.society" {
                assert_eq!(change.plugin, "canwu-society");
            }
        }
    }

    // Save/load, fork, and exact replay agree, including after one more
    // Monthly boundary.
    let snapshot = canwu.snapshot();
    let mut restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        &plugins,
    )
    .expect("restore culture snapshot");
    assert_eq!(restored.snapshot(), snapshot);
    let mut replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replay culture run");
    assert_eq!(replayed.snapshot(), snapshot);
    let mut forked = canwu.fork();
    // Day 90 retires the dormant target. Day 120 is the next Monthly
    // boundary; its culture step reads a phase-7 snapshot in which society
    // has not yet released the target, so it reconciles by re-sending the
    // release. Society applies both deliveries idempotently by day 121, and
    // once it reflects the committed lifecycle, day 150 sends nothing.
    for simulation in [&mut canwu, &mut restored, &mut replayed, &mut forked] {
        monthly(simulation, 90);
        monthly(simulation, 120);
        daily(simulation, 121);
        monthly(simulation, 150);
    }
    let deltas_sent_at = |days: i64| {
        canwu
            .boundaries()
            .iter()
            .filter(|boundary| boundary.at == day(days))
            .flat_map(|boundary| &boundary.generated_ingress)
            .filter(|generation| {
                canwu.ingress_log().iter().any(|record| {
                    record.id == generation.ingress
                        && matches!(
                            &record.payload,
                            IngressPayload::Plugin { packet_type, .. }
                                if packet_type == SOCIETY_LIFECYCLE_DELTA_INGRESS
                        )
                })
            })
            .count()
    };
    assert_eq!(
        [deltas_sent_at(90), deltas_sent_at(120), deltas_sent_at(150)],
        [1, 1, 0]
    );
    let ledger = canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("society ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    assert_eq!(ledger.lifecycle_deltas.len(), 3);
    assert!(
        ledger
            .lifecycle_deltas
            .values()
            .all(|outcome| outcome.status == SocietyIngressStatus::Applied
                && outcome.blocked_releases.is_empty())
    );
    // The retired target's release has been applied.
    let runtime = load_culture_runtime(&canwu, &plan)
        .expect("load culture runtime")
        .expect("culture state");
    assert_eq!(
        runtime.target("liberty").expect("liberty").state,
        CultureLifecycle::Retired
    );
    assert_eq!(runtime.state().tombstones().len(), 1);
    let society = load_society_state(&canwu).expect("society after release");
    assert!(
        society
            .distributions
            .values()
            .all(|distribution| distribution.target_id != "liberty")
    );
    assert!(
        society
            .distributions
            .values()
            .any(|distribution| distribution.target_id == "equality")
    );
    let final_snapshot = canwu.snapshot();

    // A retired culture root (as owner-authorized maintenance leaves it) is
    // inert: Monthly boundaries keep committing and do not touch it.
    let mut retired = scenario(&definition);
    for record in &mut retired.domain_records {
        if record.reference == canwu_culture::culture_state_reference().into_untyped() {
            record.lifecycle = DomainRecordLifecycle::Retired {
                at: SimTime::EPOCH,
                successor: None,
            };
        }
    }
    let mut inert = Canwu::new_with_plugins(7, retired, &plugins).expect("retired culture state");
    monthly(&mut inert, 30);
    monthly(&mut inert, 60);
    let state = inert
        .typed_domain_record(&canwu_culture::culture_state_reference())
        .expect("retired culture state record");
    assert_eq!(state.version, 1);
    assert!(!state.is_active());
    assert_eq!(restored.snapshot(), final_snapshot);
    assert_eq!(replayed.snapshot(), final_snapshot);
    assert_eq!(forked.snapshot(), final_snapshot);
}

fn definition() -> CultureDefinition {
    let ids = Canwu::demo_ids();
    CultureDefinition::builder("rights")
        .target("equality")
        .target("liberty")
        .cohort(CultureCohortDefinition::new(
            "town",
            ids.western_territory,
            100,
        ))
        .transition(TransitionSpec::awareness_from_influence(
            "equality-awareness",
            "equality",
            100_000,
        ))
        .transition(TransitionSpec::awareness_from_influence(
            "liberty-awareness",
            "liberty",
            0,
        ))
        .effect(CulturalEffectBinding::new(
            "equality-pressure",
            "equality",
            "legitimacy_pressure",
            EffectPersistence::Level,
        ))
        .retirement(RetirementPolicy {
            dormant_after_boundaries: 2,
            retired_after_boundaries: 3,
        })
        .build()
        .expect("valid culture definition")
}

fn scenario(definition: &CultureDefinition) -> Scenario {
    let plan = compile_culture(definition).expect("compile culture plan");
    let mut society = SocietyState::default();
    install_into_society(&plan, &mut society).expect("install culture into society");
    let snapshot = Canwu::demo(7).expect("demo scenario").snapshot();
    Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![
            society.into_record().expect("society record"),
            culture_definition_record(definition).expect("definition record"),
            CultureRuntime::new_at(&plan, snapshot.initial_time)
                .into_record(&plan)
                .expect("culture state record"),
        ],
    }
}

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

/// Settles any canonical ingress due before `days`, then a Daily boundary.
fn daily(canwu: &mut Canwu, days: i64) -> BoundaryId {
    settle(
        canwu,
        BoundaryRequest::at(day(days)).with_cadence(SystemCadence::Daily),
    )
}

/// Settles any canonical ingress due before `days`, then a Daily and
/// Monthly boundary.
fn monthly(canwu: &mut Canwu, days: i64) -> BoundaryId {
    settle(
        canwu,
        BoundaryRequest::at(day(days))
            .with_cadence(SystemCadence::Daily)
            .with_cadence(SystemCadence::Monthly),
    )
}

fn settle(canwu: &mut Canwu, request: BoundaryRequest) -> BoundaryId {
    canwu
        .advance_canonical(request.at - canwu.time())
        .expect("settle due canonical ingress");
    canwu
        .settle_boundary(request)
        .expect("calendar boundary")
        .boundary_id
}

fn society_state_key() -> StateKey {
    StateKey::new("canwu.society", "state")
}

/// A bridge that tries to write society state beside the society plugin.
struct RogueSocietyWriter;

impl SimulationPlugin for RogueSocietyWriter {
    fn name(&self) -> &'static str {
        "test-rogue-society-writer"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "7e1d2c3b4a5968778695a4b3c2d1e0f0e1d2c3b4a5968778695a4b3c2d1e0f01"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut system = BoundarySystemContract::new(
            "settle-society-dispositions",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        system.reads = vec![society_state_key()];
        system.writes = vec![society_state_key()];
        system.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(system, |_, _| Ok(BoundaryProposal::default()))
    }
}
