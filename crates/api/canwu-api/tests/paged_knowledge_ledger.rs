//! A paged checkpoint pages the knowledge ledger, so a run whose ledger is
//! larger than one state page saves and restores through it, and the restore
//! charges what it assembles to the host's budget.
//!
//! A run publishes a body of a few kilobytes to each of 1,500 holders, more
//! than one state page in all, and later revises a few holders' records,
//! which retires what they supersede. Its paged checkpoint restores to the
//! same checkpoint hash and ledger, and the restored run continues exactly
//! as the live run does, its holders' local IDs included. A later
//! checkpoint stores only the changed bodies' and holders' pages, not the
//! ledger again. A restore succeeds with the descriptor's `assembled_bytes`
//! as its budget and is refused one byte below it, and a descriptor that
//! misstates the bytes is refused.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, EntityRef, ErrorCode, EvidenceCursor,
    EvidenceJournalSegment, KnowledgeHolderRef, KnowledgeOrigin, KnowledgePublicationGroup,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeRetention, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeSupersession, KnowledgeWriteGrant, MAX_STATE_PAGE_BYTES, PagedRestoreBudget,
    PagedSimulationCheckpoint, PayloadSchema, PersonId, PluginKnowledgeSchema, PluginRegistrar,
    Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey, StatePageBlob,
    StatePageProvider, StatePageStore, StateVisibility, SystemCadence,
};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;

const PLUGIN: &str = "fixture-paged-knowledge";
const HOLDERS: u64 = 1_500;
/// Holders that receive their first report on each of the first two days.
const PER_DAY: u64 = HOLDERS / 2;
const REPORT_TEXT_BYTES: usize = 3_000;

fn kind() -> KnowledgeRecordKind {
    KnowledgeRecordKind::new("fixture.paged-knowledge", "report")
}

fn schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(kind(), 1)
}

fn person(index: u64) -> PersonId {
    PersonId::new(index + 1)
}

fn holder(index: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(person(index))
}

/// The holders that receive a report on a day: half of them on each of the
/// first two days, then a few revisions that supersede and retire earlier
/// reports.
fn reported_on(day: u64) -> Vec<u64> {
    match day {
        1 => (0..PER_DAY).collect(),
        2 => (PER_DAY..HOLDERS).collect(),
        3 => vec![7, 8, 900],
        _ => vec![7],
    }
}

// A boundary system handler returns a `Result` whatever it does.
#[allow(clippy::unnecessary_wraps)]
fn publish(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    // One boundary a day: boundary N settles day N.
    let day = context.boundary_id.get();
    let groups = reported_on(day)
        .into_iter()
        .map(|index| KnowledgePublicationGroup {
            holders: vec![holder(index)],
            records: vec![KnowledgeRecordDraft {
                schema: schema(),
                subjects: vec![KnowledgeSubject {
                    role: "about".to_owned(),
                    target: KnowledgeSubjectTarget::Entity(EntityRef::Person(person(index))),
                }],
                payload: json!({
                    "holder": index,
                    "day": day,
                    "text": format!("{index:05}-{day:03}-").repeat(REPORT_TEXT_BYTES / 10),
                }),
                as_of: Some(context.at),
                confidence_per_mille: 900,
                origin: KnowledgeOrigin {
                    method: "fixture.report".to_owned(),
                    evidence: Vec::new(),
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            }],
        })
        .collect::<Vec<_>>();
    let records = groups.len();
    let payload_bytes = groups
        .iter()
        .flat_map(|group| &group.records)
        .map(|draft| draft.payload_bytes().expect("a report encodes"))
        .sum();
    // A publisher stays inside the boundary's publication budget.
    assert!(
        view.knowledge_publication_budget()
            .admits(records, records, payload_bytes),
        "the day's reports fit one directive"
    );
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: KnowledgeSupersession::CurrentOnSubjects,
            groups,
            summary: "The day's reports".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
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
        "00000000000000000000000000000000000000000000000000000000000c4e32"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: schema(),
            schema_hash: "00000000000000000000000000000000000000000000000000000000000c4e33"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "about".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
            retention: KnowledgeRetention::RetireWhenSuperseded,
        })?;
        let mut contract = BoundarySystemContract::new(
            "publish-reports",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_knowledge()];
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, publish)
    }
}

/// A content-addressed page store, as a host backs it.
#[derive(Clone, Default)]
struct PageStore(RefCell<BTreeMap<String, StatePageBlob>>);

impl StatePageProvider for PageStore {
    fn load_state_page(&self, page_id: &str) -> Result<Option<StatePageBlob>, CanwuError> {
        Ok(self.0.borrow().get(page_id).cloned())
    }
}

impl StatePageStore for PageStore {
    fn store_state_page(
        &self,
        page: &StatePageBlob,
    ) -> Result<canwu_api::ArchiveStoreOutcome, CanwuError> {
        page.validate()?;
        let mut pages = self.0.borrow_mut();
        if let Some(existing) = pages.get(&page.page_id) {
            assert_eq!(existing, page, "one page ID must name one byte string");
            return Ok(canwu_api::ArchiveStoreOutcome::AlreadyPresent);
        }
        pages.insert(page.page_id.clone(), page.clone());
        Ok(canwu_api::ArchiveStoreOutcome::Stored)
    }
}

fn settle(canwu: &mut Canwu, day: i64) {
    canwu
        .settle_boundary(
            BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day))
                .with_cadence(SystemCadence::Daily),
        )
        .expect("a report boundary settles");
}

fn restore(
    checkpoint: &PagedSimulationCheckpoint,
    store: &PageStore,
    segments: &[EvidenceJournalSegment],
    budget: PagedRestoreBudget,
) -> Result<Canwu, CanwuError> {
    Canwu::from_paged_checkpoint_and_journal_with_plugins(
        checkpoint,
        store,
        segments.to_vec(),
        &[&ReportPlugin],
        budget,
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn a_ledger_larger_than_a_page_saves_restores_and_charges_its_budget() {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        (0..HOLDERS)
            .map(|index| EntityRef::Person(person(index)))
            .collect(),
    );
    let mut live = Canwu::new_with_plugins(5151, scenario, &[&ReportPlugin]).expect("report run");
    settle(&mut live, 1);
    settle(&mut live, 2);
    let ledger_bytes = serde_json::to_vec(&live.knowledge().records)
        .expect("encode the ledger")
        .len();
    assert!(
        ledger_bytes > MAX_STATE_PAGE_BYTES,
        "the ledger alone ({ledger_bytes} bytes) is larger than one state page"
    );

    let store = PageStore::default();
    let first = live
        .prepare_paged_checkpoint(None, &store)
        .expect("a paged checkpoint holds a ledger larger than one page");
    first.store_and_verify(&store).expect("store the pages");
    let segments = vec![
        live.journal_segment_since(EvidenceCursor::default())
            .expect("the evidence prefix"),
    ];

    // The descriptor's byte count is the exact budget the restore needs.
    let exact = PagedRestoreBudget::new(first.checkpoint.assembled_bytes);
    let restored = restore(&first.checkpoint, &store, &segments, exact)
        .expect("restore with the checkpoint's own byte count");
    assert_eq!(restored.checkpoint_hash(), live.checkpoint_hash());
    assert_eq!(restored.knowledge().records, live.knowledge().records);
    let short = PagedRestoreBudget::new(first.checkpoint.assembled_bytes - 1);
    let error = restore(&first.checkpoint, &store, &segments, short)
        .err()
        .expect("a descriptor stating more than the budget is refused before any page");
    assert_eq!(error.code, ErrorCode::RestoreBudgetExceeded);
    // A descriptor that understates its bytes passes that first check; the
    // charge of what the pages describe still stops the restore.
    let mut understated = first.checkpoint.clone();
    understated.assembled_bytes -= 1;
    let error = restore(&understated, &store, &segments, short)
        .err()
        .expect("one byte below what the pages assemble is refused");
    assert_eq!(error.code, ErrorCode::RestoreBudgetExceeded);
    let mut misstated = first.checkpoint.clone();
    misstated.assembled_bytes += 1;
    let error = restore(&misstated, &store, &segments, PagedRestoreBudget::DEFAULT)
        .err()
        .expect("a descriptor that misstates its bytes is refused");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A holder ledger page the provider lost fails closed.
    let torn = store.clone();
    let lost = first
        .delta
        .new_pages
        .iter()
        .find(|page| {
            page.bytes
                .windows(b"knowledge.holders".len())
                .any(|window| window == b"knowledge.holders")
        })
        .expect("the delta holds holder ledger pages")
        .page_id
        .clone();
    torn.0.borrow_mut().remove(&lost);
    let error = restore(
        &first.checkpoint,
        &torn,
        &segments,
        PagedRestoreBudget::DEFAULT,
    )
    .err()
    .expect("a missing holder ledger page is refused");
    assert_eq!(error.code, ErrorCode::StatePageUnavailable);

    // Revisions of three holders' reports store their pages and the paths
    // above them, not the ledger again.
    settle(&mut live, 3);
    let second = live
        .prepare_paged_checkpoint(Some(&first.checkpoint), &store)
        .expect("prepare the second paged checkpoint");
    let new_bytes = second
        .delta
        .new_pages
        .iter()
        .map(|page| page.bytes.len())
        .sum::<usize>();
    assert!(
        second.delta.new_pages.len() * 20 < first.delta.new_pages.len()
            && new_bytes * 10 < ledger_bytes,
        "{} new pages of {new_bytes} bytes against {} pages and a {ledger_bytes}-byte ledger",
        second.delta.new_pages.len(),
        first.delta.new_pages.len(),
    );
    second
        .store_and_verify(&store)
        .expect("store the second delta");
    let segments = vec![
        live.journal_segment_since(EvidenceCursor::default())
            .expect("the evidence prefix"),
    ];
    let mut resumed = restore(
        &second.checkpoint,
        &store,
        &segments,
        PagedRestoreBudget::DEFAULT,
    )
    .expect("restore the second paged checkpoint");
    assert_eq!(resumed.checkpoint_hash(), live.checkpoint_hash());

    // The restored run continues exactly as the live run does: a revised
    // holder's next local ID survived the retirement of its first report.
    settle(&mut live, 4);
    settle(&mut resumed, 4);
    assert_eq!(resumed.checkpoint_hash(), live.checkpoint_hash());
    let revised = live
        .knowledge()
        .records
        .get(&holder(7))
        .expect("the revised holder");
    assert_eq!(revised.len(), 1, "superseded reports retired");
    assert_eq!(revised.next_local().get(), 4);
    assert_eq!(resumed.knowledge().records, live.knowledge().records);
}
