//! Shared knowledge publication, the engine's publication budget, and
//! retention at publication.
//!
//! One `PublishKnowledge` directive stores each body once and gives every
//! holder of its group an entry with the holder's own local ID and its own
//! supersession. A boundary has one publication budget, spent in dispatch
//! order, which each system reads before it publishes. A schema that retires
//! what its new entries supersede does so in the commit of the publishing
//! boundary, so a restored run and an exact replay hold the same ledger.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, BoundarySystemHandler, Canwu, CanwuError, EntityRef, ErrorCode,
    EventId, KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin, KnowledgePublicationBudget,
    KnowledgePublicationGroup, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordId,
    KnowledgeRecordKind, KnowledgeRetention, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeSupersession, KnowledgeWriteGrant, PayloadSchema, PersonId, PluginKnowledgeSchema,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PLUGIN: &str = "fixture-shared";
const NAMESPACE: &str = "fixture.shared";
/// A payload of this many bytes, as a JSON string of 65,498 characters.
const LARGE_PAYLOAD_BYTES: usize = 65_500;

const fn person(index: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(index))
}

fn line_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(NAMESPACE, "line"), 1)
}

fn log_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(NAMESPACE, "log"), 1)
}

fn subject(index: u64) -> KnowledgeSubject {
    KnowledgeSubject {
        role: "line".to_owned(),
        target: KnowledgeSubjectTarget::Event(EventId::new(index)),
    }
}

fn draft(schema: KnowledgeSchemaId, line: u64, payload: Value) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema,
        subjects: vec![subject(line)],
        payload,
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "fixture.shared".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

fn publish(
    supersession: KnowledgeSupersession,
    groups: Vec<KnowledgePublicationGroup>,
    summary: &str,
) -> BoundaryDirective {
    BoundaryDirective::PublishKnowledge {
        visibility: StateVisibility::SameBoundary,
        producer_correlation: None,
        supersession,
        groups,
        summary: summary.to_owned(),
    }
}

fn group(holders: &[u64], records: Vec<KnowledgeRecordDraft>) -> KnowledgePublicationGroup {
    KnowledgePublicationGroup {
        holders: holders.iter().copied().map(person).collect(),
        records,
    }
}

fn proposal(directives: Vec<BoundaryDirective>) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn budget_json(budget: KnowledgePublicationBudget) -> Value {
    serde_json::to_value(budget).expect("a budget encodes")
}

/// Boundary 1: each of persons 1 and 2 learns its own record on line 1.
/// Every later boundary: one body on line 1 for persons 1, 2, and 3, each
/// entry superseding that holder's current record on the line.
fn shared(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() == 1 {
        return proposal(vec![publish(
            KnowledgeSupersession::Explicit,
            vec![
                group(&[1], vec![draft(line_schema(), 1, json!({ "own": 1 }))]),
                group(&[2], vec![draft(line_schema(), 1, json!({ "own": 2 }))]),
            ],
            "own line records",
        )]);
    }
    proposal(vec![publish(
        KnowledgeSupersession::CurrentOnSubjects,
        vec![group(
            &[1, 2, 3],
            vec![draft(
                line_schema(),
                1,
                json!({ "boundary": context.boundary_id.get() }),
            )],
        )],
        "shared line",
    )])
}

/// Boundary 1: 65 current records on one line for person 1. Boundary 2: a
/// `CurrentOnSubjects` record on that line, whose match is truncated.
fn truncated(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() == 1 {
        return proposal(vec![publish(
            KnowledgeSupersession::Explicit,
            vec![group(
                &[1],
                (0..65)
                    .map(|index| draft(line_schema(), 1, json!({ "index": index })))
                    .collect(),
            )],
            "many heads",
        )]);
    }
    proposal(vec![publish(
        KnowledgeSupersession::CurrentOnSubjects,
        vec![group(&[1], vec![draft(line_schema(), 1, json!({}))])],
        "one head",
    )])
}

/// The first publishing system of phase 13: records the budget it read,
/// three records to person 1, and an account of what `payload_bytes` says
/// those three cost.
fn first_reader(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let read = budget_json(view.knowledge_publication_budget());
    let records: Vec<_> = (0..3_u64)
        .map(|index| {
            draft(
                line_schema(),
                index + 1,
                json!({ "read": read, "text": "x".repeat(usize::try_from(index * 100).unwrap()) }),
            )
        })
        .collect();
    let mut charged = 0;
    for record in &records {
        charged += record.payload_bytes()?;
    }
    let mut records = records;
    records.push(draft(line_schema(), 9, json!({ "charged": charged })));
    proposal(vec![publish(
        KnowledgeSupersession::Explicit,
        vec![group(&[1], records)],
        "first reader",
    )])
}

/// The second publishing system of phase 13: records the budget left after
/// the first.
fn second_reader(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let read = budget_json(view.knowledge_publication_budget());
    proposal(vec![publish(
        KnowledgeSupersession::Explicit,
        vec![group(
            &[2],
            vec![draft(log_schema(), 1, json!({ "read": read }))],
        )],
        "second reader",
    )])
}

/// The second system of phase 13, overshooting the boundary's payload bytes
/// alone: 1,025 records of 65,500 bytes, far inside the body and entry
/// budgets.
fn overshooting_reader(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let text = "y".repeat(LARGE_PAYLOAD_BYTES - 2);
    let drafts = |range: std::ops::Range<u64>| -> Vec<KnowledgeRecordDraft> {
        range
            .map(|index| draft(log_schema(), index, Value::String(text.clone())))
            .collect()
    };
    proposal(vec![
        publish(
            KnowledgeSupersession::Explicit,
            vec![group(&[2], drafts(1..1_001))],
            "large records",
        ),
        publish(
            KnowledgeSupersession::Explicit,
            vec![group(&[2], drafts(1_001..1_026))],
            "more large records",
        ),
    ])
}

/// Line records: boundaries 1 to 4 to persons 1 and 2 together, boundary 5
/// to person 1 alone, boundary 6 to person 2 alone, each superseding the
/// holder's current line record.
fn retained_line(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    let holders: &[u64] = match boundary {
        5 => &[1],
        6 => &[2],
        _ => &[1, 2],
    };
    proposal(vec![publish(
        KnowledgeSupersession::CurrentOnSubjects,
        vec![group(
            holders,
            vec![draft(line_schema(), 1, json!({ "boundary": boundary }))],
        )],
        "line",
    )])
}

/// Log records to person 1 under a `Keep` schema, each superseding the last.
fn kept_log(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut record = draft(
        log_schema(),
        2,
        json!({ "boundary": context.boundary_id.get() }),
    );
    record.supersedes = view
        .knowledge_heads(&person(1), &log_schema().kind, &[subject(2)])?
        .records;
    proposal(vec![publish(
        KnowledgeSupersession::Explicit,
        vec![group(&[1], vec![record])],
        "log",
    )])
}

/// A malformed or limit-crossing publication, with the refusal it must
/// meet; `None` for an accepted control.
struct PublicationCase {
    name: &'static str,
    supersession: KnowledgeSupersession,
    groups: Vec<KnowledgePublicationGroup>,
    refusal: Option<ErrorCode>,
}

fn publication_cases() -> Vec<PublicationCase> {
    use KnowledgeSupersession::{CurrentOnSubjects, Explicit};
    let lines = |range: std::ops::RangeInclusive<u64>| -> Vec<KnowledgeRecordDraft> {
        range
            .map(|line| draft(line_schema(), line, json!({})))
            .collect()
    };
    let mut naming = draft(line_schema(), 1, json!({}));
    naming.supersedes = vec![KnowledgeRecordId::new(1)];
    let mut contradicting = draft(line_schema(), 1, json!({}));
    contradicting.contradicts = vec![KnowledgeRecordId::new(1)];
    let case = |name, supersession, groups, refusal| PublicationCase {
        name,
        supersession,
        groups,
        refusal,
    };
    vec![
        case(
            "unsorted holders",
            CurrentOnSubjects,
            vec![group(&[2, 1], lines(1..=1))],
            Some(ErrorCode::InvalidKnowledgeRecord),
        ),
        case(
            "duplicate holders",
            CurrentOnSubjects,
            vec![group(&[1, 1], lines(1..=1))],
            Some(ErrorCode::InvalidKnowledgeRecord),
        ),
        case(
            "Explicit with two holders",
            Explicit,
            vec![group(&[1, 2], lines(1..=1))],
            Some(ErrorCode::InvalidKnowledgeRecord),
        ),
        case(
            "supersedes on a CurrentOnSubjects draft",
            CurrentOnSubjects,
            vec![group(&[1, 2], vec![naming])],
            Some(ErrorCode::InvalidKnowledgeRecord),
        ),
        case(
            "contradicts on a CurrentOnSubjects draft",
            CurrentOnSubjects,
            vec![group(&[1, 2], vec![contradicting])],
            Some(ErrorCode::InvalidKnowledgeRecord),
        ),
        case(
            "an ineligible holder among eligible ones",
            CurrentOnSubjects,
            vec![group(&[1, 2, 99], lines(1..=1))],
            Some(ErrorCode::InvalidKnowledgeHolder),
        ),
        case(
            "a group without holders",
            CurrentOnSubjects,
            vec![group(&[], lines(1..=1))],
            Some(ErrorCode::KnowledgeLimitExceeded),
        ),
        case(
            "a group without records",
            CurrentOnSubjects,
            vec![group(&[1], Vec::new())],
            Some(ErrorCode::KnowledgeLimitExceeded),
        ),
        case(
            "no group",
            CurrentOnSubjects,
            Vec::new(),
            Some(ErrorCode::KnowledgeLimitExceeded),
        ),
        case(
            "4,097 entries in one directive",
            CurrentOnSubjects,
            vec![
                group(&[1, 2, 3, 4, 5], lines(1..=819)),
                group(&[1], lines(820..=821)),
            ],
            Some(ErrorCode::KnowledgeLimitExceeded),
        ),
        case(
            "4,096 entries in one directive",
            CurrentOnSubjects,
            vec![
                group(&[1, 2, 3, 4, 5], lines(1..=819)),
                group(&[1], lines(820..=820)),
            ],
            None,
        ),
    ]
}

/// The index into [`publication_cases`] that `publication_case` proposes.
static PUBLICATION_CASE: AtomicUsize = AtomicUsize::new(0);

fn publication_case(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let case = publication_cases().swap_remove(PUBLICATION_CASE.load(Ordering::SeqCst));
    proposal(vec![publish(case.supersession, case.groups, case.name)])
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    Shared,
    Truncated,
    Budget,
    Overshoot,
    Retention,
    Cases,
}

struct SharedPlugin {
    mode: Mode,
    /// The line schema's retention; the log schema keeps everything.
    retention: KnowledgeRetention,
}

impl SimulationPlugin for SharedPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000017001"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for (schema, retention, hash) in [
            (
                line_schema(),
                self.retention,
                "0000000000000000000000000000000000000000000000000000000000017002",
            ),
            (
                log_schema(),
                KnowledgeRetention::Keep,
                "0000000000000000000000000000000000000000000000000000000000017003",
            ),
        ] {
            registrar.register_knowledge_schema(PluginKnowledgeSchema {
                id: schema,
                schema_hash: hash.to_owned(),
                writable: true,
                payload_schema: PayloadSchema::Any,
                subjects: vec![KnowledgeSubjectSchema {
                    role: "line".to_owned(),
                    targets: vec![KnowledgeSubjectTargetKind::Event],
                    required: true,
                    multiple: false,
                }],
                retention,
            })?;
        }
        let systems: Vec<(&str, BoundarySystemHandler)> = match self.mode {
            Mode::Shared => vec![("shared", shared)],
            Mode::Truncated => vec![("truncated", truncated)],
            Mode::Budget => vec![("a-first", first_reader), ("b-second", second_reader)],
            Mode::Overshoot => vec![("a-first", first_reader), ("b-second", overshooting_reader)],
            Mode::Retention => vec![("a-line", retained_line), ("b-log", kept_log)],
            Mode::Cases => vec![("case", publication_case)],
        };
        for (name, handler) in systems {
            let mut contract = BoundarySystemContract::new(
                name,
                BoundaryPhase::PerspectiveAndReportMaterialization,
                SystemCadence::Daily,
            );
            contract.reads = vec![StateKey::core_knowledge()];
            contract.knowledge_writes = [line_schema(), log_schema()]
                .into_iter()
                .map(|schema| KnowledgeWriteGrant {
                    schema,
                    visibilities: vec![StateVisibility::SameBoundary],
                })
                .collect();
            registrar.register_boundary_system(contract, handler)?;
        }
        Ok(())
    }
}

fn day(index: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(index)
}

fn run(plugin: &SharedPlugin) -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        (1..=5)
            .map(|index| EntityRef::Person(PersonId::new(index)))
            .collect(),
    );
    Canwu::new_with_plugins(1_700, scenario, &[plugin]).expect("the fixture run should start")
}

fn settle(canwu: &mut Canwu, index: i64) -> Result<canwu_api::BoundaryReceipt, CanwuError> {
    canwu.settle_boundary(BoundaryRequest::at(day(index)).with_cadence(SystemCadence::Daily))
}

/// The holder's current line records, newest first, from its ledger's
/// index.
fn heads(canwu: &Canwu, holder: u64) -> Vec<KnowledgeRecordId> {
    canwu
        .knowledge()
        .for_holder(&person(holder))
        .map(|ledger| {
            ledger
                .current_records(&line_schema().kind, &[subject(1)])
                .map(|body| body.id)
                .collect()
        })
        .unwrap_or_default()
}

/// A restored run and an exact replay reach the run's checkpoint. The
/// restored run keeps the ledger it rebuilt from the journal, so each body
/// it holds is the journal's own.
fn assert_restores_and_replays(canwu: &Canwu, plugin: &SharedPlugin) {
    let restored = Canwu::from_snapshot_json_with_plugins(
        &canwu.snapshot_json().expect("the run should save"),
        &[plugin],
    )
    .expect("the run should restore");
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(restored.knowledge(), canwu.knowledge());
    let records = &restored.knowledge().records;
    let mut shared = 0;
    for change in restored
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.knowledge_changes)
    {
        for body in &change.records {
            if let Some(held) = records.body(body.id) {
                assert!(
                    Arc::ptr_eq(held, body),
                    "the restored ledger shares {:?}",
                    body.id
                );
                shared += 1;
            }
        }
    }
    assert_eq!(shared, records.body_count());
    let replayed = Canwu::replay_from_journal(&[plugin], &canwu.replay_journal())
        .expect("the run should replay exactly");
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
}

/// One group to three holders, two of which hold an earlier record on the
/// line and one of which holds none: the body is stored once and shared by
/// the ledger and the journal, each holder's entry supersedes exactly its own
/// earlier record, each holder reads its own local IDs and nothing of the
/// others, and the one publication event reaches its three holders and no
/// one else.
#[test]
fn a_shared_publication_stores_one_body_and_supersedes_per_holder() {
    let plugin = SharedPlugin {
        mode: Mode::Shared,
        retention: KnowledgeRetention::Keep,
    };
    let mut canwu = run(&plugin);
    settle(&mut canwu, 1).expect("the own records should settle");
    let own = |holder: u64| heads(&canwu, holder);
    let (first_own, second_own) = (own(1), own(2));
    assert_eq!((first_own.len(), second_own.len()), (1, 1));
    assert!(own(3).is_empty());

    let receipt = settle(&mut canwu, 2).expect("the shared record should settle");
    assert_eq!(
        (
            receipt.knowledge_batch_count,
            receipt.knowledge_record_count,
            receipt.knowledge_entry_count
        ),
        (1, 1, 3)
    );
    let change = &canwu.boundaries()[1].knowledge_changes[0];
    let body = Arc::clone(&change.records[0]);
    let records = &canwu.knowledge().records;
    assert_eq!(records.body_count(), 3);
    assert_eq!(records.body_holder_count(body.id), 3);
    // The journal shares the ledger's body instead of copying it.
    assert!(Arc::ptr_eq(
        records.body(body.id).expect("the body is stored"),
        &body
    ));
    for holder in 1..=3 {
        let ledger = records.for_holder_ledger(holder);
        assert!(Arc::ptr_eq(ledger.body(&body.id).expect("held"), &body));
        assert_eq!(heads(&canwu, holder), vec![body.id]);
    }
    let supersedes = |holder: u64| {
        records
            .for_holder_ledger(holder)
            .entry(&body.id)
            .expect("the holder holds the shared body")
            .supersedes
            .clone()
    };
    assert_eq!(supersedes(1), first_own);
    assert_eq!(supersedes(2), second_own);
    assert!(supersedes(3).is_empty());

    // Each holder reads local IDs from its own counter: person 3's first
    // record is its record 1, though its global ID is 3, and its view says
    // nothing of the other holders.
    let history = KnowledgeQuery {
        view: KnowledgeHistoryView::FullHistory,
        ..KnowledgeQuery::default()
    };
    let views = |holder: u64| {
        canwu
            .viewer_for_actor(PersonId::new(holder))
            .expect("a person viewer")
            .query_knowledge(&history)
            .expect("the holder's own ledger")
    };
    let third = views(3);
    assert_eq!(third.records.len(), 1);
    assert_eq!(third.records[0].id.get(), 1);
    assert!(third.records[0].supersedes.is_empty());
    assert_eq!(body.id.get(), 3);
    let first = views(1);
    assert_eq!(
        first
            .records
            .iter()
            .map(|record| (record.id.get(), record.supersedes.clone()))
            .collect::<Vec<_>>(),
        vec![(1, vec![]), (2, vec![first.records[0].id])]
    );
    assert_eq!(first.records[1].payload, third.records[0].payload);
    assert_eq!(
        serde_json::to_value(&third.records[0])
            .expect("a view encodes")
            .as_object()
            .expect("a view is an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        serde_json::to_value(&first.records[0])
            .expect("a view encodes")
            .as_object()
            .expect("a view is an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        "a shared body reads like a private one"
    );

    // One event for the directive, seen by each of its holders only.
    let published: Vec<_> = canwu
        .events()
        .iter()
        .filter(|event| event.timestamp == day(2) && event.kind.is_type("knowledge_published"))
        .collect();
    assert_eq!(published.len(), 1);
    for (holder, sees) in [(1, true), (2, true), (3, true), (4, false)] {
        let changes = canwu
            .viewer_for_actor(PersonId::new(holder))
            .expect("a person viewer")
            .visible_changes_since(day(1));
        assert_eq!(
            changes
                .iter()
                .any(|change| change.source_event == published[0].id),
            sees,
            "person {holder}"
        );
    }

    settle(&mut canwu, 3).expect("the next shared record should settle");
    assert_restores_and_replays(&canwu, &plugin);
}

/// A `CurrentOnSubjects` draft whose match for a holder is truncated fails
/// the boundary: a shared line is meant to have one head.
#[test]
fn a_truncated_current_on_subjects_match_fails_the_boundary() {
    let plugin = SharedPlugin {
        mode: Mode::Truncated,
        retention: KnowledgeRetention::Keep,
    };
    let mut canwu = run(&plugin);
    settle(&mut canwu, 1).expect("the many heads should settle");
    assert_eq!(heads(&canwu, 1).len(), 65);
    let before = canwu.checkpoint_hash().to_owned();
    let error = settle(&mut canwu, 2).expect_err("a truncated match must fail");
    assert_eq!(error.code, ErrorCode::InvalidKnowledgeRecord, "{error}");
    assert_eq!(canwu.checkpoint_hash(), before);
}

/// Every malformed directive, and every directive past the per-directive
/// entry bound, fails its boundary with its own error code and leaves the
/// run as it was; a directive of exactly 4,096 entries commits.
#[test]
fn malformed_or_oversized_publications_are_refused() {
    let plugin = SharedPlugin {
        mode: Mode::Cases,
        retention: KnowledgeRetention::Keep,
    };
    for (index, case) in publication_cases().into_iter().enumerate() {
        PUBLICATION_CASE.store(index, Ordering::SeqCst);
        let mut canwu = run(&plugin);
        let before = canwu.checkpoint_hash().to_owned();
        let settled = settle(&mut canwu, 1);
        if let Some(code) = case.refusal {
            let error = settled.expect_err(case.name);
            assert_eq!(error.code, code, "{}: {error}", case.name);
            assert_eq!(canwu.checkpoint_hash(), before, "{}", case.name);
        } else {
            let receipt = settled.expect(case.name);
            assert_eq!(receipt.knowledge_entry_count, 4_096, "{}", case.name);
        }
    }
}

/// Two systems of one phase spend one budget in dispatch order: the second
/// reads exactly what the first left, `payload_bytes` is what the engine
/// charged, and restored and replayed runs read the same. A second system
/// that publishes past the budget in payload bytes alone fails the boundary.
#[test]
fn systems_read_the_budget_left_in_dispatch_order() {
    let plugin = SharedPlugin {
        mode: Mode::Budget,
        retention: KnowledgeRetention::Keep,
    };
    let mut canwu = run(&plugin);
    settle(&mut canwu, 1).expect("the budget readers should settle");
    let change = |index: usize| &canwu.boundaries()[0].knowledge_changes[index];
    let read = |value: &Value| -> KnowledgePublicationBudget {
        serde_json::from_value(value["read"].clone()).expect("a recorded budget")
    };
    let first = read(&change(0).records[0].payload);
    let second = read(&change(1).records[0].payload);
    let limits = canwu_api::KnowledgeLimitsV1::CURRENT;
    assert_eq!(
        first,
        KnowledgePublicationBudget {
            records: limits.records_per_boundary,
            holder_entries: limits.holder_entries_per_boundary,
            payload_bytes: limits.payload_bytes_per_boundary,
            records_per_batch: limits.records_per_batch,
            holder_entries_per_batch: limits.holder_entries_per_batch,
        }
    );
    let bytes = |body: &canwu_api::KnowledgeRecordBody| {
        serde_json::to_vec(&body.payload)
            .expect("a payload encodes")
            .len()
    };
    let spent: usize = change(0).records.iter().map(|body| bytes(body)).sum();
    assert_eq!(
        second,
        KnowledgePublicationBudget {
            records: first.records - 4,
            holder_entries: first.holder_entries - 4,
            payload_bytes: first.payload_bytes - spent,
            ..first
        }
    );
    let claimed = change(0).records[3].payload["charged"]
        .as_u64()
        .expect("the account of what payload_bytes said");
    assert_eq!(
        usize::try_from(claimed).unwrap(),
        change(0).records[..3]
            .iter()
            .map(|body| bytes(body))
            .sum::<usize>()
    );
    settle(&mut canwu, 2).expect("the next boundary reads the whole budget again");
    assert_eq!(
        read(&canwu.boundaries()[1].knowledge_changes[0].records[0].payload),
        first
    );
    assert_restores_and_replays(&canwu, &plugin);

    let overshoot = SharedPlugin {
        mode: Mode::Overshoot,
        retention: KnowledgeRetention::Keep,
    };
    let mut canwu = run(&overshoot);
    let before = canwu.checkpoint_hash().to_owned();
    let error = settle(&mut canwu, 1).expect_err("publishing past the budget must fail");
    assert_eq!(error.code, ErrorCode::KnowledgeLimitExceeded, "{error}");
    assert_eq!(canwu.checkpoint_hash(), before);
}

/// Retention at publication: under `RetireWhenSuperseded` a holder keeps
/// one entry and the ledger one body per line, a body two holders share
/// leaves only once both superseded it, the superseding entry's relations
/// are empty after commit while the evidence keeps the published list, a
/// `FullHistory` read shows current entries and reports the retention, and
/// local IDs never move. A `Keep` schema beside it keeps everything.
#[test]
fn retention_retires_superseded_entries_at_publication() {
    for retention in [
        KnowledgeRetention::RetireWhenSuperseded,
        KnowledgeRetention::Keep,
    ] {
        let plugin = SharedPlugin {
            mode: Mode::Retention,
            retention,
        };
        let retires = retention == KnowledgeRetention::RetireWhenSuperseded;
        let mut canwu = run(&plugin);
        for index in 1..=4 {
            settle(&mut canwu, index).expect("the line and log should settle");
        }
        let records = &canwu.knowledge().records;
        let shared = heads(&canwu, 1)[0];
        assert_eq!(heads(&canwu, 2), vec![shared]);
        assert_eq!(records.body_holder_count(shared), 2);
        let first = records.for_holder_ledger(1);
        assert_eq!(first.len(), if retires { 1 + 4 } else { 4 + 4 });
        assert_eq!(records.body_count(), if retires { 1 + 4 } else { 4 + 4 });
        assert_eq!(first.next_local().get(), 9);
        // The first log record keeps the local ID it was published with.
        let first_log = first.id_of(canwu_api::HolderKnowledgeRecordId::new(2));
        assert_eq!(
            first_log
                .and_then(|id| first.body(&id))
                .map(|body| body.schema.clone()),
            Some(log_schema())
        );
        if retires {
            assert!(first.entry(&shared).expect("held").supersedes.is_empty());
        }
        let published = canwu.boundaries()[3].knowledge_changes[0].holders[0].entries[0]
            .supersedes
            .clone();
        assert_eq!(published.len(), 1, "the evidence keeps the published list");

        settle(&mut canwu, 5).expect("person 1 alone moves on");
        let records = &canwu.knowledge().records;
        assert_eq!(heads(&canwu, 2), vec![shared]);
        assert_eq!(
            records.body_holder_count(shared),
            if retires { 1 } else { 2 }
        );
        settle(&mut canwu, 6).expect("person 2 moves on");
        let records = &canwu.knowledge().records;
        assert_eq!(records.body(shared).is_none(), retires);
        assert_eq!(
            records.body_count(),
            if retires { 2 + 6 } else { 6 + 6 },
            "{retention:?}"
        );

        let viewer = canwu
            .viewer_for_actor(PersonId::new(1))
            .expect("a person viewer");
        let history = |schemas: Vec<KnowledgeSchemaId>| {
            viewer
                .query_knowledge(&KnowledgeQuery {
                    schemas,
                    view: KnowledgeHistoryView::FullHistory,
                    ..KnowledgeQuery::default()
                })
                .expect("the holder's own ledger")
        };
        let lines = history(vec![line_schema()]);
        assert_eq!(lines.records.len(), if retires { 1 } else { 5 });
        assert_eq!(
            lines.retention,
            BTreeMap::from([(line_schema(), retention)])
        );
        let logs = history(vec![log_schema()]);
        assert_eq!(logs.records.len(), 6);
        assert_eq!(
            logs.retention,
            BTreeMap::from([(log_schema(), KnowledgeRetention::Keep)])
        );
        assert_eq!(
            history(Vec::new()).retention,
            BTreeMap::from([
                (line_schema(), retention),
                (log_schema(), KnowledgeRetention::Keep)
            ])
        );
        assert_restores_and_replays(&canwu, &plugin);
    }
}

trait ForHolder {
    fn for_holder_ledger(&self, holder: u64) -> &canwu_api::HolderLedger;
}

impl ForHolder for canwu_api::HolderLedgers {
    fn for_holder_ledger(&self, holder: u64) -> &canwu_api::HolderLedger {
        self.get(&person(holder)).expect("the holder is listed")
    }
}
