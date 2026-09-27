//! Downstream gap set §25–§26 (gaps G-31 and G-32): policy-pressure
//! provenance and cohort headcount rebase against an external stock.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, Command, CommandAuthority, CommandEnvelope,
    CommandRequest, CommandRequestId, DecisionOrigin, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordLifecycle, DomainRecordMutation, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainReferenceTarget,
    DomainValueKindClass, EntityRef, ErrorCode, GovernmentId, IngressClass, IngressId,
    IngressPayload, Issuer, PayloadSchema, PluginIngressDescriptor, PluginIngressRequest,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, TypedDomainRecordRef,
};
use canwu_society::{
    AffiliationTarget, AssentBand, AwarenessBand, COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1,
    CohortTransferIntent, DispositionBucket, DispositionDistribution, DispositionProfile,
    InstitutionalAlignment, PolicyPressure, REBASE_STALE_STOCK_REJECTION, RebaseReason,
    SocietyCohort, SocietyCohortExchangeLedgerRecord, SocietyIngressQueueRecord,
    SocietyIngressStatus, SocietyPlugin, SocietyState, TransitionRule, TransitionWeights,
    distribution_id, load_society_state, society_cohort_exchange_ledger_reference,
    society_ingress_queue_reference, society_state_reference, validate_society_runtime,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn gap_g31_society_policy_pressure_provenance() {
    let ids = Canwu::demo_ids();
    let mut state = society_state();
    let mut issued = policy("issued-policy");
    issued.issuer = Some(EntityRef::Government(ids.government));
    issued.decision_version = 3;
    state.policies.insert(issued.id.clone(), issued.clone());
    let mut personal = policy("personal-policy");
    personal.issuer = Some(EntityRef::Person(ids.commander));
    state.policies.insert(personal.id.clone(), personal.clone());
    let plain = policy("plain-policy");
    state.policies.insert(plain.id.clone(), plain.clone());

    // Records that do not use provenance keep their canonical encoding.
    let encoded = serde_json::to_value(&plain).expect("encode plain policy");
    assert!(encoded.get("issuer").is_none() && encoded.get("decision_version").is_none());
    let encoded = serde_json::to_value(&issued).expect("encode issued policy");
    assert_eq!(encoded["decision_version"], 3);

    // A decision version without an issuer, or a non-authority issuer, is
    // rejected by the society plugin's validation.
    let mut orphan = state.clone();
    orphan
        .policies
        .get_mut("plain-policy")
        .expect("plain policy")
        .decision_version = 1;
    assert_eq!(
        orphan
            .validate()
            .expect_err("versioned policy needs an issuer")
            .code,
        ErrorCode::InvalidDomainRecord
    );
    let mut army = state.clone();
    army.policies
        .get_mut("plain-policy")
        .expect("plain policy")
        .issuer = Some(EntityRef::Army(ids.army));
    assert_eq!(
        army.validate()
            .expect_err("an army is not a policy issuer")
            .code,
        ErrorCode::InvalidDomainRecord
    );

    // The issuer is bound into the record's core references, so a missing
    // issuer entity is rejected when the scenario is admitted.
    let record = state.clone().into_record().expect("society record");
    assert!(record.references.iter().any(|reference| {
        reference.role == "institution"
            && reference.target
                == DomainReferenceTarget::Core(EntityRef::Government(ids.government))
    }));
    assert!(record.references.iter().any(|reference| {
        reference.role == "actor"
            && reference.target == DomainReferenceTarget::Core(EntityRef::Person(ids.commander))
    }));
    let mut missing = state.clone();
    missing
        .policies
        .get_mut("issued-policy")
        .expect("issued policy")
        .issuer = Some(EntityRef::Government(GovernmentId::new(9_999)));
    assert!(
        Canwu::new_with_plugins(
            42,
            scenario(vec![missing.into_record().expect("missing-issuer record")]),
            &[&SocietyPlugin],
        )
        .is_err(),
        "an issuer that is not a live entity must not be admitted"
    );

    // Provenance round-trips through settlement, save/load, and exact replay.
    let mut canwu =
        Canwu::new_with_plugins(42, scenario(vec![record]), &[&SocietyPlugin]).expect("society");
    canwu
        .settle_boundary(
            BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                .with_cadence(SystemCadence::Daily),
        )
        .expect("daily society boundary");
    let settled = load_society_state(&canwu).expect("settled society state");
    assert_eq!(settled.policies["issued-policy"], issued);
    assert_eq!(settled.policies["personal-policy"], personal);
    assert_eq!(settled.policies["plain-policy"], plain);
    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        &[&SocietyPlugin],
    )
    .expect("restore provenance snapshot");
    assert_eq!(restored.snapshot(), snapshot);
    assert_eq!(
        load_society_state(&restored)
            .expect("restored state")
            .policies["issued-policy"],
        issued
    );
    let replayed = Canwu::replay_from_journal(&[&SocietyPlugin], &canwu.replay_journal())
        .expect("replay provenance history");
    assert_eq!(replayed.snapshot(), snapshot);
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g32_society_cohort_rebase() {
    let stock = StockPlugin(StateVisibility::SameBoundary);
    let plugins: [&dyn SimulationPlugin; 2] = [&SocietyPlugin, &stock];
    let mut canwu = Canwu::new_with_plugins(
        42,
        scenario(vec![
            society_state().into_record().expect("society record"),
            stock_record(1_000),
        ]),
        &plugins,
    )
    .expect("society and stock simulation");
    let stock_v1 = DomainRecordVersionRef {
        record: stock_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };

    // A rebase citing the current stock version is accepted at admission.
    let accepted = enqueue_rebase(&mut canwu, 1_337, stock_v1.clone());
    // A cohort transfer admitted now is queued against the pre-rebase
    // distribution; the rebase makes its source stale before it is due.
    let due = canwu.time();
    enqueue_transfer(&mut canwu, 1, "resettlement", 1, due);
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the current rebase");
    // The stock then changes, so a second rebase citing version 1 is stale.
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            STOCK_PLUGIN,
            SET_STOCK_INGRESS,
            canwu.time(),
            serde_json::json!({ "headcount": 1_500 }),
        ))
        .expect("queue a stock change");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("apply the stock change");
    assert_eq!(
        canwu
            .typed_domain_record(&stock_reference())
            .expect("stock")
            .version,
        2
    );
    let stale_rebase = enqueue_rebase(&mut canwu, 1_500, stock_v1);
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the stale rebase");
    let queue = canwu
        .typed_domain_record(&society_ingress_queue_reference())
        .expect("society ingress queue")
        .decode_payload::<SocietyIngressQueueRecord>()
        .expect("queue payload");
    assert_eq!(queue.entries.len(), 2);
    assert_eq!(queue.entries[0].admission_rejection, None);
    assert_eq!(
        queue.entries[1].admission_rejection.as_deref(),
        Some(REBASE_STALE_STOCK_REJECTION)
    );
    assert_eq!(
        load_society_state(&canwu).expect("state").cohorts["market"].headcount,
        1_000,
        "a queued rebase waits for the Daily society settlement"
    );

    canwu
        .settle_boundary(
            BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
                .with_cadence(SystemCadence::Daily),
        )
        .expect("daily settlement applies the queue");
    let state = load_society_state(&canwu).expect("rebased society state");
    assert_eq!(state.cohorts["market"].headcount, 1_337);
    assert_eq!(state.cohorts["village"].headcount, 2_000);
    // 800 and 200 scale to exact shares 1069.6 and 267.4; the one leftover
    // unit goes to the largest remainder.
    let market = &state.distributions[&distribution_id("market", "new-idea")];
    assert_eq!(
        market
            .buckets
            .iter()
            .map(|bucket| bucket.headcount)
            .collect::<Vec<_>>(),
        vec![1_070, 267]
    );
    for distribution in state.distributions.values() {
        let total: u64 = distribution
            .buckets
            .iter()
            .map(|bucket| bucket.headcount)
            .sum();
        assert_eq!(total, state.cohorts[&distribution.cohort_id].headcount);
    }
    for (bucket, before) in market.buckets.iter().zip([800_u64, 200]) {
        let exact_thousandths = before * 1_337;
        assert!(bucket.headcount * 1_000 <= exact_thousandths + 1_000);
        assert!(bucket.headcount * 1_000 + 1_000 > exact_thousandths);
    }

    let ledger = canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    let applied = &ledger.rebases[&accepted.get().to_string()];
    assert_eq!(applied.status, SocietyIngressStatus::Applied);
    assert_eq!(applied.previous_headcount, Some(1_000));
    assert_eq!(
        applied.rebase.as_ref().map(|rebase| rebase.reason),
        Some(RebaseReason::ExternalStockChange)
    );
    let rejected = &ledger.rebases[&stale_rebase.get().to_string()];
    assert_eq!(rejected.status, SocietyIngressStatus::Rejected);
    assert_eq!(
        rejected.rejection.as_deref(),
        Some(REBASE_STALE_STOCK_REJECTION)
    );
    assert!(
        canwu
            .typed_domain_record(&society_ingress_queue_reference())
            .expect("queue")
            .decode_payload::<SocietyIngressQueueRecord>()
            .expect("queue payload")
            .entries
            .is_empty()
    );

    let snapshot = canwu.snapshot();
    let mut restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        &plugins,
    )
    .expect("restore rebase snapshot");
    assert_eq!(restored.snapshot(), snapshot);
    let mut replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("replay rebase history");
    assert_eq!(replayed.snapshot(), snapshot);
    for simulation in [&mut canwu, &mut restored, &mut replayed] {
        for day in [2, 3] {
            simulation
                .settle_boundary(
                    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day))
                        .with_cadence(SystemCadence::Daily),
                )
                .expect("continue after the rebase");
        }
    }
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    // The stale transfer settles as a terminal ledger outcome instead of
    // failing every later Daily boundary.
    let ledger = canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    assert_eq!(ledger.outcomes["resettlement"].result, "stale_source");
    assert_eq!(
        load_society_state(&canwu).expect("state").cohorts["market"].headcount,
        1_337
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g32_society_transfer_and_queue_authority() {
    // A deferred transfer settles after a Daily boundary has materialized
    // aggregates and while a transition keeps changing the destination's
    // dispositions; later Daily boundaries keep settling.
    let plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin];
    let neutral = DispositionProfile::neutral();
    let mut state = society_state();
    state.transition_rules.insert(
        "village-awareness".to_owned(),
        TransitionRule {
            id: "village-awareness".to_owned(),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::from(["village".to_owned()]),
            from: neutral,
            to: DispositionProfile {
                awareness: AwarenessBand::Aware,
                ..neutral
            },
            base_rate_per_million: 100_000,
            weights: TransitionWeights::default(),
        },
    );
    let mut canwu = Canwu::new_with_plugins(
        42,
        scenario(vec![state.into_record().expect("society record")]),
        &plugins,
    )
    .expect("society simulation");
    daily(&mut canwu, 1);
    assert!(
        !load_society_state(&canwu)
            .expect("state")
            .aggregates
            .is_empty()
    );
    let version = canwu
        .typed_domain_record(&society_state_reference())
        .expect("society record")
        .version;
    enqueue_transfer(&mut canwu, 1, "migration", version, day(3));
    for days in 2..=6 {
        daily(&mut canwu, days);
    }
    let state = load_society_state(&canwu).expect("state after the transfer");
    assert_eq!(state.cohorts["market"].headcount, 950);
    assert_eq!(state.cohorts["village"].headcount, 2_050);
    for distribution in state.distributions.values() {
        let total: u64 = distribution
            .buckets
            .iter()
            .map(|bucket| bucket.headcount)
            .sum();
        assert_eq!(total, state.cohorts[&distribution.cohort_id].headcount);
    }
    assert!(!state.aggregates.is_empty());
    let ledger = canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload");
    assert_eq!(ledger.outcomes["migration"].result, "completed");
    assert_eq!(ledger.outcomes["migration"].completed_at, day(3));

    // A rebase admitted at a non-Daily boundary waits in the queue; restore
    // re-derives the queued entry from the ingress journal, and replay agrees.
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canwu-society",
            COHORT_REBASE_INGRESS,
            canwu.time(),
            serde_json::to_value(CohortHeadcountRebaseV1 {
                cohort_id: "village".to_owned(),
                new_headcount: 2_100,
                external_stock: stock_version(),
                reason: RebaseReason::Correction,
            })
            .expect("rebase payload"),
        ))
        .expect("queue a rebase");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the rebase");
    let queued = canwu
        .typed_domain_record(&society_ingress_queue_reference())
        .expect("queue")
        .decode_payload::<SocietyIngressQueueRecord>()
        .expect("queue payload");
    assert_eq!(queued.entries.len(), 1);
    assert_eq!(
        queued.entries[0].admission_rejection.as_deref(),
        Some(REBASE_STALE_STOCK_REJECTION),
        "a stock record that does not exist is never current"
    );
    let snapshot = canwu.snapshot();
    let restored = Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        &plugins,
    )
    .expect("restore the queued rebase");
    validate_society_runtime(&restored).expect("queued entry matches its admitted packet");
    assert_eq!(restored.snapshot(), snapshot);
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("replay the transfer history");
    assert_eq!(replayed.snapshot(), snapshot);

    // Restore recomputes a same-boundary admission verdict from the stock
    // writer's phase and visibility: a phase-7 change is visible to the
    // phase-12 intake only with same-boundary visibility.
    for (visibility, verdict) in [
        (
            StateVisibility::SameBoundary,
            Some(REBASE_STALE_STOCK_REJECTION),
        ),
        (StateVisibility::NextBoundary, None),
    ] {
        let stock = StockPlugin(visibility);
        let plugins: [&dyn SimulationPlugin; 2] = [&SocietyPlugin, &stock];
        let mut canwu = Canwu::new_with_plugins(
            42,
            scenario(vec![
                society_state().into_record().expect("society record"),
                stock_record(1_000),
            ]),
            &plugins,
        )
        .expect("society and stock simulation");
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                STOCK_PLUGIN,
                SET_STOCK_INGRESS,
                canwu.time(),
                serde_json::json!({ "headcount": 1_200 }),
            ))
            .expect("queue a stock change");
        enqueue_rebase(&mut canwu, 1_200, stock_version());
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("admit the stock change and rebase together");
        let queued = canwu
            .typed_domain_record(&society_ingress_queue_reference())
            .expect("queue")
            .decode_payload::<SocietyIngressQueueRecord>()
            .expect("queue payload");
        assert_eq!(queued.entries[0].admission_rejection.as_deref(), verdict);
        let restored = Canwu::from_snapshot_json_with_plugins(
            &serde_json::to_string(&canwu.snapshot()).expect("encode snapshot"),
            &plugins,
        )
        .expect("restore the same-boundary rebase");
        validate_society_runtime(&restored).expect("the recomputed verdict matches");
    }

    // An initial scenario cannot seed the owner-side queue: a strictly
    // well-formed but unadmitted entry is refused, and so is an unknown field.
    let seeded_entry = serde_json::json!({
        "ingress": 999,
        "admitted_at": 1,
        "packet_type": COHORT_REBASE_INGRESS,
        "payload": serde_json::to_value(CohortHeadcountRebaseV1 {
            cohort_id: "market".to_owned(),
            new_headcount: 5,
            external_stock: stock_version(),
            reason: RebaseReason::Correction,
        })
        .expect("seeded rebase"),
    });
    let seeded = |entry: serde_json::Value| {
        let payload = serde_json::json!({ "schema_version": 1, "entries": [entry] });
        Canwu::new_with_plugins(
            42,
            scenario(vec![
                society_state().into_record().expect("society record"),
                DomainRecord {
                    reference: society_ingress_queue_reference().into_untyped(),
                    owner: "canwu-society".to_owned(),
                    class: DomainRecordClass::Record,
                    version: 1,
                    lifecycle: DomainRecordLifecycle::Active,
                    payload,
                    references: Vec::new(),
                },
            ]),
            &plugins,
        )
    };
    let mut misspelled = seeded_entry.clone();
    misspelled["admision_rejection"] = serde_json::json!("typo");
    assert!(
        seeded(misspelled).is_err(),
        "unknown queue fields are rejected"
    );
    let mut forged = seeded(seeded_entry).expect("the seeded queue is well formed");
    assert_eq!(
        load_society_state(&forged)
            .expect_err("a host load refuses the seeded queue")
            .code,
        ErrorCode::InvalidAuthority
    );
    assert_eq!(
        forged
            .settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Daily))
            .expect_err("the society owner refuses unadmitted ingress")
            .code,
        ErrorCode::InvalidAuthority
    );
    assert_eq!(
        forged
            .typed_domain_record(&society_state_reference())
            .expect("society record")
            .decode_payload::<canwu_society::SocietyStateRecord>()
            .expect("society payload")
            .cohorts["market"]
            .headcount,
        1_000
    );
}

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

/// Settles any canonical ingress due before `days`, then a Daily boundary.
fn daily(canwu: &mut Canwu, days: i64) {
    canwu
        .advance_canonical(day(days) - canwu.time())
        .expect("settle due canonical ingress");
    canwu
        .settle_boundary(BoundaryRequest::at(day(days)).with_cadence(SystemCadence::Daily))
        .expect("daily society boundary");
}

fn stock_version() -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: stock_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn enqueue_transfer(
    canwu: &mut Canwu,
    request_id: u64,
    operation_id: &str,
    expected_source_version: u64,
    due_time: SimTime,
) {
    let ids = Canwu::demo_ids();
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(ids.commander),
                    Command::Plugin {
                        plugin: "canwu-society".to_owned(),
                        command: "transfer_cohort_population".to_owned(),
                        payload: serde_json::to_value(CohortTransferIntent {
                            operation_id: operation_id.to_owned(),
                            authority_alignment_id: "council-alignment".to_owned(),
                            source_cohort_id: "market".to_owned(),
                            destination_cohort_id: "village".to_owned(),
                            quantity: 50,
                            expected_source_version,
                            due_time,
                        })
                        .expect("transfer intent"),
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
            ),
        )
        .expect("queue a cohort transfer");
}

fn enqueue_rebase(
    canwu: &mut Canwu,
    new_headcount: u64,
    external_stock: DomainRecordVersionRef,
) -> IngressId {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canwu-society",
            COHORT_REBASE_INGRESS,
            canwu.time(),
            serde_json::to_value(CohortHeadcountRebaseV1 {
                cohort_id: "market".to_owned(),
                new_headcount,
                external_stock,
                reason: RebaseReason::ExternalStockChange,
            })
            .expect("rebase payload"),
        ))
        .expect("queue a cohort rebase")
        .ingress_id
}

fn scenario(domain_records: Vec<DomainRecord>) -> Scenario {
    let snapshot = Canwu::demo(42).expect("demo scenario").snapshot();
    Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records,
    }
}

fn society_state() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let aware = DispositionProfile {
        awareness: AwarenessBand::Aware,
        assent: AssentBand::Sympathetic,
        ..neutral
    };
    let mut state = SocietyState::default();
    for (id, territory, headcount) in [
        ("market", ids.western_territory, 1_000),
        ("village", ids.central_territory, 2_000),
    ] {
        state.cohorts.insert(
            id.to_owned(),
            SocietyCohort {
                id: id.to_owned(),
                territory,
                headcount,
                classification: BTreeMap::new(),
            },
        );
    }
    state.targets.insert(
        "new-idea".to_owned(),
        AffiliationTarget {
            id: "new-idea".to_owned(),
            parent: None,
            neutral_profile: neutral,
            metadata: BTreeMap::new(),
        },
    );
    state.institutional_alignments.insert(
        "council-alignment".to_owned(),
        InstitutionalAlignment {
            id: "council-alignment".to_owned(),
            institution: EntityRef::Government(ids.government),
            target_id: "new-idea".to_owned(),
            affected_cohorts: BTreeSet::new(),
            support_per_mille: 0,
            enforcement_per_mille: 0,
            access_grant_per_mille: 0,
            authorized_actor: Some(ids.commander),
            last_decision_version: 0,
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
                    profile: aware,
                    headcount: 200,
                },
            ],
        },
    );
    state
}

fn policy(id: &str) -> PolicyPressure {
    PolicyPressure {
        id: id.to_owned(),
        target_id: "new-idea".to_owned(),
        affected_cohorts: BTreeSet::from(["village".to_owned()]),
        support_per_mille: 100,
        legal_access_per_mille: 200,
        surveillance_per_mille: 0,
        censorship_per_mille: 0,
        coercion_per_mille: 0,
        material_penalty_per_mille: 0,
        disruption_per_mille: 0,
        migration_pressure_per_mille: 0,
        issuer: None,
        decision_version: 0,
    }
}

const STOCK_PLUGIN: &str = "test-population-stock";
const SET_STOCK_INGRESS: &str = "set_population_stock";

/// An application-owned conserved stock the society cohort follows.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PopulationStock {
    headcount: u64,
}

struct PopulationStockRecord;

impl DomainRecordType for PopulationStockRecord {
    type Payload = PopulationStock;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "test.stock";
    const NAME: &'static str = "population";
}

fn stock_reference() -> TypedDomainRecordRef<PopulationStockRecord> {
    TypedDomainRecordRef::new("market")
}

fn stock_key() -> StateKey {
    StateKey::new(
        PopulationStockRecord::NAMESPACE,
        PopulationStockRecord::NAME,
    )
}

fn stock_record(headcount: u64) -> DomainRecord {
    let draft = DomainRecordDraft::from_typed(stock_reference(), &PopulationStock { headcount })
        .expect("stock draft");
    DomainRecord {
        reference: draft.reference,
        owner: STOCK_PLUGIN.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: Vec::new(),
    }
}

/// Stock owner whose phase-7 writer uses the given visibility.
struct StockPlugin(StateVisibility);

impl SimulationPlugin for StockPlugin {
    fn name(&self) -> &'static str {
        STOCK_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "0f3c6a1b9d2e4f5061728394a5b6c7d8e9f00112233445566778899aabbccdde"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar
            .register_record_schema(DomainRecordSchema::for_record::<PopulationStockRecord>())?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: SET_STOCK_INGRESS.to_owned(),
            description: "Replace the population stock".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut apply = BoundarySystemContract::new(
            "apply-population-stock",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        apply.reads = vec![stock_key(), StateKey::core_ingress()];
        apply.writes = vec![stock_key()];
        apply.visibility = self.0;
        registrar.register_boundary_system(apply, apply_population_stock)
    }
}

fn apply_population_stock(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin { payload, .. } = &ingress.payload else {
            continue;
        };
        let stock: PopulationStock = serde_json::from_value(payload.clone())
            .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?;
        let current = view
            .typed_domain_record(&stock_reference())?
            .ok_or_else(|| CanwuError::new(ErrorCode::DomainRecordNotFound, "no stock"))?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(stock_reference(), &stock)?,
                expected_version: current.version,
            },
            summary: "Replaced the population stock".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}
