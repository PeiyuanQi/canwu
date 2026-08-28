use canwu_api::{Canwu, CauseRef, Scenario, SimTime, TerritoryId};
use canwu_culture::{
    CulturalEffectBinding, CultureCohortDefinition, CultureDefinition, CultureLifecycle,
    CulturePlugin, CultureRuntime, EffectPersistence, LifecycleObservation, RetirementPolicy,
    TransitionSpec, compile_culture, culture_state_reference, install_into_society,
    load_culture_runtime, load_culture_state_for_plan, settle_culture_society_boundary,
    synchronize_society_lifecycle,
};
use canwu_society::SocietyState;

fn definition() -> CultureDefinition {
    CultureDefinition::builder("rights")
        .target("equality")
        .cohort(CultureCohortDefinition::new(
            "town",
            TerritoryId::new(1),
            100,
        ))
        .transition(TransitionSpec::awareness_from_influence(
            "awareness",
            "equality",
            100_000,
        ))
        .effect(CulturalEffectBinding::new(
            "legal-eligibility",
            "equality",
            "legitimacy_pressure",
            EffectPersistence::Commitment,
        ))
        .retirement(RetirementPolicy {
            dormant_after_boundaries: 2,
            retired_after_boundaries: 4,
        })
        .build()
        .expect("definition is valid")
}

#[test]
fn compiled_plan_installs_sparse_society_state() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut society = SocietyState::default();
    install_into_society(&plan, &mut society).expect("install");
    assert_eq!(society.targets.len(), 1);
    assert_eq!(society.cohorts.len(), 1);
    assert_eq!(society.distributions.len(), 0);
    assert_eq!(society.transition_rules.len(), 1);
}

#[test]
fn retired_generation_cannot_emit_and_reactivation_is_explicit() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let observations =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    for minute in 1..=4 {
        runtime
            .settle_boundary(SimTime::from_minutes(minute), &observations)
            .expect("settle lifecycle");
    }
    assert_eq!(
        runtime.target("equality").expect("target").state,
        CultureLifecycle::Retired
    );
    assert!(
        runtime
            .emit_effect(
                &plan,
                "old-generation",
                plan.effect_key("legal-eligibility").expect("effect key"),
                800,
                SimTime::from_minutes(5),
                SimTime::from_minutes(5),
                vec![CauseRef::System("test:retired".to_owned())],
            )
            .is_err()
    );
    runtime
        .reactivate("equality", SimTime::from_minutes(5))
        .expect("explicit reactivation");
    assert_eq!(runtime.target("equality").expect("target").generation, 2);
}

#[test]
fn dirty_set_and_commitment_effect_follow_compiled_bindings() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    assert_eq!(
        runtime.mark_target_dirty(&plan, "equality").expect("mark"),
        1
    );
    assert_eq!(runtime.dirty_pair_count(), 1);
    let dirty = runtime.drain_dirty_pairs();
    assert_eq!(dirty.len(), 1);
    assert_eq!(runtime.dirty_pair_count(), 0);

    let effect = runtime
        .emit_effect(
            &plan,
            "legal-pressure",
            plan.effect_key("legal-eligibility").expect("effect key"),
            700,
            SimTime::from_minutes(1),
            SimTime::from_minutes(1),
            vec![CauseRef::System("test:reform-debate".to_owned())],
        )
        .expect("commitment effect");
    assert_eq!(effect.signals[0].persistence, EffectPersistence::Commitment);
}

#[test]
fn lifecycle_state_uses_the_engine_domain_record_boundary() {
    let plan = compile_culture(&definition()).expect("compile");
    let record = CultureRuntime::new(&plan)
        .into_record(&plan)
        .expect("encode lifecycle state");
    let demo = Canwu::demo(77).expect("demo");
    let snapshot = demo.snapshot();
    let scenario = Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![record],
    };
    let plugin = CulturePlugin;
    let canwu = Canwu::new_with_plugins(77, scenario, &[&plugin]).expect("load culture plugin");
    let loaded = load_culture_state_for_plan(&canwu, &plan)
        .expect("load state")
        .expect("state record");
    assert_eq!(loaded.plan_hash(), plan.content_hash());
    let restored = load_culture_runtime(&canwu, &plan)
        .expect("hydrate runtime")
        .expect("runtime record");
    assert_eq!(restored.plan_hash(), plan.content_hash());
    assert!(
        canwu
            .typed_domain_record(&culture_state_reference())
            .is_some()
    );
}

#[test]
fn signal_batches_and_state_catalog_are_bound_and_bounded() {
    let mut definition = definition();
    definition.budgets.max_signals_per_batch = 1;
    definition.budgets.max_evidence_per_signal = 1;
    let plan = compile_culture(&definition).expect("compile");
    let state = CultureRuntime::new(&plan).snapshot_state();
    let mut forged = serde_json::to_value(state).expect("serialize state");
    forged["targets"]["forged"] = serde_json::json!({
        "target_id": "forged",
        "generation": 1,
        "state": "active",
        "engaged_headcount": 0,
        "quiet_boundaries": 0,
        "dormant_since_boundary": null,
        "last_active_at": 0,
        "last_work_at": null
    });
    forged["hot_targets"] = serde_json::json!(["equality", "forged"]);
    let forged = serde_json::from_value(forged).expect("decode forged state");
    assert!(canwu_culture::CultureRuntime::from_state(&plan, forged).is_err());
}

#[test]
fn persisted_runtime_restores_indexes_and_retirement_releases_society_work() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    runtime
        .mark_target_dirty(&plan, "equality")
        .expect("mark dirty");
    let state = runtime.snapshot_state();
    let restored = CultureRuntime::from_state(&plan, state).expect("restore runtime");
    assert_eq!(restored.dirty_pair_count(), 1);

    let mut runtime = restored;
    let observations =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    let mut society = SocietyState::default();
    install_into_society(&plan, &mut society).expect("install");
    canwu_society::settle_transitions(&mut society, SimTime::from_minutes(1))
        .expect("materialize through settlement");
    for minute in 1..=4 {
        settle_culture_society_boundary(
            &plan,
            &mut runtime,
            &mut society,
            SimTime::from_minutes(minute),
            &observations,
        )
        .expect("settle and synchronize lifecycle");
    }
    assert!(society.transition_rules.is_empty());
    assert!(society.distributions.is_empty());
}

#[test]
fn wildcard_transition_is_charged_for_every_cohort() {
    let mut builder = CultureDefinition::builder("bounded-rights").target("equality");
    for index in 0..129 {
        builder = builder.cohort(CultureCohortDefinition::new(
            format!("cohort-{index}"),
            TerritoryId::new(1),
            1,
        ));
    }
    let result = builder
        .transition(TransitionSpec::awareness_from_influence(
            "global-awareness",
            "equality",
            100_000,
        ))
        .build();
    assert!(result.is_err());
}

#[test]
fn disjoint_transition_union_is_charged_per_target() {
    let mut builder = CultureDefinition::builder("bounded-rights").target("equality");
    let mut first = TransitionSpec::awareness_from_influence("first", "equality", 100_000);
    let mut second = TransitionSpec::awareness_from_influence("second", "equality", 100_000);
    second.to.assent = canwu_society::AssentBand::Sympathetic;
    for index in 0..129 {
        let cohort_id = format!("cohort-{index}");
        builder = builder.cohort(CultureCohortDefinition::new(
            cohort_id.clone(),
            TerritoryId::new(1),
            1,
        ));
        if index < 65 {
            first.affected_cohorts.insert(cohort_id);
        } else {
            second.affected_cohorts.insert(cohort_id);
        }
    }
    assert!(
        builder
            .transition(first)
            .transition(second)
            .build()
            .is_err()
    );
}

#[test]
fn society_install_is_atomic_on_conflict() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut society = SocietyState::default();
    society.cohorts.insert(
        "town".to_owned(),
        canwu_society::SocietyCohort {
            id: "town".to_owned(),
            territory: TerritoryId::new(9),
            headcount: 999,
            classification: std::collections::BTreeMap::new(),
        },
    );
    let before = society.clone();
    assert!(install_into_society(&plan, &mut society).is_err());
    assert_eq!(society, before);
}

#[test]
fn live_external_society_dependency_blocks_retirement_without_data_loss() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let observations =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    let mut society = SocietyState::default();
    install_into_society(&plan, &mut society).expect("install");
    society.influence_edges.insert(
        "external-news".to_owned(),
        canwu_society::SocialInfluenceEdge {
            id: "external-news".to_owned(),
            source: canwu_society::InfluenceSource::Cohort("town".to_owned()),
            target_cohort_id: "town".to_owned(),
            target_id: "equality".to_owned(),
            channel: "news".to_owned(),
            reach_per_mille: 100,
            trust_per_mille: 100,
            active: true,
        },
    );
    for minute in 1..=3 {
        settle_culture_society_boundary(
            &plan,
            &mut runtime,
            &mut society,
            SimTime::from_minutes(minute),
            &observations,
        )
        .expect("pre-retirement boundary");
    }
    let runtime_before = runtime.clone();
    let society_before = society.clone();
    assert!(
        settle_culture_society_boundary(
            &plan,
            &mut runtime,
            &mut society,
            SimTime::from_minutes(4),
            &observations,
        )
        .is_err()
    );
    assert_eq!(runtime, runtime_before);
    assert_eq!(society, society_before);
}

#[test]
fn compiled_effect_cadence_survives_state_restore() {
    let mut definition = definition();
    definition.effects[0].cadence_boundaries = 2;
    let plan = compile_culture(&definition).expect("compile");
    let effect = plan.effect_key("legal-eligibility").expect("effect key");
    let evidence = || vec![CauseRef::System("test:cadence".to_owned())];
    let mut runtime = CultureRuntime::new(&plan);
    runtime
        .emit_effect(
            &plan,
            "first",
            effect,
            500,
            SimTime::EPOCH,
            SimTime::EPOCH,
            evidence(),
        )
        .expect("first emission");
    let mut runtime =
        CultureRuntime::from_state(&plan, runtime.snapshot_state()).expect("restore runtime");
    assert!(
        runtime
            .emit_effect(
                &plan,
                "too-soon",
                effect,
                500,
                SimTime::EPOCH,
                SimTime::EPOCH,
                evidence(),
            )
            .is_err()
    );
    let active = std::collections::BTreeMap::from([(
        "equality".to_owned(),
        LifecycleObservation {
            engaged_headcount: 1,
            admitted_work: false,
            live_dependency: false,
        },
    )]);
    runtime
        .settle_boundary(SimTime::from_minutes(1), &active)
        .expect("boundary one");
    assert!(
        runtime
            .emit_effect(
                &plan,
                "still-too-soon",
                effect,
                500,
                SimTime::from_minutes(1),
                SimTime::from_minutes(1),
                evidence(),
            )
            .is_err()
    );
    runtime
        .settle_boundary(SimTime::from_minutes(2), &active)
        .expect("boundary two");
    runtime
        .emit_effect(
            &plan,
            "eligible",
            effect,
            500,
            SimTime::from_minutes(2),
            SimTime::from_minutes(2),
            evidence(),
        )
        .expect("eligible emission");
}

#[test]
fn persisted_dormant_schedule_must_match_policy_origin() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let quiet =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    runtime
        .settle_boundary(SimTime::from_minutes(1), &quiet)
        .expect("boundary one");
    runtime
        .settle_boundary(SimTime::from_minutes(2), &quiet)
        .expect("boundary two");
    let mut forged = serde_json::to_value(runtime.snapshot_state()).expect("serialize state");
    forged["dormant_due"] = serde_json::json!({"5": ["equality"]});
    let forged = serde_json::from_value(forged).expect("decode forged schedule");
    assert!(CultureRuntime::from_state(&plan, forged).is_err());
}

#[test]
fn zero_engagement_update_preserves_quiet_progress() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let quiet =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    runtime
        .settle_boundary(SimTime::from_minutes(1), &quiet)
        .expect("boundary one");
    runtime
        .set_engaged_headcount("equality", 0, SimTime::from_minutes(1))
        .expect("zero engagement update");
    runtime
        .settle_boundary(SimTime::from_minutes(2), &quiet)
        .expect("boundary two");
    assert_eq!(
        runtime.target("equality").expect("target").state,
        CultureLifecycle::Dormant
    );
}

#[test]
fn unknown_lifecycle_observation_is_rejected_atomically() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let before = runtime.clone();
    let observations =
        std::collections::BTreeMap::from([("forged".to_owned(), LifecycleObservation::quiet())]);
    assert!(
        runtime
            .settle_boundary(SimTime::from_minutes(1), &observations)
            .is_err()
    );
    assert_eq!(runtime, before);
}

#[test]
fn initial_runtime_must_fit_the_state_byte_budget() {
    let mut definition = definition();
    definition.budgets.max_state_bytes = 1;
    assert!(compile_culture(&definition).is_err());
}

#[test]
fn forged_generation_validation_is_bounded_by_actual_tombstones() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut forged =
        serde_json::to_value(CultureRuntime::new(&plan).snapshot_state()).expect("serialize state");
    forged["targets"]["equality"]["generation"] = serde_json::json!(u64::MAX);
    let forged = serde_json::from_value(forged).expect("decode forged state");
    assert!(CultureRuntime::from_state(&plan, forged).is_err());
}

#[test]
fn restore_checks_state_bytes_before_index_semantics() {
    let mut definition = definition();
    definition.budgets.max_state_bytes = 1_000;
    let plan = compile_culture(&definition).expect("initial state fits");
    let mut forged =
        serde_json::to_value(CultureRuntime::new(&plan).snapshot_state()).expect("serialize state");
    forged["hot_targets"] = serde_json::json!(["equality", "x".repeat(500)]);
    let forged = serde_json::from_value(forged).expect("decode forged state");
    let error = CultureRuntime::from_state(&plan, forged).expect_err("state must be rejected");
    assert!(error.to_string().contains("byte budget"));
}

#[test]
fn lifecycle_reclamation_uses_net_state_bytes_near_the_limit() {
    let mut definition = definition();
    definition.cohorts.push(CultureCohortDefinition::new(
        "village",
        TerritoryId::new(2),
        100,
    ));
    definition.cohorts.push(CultureCohortDefinition::new(
        "port",
        TerritoryId::new(3),
        100,
    ));
    definition.budgets.max_state_bytes = 1_000;
    let plan = compile_culture(&definition).expect("initial state fits");
    let mut runtime = CultureRuntime::new(&plan);
    assert_eq!(
        runtime.mark_target_dirty(&plan, "equality").expect("mark"),
        3
    );
    let quiet =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    runtime
        .settle_boundary(SimTime::from_minutes(1), &quiet)
        .expect("first quiet boundary");
    runtime
        .settle_boundary(SimTime::from_minutes(2), &quiet)
        .expect("net-shrinking dormancy transition");
    assert_eq!(
        runtime.target("equality").expect("target").state,
        CultureLifecycle::Dormant
    );
    let restored =
        CultureRuntime::from_state(&plan, runtime.snapshot_state()).expect("restore exact bytes");
    assert_eq!(runtime, restored);
}

#[test]
fn forged_latest_activity_cursor_is_rejected() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut forged =
        serde_json::to_value(CultureRuntime::new(&plan).snapshot_state()).expect("serialize state");
    forged["latest_activity_at"] = serde_json::json!(999_999);
    let forged = serde_json::from_value(forged).expect("decode forged state");
    assert!(CultureRuntime::from_state(&plan, forged).is_err());
}

#[test]
fn tombstone_evidence_input_stops_at_the_count_budget() {
    let mut definition = definition();
    definition.budgets.max_tombstone_evidence = 2;
    let plan = compile_culture(&definition).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let quiet =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    for minute in 1..=4 {
        runtime
            .settle_boundary(SimTime::from_minutes(minute), &quiet)
            .expect("retirement boundary");
    }

    let calls = std::cell::Cell::new(0_usize);
    let unbounded = std::iter::from_fn(|| {
        calls.set(calls.get().saturating_add(1));
        Some(CauseRef::System("test:duplicate".to_owned()))
    });
    assert!(
        runtime
            .attach_tombstone_evidence("equality", 1, unbounded)
            .is_err()
    );
    assert_eq!(calls.get(), 3);
    assert!(runtime.state().tombstones()[0].evidence.is_empty());
}

#[test]
fn dormant_reactivation_restores_only_its_society_bindings() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let mut society = SocietyState::default();
    install_into_society(&plan, &mut society).expect("install");
    let quiet =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    for minute in 1..=2 {
        settle_culture_society_boundary(
            &plan,
            &mut runtime,
            &mut society,
            SimTime::from_minutes(minute),
            &quiet,
        )
        .expect("dormancy boundary");
    }
    assert!(society.transition_rules.is_empty());

    let engaged = std::collections::BTreeMap::from([(
        "equality".to_owned(),
        LifecycleObservation {
            engaged_headcount: 1,
            admitted_work: false,
            live_dependency: false,
        },
    )]);
    let transitions = settle_culture_society_boundary(
        &plan,
        &mut runtime,
        &mut society,
        SimTime::from_minutes(3),
        &engaged,
    )
    .expect("reactivation boundary");
    assert_eq!(transitions.len(), 1);
    assert_eq!(society.transition_rules.len(), 1);
}

#[test]
fn prefix_like_external_rule_survives_dormancy_and_blocks_retirement() {
    let plan = compile_culture(&definition()).expect("compile");
    let mut runtime = CultureRuntime::new(&plan);
    let mut society = SocietyState::default();
    install_into_society(&plan, &mut society).expect("install");
    let mut external = society
        .transition_rules
        .values()
        .next()
        .expect("compiled transition")
        .clone();
    external.id = "culture:rights:external".to_owned();
    society
        .transition_rules
        .insert(external.id.clone(), external);
    let quiet =
        std::collections::BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
    for minute in 1..=2 {
        settle_culture_society_boundary(
            &plan,
            &mut runtime,
            &mut society,
            SimTime::from_minutes(minute),
            &quiet,
        )
        .expect("pre-retirement boundary");
    }
    synchronize_society_lifecycle(&plan, &runtime, &mut society)
        .expect("maintenance reconciliation");
    settle_culture_society_boundary(
        &plan,
        &mut runtime,
        &mut society,
        SimTime::from_minutes(3),
        &quiet,
    )
    .expect("final pre-retirement boundary");
    assert!(
        society
            .transition_rules
            .contains_key("culture:rights:external")
    );
    let runtime_before = runtime.clone();
    let society_before = society.clone();
    assert!(
        settle_culture_society_boundary(
            &plan,
            &mut runtime,
            &mut society,
            SimTime::from_minutes(4),
            &quiet,
        )
        .is_err()
    );
    assert_eq!(runtime, runtime_before);
    assert_eq!(society, society_before);
}

#[test]
fn length_prefixed_binding_ids_separate_ambiguous_definition_names() {
    let make_definition = |definition_id: &str, transition_id: &str| {
        CultureDefinition::builder(definition_id)
            .target("equality")
            .cohort(CultureCohortDefinition::new(
                "town",
                TerritoryId::new(1),
                100,
            ))
            .transition(TransitionSpec::awareness_from_influence(
                transition_id,
                "equality",
                100_000,
            ))
            .build()
            .expect("definition")
    };
    let first = compile_culture(&make_definition("a", "b:c")).expect("first plan");
    let second = compile_culture(&make_definition("a:b", "c")).expect("second plan");
    let mut society = SocietyState::default();
    install_into_society(&first, &mut society).expect("first install");
    install_into_society(&second, &mut society).expect("second install");
    assert_eq!(society.transition_rules.len(), 2);
}
