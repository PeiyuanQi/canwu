//! How a society settlement checks the cohort transfers due at it.
//!
//! Three transfers out of one cohort are admitted in one `Monthly` period,
//! all decided against the same state. In a batch, the default, they are
//! checked against the state at the start of the settlement and apply in the
//! order their commands were admitted: each that still fits completes, and
//! the one that no longer fits is a recorded rejection. One by one, the first
//! in operation-ID order completes and the others are stale. Either way the
//! boundary commits, people are conserved, the mode is part of the plugin's
//! identity, and restore and exact replay agree.

use canwu_api::{
    BoundaryRequest, Canwu, Command, CommandAuthority, CommandEnvelope, CommandRequest,
    CommandRequestId, DecisionOrigin, EntityRef, ErrorCode, Issuer, Scenario, SimDuration, SimTime,
    SimulationPlugin, SystemCadence,
};
use canwu_society::{
    AffiliationTarget, AwarenessBand, COHORT_TRANSFER_INSUFFICIENT_SOURCE, CohortTransferIntent,
    DispositionBucket, DispositionDistribution, DispositionProfile, InstitutionalAlignment,
    SocietyCohort, SocietyCohortExchangeLedgerRecord, SocietyPlugin, SocietyState,
    TransferSettlement, distribution_id, load_society_state,
    society_cohort_exchange_ledger_reference, society_state_reference,
};
use std::collections::{BTreeMap, BTreeSet};

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

fn society_state() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let aware = DispositionProfile {
        awareness: AwarenessBand::Aware,
        ..neutral
    };
    let mut state = SocietyState::default();
    for (id, territory, headcount, buckets) in [
        (
            "market",
            ids.western_territory,
            1_000,
            vec![(neutral, 800), (aware, 200)],
        ),
        (
            "village",
            ids.central_territory,
            2_000,
            vec![(neutral, 2_000)],
        ),
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
        state.distributions.insert(
            distribution_id(id, "new-idea"),
            DispositionDistribution {
                id: distribution_id(id, "new-idea"),
                cohort_id: id.to_owned(),
                target_id: "new-idea".to_owned(),
                buckets: buckets
                    .into_iter()
                    .map(|(profile, headcount)| DispositionBucket { profile, headcount })
                    .collect(),
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
    state
}

fn run(plugin: &SocietyPlugin) -> Canwu {
    let snapshot = Canwu::demo(42).expect("demo scenario").snapshot();
    let scenario = Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![society_state().into_record().expect("society record")],
    };
    Canwu::new_with_plugins(42, scenario, &[plugin]).expect("society run")
}

/// Admits, in this order, transfers of 500, 300, and 400 people from the
/// market to the village, each decided against the same society root and
/// due on day 10, and settles the month.
fn transfer_and_settle(canwu: &mut Canwu) {
    let ids = Canwu::demo_ids();
    let version = canwu
        .typed_domain_record(&society_state_reference())
        .expect("society root")
        .version;
    let revision = canwu.revision();
    for (offset, (operation_id, quantity)) in
        (0_u64..).zip([("move-c", 500), ("move-b", 300), ("move-a", 400)])
    {
        let intent = CohortTransferIntent {
            operation_id: operation_id.to_owned(),
            authority_alignment_id: "council-alignment".to_owned(),
            source_cohort_id: "market".to_owned(),
            destination_cohort_id: "village".to_owned(),
            quantity,
            expected_source_version: version,
            due_time: day(10),
        };
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                CommandRequest::new(
                    CommandRequestId::new(revision + offset + 1),
                    revision + offset,
                    CommandEnvelope::new(
                        Issuer::Actor(ids.commander),
                        Command::Plugin {
                            plugin: "canwu-society".to_owned(),
                            command: "transfer_cohort_population".to_owned(),
                            payload: serde_json::to_value(intent).expect("transfer intent"),
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
            .expect("queue a transfer");
    }
    canwu
        .advance_canonical(day(2) - canwu.time())
        .expect("admit the transfers and queue their packets");
    canwu
        .settle_boundary(BoundaryRequest::at(day(30)).with_cadence(SystemCadence::Monthly))
        .expect("the Monthly settlement commits");
}

fn results(canwu: &Canwu) -> BTreeMap<String, String> {
    canwu
        .typed_domain_record(&society_cohort_exchange_ledger_reference())
        .expect("ledger")
        .decode_payload::<SocietyCohortExchangeLedgerRecord>()
        .expect("ledger payload")
        .outcomes
        .into_iter()
        .map(|(operation_id, outcome)| (operation_id, outcome.result))
        .collect()
}

/// The two cohorts' headcounts, after checking that every distribution
/// holds exactly its cohort's people.
fn headcounts(canwu: &Canwu) -> (u64, u64) {
    let state = load_society_state(canwu).expect("society state");
    for distribution in state.distributions.values() {
        let total: u64 = distribution
            .buckets
            .iter()
            .map(|bucket| bucket.headcount)
            .sum();
        assert_eq!(total, state.cohorts[&distribution.cohort_id].headcount);
    }
    (
        state.cohorts["market"].headcount,
        state.cohorts["village"].headcount,
    )
}

fn assert_restores_and_replays(canwu: &Canwu, plugin: &SocietyPlugin) {
    let plugins: [&dyn SimulationPlugin; 1] = [plugin];
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

#[test]
fn one_by_one_completes_one_transfer_of_a_cohort_per_settlement() {
    let plugin = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly)
        .with_transfer_settlement(TransferSettlement::OneByOne);
    let mut canwu = run(&plugin);
    transfer_and_settle(&mut canwu);
    assert_eq!(
        results(&canwu),
        BTreeMap::from([
            ("move-a".to_owned(), "completed".to_owned()),
            ("move-b".to_owned(), "stale_source".to_owned()),
            ("move-c".to_owned(), "stale_source".to_owned()),
        ])
    );
    assert_eq!(headcounts(&canwu), (600, 2_400));
    assert_restores_and_replays(&canwu, &plugin);
}

#[test]
fn a_batch_completes_every_transfer_that_still_fits() {
    let plugin = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly);
    assert_eq!(plugin.transfer_settlement(), TransferSettlement::Batch);
    assert_eq!(
        SocietyPlugin::default().transfer_settlement(),
        TransferSettlement::Batch
    );
    let mut canwu = run(&plugin);
    transfer_and_settle(&mut canwu);
    // Admission order: 500 leaves 500, 300 leaves 200, and 400 no longer
    // fits. Nobody is created or lost.
    assert_eq!(
        results(&canwu),
        BTreeMap::from([
            (
                "move-a".to_owned(),
                COHORT_TRANSFER_INSUFFICIENT_SOURCE.to_owned()
            ),
            ("move-b".to_owned(), "completed".to_owned()),
            ("move-c".to_owned(), "completed".to_owned()),
        ])
    );
    assert_eq!(headcounts(&canwu), (200, 2_800));
    assert_restores_and_replays(&canwu, &plugin);

    // The mode is part of the plugin's identity: the run is restored only
    // with a plugin of the same mode.
    let one_by_one = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly)
        .with_transfer_settlement(TransferSettlement::OneByOne);
    assert_eq!(
        Canwu::from_snapshot_json_with_plugins(
            &canwu.snapshot_json().expect("snapshot"),
            &[&one_by_one],
        )
        .err()
        .expect("a one-by-one plugin cannot restore a batch run")
        .code,
        ErrorCode::PluginManifestMismatch
    );
}
