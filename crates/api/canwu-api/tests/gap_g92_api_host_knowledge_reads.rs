//! Downstream gap G-92: a host reads a holder's current knowledge records
//! from the run handles as a boundary system reads them.
//!
//! A probe system runs before any publisher of its boundary and records what
//! `SimulationView::knowledge_heads` and a `CurrentHeads` page of
//! `SimulationView::knowledge_records` return for these questions: a line
//! whose record supersedes the last every boundary, for two holders sharing
//! each body, under a schema that keeps superseded records and under one
//! that retires them; a line that only grows, past the bound of
//! `knowledge_heads`; and the same line narrowed by a second subject. Before
//! every boundary the host asks the same through `&dyn CanwuRun`
//! (`knowledge_heads` and `knowledge_current_records`, whose records come
//! newest first) on a full-history run, a run that seals every boundary, a
//! run restored from a continuation checkpoint, and a run reloaded from a
//! snapshot, and must read exactly what the probe then records. The subject
//! filter takes up to 64 subjects, as in a boundary.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CanwuRun, CompactedCanwu, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    EntityRef, ErrorCode, EventId, KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin,
    KnowledgePublicationGroup, KnowledgeQuery, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeRecordView, KnowledgeRetention, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeSupersession, KnowledgeWriteGrant, PayloadSchema, PersonId, PluginKnowledgeSchema,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-host-knowledge";
/// Days the runs settle; the growing line passes the heads bound of 64 by
/// day 22.
const DAYS: i64 = 26;
/// The day the restored and reloaded runs are taken from the full run.
const BRANCH_DAY: i64 = 9;

fn schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new("fixture.host_knowledge", "line"),
        1,
    )
}

/// A line schema whose superseded entries leave their holder.
fn retired_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new("fixture.host_knowledge", "retired_line"),
        1,
    )
}

const fn person(index: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(index))
}

fn subject(role: &str, index: u64) -> KnowledgeSubject {
    KnowledgeSubject {
        role: role.to_owned(),
        target: KnowledgeSubjectTarget::Event(EventId::new(index)),
    }
}

fn probe_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "probe")
}

fn probe_ref() -> DomainRecordRef {
    DomainRecordRef {
        kind: probe_kind(),
        id: "probe".to_owned(),
    }
}

/// The questions both sides ask: a schema, a holder, and the subjects a
/// record must carry. The growing line is the third.
fn cases() -> Vec<(KnowledgeSchemaId, KnowledgeHolderRef, Vec<KnowledgeSubject>)> {
    vec![
        (schema(), person(1), vec![subject("line", 1)]),
        (schema(), person(2), vec![subject("line", 1)]),
        (schema(), person(1), vec![subject("line", 2)]),
        (
            schema(),
            person(1),
            vec![subject("line", 2), subject("tag", 1)],
        ),
        (schema(), person(2), Vec::new()),
        (schema(), person(3), vec![subject("line", 1)]),
        (retired_schema(), person(1), vec![subject("line", 1)]),
        (retired_schema(), person(2), Vec::new()),
    ]
}

fn draft(subjects: Vec<KnowledgeSubject>, payload: Value) -> KnowledgeRecordDraft {
    retired_draft(schema(), subjects, payload)
}

fn retired_draft(
    schema: KnowledgeSchemaId,
    subjects: Vec<KnowledgeSubject>,
    payload: Value,
) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema,
        subjects,
        payload,
        as_of: None,
        confidence_per_mille: 900,
        origin: KnowledgeOrigin {
            method: "fixture.host_knowledge".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

/// Every boundary: one body on line 1 for persons 1 and 2, superseding each
/// holder's current line 1 record, under each of the two schemas; three
/// records on line 2 for person 1, tagged by turns, superseding nothing;
/// and every third boundary one record on line 2 for person 2.
fn publish(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    let mut directives = [schema(), retired_schema()]
        .into_iter()
        .map(|schema| BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: KnowledgeSupersession::CurrentOnSubjects,
            groups: vec![KnowledgePublicationGroup {
                holders: vec![person(1), person(2)],
                records: vec![retired_draft(
                    schema,
                    vec![subject("line", 1)],
                    json!({ "boundary": boundary }),
                )],
            }],
            summary: "the shared line".to_owned(),
        })
        .collect::<Vec<_>>();
    let mut growing = vec![KnowledgePublicationGroup {
        holders: vec![person(1)],
        records: (0..3)
            .map(|index| {
                draft(
                    vec![subject("line", 2), subject("tag", (boundary + index) % 3)],
                    json!({ "boundary": boundary, "index": index }),
                )
            })
            .collect(),
    }];
    if boundary.is_multiple_of(3) {
        growing.push(KnowledgePublicationGroup {
            holders: vec![person(2)],
            records: vec![draft(
                vec![subject("line", 2)],
                json!({ "boundary": boundary, "holder": 2 }),
            )],
        });
    }
    directives.push(BoundaryDirective::PublishKnowledge {
        visibility: StateVisibility::SameBoundary,
        producer_correlation: None,
        supersession: KnowledgeSupersession::Explicit,
        groups: growing,
        summary: "the growing line".to_owned(),
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Records what a boundary system reads for each case before anything is
/// staged in its boundary: the heads, whether they were truncated, and the
/// records of a `CurrentHeads` page.
fn probe(view: &SimulationView<'_>, _: &BoundaryContext) -> Result<BoundaryProposal, CanwuError> {
    let mut answers = Vec::new();
    for (schema, holder, subjects) in cases() {
        let heads = view.knowledge_heads(&holder, &schema.kind, &subjects)?;
        let page = view.knowledge_records(
            holder,
            &KnowledgeQuery {
                schemas: vec![schema],
                subjects,
                view: KnowledgeHistoryView::CurrentHeads,
                limit: 1_000,
                ..KnowledgeQuery::default()
            },
        )?;
        assert!(page.next.is_none(), "the fixture's pages fit one query");
        answers.push(json!({
            "heads": heads.records.iter().map(|id| id.get()).collect::<Vec<_>>(),
            "truncated": heads.truncated,
            "records": page.records,
        }));
    }
    let record = DomainRecordDraft::new(probe_ref(), json!({ "answers": answers }));
    let mutation = match view.domain_record(&probe_ref())? {
        None => DomainRecordMutation::Create { record },
        Some(stored) => DomainRecordMutation::Update {
            record,
            expected_version: stored.version,
        },
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation,
            summary: "what the boundary read".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct HostKnowledgeFixture;

impl SimulationPlugin for HostKnowledgeFixture {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000009201"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut probe_schema = DomainRecordSchema::new(probe_kind(), DomainRecordClass::Record);
        probe_schema.payload_schema = PayloadSchema::Any;
        let probe_state = probe_schema.state_key();
        registrar.register_record_schema(probe_schema)?;
        for (id, hash, retention) in [
            (
                schema(),
                "0000000000000000000000000000000000000000000000000000000000009202",
                KnowledgeRetention::Keep,
            ),
            (
                retired_schema(),
                "0000000000000000000000000000000000000000000000000000000000009203",
                KnowledgeRetention::RetireWhenSuperseded,
            ),
        ] {
            registrar.register_knowledge_schema(PluginKnowledgeSchema {
                id,
                schema_hash: hash.to_owned(),
                writable: true,
                payload_schema: PayloadSchema::Any,
                subjects: vec![
                    KnowledgeSubjectSchema {
                        role: "line".to_owned(),
                        targets: vec![KnowledgeSubjectTargetKind::Event],
                        required: true,
                        multiple: false,
                    },
                    KnowledgeSubjectSchema {
                        role: "tag".to_owned(),
                        targets: vec![KnowledgeSubjectTargetKind::Event],
                        required: false,
                        multiple: false,
                    },
                ],
                retention,
            })?;
        }
        let mut probing = BoundarySystemContract::new(
            "a-probe",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        probing.reads = vec![StateKey::core_knowledge(), probe_state.clone()];
        probing.writes = vec![probe_state];
        probing.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(probing, probe)?;
        let mut publishing = BoundarySystemContract::new(
            "b-publish",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        publishing.reads = vec![StateKey::core_knowledge()];
        publishing.knowledge_writes = [schema(), retired_schema()]
            .into_iter()
            .map(|schema| KnowledgeWriteGrant {
                schema,
                visibilities: vec![StateVisibility::SameBoundary],
            })
            .collect();
        registrar.register_boundary_system(publishing, publish)
    }
}

enum Run {
    Full(Canwu),
    /// A compact run, sealing after every boundary when `seals` is set.
    Compact {
        run: CompactedCanwu,
        seals: bool,
    },
}

impl Run {
    fn handle(&self) -> &dyn CanwuRun {
        match self {
            Self::Full(run) => run,
            Self::Compact { run, .. } => run,
        }
    }

    fn settle(&mut self, day: i64) {
        let request = BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day))
            .with_cadence(SystemCadence::Daily);
        match self {
            Self::Full(run) => {
                run.settle_boundary(request).expect("the boundary settles");
            }
            Self::Compact { run, seals } => {
                run.settle_boundary(request).expect("the boundary settles");
                if *seals {
                    run.seal_evidence().expect("the run seals");
                }
            }
        }
    }
}

fn plugins() -> [&'static dyn SimulationPlugin; 1] {
    [&HostKnowledgeFixture]
}

fn new_run() -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        (1..=3)
            .map(|index| EntityRef::Person(PersonId::new(index)))
            .collect(),
    );
    Canwu::new_with_plugins(92, scenario, &plugins()).expect("the fixture run starts")
}

/// What the host reads for each case, shaped as the probe records it: the
/// heads, and the current records, which come newest first, sorted into a
/// page's order.
fn host_answers(run: &dyn CanwuRun) -> Value {
    let answers = cases()
        .into_iter()
        .map(|(schema, holder, subjects)| {
            let heads = run
                .knowledge_heads(&holder, &schema.kind, &subjects)
                .expect("the host reads the heads");
            let mut records: Vec<KnowledgeRecordView> = run
                .knowledge_current_records(&holder, &schema.kind, &subjects)
                .collect();
            assert!(
                records.windows(2).all(|pair| pair[0].id > pair[1].id),
                "the current records come newest first"
            );
            records.sort_by_key(|record| (record.learned_at, record.id));
            json!({
                "heads": heads.records.iter().map(|id| id.get()).collect::<Vec<_>>(),
                "truncated": heads.truncated,
                "records": records,
            })
        })
        .collect::<Vec<_>>();
    json!({ "answers": answers })
}

#[test]
fn host_knowledge_reads_answer_what_a_boundary_reads_in_every_run() {
    let mut runs = vec![
        ("full", Run::Full(new_run())),
        (
            "sealed",
            Run::Compact {
                run: new_run().into_compacted().expect("the run compacts"),
                seals: true,
            },
        ),
    ];
    let mut truncated_seen = false;
    let mut past_bound_seen = false;
    for day in 0..DAYS {
        if day == BRANCH_DAY {
            let Run::Full(full) = &runs[0].1 else {
                unreachable!("the first run is the full one")
            };
            let (checkpoint, hash) = full
                .continuation_checkpoint()
                .expect("the continuation checkpoint builds");
            let restored = CompactedCanwu::from_continuation_checkpoint_verified(
                checkpoint,
                &hash,
                &plugins(),
            )
            .expect("the run restores from its continuation checkpoint");
            let reloaded = Canwu::from_snapshot_json_with_plugins(
                &full.snapshot_json().expect("the snapshot encodes"),
                &plugins(),
            )
            .expect("the run reloads from its snapshot");
            runs.push((
                "continuation",
                Run::Compact {
                    run: restored,
                    seals: false,
                },
            ));
            runs.push(("reloaded", Run::Full(reloaded)));
        }
        let mut expected = None;
        for (name, run) in &mut runs {
            let host = host_answers(run.handle());
            run.settle(day);
            let probed = run
                .handle()
                .domain_record(&probe_ref())
                .expect("the probe recorded its read")
                .payload
                .clone();
            assert_eq!(
                host, probed,
                "{name}, day {day}: the host read differs from the boundary's"
            );
            match &expected {
                None => expected = Some(host),
                Some(first) => assert_eq!(&host, first, "{name}, day {day}: runs disagree"),
            }
        }
        let answers = &expected.expect("a run answered")["answers"];
        truncated_seen |= answers[2]["truncated"] == true;
        past_bound_seen |= answers[2]["records"].as_array().map_or(0, Vec::len) > 64;
    }
    assert!(
        truncated_seen && past_bound_seen,
        "the growing line passes the heads bound, and the records read goes past it"
    );

    // The subject filter takes up to 64 subjects, as in a boundary.
    let Run::Full(full) = &runs[0].1 else {
        unreachable!("the first run is the full one")
    };
    let at_bound = (0..64)
        .map(|index| subject("line", index))
        .collect::<Vec<_>>();
    assert!(
        full.knowledge_heads(&person(1), &schema().kind, &at_bound)
            .expect("64 subjects are read")
            .records
            .is_empty()
    );
    let too_many = (0..=64)
        .map(|index| subject("line", index))
        .collect::<Vec<_>>();
    assert_eq!(
        full.knowledge_heads(&person(1), &schema().kind, &too_many)
            .expect_err("an oversized subject filter is refused")
            .code,
        ErrorCode::KnowledgeLimitExceeded
    );
}
