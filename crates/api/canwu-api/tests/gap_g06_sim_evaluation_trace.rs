//! Gap G-06: rule-evaluation traces. Phase-7 and phase-12 systems explain the
//! rule results they compute as hash-chained boundary evidence; a holder's
//! viewer returns only the subjects its knowledge permits; the run's declared
//! limits fail the boundary closed; and save/load and exact replay reproduce
//! the traces under strict loading.

#![allow(clippy::unnecessary_wraps)]

use canwu_api::{
    ArtifactManifest, BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase,
    BoundaryProposal, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, EntityRef,
    ErrorCode, EvaluationLimitsV1, EvaluationTerm, EvaluationTraceRecord, EvaluationTraceView,
    EvidenceRef, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeRecordDraft, KnowledgeRecordKind,
    KnowledgeSchemaId, KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget,
    KnowledgeSubjectTargetKind, KnowledgeWriteGrant, OrganizationId, PayloadSchema, PersonId,
    PluginKnowledgeSchema, PluginRegistrar, RunConfiguration, RunManifest, Scenario, SimDuration,
    SimTime, SimulationPlugin, SimulationView, StateVisibility, SystemCadence,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-evaluation";
const LIMITS: EvaluationLimitsV1 = EvaluationLimitsV1 {
    traces_per_boundary: 4,
    terms_per_trace: 3,
};

const fn holder() -> PersonId {
    PersonId::new(1)
}

const fn rival() -> EntityRef {
    EntityRef::Person(PersonId::new(2))
}

const fn known_office() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(10))
}

const fn hidden_office() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(11))
}

fn sighting_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new("fixture.evaluation", "sighting"),
        1,
    )
}

/// A trace of `rule` for `subject`, citing the previous boundary as evidence.
fn trace(
    context: &BoundaryContext,
    rule: &str,
    subject: EntityRef,
    terms: &[(&str, i64)],
) -> BoundaryDirective {
    let evidence: Vec<_> = (context.boundary_id.get() > 1)
        .then(|| EvidenceRef::Boundary(BoundaryId::new(context.boundary_id.get() - 1)))
        .into_iter()
        .collect();
    BoundaryDirective::RecordEvaluationTrace {
        trace: EvaluationTraceRecord {
            rule_id: rule.to_owned(),
            rule_version: "1".to_owned(),
            subject,
            terms: terms
                .iter()
                .map(|(term_id, contribution)| EvaluationTerm {
                    term_id: (*term_id).to_owned(),
                    contribution: *contribution,
                    evidence: evidence.clone(),
                })
                .collect(),
            result: terms.iter().map(|(_, contribution)| contribution).sum(),
            boundary: context.boundary_id,
        },
    }
}

fn proposal(directives: Vec<BoundaryDirective>) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Phase 7: three levy evaluations whose base grows with the boundary.
fn assess_levy(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let base = i64::try_from(context.boundary_id.get()).expect("small boundary ID") * 10;
    proposal(
        [known_office(), hidden_office(), rival()]
            .into_iter()
            .map(|subject| {
                trace(
                    context,
                    "fixture.levy",
                    subject,
                    &[("base", base), ("remission", -3)],
                )
            })
            .collect(),
    )
}

/// Phase 12: one standing aggregation of the holder itself.
fn aggregate_standing(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    proposal(vec![trace(
        context,
        "fixture.standing",
        EntityRef::Person(holder()),
        &[("office", 5), ("kin", 2), ("rumour", -1)],
    )])
}

/// Monthly phase-12 system whose extra trace exceeds the boundary total.
fn surge(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    proposal(vec![trace(
        context,
        "fixture.surge",
        known_office(),
        &[("surge", 1)],
    )])
}

/// Seasonal phase-7 system whose single trace exceeds the per-trace terms.
fn census(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    proposal(vec![trace(
        context,
        "fixture.census",
        known_office(),
        &[("north", 1), ("south", 1), ("east", 1), ("west", 1)],
    )])
}

/// Phase 13: at boundary 2 the holder learns of the known office.
fn report_sighting(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 2 {
        return proposal(Vec::new());
    }
    proposal(vec![BoundaryDirective::PublishKnowledge {
        holder: KnowledgeHolderRef::Person(holder()),
        visibility: StateVisibility::SameBoundary,
        producer_correlation: None,
        records: vec![KnowledgeRecordDraft {
            schema: sighting_schema(),
            subjects: vec![KnowledgeSubject {
                role: "subject".to_owned(),
                target: KnowledgeSubjectTarget::Entity(known_office()),
            }],
            payload: json!({ "seen": true }),
            as_of: None,
            confidence_per_mille: 1_000,
            origin: KnowledgeOrigin {
                method: "fixture.report".to_owned(),
                evidence: Vec::new(),
            },
            supersedes: Vec::new(),
            contradicts: Vec::new(),
        }],
        summary: "The holder learned of the office".to_owned(),
    }])
}

struct EvaluationPlugin;

impl SimulationPlugin for EvaluationPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000606"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: sighting_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000000607"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "subject".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
        })?;
        for (name, phase, cadence, handler) in [
            (
                "levy",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Daily,
                assess_levy as canwu_api::BoundarySystemHandler,
            ),
            (
                "standing",
                BoundaryPhase::StrategicAggregation,
                SystemCadence::Daily,
                aggregate_standing,
            ),
            (
                "surge",
                BoundaryPhase::StrategicAggregation,
                SystemCadence::Monthly,
                surge,
            ),
            (
                "census",
                BoundaryPhase::DomainDeltaProposal,
                SystemCadence::Seasonal,
                census,
            ),
        ] {
            registrar.register_boundary_system(
                BoundarySystemContract::new(name, phase, cadence),
                handler,
            )?;
        }
        let mut sighting = BoundarySystemContract::new(
            "sighting",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        sighting.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: sighting_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(sighting, report_sighting)
    }
}

fn configuration() -> RunConfiguration {
    RunConfiguration::play_as_character("seat.fixture", "controller.fixture", holder(), "profile")
        .with_evaluation_limits(LIMITS)
}

fn new_run(plugin: &EvaluationPlugin) -> Canwu {
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![
            known_office(),
            hidden_office(),
            EntityRef::Person(holder()),
            rival(),
        ],
    );
    let configuration = configuration();
    let manifest = RunManifest::declared(
        ArtifactManifest::for_scenario("fixture", "evaluation", "1", &scenario).expect("scenario"),
        ArtifactManifest::for_run_configuration("fixture", "holder-seat", "1", &configuration)
            .expect("configuration"),
    );
    Canwu::new_with_run_configuration_and_plugins(606, scenario, manifest, configuration, &[plugin])
        .expect("declared run")
}

fn at_day(day: i64, cadences: &[SystemCadence]) -> BoundaryRequest {
    cadences.iter().fold(
        BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(day)),
        |request, cadence| request.with_cadence(cadence.clone()),
    )
}

fn visible(canwu: &Canwu, subject: &EntityRef, after: Option<u64>) -> Vec<EvaluationTraceView> {
    canwu
        .viewer()
        .expect("holder viewer")
        .evaluation_traces(subject, after.map(BoundaryId::new))
        .expect("holder trace read")
}

fn boundaries_of(traces: &[EvaluationTraceView]) -> Vec<u64> {
    traces.iter().map(|trace| trace.boundary.get()).collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g06_sim_evaluation_trace() {
    // Default limits keep the run configuration's canonical encoding.
    let plain =
        RunConfiguration::play_as_character("seat.fixture", "controller.fixture", holder(), "p");
    assert_eq!(plain.evaluation_limits, EvaluationLimitsV1::DEFAULT);
    assert!(
        serde_json::to_value(&plain)
            .expect("configuration json")
            .get("evaluation_limits")
            .is_none()
    );

    let plugin = EvaluationPlugin;
    let mut canwu = new_run(&plugin);
    for day in 1..=2 {
        canwu
            .settle_boundary(at_day(day, &[SystemCadence::Daily]))
            .expect("daily boundary at the trace limit");
    }

    // Phase-7 and phase-12 traces are boundary evidence with producer
    // provenance, in system execution order.
    let recorded = &canwu.boundaries()[1].evaluation_traces;
    assert_eq!(
        recorded
            .iter()
            .map(|entry| (
                entry.system.as_str(),
                entry.phase,
                entry.trace.subject.clone()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("levy", BoundaryPhase::DomainDeltaProposal, known_office()),
            ("levy", BoundaryPhase::DomainDeltaProposal, hidden_office()),
            ("levy", BoundaryPhase::DomainDeltaProposal, rival()),
            (
                "standing",
                BoundaryPhase::StrategicAggregation,
                EntityRef::Person(holder())
            ),
        ]
    );
    assert_eq!(recorded[0].trace.result, 17);
    assert_eq!(
        recorded[0].trace.terms[0].evidence,
        vec![EvidenceRef::Boundary(BoundaryId::new(1))]
    );

    // Limits fail the boundary closed and deterministically: one trace over
    // the boundary total, or one term over the per-trace bound. Each retry
    // fails the same way and commits nothing.
    let before = (canwu.revision(), canwu.checkpoint_hash().to_owned());
    for request in [
        at_day(3, &[SystemCadence::Daily, SystemCadence::Monthly]),
        at_day(3, &[SystemCadence::Seasonal]),
    ] {
        for _retry in 0..2 {
            let error = canwu
                .settle_boundary(request.clone())
                .expect_err("over-limit proposals must fail the boundary");
            assert_eq!(error.code, ErrorCode::EvaluationTraceLimitExceeded);
            assert_eq!(
                (canwu.revision(), canwu.checkpoint_hash().to_owned()),
                before
            );
            assert_eq!(canwu.boundaries().len(), 2);
        }
    }
    for day in 3..=4 {
        canwu
            .settle_boundary(at_day(day, &[SystemCadence::Daily]))
            .expect("the run continues after a rejected boundary");
    }

    // The holder sees its own subject from every boundary. It learned of the
    // known office in phase 13 of boundary 2, after that boundary's phase-7
    // evaluation, so it sees that office's breakdowns only from boundary 3;
    // it sees nothing of the hidden office or the rival. The holder view
    // omits evidence, and a trusted read keeps every full trace.
    let own = EntityRef::Person(holder());
    assert_eq!(
        boundaries_of(&visible(&canwu, &own, None)),
        vec![1, 2, 3, 4]
    );
    assert_eq!(boundaries_of(&visible(&canwu, &own, Some(2))), vec![3, 4]);
    let known = visible(&canwu, &known_office(), None);
    assert_eq!(boundaries_of(&known), vec![3, 4]);
    assert_eq!(
        known[0],
        EvaluationTraceView::from(&canwu.boundaries()[2].evaluation_traces[0].trace)
    );
    assert!(
        serde_json::to_value(&known[0])
            .expect("view json")
            .to_string()
            .find("evidence")
            .is_none()
    );
    assert!(visible(&canwu, &hidden_office(), None).is_empty());
    assert!(visible(&canwu, &rival(), None).is_empty());
    assert_eq!(
        canwu
            .boundaries()
            .iter()
            .flat_map(|boundary| &boundary.evaluation_traces)
            .filter(|entry| entry.trace.subject == hidden_office())
            .count(),
        4
    );

    // Save/load and exact replay reproduce the traces and the holder reads.
    let json = canwu.snapshot_json().expect("snapshot json");
    let restored = Canwu::from_snapshot_json_with_plugins(&json, &[&plugin]).expect("restore");
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(visible(&restored, &known_office(), None), known);
    let journal = serde_json::to_string(&canwu.replay_journal()).expect("journal json");
    let replayed = Canwu::replay_from_journal_json(&[&plugin], &journal).expect("exact replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    assert_eq!(visible(&replayed, &known_office(), None), known);

    // Traces are committed by the boundary hash chain, and strict loading
    // rejects an unknown field inside a recorded trace.
    let reload = |edit: &dyn Fn(&mut Value)| {
        let mut tampered: Value = serde_json::from_str(&json).expect("snapshot value");
        edit(&mut tampered["boundaries"][1]["evaluation_traces"][0]["trace"]["terms"][0]);
        Canwu::from_snapshot_json_with_plugins(
            &serde_json::to_string(&tampered).expect("tampered json"),
            &[&plugin],
        )
        .err()
        .expect("a tampered trace must be rejected")
    };
    let error = reload(&|term| term["contribution"] = json!(999));
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("hash chain"), "{}", error.message);
    let error = reload(&|term| term["weight"] = json!(1));
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    assert!(error.message.contains("unknown field"), "{}", error.message);
}
