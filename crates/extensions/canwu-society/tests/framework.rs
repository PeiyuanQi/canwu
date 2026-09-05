use canwu_api::{
    Canwu, Command, CommandAuthority, CommandEnvelope, CommandRequest, CommandRequestId,
    DecisionAuthority, DecisionControllerBinding, DecisionEvaluation, DecisionIngressRequest,
    DecisionMutation, DecisionOrigin, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRequestId, DecisionTicketId, EntityRef, ErrorCode, Issuer, KnowledgeSnapshot, Scenario,
    SimDuration, SimTime, UtilityProfile, WeightedUtilityPolicy,
};
use canwu_society::{
    AffiliationTarget, AssentBand, AwarenessBand, CohortTransferIntent, DispositionBucket,
    DispositionDistribution, DispositionProfile, InfluenceSource, InstitutionalAlignment,
    MobilizationBand, ObserverProfile, OrganizationNode, OrganizationRelation,
    OrganizationalTieBand, PolicyChoice, PolicyDecision, PolicyPressure, PracticeBand,
    PublicAlignmentBand, SocialInfluenceEdge, SocietyCohort, SocietyCohortExchangeLedgerRecord,
    SocietyPlugin, SocietyState, TransitionRule, TransitionWeights, VisibilityBand,
    distribution_id, from_society_snapshot_json, institutional_policy_ticket, load_society_state,
    projection_for_viewer, settle_transitions, society_cohort_exchange_ledger_reference,
};
use std::collections::{BTreeMap, BTreeSet};

#[test]
#[ignore = "cohort-transfer canonical scheduling needs a dedicated daily-boundary harness"]
fn owner_side_cohort_transfer_is_conservative_idempotent_stale_checked_and_replayable() {
    let (mut canwu, _) = tutorial_simulation();
    let ids = Canwu::demo_ids();
    let intent = CohortTransferIntent {
        operation_id: "enlistment-1".to_owned(),
        authority_alignment_id: "council-alignment".to_owned(),
        source_cohort_id: "market".to_owned(),
        destination_cohort_id: "village".to_owned(),
        quantity: 101,
        expected_source_version: 1,
        due_time: SimTime::EPOCH,
    };
    let envelope = || {
        CommandEnvelope::new(
            Issuer::Actor(ids.commander),
            Command::Plugin {
                plugin: "canwu-society".to_owned(),
                command: "transfer_cohort_population".to_owned(),
                payload: serde_json::to_value(&intent).expect("transfer intent"),
            },
        )
        .with_authority(CommandAuthority {
            decision_origin: DecisionOrigin::Actor {
                actor: ids.commander,
            },
            seat_id: None,
            permission_profile_id: None,
            command_subject: Some(EntityRef::Government(ids.government)),
        })
    };
    canwu
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(CommandRequestId::new(1), canwu.revision(), envelope()),
        )
        .expect("first transfer intent");
    canwu
        .enqueue_command(
            SimTime::EPOCH,
            1,
            CommandRequest::new(CommandRequestId::new(2), canwu.revision(), envelope()),
        )
        .expect("duplicate transfer intent");
    canwu
        .advance_canonical(SimDuration::days(2))
        .expect("settle admitted transfer");
    let state = load_society_state(&canwu).expect("transferred society state");
    assert_eq!(state.cohorts["market"].headcount, 899);
    assert_eq!(state.cohorts["village"].headcount, 2_101);
    assert_distribution_conservation(&state);
    let ledger = canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("ledger exists")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    assert_eq!(ledger.outcomes["enlistment-1"].quantity, 101);
    assert_eq!(ledger.outcomes["enlistment-1"].source_record_version, 1);
    let stale_intent = CohortTransferIntent {
        operation_id: "stale-1".to_owned(),
        expected_source_version: 1,
        ..intent
    };
    let stale_envelope = CommandEnvelope::new(
        Issuer::Actor(ids.commander),
        Command::Plugin {
            plugin: "canwu-society".to_owned(),
            command: "transfer_cohort_population".to_owned(),
            payload: serde_json::to_value(stale_intent).expect("stale intent"),
        },
    )
    .with_authority(CommandAuthority {
        decision_origin: DecisionOrigin::Actor {
            actor: ids.commander,
        },
        seat_id: None,
        permission_profile_id: None,
        command_subject: Some(EntityRef::Government(ids.government)),
    });
    canwu
        .enqueue_command(
            canwu.time(),
            2,
            CommandRequest::new(CommandRequestId::new(3), canwu.revision(), stale_envelope),
        )
        .expect("queue stale transfer");
    let stale_error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("stale transfer must reject");
    assert!(matches!(
        stale_error.code,
        ErrorCode::DomainRecordVersionConflict | ErrorCode::InvalidDecision
    ));
    let snapshot = canwu.snapshot();
    let restored = from_society_snapshot_json(&serde_json::to_string(&snapshot).expect("snapshot"))
        .expect("restore transfer snapshot");
    let replayed = Canwu::replay_from_journal(&[&SocietyPlugin], &canwu.replay_journal())
        .expect("replay transfer history");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(replayed.snapshot(), snapshot);
}

#[test]
#[allow(clippy::too_many_lines)]
fn aggregate_diffusion_decision_projection_and_replay_share_one_contract() {
    let (mut canwu, _initial_scenario) = tutorial_simulation();
    let plugin = SocietyPlugin;
    let ids = Canwu::demo_ids();

    canwu
        .settle_boundary(
            canwu_api::BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                .with_cadence(canwu_api::SystemCadence::Daily),
        )
        .expect("first daily social boundary");
    let before_policy = load_society_state(&canwu).expect("society state after first day");
    assert_distribution_conservation(&before_policy);

    register_policy_decision(&mut canwu);
    let policy = WeightedUtilityPolicy::new(
        "institutional-policy",
        "1",
        UtilityProfile {
            weights: BTreeMap::from([("control".to_owned(), 1)]),
        },
    );
    let evaluation = canwu
        .drive_decision(
            canwu.time(),
            0,
            DecisionRequestId::new(3),
            Some(CommandRequestId::new(1)),
            DecisionTicketId::new(1),
            &policy,
        )
        .expect("prepare institutional policy resolution");
    assert!(matches!(evaluation, DecisionEvaluation::Prepared(_)));
    canwu
        .step_canonical()
        .expect("resolve institutional policy")
        .expect("decision resolution boundary");
    let immediately_after_decision =
        load_society_state(&canwu).expect("society state after policy command");
    assert_eq!(
        immediately_after_decision.distributions, before_policy.distributions,
        "an institutional decision must not instantly rewrite population dispositions"
    );
    assert_eq!(
        immediately_after_decision.institutional_alignments["council-alignment"]
            .last_decision_version,
        0,
        "policy components apply only through the next social boundary"
    );

    canwu
        .settle_boundary(
            canwu_api::BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(2))
                .with_cadence(canwu_api::SystemCadence::Daily),
        )
        .expect("second daily social boundary");
    let after_policy = load_society_state(&canwu).expect("society state after policy");
    assert_distribution_conservation(&after_policy);
    assert!(
        after_policy
            .remainders
            .values()
            .any(|remainder| remainder.remainder > 0),
        "fractional expected transfers must persist as authoritative state"
    );

    let alignment = &after_policy.institutional_alignments["council-alignment"];
    assert_eq!(alignment.last_decision_version, 1);
    assert_eq!(alignment.enforcement_per_mille, 800);

    let village = &after_policy.aggregates[&distribution_id("village", "new-idea")];
    assert!(village.assenting > 0);
    assert!(village.publicly_aligned > 0);
    assert!(village.publicly_aligned < village.headcount);

    let viewer = canwu
        .viewer_context(ids.observer)
        .expect("observer context is authorized");
    let projection = projection_for_viewer(&canwu, &viewer).expect("observer projection");
    let estimate = &projection.entries[&distribution_id("village", "new-idea")];
    assert_ne!(
        estimate.estimated_publicly_aligned,
        village.publicly_aligned
    );
    let commander_view = canwu
        .viewer_context(ids.commander)
        .expect("commander context exists");
    assert!(projection_for_viewer(&canwu, &commander_view).is_err());

    let day_two_snapshot = canwu.snapshot();
    let mut restored = from_society_snapshot_json(
        &serde_json::to_string(&day_two_snapshot).expect("encode snapshot"),
    )
    .expect("restore society snapshot");
    assert_eq!(restored.snapshot(), day_two_snapshot);

    let mut forked = canwu.fork();
    assert_eq!(forked.snapshot(), day_two_snapshot);
    for simulation in [&mut canwu, &mut restored, &mut forked] {
        simulation
            .settle_boundary(
                canwu_api::BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(3))
                    .with_cadence(canwu_api::SystemCadence::Daily),
            )
            .expect("third daily social boundary");
    }
    let final_snapshot = canwu.snapshot();
    assert_eq!(restored.snapshot(), final_snapshot);
    assert_eq!(forked.snapshot(), final_snapshot);

    let replayed = Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("exactly replay society history");
    assert_eq!(replayed.snapshot(), final_snapshot);
}

#[test]
fn inactive_target_catalog_growth_stays_sparse_and_does_not_change_existing_transition() {
    let (mut baseline, _) = tutorial_simulation();
    let plugin = SocietyPlugin;
    let initial = load_society_state(&baseline).expect("baseline society state");
    let mut extended = initial.clone();
    for index in 0..128 {
        let id = format!("unrelated-{index:03}");
        extended.targets.insert(
            id.clone(),
            AffiliationTarget {
                id,
                parent: None,
                neutral_profile: DispositionProfile::neutral(),
                metadata: BTreeMap::new(),
            },
        );
    }
    assert_eq!(extended.distributions.len(), initial.distributions.len());
    let demo = Canwu::demo(42).expect("demo");
    let snapshot = demo.snapshot();
    let scenario = Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![extended.into_record().expect("extended state record")],
    };
    let mut with_unrelated =
        Canwu::new_with_plugins(42, scenario, &[&plugin]).expect("extended simulation");

    for canwu in [&mut baseline, &mut with_unrelated] {
        canwu
            .settle_boundary(
                canwu_api::BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                    .with_cadence(canwu_api::SystemCadence::Daily),
            )
            .expect("daily boundary");
    }
    let baseline_state = load_society_state(&baseline).expect("baseline result");
    let unrelated_state = load_society_state(&with_unrelated).expect("extended result");
    assert_eq!(
        baseline_state.distributions[&distribution_id("village", "new-idea")],
        unrelated_state.distributions[&distribution_id("village", "new-idea")]
    );
    assert_eq!(baseline_state.remainders, unrelated_state.remainders);
    assert_eq!(
        unrelated_state.distributions.len(),
        baseline_state.distributions.len(),
        "inactive catalog entries must not materialize a dense cohort/target matrix"
    );
}

#[test]
fn unauthorized_actor_cannot_change_institutional_policy() {
    let (mut canwu, _) = tutorial_simulation();
    let ids = Canwu::demo_ids();
    let before = load_society_state(&canwu).expect("initial society state");
    let error = canwu
        .submit(CommandEnvelope::new(
            Issuer::Actor(ids.observer),
            Command::Plugin {
                plugin: "canwu-society".to_owned(),
                command: "set_institutional_policy".to_owned(),
                payload: serde_json::to_value(PolicyDecision {
                    alignment_id: "council-alignment".to_owned(),
                    decision_version: 1,
                    support_per_mille: 900,
                    enforcement_per_mille: 900,
                    access_grant_per_mille: 900,
                })
                .expect("policy payload"),
            },
        ))
        .expect_err("observer authority must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    let after = load_society_state(&canwu).expect("society state after rejection");
    assert_eq!(after, before);
}

#[test]
fn institutional_policy_requires_the_exact_command_subject() {
    let ids = Canwu::demo_ids();
    let authorities = [
        CommandAuthority::for_actor(ids.commander),
        CommandAuthority {
            decision_origin: DecisionOrigin::Actor {
                actor: ids.commander,
            },
            seat_id: None,
            permission_profile_id: None,
            command_subject: Some(EntityRef::Army(ids.army)),
        },
    ];
    for authority in authorities {
        let (mut canwu, _) = tutorial_simulation();
        let error = canwu
            .submit(
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "canwu-society".to_owned(),
                        command: "set_institutional_policy".to_owned(),
                        payload: serde_json::to_value(PolicyDecision {
                            alignment_id: "council-alignment".to_owned(),
                            decision_version: 1,
                            support_per_mille: 900,
                            enforcement_per_mille: 900,
                            access_grant_per_mille: 900,
                        })
                        .expect("policy payload"),
                    },
                )
                .with_authority(authority),
            )
            .expect_err("missing or mismatched command subjects must be rejected");
        assert_eq!(error.code, ErrorCode::InvalidAuthority);
    }
}

#[test]
fn forged_controller_authority_cannot_bypass_decision_ingress() {
    let (mut canwu, _) = tutorial_simulation();
    let ids = Canwu::demo_ids();
    let error = canwu
        .submit(
            CommandEnvelope::new(
                Issuer::Ai("council-controller".to_owned()),
                Command::Plugin {
                    plugin: "canwu-society".to_owned(),
                    command: "set_institutional_policy".to_owned(),
                    payload: serde_json::to_value(PolicyDecision {
                        alignment_id: "council-alignment".to_owned(),
                        decision_version: 1,
                        support_per_mille: 900,
                        enforcement_per_mille: 900,
                        access_grant_per_mille: 900,
                    })
                    .expect("policy payload"),
                },
            )
            .with_authority(CommandAuthority {
                decision_origin: DecisionOrigin::Actor {
                    actor: ids.commander,
                },
                seat_id: None,
                permission_profile_id: None,
                command_subject: Some(EntityRef::Government(ids.government)),
            }),
        )
        .expect_err("an envelope cannot manufacture DecisionTicket provenance");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
}

#[test]
fn epoch_and_negative_time_boundaries_are_valid_materialization_times() {
    let (mut epoch, _) = tutorial_simulation();
    epoch
        .settle_boundary(
            canwu_api::BoundaryRequest::at(SimTime::EPOCH)
                .with_cadence(canwu_api::SystemCadence::Daily),
        )
        .expect("epoch social boundary");
    let epoch_state = load_society_state(&epoch).expect("epoch society state");
    assert_eq!(epoch_state.last_transition_at, Some(SimTime::EPOCH));
    assert_eq!(epoch_state.last_aggregation_at, Some(SimTime::EPOCH));
    assert!(!epoch_state.aggregates.is_empty());

    let plugin = SocietyPlugin;
    let demo = Canwu::demo(42).expect("demo scenario");
    let snapshot = demo.snapshot();
    let start_time = SimTime::from_minutes(-2 * 24 * 60);
    let boundary_time = SimTime::from_minutes(-24 * 60);
    let mut negative = Canwu::new_with_plugins(
        42,
        Scenario {
            start_time,
            entities: snapshot.entities,
            world: snapshot.world,
            knowledge: KnowledgeSnapshot::default(),
            domain_records: vec![tutorial_state().into_record().expect("society record")],
        },
        &[&plugin],
    )
    .expect("negative-time society simulation");
    negative
        .settle_boundary(
            canwu_api::BoundaryRequest::at(boundary_time)
                .with_cadence(canwu_api::SystemCadence::Daily),
        )
        .expect("negative-time social boundary");
    let negative_state = load_society_state(&negative).expect("negative-time society state");
    assert_eq!(negative_state.last_transition_at, Some(boundary_time));
    assert!(negative_state.aggregates[&distribution_id("village", "new-idea")].aware > 0);
}

#[test]
fn society_payload_and_core_references_must_remain_bound() {
    let plugin = SocietyPlugin;
    let demo = Canwu::demo(42).expect("demo scenario");
    let ids = Canwu::demo_ids();
    let snapshot = demo.snapshot();
    let mut record = tutorial_state().into_record().expect("society record");
    record.payload["cohorts"]["village"]["territory"] =
        serde_json::to_value(ids.western_territory).expect("territory payload");
    let canwu = Canwu::new_with_plugins(
        42,
        Scenario {
            start_time: snapshot.initial_time,
            entities: snapshot.entities,
            world: snapshot.world,
            knowledge: snapshot.knowledge,
            domain_records: vec![record],
        },
        &[&plugin],
    )
    .expect("generic schema admits the structurally valid record");
    let error =
        load_society_state(&canwu).expect_err("society loading must reject stale core references");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
}

#[test]
fn materialized_derived_state_must_match_authoritative_state() {
    let (mut canwu, _) = tutorial_simulation();
    canwu
        .settle_boundary(
            canwu_api::BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                .with_cadence(canwu_api::SystemCadence::Daily),
        )
        .expect("daily boundary");
    let mut state = load_society_state(&canwu).expect("materialized society state");
    state
        .aggregates
        .get_mut(&distribution_id("village", "new-idea"))
        .expect("village aggregate")
        .aware += 1;
    let error = state
        .validate()
        .expect_err("tampered derived aggregates must be rejected");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
}

#[test]
fn affiliation_target_ancestry_cycles_are_rejected() {
    let mut state = tutorial_state();
    state
        .targets
        .get_mut("new-idea")
        .expect("tutorial target")
        .parent = Some("new-idea".to_owned());
    let error = state
        .validate()
        .expect_err("affiliation ancestry must remain acyclic");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
}

#[test]
fn inactive_organizations_neither_receive_nor_relay_influence() {
    let mut state = tutorial_state();
    state.influence_edges.clear();
    state.organizations.clear();
    state.organization_relations.clear();
    state.organizations.insert(
        "active-source".to_owned(),
        OrganizationNode {
            id: "active-source".to_owned(),
            target_id: "new-idea".to_owned(),
            base_reach_per_mille: 1_000,
            concealment_per_mille: 0,
            active: true,
        },
    );
    state.organizations.insert(
        "inactive-relay".to_owned(),
        OrganizationNode {
            id: "inactive-relay".to_owned(),
            target_id: "new-idea".to_owned(),
            base_reach_per_mille: 0,
            concealment_per_mille: 0,
            active: false,
        },
    );
    state.organization_relations.insert(
        "source-to-relay".to_owned(),
        OrganizationRelation {
            id: "source-to-relay".to_owned(),
            source_organization_id: "active-source".to_owned(),
            target_organization_id: "inactive-relay".to_owned(),
            relation: "relay".to_owned(),
            strength_per_mille: 1_000,
        },
    );
    state.influence_edges.insert(
        "inactive-contact".to_owned(),
        SocialInfluenceEdge {
            id: "inactive-contact".to_owned(),
            source: InfluenceSource::Organization("inactive-relay".to_owned()),
            target_cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            channel: "inactive-relay".to_owned(),
            reach_per_mille: 1_000,
            trust_per_mille: 1_000,
            active: true,
        },
    );

    settle_transitions(&mut state, SimTime::EPOCH + SimDuration::days(1))
        .expect("settle inactive relay state");
    let distribution = &state.distributions[&distribution_id("village", "new-idea")];
    assert_eq!(distribution.buckets.len(), 1);
    assert_eq!(
        distribution.buckets[0].profile,
        DispositionProfile::neutral()
    );
    assert_eq!(distribution.buckets[0].headcount, 2_000);
}

fn tutorial_simulation() -> (Canwu, Scenario) {
    let plugin = SocietyPlugin;
    let demo = Canwu::demo(42).expect("demo scenario");
    let ids = Canwu::demo_ids();
    let snapshot = demo.snapshot();
    let scenario = Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![
            tutorial_state()
                .into_record()
                .expect("society state record"),
        ],
    };
    let replay_scenario = scenario.clone();
    let canwu = Canwu::new_with_plugins(42, scenario, &[&plugin]).expect("society simulation");
    assert_eq!(
        canwu.world().person(ids.observer).map(|person| person.id),
        Some(ids.observer)
    );
    (canwu, replay_scenario)
}

#[allow(clippy::too_many_lines)]
fn tutorial_state() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let source = DispositionProfile {
        awareness: AwarenessBand::Aware,
        assent: AssentBand::Sympathetic,
        practice: PracticeBand::Occasional,
        public_alignment: PublicAlignmentBand::Advocating,
        organizational_tie: OrganizationalTieBand::Member,
        mobilization: MobilizationBand::Latent,
        visibility: VisibilityBand::Public,
    };
    let aware = DispositionProfile {
        awareness: AwarenessBand::Aware,
        ..neutral
    };
    let sympathetic = DispositionProfile {
        awareness: AwarenessBand::Aware,
        assent: AssentBand::Sympathetic,
        ..neutral
    };
    let conforming = DispositionProfile {
        public_alignment: PublicAlignmentBand::Conforming,
        visibility: VisibilityBand::Public,
        ..sympathetic
    };

    let mut state = SocietyState::default();
    state.cohorts.insert(
        "market".to_owned(),
        SocietyCohort {
            id: "market".to_owned(),
            territory: ids.western_territory,
            headcount: 1_000,
            classification: BTreeMap::from([("setting".to_owned(), "market".to_owned())]),
        },
    );
    state.cohorts.insert(
        "village".to_owned(),
        SocietyCohort {
            id: "village".to_owned(),
            territory: ids.central_territory,
            headcount: 2_000,
            classification: BTreeMap::from([("setting".to_owned(), "village".to_owned())]),
        },
    );
    state.targets.insert(
        "new-idea".to_owned(),
        AffiliationTarget {
            id: "new-idea".to_owned(),
            parent: None,
            neutral_profile: neutral,
            metadata: BTreeMap::new(),
        },
    );
    state.distributions.insert(
        distribution_id("market", "new-idea"),
        DispositionDistribution {
            id: distribution_id("market", "new-idea"),
            cohort_id: "market".to_owned(),
            target_id: "new-idea".to_owned(),
            buckets: vec![
                DispositionBucket {
                    profile: neutral,
                    headcount: 800,
                },
                DispositionBucket {
                    profile: source,
                    headcount: 200,
                },
            ],
        },
    );
    state.distributions.insert(
        distribution_id("village", "new-idea"),
        DispositionDistribution {
            id: distribution_id("village", "new-idea"),
            cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            buckets: vec![DispositionBucket {
                profile: neutral,
                headcount: 2_000,
            }],
        },
    );
    state.organizations.insert(
        "local-network".to_owned(),
        OrganizationNode {
            id: "local-network".to_owned(),
            target_id: "new-idea".to_owned(),
            base_reach_per_mille: 300,
            concealment_per_mille: 500,
            active: true,
        },
    );
    state.influence_edges.insert(
        "market-contact".to_owned(),
        SocialInfluenceEdge {
            id: "market-contact".to_owned(),
            source: InfluenceSource::Cohort("market".to_owned()),
            target_cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            channel: "local-contact".to_owned(),
            reach_per_mille: 800,
            trust_per_mille: 800,
            active: true,
        },
    );
    state.influence_edges.insert(
        "network-contact".to_owned(),
        SocialInfluenceEdge {
            id: "network-contact".to_owned(),
            source: InfluenceSource::Organization("local-network".to_owned()),
            target_cohort_id: "village".to_owned(),
            target_id: "new-idea".to_owned(),
            channel: "organized-contact".to_owned(),
            reach_per_mille: 700,
            trust_per_mille: 800,
            active: true,
        },
    );
    state.institutional_alignments.insert(
        "council-alignment".to_owned(),
        InstitutionalAlignment {
            id: "council-alignment".to_owned(),
            institution: EntityRef::Government(ids.government),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            support_per_mille: 100,
            enforcement_per_mille: 0,
            access_grant_per_mille: 300,
            authorized_actor: Some(ids.commander),
            last_decision_version: 0,
        },
    );
    state.policies.insert(
        "local-policy".to_owned(),
        PolicyPressure {
            id: "local-policy".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            support_per_mille: 0,
            legal_access_per_mille: 200,
            surveillance_per_mille: 100,
            censorship_per_mille: 0,
            coercion_per_mille: 0,
            material_penalty_per_mille: 0,
            disruption_per_mille: 0,
            migration_pressure_per_mille: 0,
        },
    );
    state.transition_rules.insert(
        "01-awareness".to_owned(),
        TransitionRule {
            id: "01-awareness".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            from: neutral,
            to: aware,
            base_rate_per_million: 0,
            weights: TransitionWeights {
                influence: 200_000,
                ..TransitionWeights::default()
            },
        },
    );
    state.transition_rules.insert(
        "02-assent".to_owned(),
        TransitionRule {
            id: "02-assent".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            from: aware,
            to: sympathetic,
            base_rate_per_million: 0,
            weights: TransitionWeights {
                influence: 100_000,
                institutional_support: 100_000,
                policy_coercion: -50_000,
                ..TransitionWeights::default()
            },
        },
    );
    state.transition_rules.insert(
        "03-public".to_owned(),
        TransitionRule {
            id: "03-public".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            from: sympathetic,
            to: conforming,
            base_rate_per_million: 0,
            weights: TransitionWeights {
                institutional_enforcement: 300_000,
                policy_coercion: 300_000,
                ..TransitionWeights::default()
            },
        },
    );
    state.observer_profiles.insert(
        ids.observer.get().to_string(),
        ObserverProfile {
            actor: ids.observer,
            cohorts: BTreeSet::from(["village".to_owned()]),
            targets: BTreeSet::from(["new-idea".to_owned()]),
            public_detection_per_mille: 800,
            private_detection_per_mille: 200,
            false_positive_per_mille: 10,
            confidence_per_mille: 700,
        },
    );
    state
}

fn register_policy_decision(canwu: &mut Canwu) {
    let ids = Canwu::demo_ids();
    let controller = DecisionControllerBinding::new(
        "council-controller",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "institutional-policy", "1"),
        DecisionAuthority::Actor {
            actor: ids.commander,
        },
    )
    .with_command_subject(EntityRef::Government(ids.government));
    canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(1),
                canwu.revision(),
                DecisionMutation::RegisterController { controller },
            ),
        )
        .expect("register decision controller");
    let ticket = institutional_policy_ticket(
        DecisionTicketId::new(1),
        "tutorial.institutional-policy",
        EntityRef::Person(ids.commander),
        "council-controller",
        "Choose the institution's public response",
        vec![
            PolicyChoice {
                id: "limited-access".to_owned(),
                label: "Allow limited access".to_owned(),
                decision: PolicyDecision {
                    alignment_id: "council-alignment".to_owned(),
                    decision_version: 1,
                    support_per_mille: 300,
                    enforcement_per_mille: 0,
                    access_grant_per_mille: 600,
                },
                utility_inputs: BTreeMap::from([("control".to_owned(), 10)]),
            },
            PolicyChoice {
                id: "public-conformity".to_owned(),
                label: "Require public conformity".to_owned(),
                decision: PolicyDecision {
                    alignment_id: "council-alignment".to_owned(),
                    decision_version: 1,
                    support_per_mille: 300,
                    enforcement_per_mille: 800,
                    access_grant_per_mille: 700,
                },
                utility_inputs: BTreeMap::from([("control".to_owned(), 100)]),
            },
        ],
        None,
    )
    .expect("build institutional policy ticket");
    canwu
        .enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(2),
                canwu.revision(),
                DecisionMutation::Open { ticket },
            ),
        )
        .expect("open institutional policy ticket");
    canwu
        .step_canonical()
        .expect("admit decision setup")
        .expect("decision setup boundary");
}

fn assert_distribution_conservation(state: &SocietyState) {
    for distribution in state.distributions.values() {
        let cohort = &state.cohorts[&distribution.cohort_id];
        let total: u64 = distribution
            .buckets
            .iter()
            .map(|bucket| bucket.headcount)
            .sum();
        assert_eq!(total, cohort.headcount, "{}", distribution.id);
    }
}
