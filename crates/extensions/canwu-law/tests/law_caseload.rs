//! Aggregate caseload (downstream gap G-30): a host reports caseload per
//! jurisdiction and period, and the law admission system settles each report
//! into a conserved caseload record with its clearance rate. Refused reports
//! are recorded and never fail a boundary, a repeated period is idempotent,
//! records keep a bounded window of periods, and a sealed run decides as its
//! unsealed fork and its exact replay do.

use canwu_api::{
    Canwu, CompactedCanwu, DomainRecord, DomainRecordClass, DomainRecordLifecycle, ErrorCode,
    EvidenceRef, PluginIngressRequest, Scenario, SimTime,
};
use canwu_law::*;
use std::collections::BTreeMap;

const ORDER: &str = "realm-order";

fn definition() -> LegalDefinition {
    let mut definition = LegalDefinition::new("caseload-law");
    definition.orders.push(LegalOrderDefinition {
        id: ORDER.to_owned(),
        precedence_profile: "later-in-time".to_owned(),
    });
    definition
        .precedence_profiles
        .push(PrecedenceProfileDefinition {
            id: "later-in-time".to_owned(),
            ordered_bases: vec![ConflictResolutionBasis::Temporal],
        });
    for id in ["county-a", "county-b"] {
        definition.jurisdictions.push(LegalJurisdictionDefinition {
            id: id.to_owned(),
            relations: Vec::new(),
            metadata: BTreeMap::new(),
        });
    }
    let mut caseload = CaseloadProfileDefinition::new(["serious", "minor"]);
    caseload.review_levels = vec!["province".to_owned(), "prefecture".to_owned()];
    caseload.retained_periods = 2;
    caseload.max_reports_per_boundary = 5;
    definition.caseload = Some(caseload);
    definition
}

fn counts(entries: &[(&str, u64)]) -> BTreeMap<String, u64> {
    entries
        .iter()
        .map(|(key, count)| ((*key).to_owned(), *count))
        .collect()
}

fn report(jurisdiction: &str, start: i64, end: i64) -> CaseloadIntakeReport {
    CaseloadIntakeReport {
        jurisdiction: jurisdiction.to_owned(),
        period: CaseloadPeriod {
            start: SimTime::from_minutes(start),
            end: SimTime::from_minutes(end),
        },
        opening: None,
        intake_by_class: BTreeMap::new(),
        resolved_by_class: BTreeMap::new(),
        review_by_level: BTreeMap::new(),
        evidence: Vec::new(),
    }
}

fn review(entries: &[(&str, u64, u64)]) -> BTreeMap<String, CaseloadReviewFlow> {
    entries
        .iter()
        .map(|(level, entered, concluded)| {
            (
                (*level).to_owned(),
                CaseloadReviewFlow {
                    entered: *entered,
                    concluded: *concluded,
                },
            )
        })
        .collect()
}

/// County A's first period: it opens with five minor cases pending and one
/// case awaiting prefecture review.
fn county_a_first() -> CaseloadIntakeReport {
    let mut report = report("county-a", 0, 100);
    report.opening = Some(CaseloadStock {
        pending_by_class: counts(&[("minor", 5)]),
        review_queue_by_level: counts(&[("prefecture", 1)]),
    });
    // A zero count is the same as no count.
    report.intake_by_class = counts(&[("minor", 10), ("serious", 2)]);
    report.resolved_by_class = counts(&[("minor", 12), ("serious", 0)]);
    report.review_by_level = review(&[("prefecture", 2, 1)]);
    report
}

fn county_a_second() -> CaseloadIntakeReport {
    let mut report = report("county-a", 100, 200);
    report.intake_by_class = counts(&[("minor", 1)]);
    report.resolved_by_class = counts(&[("minor", 4), ("serious", 1)]);
    report.review_by_level = review(&[("prefecture", 0, 2), ("province", 1, 0)]);
    report
}

fn initial_records(plan: &CompiledLawPlan) -> Vec<DomainRecord> {
    LegalRuntime::new(plan)
        .to_record_drafts()
        .expect("encode empty legal shards")
        .into_iter()
        .map(|draft| DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
        .collect()
}

/// Queues one caseload batch for minute `at` and settles the boundary there.
macro_rules! settle_caseload {
    ($run:expr, $at:expr, $reports:expr) => {{
        let reports: Vec<CaseloadIntakeReport> = $reports;
        let receipt = $run
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                LAW_CASELOAD_INGRESS,
                SimTime::from_minutes($at),
                serde_json::json!({ "reports": reports }),
            ))
            .expect("queue caseload intake");
        let boundary = $run
            .step_canonical()
            .expect("a caseload boundary settles")
            .expect("caseload boundary");
        assert_eq!(boundary.settled_at, SimTime::from_minutes($at));
        receipt.ingress_id
    }};
}

fn refusal_codes<'a>(events: impl Iterator<Item = &'a canwu_api::SimEvent>) -> Vec<String> {
    events
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, LAW_CASELOAD_REJECTED_EVENT))
        })
        .map(|event| {
            event
                .summary
                .split("rejected: ")
                .nth(1)
                .and_then(|reason| reason.split(':').next())
                .expect("a refusal names its reason")
                .to_owned()
        })
        .collect()
}

fn caseload(run: &Canwu, jurisdiction: &str) -> LegalCaseloadState {
    let plan = compile_law(&definition()).expect("compile caseload plan");
    load_legal_caseload(run, &plan, jurisdiction)
        .expect("read caseload")
        .expect("caseload record")
}

/// Reports settle in order into conserved stocks; each refused report
/// changes nothing, is recorded with its reason, and leaves the boundary and
/// the reports beside it settling; the record keeps the profile's window of
/// periods; exact replay reproduces the run.
#[test]
#[allow(clippy::too_many_lines)]
fn aggregate_caseload_settles_conserves_and_refuses_without_failing_the_boundary() {
    let plan = compile_law(&definition()).expect("compile caseload plan");
    let mut run = Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(initial_records(&plan)),
        &[&LawPlugin],
    )
    .expect("law plugin");

    let mut county_b = report("county-b", 0, 100);
    county_b.intake_by_class = counts(&[("minor", 4)]);
    county_b.resolved_by_class = counts(&[("minor", 1)]);
    let mut conflict = county_a_first();
    conflict.intake_by_class = counts(&[("minor", 11), ("serious", 2)]);
    let at = SimTime::from_minutes(100);
    // A packet of six reports does not fit the budget of five on its own and
    // is refused whole; a later packet of the same boundary past the budget
    // refuses its reports one by one.
    let oversized = vec![report("county-a", 0, 100); 6];
    let mut packets = Vec::new();
    for reports in [
        oversized,
        vec![
            county_a_first(),
            county_b,
            report("county-z", 0, 100),
            // An exact repeat is already settled; other content conflicts.
            county_a_first(),
            conflict,
        ],
        vec![report("county-b", 100, 200)],
    ] {
        let receipt = run
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                LAW_CASELOAD_INGRESS,
                at,
                serde_json::json!({ "reports": reports }),
            ))
            .expect("queue caseload intake");
        packets.push(receipt.ingress_id);
    }
    let first = packets[1];
    let boundary = run
        .step_canonical()
        .expect("a caseload boundary settles")
        .expect("caseload boundary");
    assert_eq!(boundary.settled_at, at);
    assert_eq!(
        refusal_codes(run.events().iter()),
        [
            "caseload_jurisdiction_unknown",
            "caseload_period_settled",
            "caseload_period_conflict",
            "caseload_boundary_budget",
        ]
    );
    let packet_refusals = run
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, LAW_INGRESS_REJECTED_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect::<Vec<_>>();
    assert_eq!(packet_refusals.len(), 1, "{packet_refusals:?}");
    assert!(packet_refusals[0].contains("more reports than one boundary settles"));
    let a = caseload(&run, "county-a");
    assert_eq!(a.periods.len(), 1);
    let settled = &a.periods[0];
    // 5 + 10 - 12 minor and 2 serious pending; 1 + 2 - 1 awaiting review.
    assert_eq!(
        settled.pending_by_class,
        counts(&[("minor", 3), ("serious", 2)])
    );
    assert_eq!(
        settled.review_queue_by_level,
        BTreeMap::from([(
            "prefecture".to_owned(),
            CaseloadReviewQueue {
                entered: 2,
                concluded: 1,
                pending: 2,
            }
        )])
    );
    assert_eq!(settled.clearance_bp, Some(10_000));
    assert_eq!(settled.report, EvidenceRef::Ingress(first));
    assert_eq!(
        caseload(&run, "county-b").periods[0].clearance_bp,
        Some(2_500)
    );

    let mut overdrawn = county_a_second();
    overdrawn.resolved_by_class = counts(&[("minor", 5)]);
    let mut stale_opening = county_a_second();
    stale_opening.opening = Some(CaseloadStock {
        pending_by_class: counts(&[("minor", 2)]),
        review_queue_by_level: BTreeMap::new(),
    });
    let mut unknown_class = county_a_second();
    unknown_class.intake_by_class = counts(&[("treason", 1)]);
    let mut stated_opening = county_a_second();
    stated_opening.opening = Some(settled.closing_stock());
    settle_caseload!(
        run,
        200,
        vec![
            overdrawn,
            report("county-a", 150, 200),
            stale_opening,
            unknown_class,
            stated_opening,
        ]
    );
    // County A now carries one serious case and one case awaiting province
    // review into the period from minute 200.
    let mut unknown_level = report("county-a", 200, 250);
    unknown_level.review_by_level = review(&[("capital", 1, 0)]);
    let mut overdrawn_review = report("county-a", 200, 250);
    overdrawn_review.review_by_level = review(&[("province", 0, 2)]);
    let mut overflowing = report("county-a", 200, 250);
    overflowing.intake_by_class = counts(&[("serious", u64::MAX)]);
    let mut overcited = report("county-a", 200, 250);
    overcited.evidence = (1..=17)
        .map(|boundary| EvidenceRef::Boundary(canwu_api::BoundaryId::new(boundary)))
        .collect();
    settle_caseload!(
        run,
        260,
        vec![unknown_level, overdrawn_review, overflowing, overcited]
    );
    settle_caseload!(run, 300, vec![report("county-a", 200, 400)]);
    settle_caseload!(run, 400, vec![report("county-a", 200, 400)]);
    // With the evidence refusal of the sealed-run test, every refusal code.
    assert_eq!(
        refusal_codes(run.events().iter()).split_off(4),
        [
            "caseload_resolved_exceeds_pending",
            "caseload_period_not_next",
            "caseload_opening_mismatch",
            "caseload_class_unknown",
            "caseload_level_unknown",
            "caseload_review_exceeds_queue",
            "caseload_count_overflow",
            "caseload_evidence_budget",
            "caseload_period_invalid",
        ]
    );
    // Two periods are kept, in two slot records: the third overwrote the
    // first.
    let a = caseload(&run, "county-a");
    assert_eq!(a.settled_periods, 3);
    assert_eq!(
        a.periods
            .iter()
            .map(|period| (period.ordinal, period.period.start.as_minutes()))
            .collect::<Vec<_>>(),
        [(2, 100), (3, 200)]
    );
    let county_a_periods = run
        .domain_records()
        .filter(|record| {
            record.reference.kind.name == LAW_CASELOAD_PERIOD
                && record.reference.id.starts_with("caseload:county-a:")
        })
        .count();
    assert_eq!(county_a_periods, 2);
    let second = &a.periods[0];
    assert_eq!(second.pending_by_class, counts(&[("serious", 1)]));
    assert_eq!(
        second.review_queue_by_level,
        BTreeMap::from([
            (
                "prefecture".to_owned(),
                CaseloadReviewQueue {
                    entered: 0,
                    concluded: 2,
                    pending: 0,
                }
            ),
            (
                "province".to_owned(),
                CaseloadReviewQueue {
                    entered: 1,
                    concluded: 0,
                    pending: 1,
                }
            ),
        ])
    );
    assert_eq!(second.clearance_bp, Some(50_000));
    assert_eq!(a.periods[1].clearance_bp, None);
    assert_eq!(a.periods[1].closing_stock(), second.closing_stock());

    let replayed =
        Canwu::replay_from_journal(&[&LawPlugin], &run.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), run.snapshot());
}

/// A caseload record declares no evidence, so a run with caseload records
/// seals, and seals again after more reports; a sealed run, its unsealed
/// fork, and the sealed run's exact replay settle the same reports alike,
/// including a retry of a period settled before the seal whose evidence can
/// no longer be cited (already settled, not refused for its evidence) and a
/// new period citing a packet the seal may have dropped.
#[test]
fn caseload_decides_alike_in_sealed_and_unsealed_runs() {
    let plan = compile_law(&definition()).expect("compile caseload plan");
    let plugins: [&dyn canwu_api::SimulationPlugin; 1] = [&LawPlugin];
    let mut base = Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(initial_records(&plan)),
        &plugins,
    )
    .expect("law plugin");
    // County A's first report cites a packet its own boundary admits.
    let mut county_b = report("county-b", 0, 100);
    county_b.intake_by_class = counts(&[("minor", 4)]);
    let cited = base
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            LAW_CASELOAD_INGRESS,
            SimTime::from_minutes(100),
            serde_json::json!({ "reports": [county_b] }),
        ))
        .expect("queue the cited packet")
        .ingress_id;
    let mut citing_first = county_a_first();
    citing_first.evidence = vec![EvidenceRef::Ingress(cited)];
    let first = settle_caseload!(base, 100, vec![citing_first.clone()]);
    assert_eq!(caseload(&base, "county-a").periods[0].evidence.len(), 1);
    base.settle_boundary(canwu_api::BoundaryRequest::at(base.time()))
        .expect("a quiet boundary before sealing");
    let mut unsealed = base.fork();
    let mut sealed: CompactedCanwu = base.into_compacted().expect("enter compact mode");
    let segment = sealed
        .seal_evidence()
        .expect("caseload history seals")
        .expect("the settled history holds evidence");
    // Restoring validates every caseload record against the plan.
    let mut restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segment restores with its caseload record");

    let mut cites_old_packet = county_a_second();
    cites_old_packet.evidence = vec![EvidenceRef::Ingress(first)];
    let batch = vec![citing_first, cites_old_packet, county_a_second()];
    settle_caseload!(unsealed, 200, batch.clone());
    settle_caseload!(sealed, 200, batch.clone());
    settle_caseload!(restored, 200, batch);
    assert_eq!(
        refusal_codes(unsealed.events().iter()),
        ["caseload_period_settled", "caseload_evidence_unavailable"]
    );
    assert_eq!(caseload(&unsealed, "county-a").periods.len(), 2);
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(restored.checkpoint_hash(), unsealed.checkpoint_hash());

    // The run seals again with caseload records present, and its exact
    // replay equals the unsealed fork.
    let later = sealed
        .seal_evidence()
        .expect("caseload history seals again")
        .expect("the later history holds evidence");
    let journal = sealed
        .replay_journal_with_segments(vec![segment, later])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
}

/// Activation, which every restore runs, checks each caseload head and its
/// kept period records against the plan: a period record whose stocks do not
/// follow from the period before it, two period records in each other's
/// slots, and a period record without its head are each refused, while the
/// records a run settled activate.
#[test]
fn activation_refuses_forged_caseload_records() {
    let plan = compile_law(&definition()).expect("compile caseload plan");
    let mut run = Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(initial_records(&plan)),
        &[&LawPlugin],
    )
    .expect("law plugin");
    settle_caseload!(run, 100, vec![county_a_first()]);
    settle_caseload!(run, 200, vec![county_a_second()]);
    let settled = run
        .domain_records()
        .filter(|record| {
            [LAW_CASELOAD_HEAD, LAW_CASELOAD_PERIOD].contains(&record.reference.kind.name.as_str())
        })
        .cloned()
        .map(|mut record| {
            record.version = 1;
            record
        })
        .collect::<Vec<_>>();
    assert_eq!(settled.len(), 3);
    let activate = |caseload: Vec<DomainRecord>| {
        let records = initial_records(&plan).into_iter().chain(caseload).collect();
        Canwu::new_with_plugins(
            7,
            Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(records),
            &[&LawPlugin],
        )
        .map(|_| ())
    };
    activate(settled.clone()).expect("the settled caseload records activate");

    let mut broken_chain = settled.clone();
    broken_chain
        .iter_mut()
        .find(|record| record.reference.id == "caseload:county-a:1")
        .expect("the second period's slot")
        .payload["pending_by_class"]["serious"] = serde_json::json!(5);
    let mut swapped_slots = settled.clone();
    for record in &mut swapped_slots {
        record.reference.id = match record.reference.id.as_str() {
            "caseload:county-a:0" => "caseload:county-a:1".to_owned(),
            "caseload:county-a:1" => "caseload:county-a:0".to_owned(),
            other => other.to_owned(),
        };
    }
    let headless = settled
        .iter()
        .filter(|record| record.reference.kind.name == LAW_CASELOAD_PERIOD)
        .cloned()
        .collect::<Vec<_>>();
    for forged in [broken_chain, swapped_slots, headless] {
        assert_eq!(
            activate(forged)
                .expect_err("a forged caseload record must block activation")
                .code,
            ErrorCode::InvalidDomainRecord
        );
    }
}
