#![allow(clippy::unnecessary_wraps)]

use canwu_core::{DomainRecordKind, DomainRecordRef, EntityRef, PersonId};
use canwu_sim::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, CanwuError, DomainRecordClass, DomainRecordDraft, DomainRecordMutation,
    DomainRecordSchema, DomainRecordVersionRef, DomainRecordVersionSource, ErrorCode, EvidenceRef,
    KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeQuery,
    KnowledgeRecordDraft, KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId,
    KnowledgeWriteGrant, PayloadSchema, PluginKnowledgeSchema, PluginRegistrar, Simulation,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
};
use canwu_time::{SimDuration, SimTime};
use serde_json::json;

fn schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new("fixture.kernel", "notice"), 1)
}

fn draft(value: u64) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema: schema(),
        subjects: Vec::new(),
        payload: json!({ "value": value }),
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "fixture".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

fn register_schema(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    registrar.register_knowledge_schema(PluginKnowledgeSchema {
        id: schema(),
        schema_hash: "1000000000000000000000000000000000000000000000000000000000000000".to_owned(),
        writable: true,
        payload_schema: PayloadSchema::Any,
        subjects: Vec::new(),
    })
}

fn publication_contract(name: &str, phase: BoundaryPhase) -> BoundarySystemContract {
    let mut contract = BoundarySystemContract::new(name, phase, SystemCadence::Daily);
    contract.knowledge_writes = vec![KnowledgeWriteGrant {
        schema: schema(),
        visibilities: vec![StateVisibility::SameBoundary],
    }];
    contract
}

fn interleaved_publication(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let state = StateKey::new("fixture-kernel", "ordinary-phase13");
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(1)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some("first-holder-one".to_owned()),
                records: vec![draft(1)],
                summary: "Publish first holder record".to_owned(),
            },
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(2)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some("holder-two".to_owned()),
                records: vec![draft(2)],
                summary: "Publish second holder record".to_owned(),
            },
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(1)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some("second-holder-one".to_owned()),
                records: vec![draft(3)],
                summary: "Publish second record for first holder".to_owned(),
            },
            BoundaryDirective::SetComponent {
                state,
                entity: EntityRef::Person(PersonId::new(1)),
                component: "committed".to_owned(),
                value: json!(true),
                summary: "Preserve ordinary phase-13 behavior".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

struct InterleavedPublicationPlugin;

impl SimulationPlugin for InterleavedPublicationPlugin {
    fn name(&self) -> &'static str {
        "fixture-kernel"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "2000000000000000000000000000000000000000000000000000000000000000"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_schema(registrar)?;
        let mut contract = publication_contract(
            "interleaved-publication",
            BoundaryPhase::PerspectiveAndReportMaterialization,
        );
        contract.writes = vec![StateKey::new("fixture-kernel", "ordinary-phase13")];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, interleaved_publication)
    }
}

fn duplicate_correlation(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(1)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some("duplicate".to_owned()),
                records: vec![draft(1)],
                summary: "First duplicate correlation".to_owned(),
            },
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(2)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some("duplicate".to_owned()),
                records: vec![draft(2)],
                summary: "Second duplicate correlation".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

struct DuplicateCorrelationPlugin;

impl SimulationPlugin for DuplicateCorrelationPlugin {
    fn name(&self) -> &'static str {
        "fixture-duplicate-correlation"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "3000000000000000000000000000000000000000000000000000000000000000"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_schema(registrar)?;
        registrar.register_boundary_system(
            publication_contract(
                "duplicate-correlation",
                BoundaryPhase::PerspectiveAndReportMaterialization,
            ),
            duplicate_correlation,
        )
    }
}

fn stage_early_publication(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("must-roll-back".to_owned()),
            records: vec![draft(1)],
            summary: "Stage publication before later failure".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn fail_later_phase(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state: StateKey::new("fixture-late-failure", "undeclared"),
            entity: EntityRef::Person(PersonId::new(1)),
            component: "invalid".to_owned(),
            value: json!(true),
            summary: "Force a later validation failure".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct LateFailurePlugin;

impl SimulationPlugin for LateFailurePlugin {
    fn name(&self) -> &'static str {
        "fixture-late-failure"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "4000000000000000000000000000000000000000000000000000000000000000"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_schema(registrar)?;
        registrar.register_boundary_system(
            publication_contract(
                "early-publication",
                BoundaryPhase::PerceptionAndAttentionRefresh,
            ),
            stage_early_publication,
        )?;
        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "later-invalid-write",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
            ),
            fail_later_phase,
        )
    }
}

fn evidence_kind() -> DomainRecordKind {
    DomainRecordKind::new("fixture-stage-evidence", "marker")
}

fn evidence_ref(id: &str) -> DomainRecordRef {
    DomainRecordRef {
        kind: evidence_kind(),
        id: id.to_owned(),
    }
}

fn evidence_state() -> StateKey {
    DomainRecordSchema::new(evidence_kind(), DomainRecordClass::Record).state_key()
}

fn create_stage_record(id: &str, stage: &str) -> BoundaryProposal {
    BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: DomainRecordDraft::new(evidence_ref(id), json!({ "stage": stage })),
            },
            summary: format!("Create {stage} evidence"),
        }],
        ..BoundaryProposal::default()
    }
}

fn create_phase9_evidence(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(create_stage_record("phase9", "phase9"))
}

fn create_phase11_evidence(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(create_stage_record("phase11", "phase11"))
}

fn create_phase12_evidence(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(create_stage_record("phase12", "phase12"))
}

fn publish_from_all_committed_stages(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut evidence = Vec::new();
    for (index, id) in ["phase9", "phase11", "phase12"].into_iter().enumerate() {
        let reference = evidence_ref(id);
        let record = view.domain_record(&reference)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                format!("{id} record is unavailable to phase 13"),
            )
        })?;
        evidence.push(EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
            record: reference,
            version: record.version,
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: context.boundary_id,
                change_index: u64::try_from(index).expect("three indexes fit u64"),
            },
        }));
    }
    evidence.sort();
    let mut knowledge = draft(13);
    knowledge.origin.evidence = evidence;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("all-committed-stages".to_owned()),
            records: vec![knowledge],
            summary: "Publish from phase 9, 11, and 12 evidence".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct CommittedStageEvidencePlugin;

impl SimulationPlugin for CommittedStageEvidencePlugin {
    fn name(&self) -> &'static str {
        "fixture-stage-evidence"
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "5000000000000000000000000000000000000000000000000000000000000000"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            evidence_kind(),
            DomainRecordClass::Record,
        ))?;
        register_schema(registrar)?;

        let mut phase7 = BoundarySystemContract::new(
            "create-phase9-evidence",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        phase7.writes = vec![evidence_state()];
        phase7.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(phase7, create_phase9_evidence)?;

        let mut phase10 = BoundarySystemContract::new(
            "create-phase11-evidence",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::Daily,
        );
        phase10.writes = vec![evidence_state()];
        phase10.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(phase10, create_phase11_evidence)?;

        let mut phase12 = BoundarySystemContract::new(
            "create-phase12-evidence",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Daily,
        );
        phase12.writes = vec![evidence_state()];
        phase12.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(phase12, create_phase12_evidence)?;

        let mut publisher = publication_contract(
            "publish-all-committed-stages",
            BoundaryPhase::PerspectiveAndReportMaterialization,
        );
        publisher.reads = vec![evidence_state()];
        registrar.register_boundary_system(publisher, publish_from_all_committed_stages)
    }
}

#[test]
fn interleaved_publication_keeps_holder_local_ids_dense_and_phase13_writes_working() {
    let (scenario, _) = canwu_sim::demo_scenario();
    let plugin = InterleavedPublicationPlugin;
    let mut simulation = Simulation::new(101, scenario).expect("fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("fixture plugin should register");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("interleaved publication should settle");

    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let result = simulation
        .knowledge()
        .query_current(
            holder.clone(),
            &KnowledgeQuery {
                view: KnowledgeHistoryView::FullHistory,
                ..KnowledgeQuery::default()
            },
            None,
        )
        .expect("holder projection should query");
    assert_eq!(
        result
            .records
            .iter()
            .map(|record| record.id.get())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        simulation
            .knowledge()
            .for_holder(&holder)
            .expect("holder ledger should exist")
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![KnowledgeRecordId::new(1), KnowledgeRecordId::new(3)]
    );
    assert!(
        simulation
            .snapshot()
            .plugin_components
            .iter()
            .any(|component| {
                component.state == StateKey::new("fixture-kernel", "ordinary-phase13")
                    && component.value == json!(true)
            })
    );
}

#[test]
fn duplicate_correlation_and_later_failure_roll_back_every_publication_surface() {
    for plugin in [
        &DuplicateCorrelationPlugin as &dyn SimulationPlugin,
        &LateFailurePlugin as &dyn SimulationPlugin,
    ] {
        let (scenario, _) = canwu_sim::demo_scenario();
        let mut simulation = Simulation::new(103, scenario).expect("fixture should initialize");
        simulation
            .register_plugin(plugin)
            .expect("fixture plugin should register");
        let before = simulation.snapshot();
        let error = simulation
            .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
            .expect_err("invalid boundary must fail atomically");
        assert!(matches!(
            error.code,
            ErrorCode::InvalidKnowledgeRecord | ErrorCode::UndeclaredStateWrite
        ));
        assert_eq!(simulation.snapshot(), before);
    }
}

#[test]
fn phase13_publication_resolves_phase9_phase11_and_phase12_record_versions() {
    let (scenario, _) = canwu_sim::demo_scenario();
    let plugin = CommittedStageEvidencePlugin;
    let mut simulation = Simulation::new(107, scenario).expect("fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("stage-evidence plugin should register");
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily))
        .expect("phase 13 should resolve all earlier committed record stages");

    let snapshot = simulation.snapshot();
    assert_eq!(snapshot.boundaries[0].record_changes.len(), 3);
    let evidence = &snapshot
        .knowledge
        .for_holder(&KnowledgeHolderRef::Person(PersonId::new(1)))
        .and_then(|records| records.values().next())
        .expect("published record should exist")
        .origin
        .evidence;
    assert_eq!(evidence.len(), 3);
    Simulation::from_snapshot_with_plugins(snapshot, &[&plugin])
        .expect("all three exact evidence indexes should restore");
}

#[test]
fn forks_before_and_after_publication_remain_exact() {
    let (scenario, _) = canwu_sim::demo_scenario();
    let plugin = InterleavedPublicationPlugin;
    let mut original = Simulation::new(109, scenario).expect("fixture should initialize");
    original
        .register_plugin(&plugin)
        .expect("fixture plugin should register");
    let mut forked_before = original.fork();

    let first = BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    original
        .settle_boundary(first.clone())
        .expect("original first publication should settle");
    forked_before
        .settle_boundary(first)
        .expect("pre-publication fork should settle identically");
    assert_eq!(original.snapshot(), forked_before.snapshot());

    let mut forked_after = original.fork();
    let second = BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(1))
        .with_cadence(SystemCadence::Daily);
    original
        .settle_boundary(second.clone())
        .expect("original second publication should settle");
    forked_after
        .settle_boundary(second)
        .expect("post-publication fork should settle identically");
    assert_eq!(original.snapshot(), forked_after.snapshot());
}
