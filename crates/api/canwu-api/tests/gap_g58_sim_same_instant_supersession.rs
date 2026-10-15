//! Gap G-58: a holder's record may supersede one the holder learned at the
//! same instant in an earlier boundary.
//!
//! Settlement admits such a publication and the run continues, so every load
//! must accept the saved run: the snapshot, the checkpoint journal, the paged
//! checkpoint, sealed segments, and exact replay. A record of the settling
//! boundary that is not visible at the publisher's cut is still refused, by
//! settlement and by a load, and so is a relation that points at a later
//! boundary.
//!
//! The publishing systems find the records to supersede through
//! `SimulationView::knowledge_heads`, which reads committed state. A run that
//! sealed its evidence therefore supersedes the same records as an unsealed
//! fork, a restore from the sealed segments, and exact replay.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    ArchiveStoreOutcome, BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal,
    BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, CompactedCanwu, EntityRef,
    ErrorCode, EvidenceCursor, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeQuery,
    KnowledgeRecord, KnowledgeRecordDraft, KnowledgeRecordId, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget,
    KnowledgeSubjectTargetKind, KnowledgeWriteGrant, OrganizationId, PayloadSchema, PersonId,
    PluginKnowledgeSchema, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationSnapshot, SimulationView, StateKey, StatePageBlob, StatePageProvider, StatePageStore,
    StateVisibility, SystemCadence,
};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;

const PLUGIN: &str = "fixture-report";
const NAMESPACE: &str = "fixture.report";

const fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

/// The subject of the daily reports.
const fn treasury() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(10))
}

/// The subject of sightings, whispers, and their digests.
const fn frontier() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(11))
}

fn report_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(NAMESPACE, "standing"), 1)
}

fn about(subject: EntityRef) -> KnowledgeSubject {
    KnowledgeSubject {
        role: "about".to_owned(),
        target: KnowledgeSubjectTarget::Entity(subject),
    }
}

/// The holder's current records about `subject` at the reader's cut, read
/// from committed state and the boundary being settled. A publisher
/// supersedes whatever the read returns: were the result truncated, the older
/// records would stay current until a later publication.
fn heads(
    view: &SimulationView<'_>,
    subject: EntityRef,
) -> Result<Vec<KnowledgeRecordId>, CanwuError> {
    Ok(view
        .knowledge_heads(&holder(), &report_schema().kind, &[about(subject)])?
        .records)
}

fn publish(
    context: &BoundaryContext,
    label: &str,
    subject: EntityRef,
    visibility: StateVisibility,
    supersedes: Vec<KnowledgeRecordId>,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: holder(),
            visibility,
            producer_correlation: None,
            records: vec![KnowledgeRecordDraft {
                schema: report_schema(),
                subjects: vec![about(subject)],
                payload: json!({ "source": label, "boundary": context.boundary_id.get() }),
                as_of: Some(context.at),
                confidence_per_mille: 1_000,
                origin: KnowledgeOrigin {
                    method: "fixture.report".to_owned(),
                    evidence: Vec::new(),
                },
                supersedes,
                contradicts: Vec::new(),
            }],
            summary: format!("The holder received a {label}"),
        }],
        ..BoundaryProposal::default()
    })
}

/// Phase 13, every `Daily` boundary: one report that supersedes the holder's
/// current reports, whichever boundary and instant they were learned in.
fn report(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    publish(
        context,
        "report",
        treasury(),
        StateVisibility::SameBoundary,
        heads(view, treasury())?,
    )
}

/// Phase 4, `Monthly`: a record that phase 13 of the same boundary may read.
fn sighting(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    publish(
        context,
        "sighting",
        frontier(),
        StateVisibility::SameBoundary,
        Vec::new(),
    )
}

/// Phase 4, `Seasonal` and `Annual`: a record that becomes visible at the
/// next boundary.
fn whisper(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    publish(
        context,
        "whisper",
        frontier(),
        StateVisibility::NextBoundary,
        Vec::new(),
    )
}

/// Phase 13, `Monthly` and `Seasonal`: supersedes every record about the
/// frontier that is current and visible at its cut.
fn digest(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    publish(
        context,
        "digest",
        frontier(),
        StateVisibility::SameBoundary,
        heads(view, frontier())?,
    )
}

/// Phase 13, `Annual`: names the whisper its own boundary published for the
/// next boundary. Record IDs are allocated in staging order after every
/// committed record, so the hidden record has the next ID.
fn eager_digest(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let visible = view.knowledge_record_count_in_namespace(NAMESPACE)?;
    let hidden = KnowledgeRecordId::new(u64::try_from(visible).expect("small fixture ledger") + 1);
    publish(
        context,
        "eager digest",
        frontier(),
        StateVisibility::SameBoundary,
        vec![hidden],
    )
}

struct ReportPlugin;

impl SimulationPlugin for ReportPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000005801"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: report_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000005802"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "about".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
        })?;
        let (early, late) = (
            BoundaryPhase::PerceptionAndAttentionRefresh,
            BoundaryPhase::PerspectiveAndReportMaterialization,
        );
        for (name, phase, cadence, handler) in [
            (
                "report",
                late,
                SystemCadence::Daily,
                report as canwu_api::BoundarySystemHandler,
            ),
            ("sighting", early, SystemCadence::Monthly, sighting),
            ("digest", late, SystemCadence::Monthly, digest),
            ("whisper", early, SystemCadence::Seasonal, whisper),
            ("seasonal-digest", late, SystemCadence::Seasonal, digest),
            ("annual-whisper", early, SystemCadence::Annual, whisper),
            ("eager-digest", late, SystemCadence::Annual, eager_digest),
        ] {
            let mut contract = BoundarySystemContract::new(name, phase, cadence);
            contract.reads = vec![StateKey::core_knowledge()];
            contract.knowledge_writes = vec![KnowledgeWriteGrant {
                schema: report_schema(),
                visibilities: vec![StateVisibility::SameBoundary, StateVisibility::NextBoundary],
            }];
            registrar.register_boundary_system(contract, handler)?;
        }
        Ok(())
    }
}

/// A content-addressed page store, as a host backs it.
#[derive(Default)]
struct PageStore(RefCell<BTreeMap<String, StatePageBlob>>);

impl StatePageProvider for PageStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl StatePageStore for PageStore {
    fn store_state_page(&self, page: &StatePageBlob) -> Result<ArchiveStoreOutcome, CanwuError> {
        page.validate()?;
        Ok(
            match self
                .0
                .borrow_mut()
                .insert(page.page_id.clone(), page.clone())
            {
                Some(_) => ArchiveStoreOutcome::AlreadyPresent,
                None => ArchiveStoreOutcome::Stored,
            },
        )
    }
}

fn at_day(day: i64, cadence: SystemCadence) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day)).with_cadence(cadence)
}

fn load(snapshot: &SimulationSnapshot, plugin: &ReportPlugin) -> Result<Canwu, CanwuError> {
    Canwu::from_snapshot_json_with_plugins(
        &serde_json::to_string(snapshot).expect("snapshot json"),
        &[plugin],
    )
}

/// The record a boundary's `index`-th knowledge change published.
fn published(canwu: &Canwu, boundary: usize, index: usize) -> KnowledgeRecord {
    canwu.boundaries()[boundary].knowledge_changes[index].records[0].clone()
}

#[test]
fn gap_g58_sim_same_instant_supersession() {
    let plugin = ReportPlugin;
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![treasury(), frontier(), EntityRef::Person(PersonId::new(1))],
    );
    let mut canwu = Canwu::new_with_plugins(5858, scenario, &[&plugin]).expect("report run");

    // Two boundaries at one instant: the second report supersedes the first,
    // which the holder learned at the same time in the earlier boundary.
    canwu
        .settle_boundary(at_day(1, SystemCadence::Daily))
        .expect("first report");
    canwu
        .settle_boundary(at_day(1, SystemCadence::Daily))
        .expect("a second boundary at the same instant supersedes the first report");
    let (first, second) = (published(&canwu, 0, 0), published(&canwu, 1, 0));
    assert_eq!(first.learned_at, second.learned_at);
    assert_eq!(second.supersedes, vec![first.id]);
    let current = canwu
        .admin_query_knowledge(holder(), &KnowledgeQuery::default())
        .expect("holder heads");
    assert_eq!(current.records.len(), 1);
    assert_eq!(current.records[0].payload["boundary"], json!(2));

    // Within one boundary, phase 13 may supersede a phase-4 record published
    // for the same boundary, and the read returns it.
    canwu
        .settle_boundary(at_day(1, SystemCadence::Monthly))
        .expect("a visible same-boundary record is a valid target");
    let (sighted, digested) = (published(&canwu, 2, 0), published(&canwu, 2, 1));
    assert_eq!(digested.supersedes, vec![sighted.id]);

    // A phase-4 record published for the next boundary is not visible to
    // phase 13 of its own boundary: the read leaves it out, so the digest
    // supersedes only the earlier digest.
    canwu
        .settle_boundary(at_day(1, SystemCadence::Seasonal))
        .expect("a hidden same-boundary record is not a current head yet");
    let (whispered, redigested) = (published(&canwu, 3, 0), published(&canwu, 3, 1));
    assert_eq!(redigested.supersedes, vec![digested.id]);

    // A system that names the hidden record anyway is refused by settlement,
    // which commits nothing.
    let before = (canwu.revision(), canwu.checkpoint_hash().to_owned());
    let hidden = canwu
        .settle_boundary(at_day(1, SystemCadence::Annual))
        .expect_err("a hidden same-boundary record is not a valid target");
    assert_eq!(hidden.code, ErrorCode::KnowledgeRecordNotFound);
    assert_eq!(
        (canwu.revision(), canwu.checkpoint_hash().to_owned()),
        before
    );
    assert_eq!(canwu.boundaries().len(), 4);

    // The run continues: the next day's report supersedes the latest one.
    canwu
        .settle_boundary(at_day(2, SystemCadence::Daily))
        .expect("the run continues past the same-instant supersession");
    let latest = published(&canwu, 4, 0);
    assert_eq!(latest.supersedes, vec![second.id]);

    // Every load accepts the saved run and reproduces it.
    let snapshot = canwu.snapshot();
    let mut restored = load(&snapshot, &plugin).expect("snapshot restore");
    assert_eq!(restored.snapshot(), snapshot);

    let bundle = canwu.checkpoint_journal_json().expect("checkpoint journal");
    let from_bundle = Canwu::from_checkpoint_journal_json_with_plugins(&bundle, &[&plugin])
        .expect("checkpoint journal restore");
    assert_eq!(from_bundle.snapshot(), snapshot);

    let store = PageStore::default();
    let paged = canwu
        .prepare_paged_checkpoint(None, &store)
        .expect("prepare the paged checkpoint");
    paged.store_and_verify(&store).expect("store the pages");
    let journal = canwu
        .journal_segment_since(EvidenceCursor::default())
        .expect("evidence prefix");
    let from_pages = Canwu::from_paged_checkpoint_and_journal_with_plugins(
        &paged.checkpoint,
        &store,
        vec![journal],
        &[&plugin],
    )
    .expect("paged checkpoint restore");
    assert_eq!(from_pages.snapshot(), snapshot);

    let replayed =
        Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), snapshot);

    // A compact host seals every boundary that published so far. The sealed
    // segment validates as the full run and restores it.
    let mut base = canwu.fork();
    base.settle_boundary(BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(3)))
        .expect("quiet boundary admits the last publication events");
    let mut unsealed = base.fork();
    let mut sealed = base.into_compacted().expect("compact interface");
    let segment = sealed
        .seal_evidence()
        .expect("seal the settled prefix")
        .expect("the prefix is sealable");
    assert!(
        canwu
            .boundaries()
            .iter()
            .all(|boundary| segment.boundaries.contains(boundary)),
        "the seal covers every publishing boundary"
    );
    let full = sealed
        .snapshot_with_segments(vec![segment.clone()])
        .expect("sealed segments validate as a full snapshot");
    assert_eq!(full.knowledge, snapshot.knowledge);
    let mut segments = vec![segment.clone()];
    let tail = sealed
        .journal_segment_since(segment.end)
        .expect("retained tail");
    if tail.start != tail.end {
        segments.push(tail);
    }
    let mut from_sealed = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        segments,
        &[&plugin],
    )
    .expect("sealed segments restore the compact run");
    assert_eq!(from_sealed.checkpoint_hash(), sealed.checkpoint_hash());

    // After the seal no publishing boundary is retained, and the systems
    // still supersede the same records: the report its latest predecessor,
    // the digest both current frontier records and the new sighting. The
    // sealed run, the unsealed fork, and the restore from sealed segments
    // settle to one state.
    for request in [
        at_day(3, SystemCadence::Daily),
        at_day(3, SystemCadence::Monthly),
    ] {
        sealed
            .settle_boundary(request.clone())
            .expect("sealed run settles a superseding boundary");
        from_sealed
            .settle_boundary(request.clone())
            .expect("restored sealed run settles a superseding boundary");
        unsealed
            .settle_boundary(request)
            .expect("unsealed fork settles a superseding boundary");
    }
    let settled = unsealed.boundaries().len();
    assert_eq!(
        published(&unsealed, settled - 2, 0).supersedes,
        vec![latest.id]
    );
    let resighted = published(&unsealed, settled - 1, 0);
    assert_eq!(
        published(&unsealed, settled - 1, 1).supersedes,
        vec![whispered.id, redigested.id, resighted.id]
    );
    assert_eq!(sealed.knowledge(), unsealed.knowledge());
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(from_sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    // Exact replay of the sealed run regenerates every boundary, including
    // the two settled after the seal, and equals the unsealed fork.
    let sealed_journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let sealed_replay = Canwu::replay_from_journal(&[&plugin], &sealed_journal)
        .expect("the sealed run replays exactly");
    assert_eq!(
        sealed_replay.snapshot(),
        sealed
            .snapshot_with_segments(vec![segment])
            .expect("the sealed run reconstructs a full snapshot")
    );
    assert_eq!(sealed_replay.snapshot(), unsealed.snapshot());

    // The restored run settles a further same-instant pair exactly as the
    // live run does.
    for run in [&mut canwu, &mut restored] {
        run.settle_boundary(at_day(2, SystemCadence::Daily))
            .expect("same-instant report after the load");
        run.settle_boundary(at_day(3, SystemCadence::Daily))
            .expect("next report after the load");
    }
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(published(&canwu, 5, 0).supersedes, vec![latest.id]);
    let restored_replay = Canwu::replay_from_journal(&[&plugin], &restored.replay_journal())
        .expect("the restored run replays exactly");
    assert_eq!(restored_replay.checkpoint_hash(), canwu.checkpoint_hash());

    // A load still refuses what settlement refuses. The digest's target was
    // admitted because it was visible; recorded as published for the next
    // boundary, it is hidden at the digest's cut.
    let refused = |tampered: &SimulationSnapshot| {
        let error = load(tampered, &plugin)
            .err()
            .expect("a relation settlement refuses must not load");
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
        assert!(
            error.message.contains("publisher's cut"),
            "{}",
            error.message
        );
    };
    let mut hidden_target = snapshot.clone();
    hidden_target.boundaries[2].knowledge_changes[0].visibility = StateVisibility::NextBoundary;
    refused(&hidden_target);

    // A relation cannot point at a later boundary of the same instant: the
    // first report did not have the second in its ledger.
    let mut wrong_way = snapshot;
    wrong_way.boundaries[0].knowledge_changes[0].records[0].supersedes = vec![second.id];
    let mut first_report = wrong_way
        .knowledge
        .records
        .record(first.id)
        .cloned()
        .expect("first report");
    first_report.supersedes = vec![second.id];
    wrong_way.knowledge.records.insert(first_report);
    refused(&wrong_way);
}
