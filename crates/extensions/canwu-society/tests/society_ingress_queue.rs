//! The owner-side society ingress queue is a small head record and one batch
//! record for each boundary that admits packets to it, so what a boundary
//! journals for the queue follows what it admits, not how long the queue
//! is. A packet that finds the queue full is refused by the boundary that
//! admits it, with a terminal outcome in the cohort exchange ledger.

use canwu_api::{
    BoundaryRecord, BoundaryRequest, Canwu, CanwuError, CompactedCanwu, DomainRecord,
    DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass,
    IngressId, PluginIngressRequest, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin, SystemCadence, TypedDomainRecordRef,
};
use canwu_society::{
    AffiliationTarget, COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1, DispositionBucket,
    DispositionDistribution, DispositionProfile, MAX_SOCIETY_INGRESS_QUEUE, OutcomeRetention,
    RebaseReason, SOCIETY_INGRESS_QUEUE_FULL_REJECTION, SOCIETY_INGRESS_REJECTED_EVENT,
    SocietyCohort, SocietyCohortExchangeLedger, SocietyCohortExchangeLedgerRecord,
    SocietyIngressBatchRecord, SocietyIngressStatus, SocietyPlugin, SocietyState, distribution_id,
    load_society_state, society_cohort_exchange_ledger_reference, society_ingress_batch_reference,
    society_ingress_queue_entries, validate_society_runtime,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MONTH: i64 = 30;
const STOCK_PLUGIN: &str = "fixture-population-stock";

/// An application-owned conserved stock the market cohort follows. It never
/// changes, so a rebase that cites its first version is always current.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PopulationStock {
    headcount: u64,
}

struct PopulationStockRecord;

impl DomainRecordType for PopulationStockRecord {
    type Payload = PopulationStock;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.stock";
    const NAME: &'static str = "population";
}

/// Owns the stock record.
struct StockPlugin;

impl SimulationPlugin for StockPlugin {
    fn name(&self) -> &'static str {
        STOCK_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c7"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::for_record::<PopulationStockRecord>())
    }
}

fn stock_record() -> (DomainRecord, DomainRecordVersionRef) {
    let draft = DomainRecordDraft::from_typed(
        TypedDomainRecordRef::<PopulationStockRecord>::new("market"),
        &PopulationStock { headcount: 1_000 },
    )
    .expect("stock draft");
    let version = DomainRecordVersionRef {
        record: draft.reference.clone(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let record = DomainRecord {
        reference: draft.reference,
        owner: STOCK_PLUGIN.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: Vec::new(),
    };
    (record, version)
}

fn society_state() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let mut state = SocietyState::default();
    state.cohorts.insert(
        "market".to_owned(),
        SocietyCohort {
            id: "market".to_owned(),
            territory: ids.western_territory,
            headcount: 1_000,
            classification: BTreeMap::new(),
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
            buckets: vec![DispositionBucket {
                profile: neutral,
                headcount: 1_000,
            }],
        },
    );
    state
}

/// A Monthly society beside the stock, and the stock version rebases cite.
fn monthly_society(plugins: &[&dyn SimulationPlugin]) -> (Canwu, DomainRecordVersionRef) {
    let snapshot = Canwu::demo(42).expect("demo scenario").snapshot();
    let (stock, version) = stock_record();
    let scenario = Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: vec![
            society_state().into_record().expect("society record"),
            stock,
        ],
    };
    (
        Canwu::new_with_plugins(42, scenario, plugins).expect("society run"),
        version,
    )
}

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

fn rebase(at: SimTime, headcount: u64, stock: &DomainRecordVersionRef) -> PluginIngressRequest {
    PluginIngressRequest::new(
        "canwu-society",
        COHORT_REBASE_INGRESS,
        at,
        serde_json::to_value(CohortHeadcountRebaseV1 {
            cohort_id: "market".to_owned(),
            new_headcount: headcount,
            external_stock: stock.clone(),
            reason: RebaseReason::ExternalStockChange,
        })
        .expect("rebase payload"),
    )
}

fn boundary(at: SimTime, monthly: bool) -> BoundaryRequest {
    let request = BoundaryRequest::at(at).with_cadence(SystemCadence::Daily);
    if monthly {
        request.with_cadence(SystemCadence::Monthly)
    } else {
        request
    }
}

/// The JSON bytes of the queue's record changes in `boundaries`: its head
/// and its batches, each change with its previous and its current record.
fn queue_evidence(boundaries: &[BoundaryRecord]) -> usize {
    boundaries
        .iter()
        .flat_map(|boundary| &boundary.record_changes)
        .filter(|change| {
            let kind = &change.current.reference.kind;
            kind.namespace == "canwu.society" && kind.name.starts_with("ingress-")
        })
        .map(|change| serde_json::to_vec(change).expect("record change").len())
        .sum()
}

fn ledger(knowledge: Option<&DomainRecord>) -> SocietyCohortExchangeLedger {
    knowledge.map_or_else(SocietyCohortExchangeLedger::empty, |record| {
        record
            .decode_payload::<SocietyCohortExchangeLedgerRecord>()
            .expect("ledger payload")
    })
}

fn market(canwu: &Canwu) -> u64 {
    load_society_state(canwu).expect("society state").cohorts["market"].headcount
}

/// One rebase a day waits a month for a Monthly settlement. Each admitting
/// boundary journals the same few hundred bytes for the queue, its own batch
/// and the head, however many packets already wait; under 0.15.0 it
/// rewrote the whole queue each day. The settlement applies every batch in
/// admission order and empties the slots, which the next packets reuse. A
/// run restored or sealed while the queue waits settles it as the
/// uninterrupted run and its exact replay do.
#[test]
#[allow(clippy::too_many_lines)]
fn a_waiting_queue_journals_what_each_boundary_admits() {
    let society = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly);
    let stock_plugin = StockPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&society, &stock_plugin];
    let (mut canwu, stock) = monthly_society(&plugins);

    let mut evidence = Vec::new();
    for days in 1..MONTH {
        let before = canwu.boundaries().len();
        canwu
            .enqueue_plugin_ingress(rebase(day(days), 1_000 + days.unsigned_abs(), &stock))
            .expect("rebase");
        canwu
            .settle_boundary(boundary(day(days), false))
            .expect("a Daily boundary");
        evidence.push(queue_evidence(&canwu.boundaries()[before..]));
    }
    let waiting = usize::try_from(MONTH - 1).expect("days");
    assert_eq!(
        society_ingress_queue_entries(&canwu)
            .expect("queued society ingress")
            .len(),
        waiting
    );
    // The first admission creates the head and the first batch record; from
    // then on each costs what the second did, a counter digit more at most,
    // and never the packets already waiting.
    assert!(
        evidence[1..].iter().all(|bytes| *bytes <= evidence[1] + 64),
        "{evidence:?}"
    );
    assert_eq!(
        market(&canwu),
        1_000,
        "nothing applies before the settlement"
    );

    // A restore re-derives every queued packet from the ingress journal.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    validate_society_runtime(&restored).expect("queued packets match their ingress");

    // Seal the history while the queue waits.
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a quiet boundary before sealing");
    let mut unsealed = canwu.fork();
    let mut sealed = canwu.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    let mut from_sealed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the compact run");

    // The Monthly settlement applies the queue in admission order, so the
    // last rebase sets the headcount, and every rebase has its outcome.
    unsealed
        .settle_boundary(boundary(day(MONTH), true))
        .expect("the Monthly settlement");
    for run in [&mut sealed, &mut from_sealed] {
        run.settle_boundary(boundary(day(MONTH), true))
            .expect("the compact Monthly settlement");
    }
    assert_eq!(market(&unsealed), 1_000 + (MONTH - 1).unsigned_abs());
    let outcomes =
        ledger(unsealed.typed_domain_record(&society_cohort_exchange_ledger_reference()));
    assert_eq!(outcomes.rebases.len(), waiting);
    assert!(
        outcomes
            .rebases
            .values()
            .all(|outcome| outcome.status == SocietyIngressStatus::Applied)
    );
    assert!(
        society_ingress_queue_entries(&unsealed)
            .expect("queued society ingress")
            .is_empty()
    );
    for slot in 0..u32::try_from(waiting).expect("slots") {
        assert!(
            unsealed
                .typed_domain_record(&society_ingress_batch_reference(slot))
                .expect("a used slot")
                .decode_payload::<SocietyIngressBatchRecord>()
                .expect("batch payload")
                .entries
                .is_empty()
        );
    }
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(from_sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    let journal = sealed
        .replay_journal_with_segments(vec![segment])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());

    // The next packet reuses the first slot.
    unsealed
        .enqueue_plugin_ingress(rebase(day(MONTH + 1), 1_100, &stock))
        .expect("rebase");
    unsealed
        .settle_boundary(boundary(day(MONTH + 1), false))
        .expect("a Daily boundary");
    assert_eq!(
        unsealed
            .typed_domain_record(&society_ingress_batch_reference(0))
            .expect("the first slot")
            .decode_payload::<SocietyIngressBatchRecord>()
            .expect("batch payload")
            .entries
            .len(),
        1
    );
}

/// A packet that arrives while the queue is full is refused by the boundary
/// that admits it: a terminal rejected outcome with `queue_full` in the
/// ledger, where the provider reads every other rebase outcome, and one
/// event. The boundary commits, the queue is untouched, and once a
/// settlement empties it the provider's next rebase is queued again.
#[test]
fn a_packet_that_finds_the_queue_full_is_refused_with_a_recorded_outcome() {
    let society = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly);
    let stock_plugin = StockPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&society, &stock_plugin];
    let (mut canwu, stock) = monthly_society(&plugins);

    for index in 0..MAX_SOCIETY_INGRESS_QUEUE {
        canwu
            .enqueue_plugin_ingress(rebase(
                day(1),
                1_000 + u64::try_from(index % 7).expect("index"),
                &stock,
            ))
            .expect("rebase");
    }
    canwu
        .settle_boundary(boundary(day(1), false))
        .expect("the boundary that fills the queue");
    let ledger_reference = society_cohort_exchange_ledger_reference();
    assert!(canwu.typed_domain_record(&ledger_reference).is_none());

    let before = canwu.boundaries().len();
    let refused: IngressId = canwu
        .enqueue_plugin_ingress(rebase(day(2), 1_500, &stock))
        .expect("one rebase too many")
        .ingress_id;
    canwu
        .settle_boundary(boundary(day(2), false))
        .expect("the boundary that refuses it commits");
    assert_eq!(queue_evidence(&canwu.boundaries()[before..]), 0);
    assert_eq!(
        society_ingress_queue_entries(&canwu)
            .expect("queued society ingress")
            .len(),
        MAX_SOCIETY_INGRESS_QUEUE
    );
    let outcome =
        &ledger(canwu.typed_domain_record(&ledger_reference)).rebases[&refused.get().to_string()];
    assert_eq!(outcome.status, SocietyIngressStatus::Rejected);
    assert_eq!(
        outcome.rejection.as_deref(),
        Some(SOCIETY_INGRESS_QUEUE_FULL_REJECTION)
    );
    assert_eq!(outcome.settled_at, day(2));
    assert_eq!(
        outcome.rebase.as_ref().map(|rebase| rebase.new_headcount),
        Some(1_500)
    );
    let events: Vec<_> = canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some(("canwu-society", SOCIETY_INGRESS_REJECTED_EVENT))
        })
        .collect();
    assert_eq!(events.len(), 1);
    assert!(
        events[0]
            .summary
            .ends_with(SOCIETY_INGRESS_QUEUE_FULL_REJECTION)
    );

    // The settlement applies what waited; the refused rebase stays refused,
    // and the provider's next one is queued.
    canwu
        .settle_boundary(boundary(day(MONTH), true))
        .expect("the Monthly settlement");
    let outcomes = ledger(canwu.typed_domain_record(&ledger_reference)).rebases;
    assert_eq!(outcomes.len(), MAX_SOCIETY_INGRESS_QUEUE + 1);
    assert_eq!(
        outcomes
            .values()
            .filter(|outcome| outcome.status == SocietyIngressStatus::Applied)
            .count(),
        MAX_SOCIETY_INGRESS_QUEUE
    );
    canwu
        .enqueue_plugin_ingress(rebase(day(MONTH + 1), 1_500, &stock))
        .expect("the provider sends the rebase again");
    canwu
        .settle_boundary(boundary(day(MONTH + 1), false))
        .expect("a Daily boundary");
    assert_eq!(
        society_ingress_queue_entries(&canwu)
            .expect("queued society ingress")
            .len(),
        1
    );

    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    validate_society_runtime(&restored).expect("the queued rebase matches its ingress");
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

/// Under `OutcomeRetention::LatestSettlement` a settlement that records
/// rebase outcomes first drops those of earlier settlements, so the ledger,
/// which each such settlement rewrites whole, stays the size of one
/// settlement's outcomes; under the default `Keep` it holds every outcome of
/// the run. The retention is part of the plugin's identity, and a run on it
/// restores and replays exactly.
#[test]
fn latest_settlement_retention_bounds_the_rebase_outcomes() {
    let stock_plugin = StockPlugin;
    let keep = SocietyPlugin::with_settlement_cadence(SystemCadence::Monthly);
    assert_eq!(keep.outcome_retention(), OutcomeRetention::Keep);
    let latest = keep
        .clone()
        .with_outcome_retention(OutcomeRetention::LatestSettlement);
    for (society, settled) in [(&keep, 2 * 3), (&latest, 3)] {
        let plugins: [&dyn SimulationPlugin; 2] = [society, &stock_plugin];
        let (mut canwu, stock) = monthly_society(&plugins);
        for month in 1..=2_i64 {
            for offset in 0..3 {
                canwu
                    .enqueue_plugin_ingress(rebase(
                        day(month * MONTH - 2),
                        1_000 + u64::try_from(month * 10 + offset).expect("headcount"),
                        &stock,
                    ))
                    .expect("rebase");
            }
            canwu
                .settle_boundary(boundary(day(month * MONTH - 2), false))
                .expect("the admitting boundary");
            canwu
                .settle_boundary(boundary(day(month * MONTH), true))
                .expect("the Monthly settlement");
        }
        let outcomes =
            ledger(canwu.typed_domain_record(&society_cohort_exchange_ledger_reference())).rebases;
        assert_eq!(outcomes.len(), settled);
        assert!(
            outcomes
                .values()
                .all(|outcome| outcome.status == SocietyIngressStatus::Applied)
        );
        assert_eq!(market(&canwu), 1_022);
        if *society == latest {
            assert!(
                outcomes
                    .values()
                    .all(|outcome| outcome.settled_at == day(2 * MONTH))
            );
            let snapshot = canwu.snapshot_json().expect("snapshot");
            let restored =
                Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
            assert_eq!(
                restored.snapshot_json().expect("restored snapshot"),
                snapshot
            );
            let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
                .expect("replayed run");
            assert_eq!(
                replayed.snapshot_json().expect("replayed snapshot"),
                snapshot
            );
            assert_eq!(
                Canwu::from_snapshot_json_with_plugins(&snapshot, &[&keep, &stock_plugin])
                    .err()
                    .expect("a Keep plugin cannot restore a LatestSettlement run")
                    .code,
                canwu_api::ErrorCode::PluginManifestMismatch
            );
        }
    }
}
