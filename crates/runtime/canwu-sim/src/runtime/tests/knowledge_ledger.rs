//! The committed knowledge ledger, its index of current records, its
//! retention, and the chunked knowledge commitment follow one contract on
//! every run path: live, forked, restored, sealed, and replayed runs, and a
//! boundary that fails after its records were committed.

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
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some(format!("lines-{}", context.boundary_id.get())),
                supersession: crate::KnowledgeSupersession::Explicit,
                groups: vec![crate::KnowledgePublicationGroup {
                    holders: vec![line_holder()],
                    records: visible,
                }],
                summary: "Publish the visible lines".to_owned(),
            },
            BoundaryDirective::PublishKnowledge {
                visibility: StateVisibility::NextBoundary,
                producer_correlation: Some(format!("deferred-{}", context.boundary_id.get())),
                supersession: crate::KnowledgeSupersession::Explicit,
                groups: vec![crate::KnowledgePublicationGroup {
                    holders: vec![line_holder()],
                    records: vec![line_draft(view, context, LINES)?],
                }],
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
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: crate::KnowledgeSupersession::Explicit,
            groups: vec![crate::KnowledgePublicationGroup {
                holders: vec![line_holder()],
                records: vec![line_draft(view, context, 0)?],
            }],
            summary: "Revise the first line".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Publishes every line each boundary, under the schema retention it
/// carries.
struct KnowledgeLinePlugin(KnowledgeRetention);

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
            retention: self.0,
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
/// equals the one a load recomputes from the snapshot. Under
/// `RetireWhenSuperseded` the holder holds nothing else: one entry and one
/// body per line, none of whose entries still names what it superseded.
fn assert_ledger(simulation: &Simulation, retention: KnowledgeRetention) {
    let knowledge = simulation.knowledge();
    let ledger = knowledge
        .for_holder(&line_holder())
        .expect("the fixture holder holds records");
    if retention == KnowledgeRetention::RetireWhenSuperseded {
        let lines = usize::try_from(LINES + 1).unwrap();
        assert_eq!(ledger.len(), lines);
        assert_eq!(knowledge.records.body_count(), lines);
        assert!(
            ledger
                .entries()
                .all(|(entry, _)| entry.supersedes.is_empty())
        );
    }
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

#[test]
fn knowledge_ledger_and_commitment_follow_every_run_path() {
    ledger_follows_every_run_path(KnowledgeRetention::Keep);
}

/// Retirement at publication is part of the boundary's state transition:
/// a run that retires what each publication supersedes keeps one entry per
/// line, keeps counting local IDs, and stays equal, boundary by boundary, to
/// its fork, its restore, its replay, and its sealed and restored-from-seal
/// twins, by checkpoint hash and by ledger.
#[test]
fn retirement_at_publication_follows_every_run_path() {
    ledger_follows_every_run_path(KnowledgeRetention::RetireWhenSuperseded);
}

#[allow(clippy::too_many_lines)]
fn ledger_follows_every_run_path(retention: KnowledgeRetention) {
    let (scenario, _) = demo_scenario();
    let plugin = KnowledgeLinePlugin(retention);
    let mut live = Simulation::new(71, scenario.clone()).expect("the fixture should load");
    live.register_plugin(&plugin)
        .expect("the line plugin should register");
    for index in 0..4 {
        live.settle_boundary(publishing_boundary(day(index)))
            .expect("a publishing boundary should settle");
        assert_ledger(&live, retention);
    }
    let ledger = live
        .knowledge()
        .for_holder(&line_holder())
        .expect("the fixture holder holds records");
    let published = 4 * (usize::try_from(LINES).unwrap() + 2);
    assert_eq!(
        ledger.len(),
        match retention {
            KnowledgeRetention::Keep => published,
            KnowledgeRetention::RetireWhenSuperseded => usize::try_from(LINES + 1).unwrap(),
        }
    );
    // Local IDs come from the holder's counter, which never goes back, and a
    // held entry keeps the local ID it was published with.
    assert_eq!(
        ledger.next_local().get(),
        u64::try_from(published).unwrap() + 1
    );
    assert!(
        ledger
            .entries()
            .all(|(entry, _)| ledger.id_of(entry.local) == Some(entry.record))
    );
    // The boundary evidence keeps what each entry superseded when it was
    // published, whatever retention cleared since.
    let revised = live.boundaries()[3]
        .knowledge_changes
        .iter()
        .find(|change| change.phase == BoundaryPhase::PerspectiveAndReportMaterialization)
        .expect("the last boundary revised the first line");
    assert_eq!(revised.holders[0].entries[0].supersedes.len(), 1);

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
    let root = live.knowledge().records.commitment_root();
    let revision = live.state.counters.state_revision;
    live.state.counters.state_revision = u64::MAX;
    let error = live
        .settle_boundary(publishing_boundary(day(5)))
        .expect_err("an exhausted revision should fail the boundary after it commits");
    assert_eq!(error.code, ErrorCode::IdentifierExhausted);
    live.state.counters.state_revision = revision;
    assert_eq!(live.snapshot(), before);
    assert!(Arc::ptr_eq(&ledger, &live.state.current.knowledge));
    assert_eq!(live.knowledge().records.commitment_root(), root);
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
            assert_ledger(run, retention);
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

fn shared_line_kind() -> KnowledgeRecordKind {
    KnowledgeRecordKind::new("fixture.shared_lines", "line")
}

fn shared_line_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(shared_line_kind(), 1)
}

pub(super) fn shared_line_holder(index: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(index))
}

/// The holders of boundary `boundary`'s shared line record: persons 1 and
/// 2, except that odd boundaries from the third on publish to person 1
/// alone, so person 2 keeps a body person 1 has retired.
fn shared_line_holders(boundary: u64) -> Vec<KnowledgeHolderRef> {
    if boundary >= 3 && boundary % 2 == 1 {
        vec![shared_line_holder(1)]
    } else {
        vec![shared_line_holder(1), shared_line_holder(2)]
    }
}

/// Phase 13: one `CurrentOnSubjects` record on one line for
/// [`shared_line_holders`], each entry superseding its holder's current
/// record on the line.
fn publish_shared_line(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: crate::KnowledgeSupersession::CurrentOnSubjects,
            groups: vec![crate::KnowledgePublicationGroup {
                holders: shared_line_holders(boundary),
                records: vec![KnowledgeRecordDraft {
                    schema: shared_line_schema(),
                    subjects: vec![line_subject(0)],
                    payload: serde_json::json!({ "boundary": boundary }),
                    as_of: None,
                    confidence_per_mille: 1_000,
                    origin: KnowledgeOrigin {
                        method: "fixture_shared_line".to_owned(),
                        evidence: Vec::new(),
                    },
                    supersedes: Vec::new(),
                    contradicts: Vec::new(),
                }],
            }],
            summary: "Publish the shared line".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Publishes one shared line record each Daily boundary, under the schema
/// retention it carries.
pub(super) struct SharedLinePlugin(pub(super) KnowledgeRetention);

impl SimulationPlugin for SharedLinePlugin {
    fn name(&self) -> &'static str {
        "fixture-knowledge-shared-lines"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000c1a");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: shared_line_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000000c1b"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "line".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::Event],
                required: true,
                multiple: false,
            }],
            retention: self.0,
        })?;
        let mut contract = BoundarySystemContract::new(
            "publish-shared-line",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_knowledge()];
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: shared_line_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, publish_shared_line)
    }
}

pub(super) fn shared_line_boundary(index: i64) -> BoundaryRequest {
    publishing_boundary(day(index))
}

/// Each tamper of a retiring ledger, written so that every commitment over
/// it is recomputed, is refused by the one check that owns it. After four
/// boundaries, each publishing record `b` at boundary `b`, both persons hold
/// record 4 alone: person 1 retired record 3 at boundary 4, and person 2
/// retired record 2, which it had kept while person 1 retired it at
/// boundary 3.
///
/// - P1: person 1's record 3 back, named by nobody: only the rebuilt
///   ledger differs.
/// - P2: P1, with record 4 still naming record 3: a retiring schema's entry
///   names what its publication retired.
/// - P3: person 1's local-ID counter one higher: only the rebuilt ledger
///   differs.
/// - P5: person 2's computed list at boundary 4 emptied in the evidence,
///   with the ledger keeping record 2 as that evidence implies: only the
///   recomputed list differs.
#[test]
#[allow(clippy::too_many_lines)]
fn a_tampered_retiring_ledger_is_refused_by_the_check_that_owns_it() {
    let (scenario, _) = demo_scenario();
    let plugin = SharedLinePlugin(KnowledgeRetention::RetireWhenSuperseded);
    let mut simulation = Simulation::new(73, scenario).expect("the fixture should load");
    simulation
        .register_plugin(&plugin)
        .expect("the shared line plugin should register");
    for index in 0..4 {
        simulation
            .settle_boundary(shared_line_boundary(index))
            .expect("a publishing boundary should settle");
    }
    let snapshot = simulation.snapshot();
    let (first, second) = (shared_line_holder(1), shared_line_holder(2));
    let record = |boundary: usize| {
        Arc::clone(&snapshot.boundaries[boundary].knowledge_changes[0].records[0])
    };
    // A holder's entry as its boundary published it, with the names its
    // retirement since cleared.
    let published = |boundary: usize, holder: &KnowledgeHolderRef| {
        let mut entry =
            snapshot.boundaries[boundary].knowledge_changes[0].entries_of(holder)[0].clone();
        entry.supersedes.clear();
        entry.contradicts.clear();
        entry
    };
    let (second_record, third, fourth) = (record(1), record(2), record(3));
    let ledger = &snapshot.knowledge.records;
    assert_eq!(ledger.body_count(), 1);
    for holder in [&first, &second] {
        let held = ledger.get(holder).expect("the holder is listed");
        assert_eq!(held.len(), 1);
        assert!(
            held.entry(&fourth.id)
                .expect("record 4 is held")
                .supersedes
                .is_empty()
        );
    }

    // Puts a retired entry back, with its body.
    let resurrect = |tampered: &mut SimulationSnapshot,
                     holder: &KnowledgeHolderRef,
                     body: &KnowledgeRecordBody,
                     entry: &HolderEntry| {
        let holder = serde_json::to_value(holder).expect("the holder encodes");
        let body = serde_json::to_value(body).expect("the body encodes");
        let entry = serde_json::to_value(entry).expect("the entry encodes");
        assert!(tamper_knowledge_ledger(tampered, |wire| {
            let bodies = wire["bodies"].as_array_mut().expect("bodies");
            let at = bodies
                .iter()
                .position(|stored| stored["id"].as_u64() > body["id"].as_u64())
                .unwrap_or(bodies.len());
            bodies.insert(at, body);
            let entries = wire["holders"]
                .as_array_mut()
                .expect("holders")
                .iter_mut()
                .find(|ledger| ledger["holder"] == holder)
                .expect("the holder is listed")["entries"]
                .as_array_mut()
                .expect("entries");
            let at = entries
                .iter()
                .position(|stored| stored["local"].as_u64() > entry["local"].as_u64())
                .unwrap_or(entries.len());
            entries.insert(at, entry);
        }));
    };
    let load = |mut tampered: SimulationSnapshot| {
        refresh_snapshot_commitments_and_checkpoint(&mut tampered);
        let head = tampered.boundaries.len() - 1;
        tampered.boundaries[head].state_hash = Some(
            snapshot_boundary_head_state_hash(&tampered)
                .expect("the tampered head should have a state commitment"),
        );
        rehash_tampered_snapshot(&mut tampered);
        Simulation::from_snapshot_with_plugins(tampered, &[&plugin])
    };
    let refuse = |label: &str, tampered: SimulationSnapshot, check: &str| {
        let error = load(tampered)
            .err()
            .unwrap_or_else(|| panic!("{label} unexpectedly loaded"));
        assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{label}");
        assert!(error.message.contains(check), "{label}: {}", error.message);
    };
    let rebuilt = "boundary knowledge changes do not reconstruct the persisted generic ledger";
    load(snapshot.clone()).expect("the untampered snapshot loads through the same rehash");

    let mut unnamed = snapshot.clone();
    resurrect(&mut unnamed, &first, &third, &published(2, &first));
    refuse("P1", unnamed.clone(), rebuilt);

    let mut named = unnamed;
    assert!(tamper_knowledge_entry(
        &mut named,
        &first,
        fourth.id,
        |entry| entry.supersedes = vec![third.id],
    ));
    refuse(
        "P2",
        named,
        "a knowledge entry of a retiring schema still names what its publication retired",
    );

    let mut counter = snapshot.clone();
    let holder = serde_json::to_value(&first).expect("the holder encodes");
    assert!(tamper_knowledge_ledger(&mut counter, |wire| {
        let ledger = wire["holders"]
            .as_array_mut()
            .expect("holders")
            .iter_mut()
            .find(|ledger| ledger["holder"] == holder)
            .expect("the holder is listed");
        let raised = ledger["next_local"].as_u64().expect("a counter") + 1;
        ledger["next_local"] = serde_json::json!(raised);
    }));
    refuse("P3", counter, rebuilt);

    let mut emptied = snapshot.clone();
    let evidence = emptied.boundaries[3].knowledge_changes[0]
        .holders
        .iter_mut()
        .find(|publication| publication.holder == second)
        .expect("person 2 received record 4");
    assert_eq!(evidence.entries[0].supersedes, vec![second_record.id]);
    evidence.entries[0].supersedes.clear();
    resurrect(
        &mut emptied,
        &second,
        &second_record,
        &published(1, &second),
    );
    refuse(
        "P5",
        emptied,
        "boundary knowledge supersession list is not the one its rule gives at the publisher's cut",
    );
}
