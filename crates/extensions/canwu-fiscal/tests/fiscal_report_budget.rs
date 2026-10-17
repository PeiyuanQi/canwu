//! The report system publishes against the engine's knowledge publication
//! budget, and the report schema retires superseded records.
//!
//! Fiscal publishes the least of its share of a boundary's budget
//! (`FiscalReportBudget`, half of the engine's by default) and what the
//! engine has left, one `Explicit` group per holder, packed into directives
//! up to the per-directive bounds; there is no count of publications or
//! holders per boundary. So every observer due at one instant, each owing one
//! dirty cycle, is published in one boundary, where 0.16.0 needed a drain for
//! every 64 observers and 1,024 records. The share is bounded and bound to
//! the run. Under the default retention (`RetireWhenSuperseded`) a holder
//! keeps one current record per report line, so held reports stay flat cycle
//! after cycle, in a sealed run as in its unsealed twin and its replay; under
//! `Keep` a closed cycle's figures stay readable through a full-history
//! query. A boundary in which the systems before the report system spent the
//! budget does none of the work owed and schedules no drain, so the instant
//! ends, and the work is published once a boundary has budget for it.

mod support;

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    BoundarySystemHandler, Canwu, CanwuError, CanwuRun, CompactedCanwu, ErrorCode, EventId,
    EvidenceJournalSegment, KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin,
    KnowledgePublicationGroup, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeRetention, KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema,
    KnowledgeSubjectTarget, KnowledgeSubjectTargetKind, KnowledgeSupersession, KnowledgeWriteGrant,
    PayloadSchema, PersonId, PluginKnowledgeSchema, PluginRegistrar, SimDuration, SimTime,
    SimulationGranularity, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence,
};
use canwu_fiscal::{
    FiscalAdoptionStage, FiscalAdoptionState, FiscalMechanism, FiscalObserverBinding,
    FiscalReportBudget, FiscalReportCadence, FiscalReportGranularity, FiscalReportRecord,
    FiscalScopeBinding, compute_projection, enqueue_fiscal_archive,
    finalize_fiscal_archive_retention, fiscal_report_knowledge_schema_id, fiscal_reports,
    prepare_fiscal_archive,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, DEPUTY, Fixture, GRANARY_PRINCIPAL, MemoryArchiveStore, OBSERVER, PRINCIPAL, RULE,
    SECOND_AUDITOR, SUCCESSOR, TREASURY_AUTHORITY, TREASURY_SCOPES, assessment, close,
    enqueue_tick, fiscal_state, granary, held_records, observer_binding, published_report_records,
    scenario, settle, submit, submit_sealed, treasury, treasury_scope,
};

/// Extra treasury scopes of the gate in miniature.
const EXTRA_SCOPES: usize = 316;
/// Scope observers of the treasury, all held by the principal.
const SCOPE_OBSERVERS: usize = 100;
/// Institution observers of both institutions, held by the auditor.
const COURTS: usize = 20;

fn extra_scope(index: usize) -> String {
    format!("scope.treasury.x.{index:03}")
}

/// Every operational treasury scope.
fn scopes() -> Vec<String> {
    (0..TREASURY_SCOPES)
        .map(treasury_scope)
        .chain((0..EXTRA_SCOPES).map(extra_scope))
        .collect()
}

fn monthly() -> FiscalReportCadence {
    FiscalReportCadence::Interval {
        every: SimDuration::days(30),
        offset: SimDuration::ZERO,
    }
}

/// The gate case in miniature: 120 observers, all due at the same instants,
/// a hundred of them over 320 treasury scopes (21 parts each) and twenty at
/// institution granularity.
fn gate_scenario() -> canwu_api::Scenario {
    scenario(|state| {
        for index in 0..EXTRA_SCOPES {
            let id = extra_scope(index);
            state.add_scope(FiscalScopeBinding {
                id: id.clone(),
                institution: treasury(),
                jurisdiction_id: "region.a".to_owned(),
                subject_scope: "registered_land".to_owned(),
                mechanism: FiscalMechanism::LandTax,
                authoritative_granularity: SimulationGranularity::Aggregate,
            });
            state
                .add_adoption(FiscalAdoptionState {
                    id: format!("adopt.x.{index:03}"),
                    rule_id: RULE.to_owned(),
                    scope_binding_id: id,
                    stage: FiscalAdoptionStage::Implemented,
                    generation: 1,
                    changed_at: SimTime::EPOCH,
                    source_action_id: None,
                })
                .expect("an adoption of a new scope");
        }
        state.remove_observer(OBSERVER);
        for index in 0..SCOPE_OBSERVERS {
            state.add_observer(FiscalObserverBinding {
                cadence: monthly(),
                ..observer_binding(
                    &format!("observer.scope.{index:03}"),
                    PRINCIPAL,
                    BTreeSet::from([treasury()]),
                )
            });
        }
        for index in 0..COURTS {
            state.add_observer(FiscalObserverBinding {
                cadence: monthly(),
                granularity: FiscalReportGranularity::Institution,
                ..observer_binding(
                    &format!("observer.court.{index:03}"),
                    AUDITOR,
                    BTreeSet::from([treasury(), granary()]),
                )
            });
        }
    })
}

/// Every observer due at one instant, each owing one dirty cycle, is
/// published in one boundary under the default share: about 2,200 records
/// to 120 observers, where 0.16.0 published 64 observers and 1,024 records a
/// boundary and drained the rest. No drain is scheduled, and every report is
/// complete and is what the records derive at that boundary.
#[test]
fn every_observer_due_at_one_instant_publishes_in_one_boundary() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(gate_scenario());
    // Every scope assessed at the start: nothing is due, nothing published.
    let batch = scopes()
        .iter()
        .map(|scope| {
            (
                PRINCIPAL,
                assessment(
                    &format!("assess.{scope}"),
                    TREASURY_AUTHORITY,
                    scope,
                    "1644",
                ),
            )
        })
        .collect();
    support::enqueue_batches(&mut canwu, batch);
    settle(&mut canwu);
    assert!(published_report_records(&canwu, PRINCIPAL).is_empty());

    // Thirty days on, everyone is due.
    let due = SimTime::EPOCH + SimDuration::days(30);
    canwu
        .advance_canonical(due.checked_sub(canwu.time()).expect("later"))
        .expect("the clock moves");
    enqueue_tick(&mut canwu);
    let receipt = canwu
        .step_canonical()
        .expect("the due boundary")
        .expect("the tick opens a boundary");
    assert!(receipt.generated_ingress.is_empty(), "no drain");
    assert!(
        canwu.step_canonical().expect("nothing more").is_none(),
        "one boundary at the due instant"
    );
    // The treasury's rows: its operational scopes and its pending one.
    let parts = (scopes().len() + 1).div_ceil(16);
    assert_eq!(
        receipt.knowledge_record_count,
        SCOPE_OBSERVERS * (1 + parts) + COURTS * 2
    );
    assert!(receipt.knowledge_record_count > 1_024);
    let state = fiscal_state(&canwu);
    for (holder, count) in [(PRINCIPAL, SCOPE_OBSERVERS), (AUDITOR, COURTS)] {
        let viewer = canwu.viewer_for_actor(holder).expect("viewer");
        let reports = fiscal_reports(&viewer).expect("reports");
        assert_eq!(reports.len(), count);
        for (observer, report) in reports {
            assert!(report.is_complete(), "{observer}");
            assert_eq!(report.projection.as_of, due);
            let derived = compute_projection(&state, &observer, due)
                .expect("projection")
                .expect("an observer");
            assert_eq!(report.projection.facts, derived.facts, "{observer}");
        }
    }
}

/// A share outside its bounds is refused when the plugin registers, and the
/// share is bound to the run: a run restores only with the share it was
/// created with.
#[test]
fn a_report_share_is_bounded_and_bound_to_its_run() {
    let refused = |budget: FiscalReportBudget| {
        Fixture::with_report_budget(budget)
            .try_runtime(scenario(|_| {}))
            .err()
            .expect("a share outside its bounds is refused")
            .code
    };
    let limits = canwu_api::KnowledgeLimitsV1::CURRENT;
    for budget in [
        FiscalReportBudget {
            bodies: limits.records_per_boundary + 1,
            ..FiscalReportBudget::DEFAULT
        },
        FiscalReportBudget {
            payload_bytes: limits.payload_bytes_per_boundary + 1,
            ..FiscalReportBudget::DEFAULT
        },
        FiscalReportBudget {
            payload_bytes: FiscalReportBudget::MINIMUM.payload_bytes - 1,
            ..FiscalReportBudget::DEFAULT
        },
        FiscalReportBudget {
            entries: 1,
            ..FiscalReportBudget::DEFAULT
        },
    ] {
        assert_eq!(refused(budget), ErrorCode::InvalidPluginRegistration);
    }
    let small = Fixture::with_report_budget(FiscalReportBudget::MINIMUM);
    let canwu = small.runtime(scenario(|_| {}));
    let snapshot = canwu.snapshot_json().expect("snapshot");
    assert!(Canwu::from_snapshot_json_with_plugins(&snapshot, &small.plugins()).is_ok());
    assert!(Canwu::from_snapshot_json_with_plugins(&snapshot, &Fixture::new().plugins()).is_err());
}

/// Archives every closed book through either run handle, settling with
/// `settle`.
fn archive_all<H: CanwuRun + ?Sized>(
    run: &mut H,
    store: &MemoryArchiveStore,
    settle: impl Fn(&mut H),
) {
    let prepared = prepare_fiscal_archive(run, 64, store).expect("archive batch");
    let receipt = enqueue_fiscal_archive(run, &prepared, store).expect("archive queued");
    settle(run);
    finalize_fiscal_archive_retention(run, store, &receipt).expect("retention finalized");
    settle(run);
}

/// One cycle's drafts: an assessment of every treasury scope, and then the
/// close of each book.
fn cycle_drafts(cycle: usize) -> [Vec<(PersonId, support::Draft)>; 2] {
    let id = format!("c{cycle:02}");
    let scopes: Vec<String> = (0..TREASURY_SCOPES).map(treasury_scope).collect();
    [
        scopes
            .iter()
            .map(|scope| {
                (
                    PRINCIPAL,
                    assessment(
                        &format!("assess.{id}.{scope}"),
                        TREASURY_AUTHORITY,
                        scope,
                        &id,
                    ),
                )
            })
            .collect(),
        scopes
            .iter()
            .map(|scope| (PRINCIPAL, close(&format!("close.{id}.{scope}"), scope, &id)))
            .collect(),
    ]
}

/// What the observer's holder holds of the report schema.
fn held(knowledge: &canwu_api::KnowledgeSnapshot) -> usize {
    held_records(knowledge, &KnowledgeHolderRef::Person(PRINCIPAL)).map_or(0, |records| {
        records
            .iter()
            .filter(|record| record.schema == fiscal_report_knowledge_schema_id())
            .count()
    })
}

/// Six cycles, each assessed, closed, and archived: under the default
/// retention the holder holds the summary and one part after every cycle,
/// the archived cycle's figures superseded by its emptied part and that by
/// the next cycle's, while a run that keeps every record holds more after
/// each. A twin that seals its evidence after every cycle holds and hashes
/// the same, and replays exactly.
#[test]
fn retention_keeps_held_reports_flat_across_cycles() {
    const CYCLES: usize = 6;
    let fixture = Fixture::new();
    let keeping = Fixture::with_report_retention(KnowledgeRetention::Keep);
    let mut canwu = fixture.runtime(scenario(|_| {}));
    let mut kept = keeping.runtime(scenario(|_| {}));
    let mut sealed = canwu.fork().into_compacted().expect("compact interface");
    let (store, kept_store, sealed_store) = (
        MemoryArchiveStore::default(),
        MemoryArchiveStore::default(),
        MemoryArchiveStore::default(),
    );
    let mut segments: Vec<EvidenceJournalSegment> = Vec::new();
    let mut held_after = Vec::new();
    let mut kept_after = Vec::new();
    for cycle in 0..CYCLES {
        for drafts in cycle_drafts(cycle) {
            submit(&mut canwu, drafts.clone());
            submit(&mut kept, drafts.clone());
            submit_sealed(&mut sealed, drafts);
        }
        archive_all(&mut canwu, &store, |run| {
            settle(run);
        });
        archive_all(&mut kept, &kept_store, |run| {
            settle(run);
        });
        archive_all(&mut sealed, &sealed_store, |run: &mut CompactedCanwu| {
            run.advance_canonical(SimDuration::minutes(1))
                .expect("sealed boundaries");
        });
        segments.extend(sealed.seal_evidence().expect("a seal"));
        assert_eq!(sealed.checkpoint_hash(), canwu.checkpoint_hash());
        assert_eq!(held(sealed.knowledge()), held(canwu.knowledge()));
        held_after.push(held(canwu.knowledge()));
        kept_after.push(held(kept.knowledge()));
    }
    assert_eq!(held_after, vec![2; CYCLES], "flat");
    assert!(
        kept_after.windows(2).all(|pair| pair[1] > pair[0]),
        "{kept_after:?}"
    );
    // The holder still holds a complete report, of no cycle now.
    let report = support::fiscal_report(&canwu, PRINCIPAL);
    assert!(report.is_complete() && report.projection.facts.is_empty());
    let journal = sealed
        .replay_journal_with_segments(segments)
        .expect("the sealed run produces an exact replay journal");
    let replayed = Canwu::replay_from_journal(&fixture.plugins(), &journal)
        .expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// Under `Keep`, a closed and archived cycle's figures stay readable through
/// a full-history query after the emptied parts supersede them; under the
/// default they leave the holder, and a full-history query shows only the
/// current records. The query says which retention applies.
#[test]
fn keep_keeps_closed_cycle_reports_readable() {
    let facts_in_history = |fixture: &Fixture| {
        let mut canwu = fixture.runtime(scenario(|_| {}));
        let store = MemoryArchiveStore::default();
        for drafts in cycle_drafts(0) {
            submit(&mut canwu, drafts);
        }
        archive_all(&mut canwu, &store, |run| {
            settle(run);
        });
        let viewer = canwu.viewer_for_actor(PRINCIPAL).expect("viewer");
        let page = viewer
            .query_knowledge(&KnowledgeQuery {
                schemas: vec![fiscal_report_knowledge_schema_id()],
                view: KnowledgeHistoryView::FullHistory,
                limit: canwu_api::MAX_KNOWLEDGE_PAGE_SIZE,
                ..KnowledgeQuery::default()
            })
            .expect("history");
        let retention = page.retention[&fiscal_report_knowledge_schema_id()];
        let facts: usize = page
            .records
            .iter()
            .filter_map(|record| {
                match serde_json::from_value(record.payload.clone()).expect("a report record") {
                    FiscalReportRecord::Part(part) => Some(part.rows.len()),
                    FiscalReportRecord::Summary(_) => None,
                }
            })
            .sum();
        (retention, facts)
    };
    assert_eq!(
        facts_in_history(&Fixture::with_report_retention(KnowledgeRetention::Keep)),
        (KnowledgeRetention::Keep, TREASURY_SCOPES)
    );
    assert_eq!(
        facts_in_history(&Fixture::new()),
        (KnowledgeRetention::RetireWhenSuperseded, 0)
    );
}

/// A plugin whose name sorts before fiscal's, so its system of the report
/// phase (13) spends the boundary's publication budget first. In every
/// event-bearing boundary before [`hog_until`] it publishes noise to four
/// persons other than the observer's holder until every holder entry is
/// spent, or all but one (`leave_one`), so that a report would fit its
/// summary and no part.
struct BudgetHog {
    leave_one: bool,
}

const HOG_PLUGIN: &str = "canwu-budget-hog";

fn hog_until() -> SimTime {
    SimTime::EPOCH + SimDuration::days(1)
}

fn noise_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new("fixture.hog", "noise"), 1)
}

fn noise(index: usize) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema: noise_schema(),
        subjects: vec![KnowledgeSubject {
            role: "noise".to_owned(),
            target: KnowledgeSubjectTarget::Event(EventId::new(
                u64::try_from(index).unwrap_or(0) + 1,
            )),
        }],
        payload: serde_json::json!(index),
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "fixture.hog".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

/// Spends the boundary's holder entries but `LEAVE`, before [`hog_until`].
/// (A boundary system handler returns a `Result`.)
#[allow(clippy::unnecessary_wraps)]
fn spend_the_budget<const LEAVE: usize>(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.at >= hog_until() {
        return Ok(BoundaryProposal::default());
    }
    let budget = view.knowledge_publication_budget();
    let wide: Vec<KnowledgeHolderRef> = [DEPUTY, SUCCESSOR, GRANARY_PRINCIPAL, SECOND_AUDITOR]
        .into_iter()
        .map(KnowledgeHolderRef::Person)
        .collect();
    let mut left = budget.holder_entries.saturating_sub(LEAVE);
    let mut index = 0;
    let mut directives = Vec::new();
    while left > 0 {
        let holders = if left >= wide.len() {
            wide.clone()
        } else {
            wide[..1].to_vec()
        };
        let count = (left / holders.len())
            .min(budget.records_per_batch)
            .min(budget.holder_entries_per_batch / holders.len());
        left -= count * holders.len();
        let records = (index..index + count).map(noise).collect();
        index += count;
        directives.push(BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: KnowledgeSupersession::CurrentOnSubjects,
            groups: vec![KnowledgePublicationGroup { holders, records }],
            summary: "Spend the publication budget".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for BudgetHog {
    fn name(&self) -> &'static str {
        HOG_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000e5"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: noise_schema(),
            schema_hash: "00000000000000000000000000000000000000000000000000000000000000e6"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "noise".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::Event],
                required: true,
                multiple: false,
            }],
            retention: KnowledgeRetention::Keep,
        })?;
        let mut contract = BoundarySystemContract::new(
            "spend-the-budget",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_knowledge()];
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: noise_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        let handler: BoundarySystemHandler = if self.leave_one {
            spend_the_budget::<1>
        } else {
            spend_the_budget::<0>
        };
        registrar.register_boundary_system(contract, handler)
    }
}

/// When a system before the report system spends the boundary's publication
/// budget, every holder entry or all but one (room for a summary and no
/// part), the report system does none of the work owed and schedules no
/// drain: the instant ends with the boundary that settles the assessments,
/// the work stays queued and due, and nothing is published. The next
/// boundary that runs the report system with the budget free publishes the
/// whole report, without a drain, and restore and replay agree.
#[test]
fn a_boundary_without_budget_left_schedules_no_drain() {
    for leave_one in [false, true] {
        let fixture = Fixture::new();
        let hog = BudgetHog { leave_one };
        let plugins = fixture.plugins_with(&hog);
        let mut canwu =
            Canwu::new_with_plugins(41, scenario(|_| {}), &plugins).expect("a run with the hog");
        let drafts = (0..TREASURY_SCOPES)
            .map(|index| {
                let scope = treasury_scope(index);
                (
                    PRINCIPAL,
                    assessment(
                        &format!("assess.{scope}"),
                        TREASURY_AUTHORITY,
                        &scope,
                        "1644",
                    ),
                )
            })
            .collect();
        support::enqueue(&mut canwu, drafts);
        let start = canwu.time();
        let mut boundaries = 0;
        while let Some(receipt) = canwu.step_canonical().expect("every boundary commits") {
            boundaries += 1;
            assert!(boundaries <= 4, "the instant ends (leave one: {leave_one})");
            assert!(receipt.knowledge_entry_count > 0, "the hog spends");
        }
        assert_eq!(canwu.time(), start);
        assert!(published_report_records(&canwu, PRINCIPAL).is_empty());
        let state = fiscal_state(&canwu);
        assert!(state.report_queue.contains_key(OBSERVER), "still owed");
        assert!(state.root.earliest_due.is_some_and(|due| due <= start));

        // A day on the hog spends nothing, and the next boundary that runs
        // the report system publishes the whole report, without a drain.
        canwu
            .advance_canonical(hog_until().checked_sub(start).expect("later"))
            .expect("the clock moves");
        enqueue_tick(&mut canwu);
        let receipt = canwu
            .step_canonical()
            .expect("the boundary a day on")
            .expect("the tick opens a boundary");
        assert!(receipt.generated_ingress.is_empty(), "no drain");
        assert!(canwu.step_canonical().expect("nothing more").is_none());
        let report = support::fiscal_report(&canwu, PRINCIPAL);
        assert!(report.is_complete());
        let derived = compute_projection(&fiscal_state(&canwu), OBSERVER, report.projection.as_of)
            .expect("projection")
            .expect("the observer's projection");
        assert_eq!(report.projection, derived);
        assert!(fiscal_state(&canwu).report_queue.is_empty());
        fixture.assert_restore_and_replay_with(&canwu, &hog);
    }
}
