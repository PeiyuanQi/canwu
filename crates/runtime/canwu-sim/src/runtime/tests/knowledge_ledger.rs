//! The committed knowledge ledger, its index of current records, and the
//! incremental knowledge commitment follow one contract on every run path:
//! live, forked, restored, sealed, and replayed runs, and a boundary that
//! fails after its records were committed.

use super::*;
use std::sync::Arc;

const LINES: u64 = 3;

fn line_holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

fn line_kind() -> KnowledgeRecordKind {
    KnowledgeRecordKind::new("fixture.lines", "line")
}

fn line_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(line_kind(), 1)
}

fn line_subject(line: u64) -> KnowledgeSubject {
    KnowledgeSubject {
        role: "line".to_owned(),
        target: KnowledgeSubjectTarget::Event(EventId::new(line + 1)),
    }
}

/// A record on one line that supersedes whatever the line currently holds,
/// as a publisher finds it through `knowledge_heads`.
fn line_draft(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    line: u64,
) -> Result<KnowledgeRecordDraft, CanwuError> {
    let heads = view.knowledge_heads(&line_holder(), &line_kind(), &[line_subject(line)])?;
    Ok(KnowledgeRecordDraft {
        schema: line_schema(),
        subjects: vec![line_subject(line)],
        payload: serde_json::json!({
            "line": line,
            "boundary": context.boundary_id.get(),
            "phase": format!("{:?}", context.phase),
        }),
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "fixture_line".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: heads.records,
        contradicts: Vec::new(),
    })
}

/// Phase 4: a same-boundary record on every line but the last, and a
/// next-boundary record on the last line.
fn publish_lines(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let visible = (0..LINES)
        .map(|line| line_draft(view, context, line))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::PublishKnowledge {
                holder: line_holder(),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some(format!("lines-{}", context.boundary_id.get())),
                records: visible,
                summary: "Publish the visible lines".to_owned(),
            },
            BoundaryDirective::PublishKnowledge {
                holder: line_holder(),
                visibility: StateVisibility::NextBoundary,
                producer_correlation: Some(format!("deferred-{}", context.boundary_id.get())),
                records: vec![line_draft(view, context, LINES)?],
                summary: "Publish the deferred line".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

/// Phase 13: revises the first line, superseding the record phase 4 of the
/// same boundary published on it.
fn revise_first_line(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: line_holder(),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            records: vec![line_draft(view, context, 0)?],
            summary: "Revise the first line".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct KnowledgeLinePlugin;

impl SimulationPlugin for KnowledgeLinePlugin {
    fn name(&self) -> &'static str {
        "fixture-knowledge-lines"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000c16");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: line_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000000c17"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "line".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::Event],
                required: true,
                multiple: false,
            }],
        })?;
        let mut phase4 = BoundarySystemContract::new(
            "publish-lines",
            BoundaryPhase::PerceptionAndAttentionRefresh,
            SystemCadence::Daily,
        );
        phase4.reads = vec![StateKey::core_knowledge()];
        phase4.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: line_schema(),
            visibilities: vec![StateVisibility::SameBoundary, StateVisibility::NextBoundary],
        }];
        registrar.register_boundary_system(phase4, publish_lines)?;
        let mut phase13 = BoundarySystemContract::new(
            "revise-first-line",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        phase13.reads = vec![StateKey::core_knowledge()];
        phase13.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: line_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(phase13, revise_first_line)
    }
}

/// The current records of one line, from the committed ledger's index.
fn line_heads(knowledge: &KnowledgeSnapshot, line: u64) -> Vec<KnowledgeRecordId> {
    super::super::knowledge::current_heads_at_cut(
        &line_holder(),
        &line_kind(),
        &[line_subject(line)],
        &knowledge.records,
        None,
        KnowledgeLimitsV1::CURRENT.relations_per_record,
    )
    .records
}

/// Every line holds exactly its newest record, and the cached commitment
/// equals the one a load recomputes from the snapshot.
fn assert_ledger(simulation: &Simulation) {
    let knowledge = simulation.knowledge();
    let ledger = knowledge
        .for_holder(&line_holder())
        .expect("the fixture holder holds records");
    for line in 0..=LINES {
        let newest = ledger
            .values()
            .filter(|record| record.subjects == [line_subject(line)])
            .map(|record| record.id)
            .max()
            .expect("every line has a record");
        assert_eq!(line_heads(knowledge, line), vec![newest], "line {line}");
    }
    let snapshot = simulation.snapshot();
    assert_eq!(
        snapshot.commitment_roots,
        Some(
            snapshot_commitment_roots(&snapshot)
                .expect("the snapshot should reproduce its commitment roots")
        )
    );
}

fn publishing_boundary(at: SimTime) -> BoundaryRequest {
    BoundaryRequest::at(at).with_cadence(SystemCadence::Daily)
}

fn day(index: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(index)
}

fn knowledge_records_accumulator_end(simulation: &Simulation) -> Option<KnowledgeRecordId> {
    simulation
        .state
        .metadata
        .commitment_cache
        .as_ref()
        .and_then(|cache| cache.knowledge_records.as_ref())
        .and_then(super::super::state::KnowledgeRecordAccumulator::last)
}

#[test]
fn knowledge_ledger_and_commitment_follow_every_run_path() {
    let (scenario, _) = demo_scenario();
    let plugin = KnowledgeLinePlugin;
    let mut live = Simulation::new(71, scenario.clone()).expect("the fixture should load");
    live.register_plugin(&plugin)
        .expect("the line plugin should register");
    for index in 0..4 {
        live.settle_boundary(publishing_boundary(day(index)))
            .expect("a publishing boundary should settle");
        assert_ledger(&live);
    }
    let held = live
        .knowledge()
        .for_holder(&line_holder())
        .map_or(0, HolderLedger::len);
    assert_eq!(held, 4 * (usize::try_from(LINES).unwrap() + 2));

    // A boundary that publishes nothing neither copies nor rehashes the
    // knowledge it carries: the state is the same allocation afterwards, and
    // the cached knowledge root was not recomputed.
    let knowledge = Arc::clone(&live.state.current.knowledge);
    let root = live
        .snapshot()
        .commitment_roots
        .map(|roots| roots.knowledge);
    live.settle_boundary(BoundaryRequest::at(day(4)))
        .expect("an idle boundary should settle");
    assert!(Arc::ptr_eq(&knowledge, &live.state.current.knowledge));
    assert_eq!(
        live.snapshot()
            .commitment_roots
            .map(|roots| roots.knowledge),
        root
    );
    drop(knowledge);

    // A boundary that fails after it committed its records restores the
    // ledger, its index, and the commitment cache with the rest of the state.
    let control = live.fork();
    let before = live.snapshot();
    let ledger = Arc::clone(&live.state.current.knowledge);
    let accumulated = knowledge_records_accumulator_end(&live);
    assert_eq!(accumulated, live.knowledge().records.last_record_id());
    let revision = live.state.counters.state_revision;
    live.state.counters.state_revision = u64::MAX;
    let error = live
        .settle_boundary(publishing_boundary(day(5)))
        .expect_err("an exhausted revision should fail the boundary after it commits");
    assert_eq!(error.code, ErrorCode::IdentifierExhausted);
    live.state.counters.state_revision = revision;
    assert_eq!(live.snapshot(), before);
    assert!(Arc::ptr_eq(&ledger, &live.state.current.knowledge));
    assert_eq!(knowledge_records_accumulator_end(&live), accumulated);
    drop(ledger);

    // The same boundary then settles exactly as on a run that never failed,
    // and as on a run restored from its snapshot, a run that sealed its
    // evidence, a run restored from that seal's checkpoint and segment, and
    // a replay.
    let restored =
        Simulation::from_snapshot_json_with_plugins(&live.snapshot_json().unwrap(), &[&plugin])
            .expect("the run should restore");
    let mut replayed =
        Simulation::replay_from_journal_with_scenario(scenario, &[&plugin], &live.replay_journal())
            .expect("the run should replay exactly");
    let mut sealed = live
        .fork()
        .into_compacted()
        .expect("the run should compact");
    let segment = sealed
        .seal_evidence()
        .expect("the evidence should seal")
        .expect("there is evidence to seal");
    let from_sealed = CompactedSimulation::from_checkpoint_and_journal_with_plugins(
        sealed
            .checkpoint()
            .expect("the sealed run should checkpoint"),
        vec![segment],
        &[&plugin],
    )
    .expect("the sealed checkpoint and its segment should restore");
    let mut compacted = vec![sealed, from_sealed];
    let mut runs = vec![live, control, restored];
    for index in 5..7 {
        for run in &mut runs {
            run.settle_boundary(publishing_boundary(day(index)))
                .expect("every run should settle the boundary");
            assert_ledger(run);
        }
        replayed
            .settle_boundary(publishing_boundary(day(index)))
            .expect("the replayed run should continue");
        let expected = runs[0].checkpoint_hash().to_owned();
        for run in &runs[1..] {
            assert_eq!(run.checkpoint_hash(), expected);
            assert_eq!(run.knowledge(), runs[0].knowledge());
        }
        assert_eq!(replayed.checkpoint_hash(), expected);
        for run in &mut compacted {
            run.settle_boundary(publishing_boundary(day(index)))
                .expect("the sealed runs should continue");
            assert_eq!(run.checkpoint_hash(), expected);
            assert_eq!(run.knowledge(), runs[0].knowledge());
            for line in 0..=LINES {
                assert_eq!(
                    line_heads(run.knowledge(), line),
                    line_heads(runs[0].knowledge(), line)
                );
            }
        }
    }
}
