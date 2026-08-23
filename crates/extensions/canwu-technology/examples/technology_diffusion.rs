use canwu_api::{
    BoundaryRequest, Canwu, Command, CommandEnvelope, CommandRequest, CommandRequestId,
    DomainRecordVersionRef, DomainRecordVersionSource, EntityRef, EvidenceRef, Government,
    GovernmentId, Issuer, KnowledgeHolderRef, KnowledgeSnapshot, MapPoint, Person, PersonId,
    PluginIngressRequest, Scenario, SimTime, Territory, TerritoryId, WorldSnapshot,
};
use canwu_technology::{
    AdoptionPayload, AdoptionRecord, AdoptionStatus, ApplicationSpec, ApplicationSpecPayload,
    AttemptObservation, AttemptObservationPayload, CapabilityQualification,
    CapabilityQualificationPayload, ExperimentAttempt, ExperimentAttemptPayload,
    ImplementationPayload, ImplementationRecord, MetricComparison, MetricContext, MetricSchema,
    MetricSchemaPayload, MetricThreshold, MetricValue, ProgramMode, ProgramStatus,
    QualificationRule, REFERENCE_EVALUATOR_V1, RequirementGroup, TECHNOLOGY_COMMAND,
    TECHNOLOGY_RESULT_INGRESS, TechnicalProgram, TechnicalProgramPayload, TechniqueRevision,
    TechniqueRevisionPayload, TechniqueSpec, TechniqueSpecPayload, TechnologyCatalogRecord,
    TechnologyCommandEnvelope, TechnologyExecutionIntent, TechnologyExecutionIntentPayload,
    TechnologyIntentRequest, TechnologyIntentState, TechnologyPlugin, TechnologyRecordChange,
    TechnologyRecordPayload, TechnologyResultEnvelope, TransmissionMode,
    TransmissionOpportunityPayload, evaluate_application, evaluate_attempt,
    from_technology_snapshot_json, initial_record_version, replay_technology_from_journal,
};
use std::collections::BTreeMap;
use std::error::Error;

// This executable is deliberately linear so each public API step is visible.
#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn Error>> {
    let plugin = TechnologyPlugin;
    let (scenario, operator, learner, workshop, destination, catalog) = scenario()?;
    let mut canwu = Canwu::new_with_plugins(41, scenario.clone(), &[&plugin])?;

    apply_command(
        &mut canwu,
        operator,
        1,
        TechnologyCommandEnvelope {
            id: "start-program".to_owned(),
            subject: KnowledgeHolderRef::Person(operator),
            change: TechnologyRecordChange::Create {
                id: "program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    sponsor: KnowledgeHolderRef::Person(operator),
                    site: EntityRef::Territory(workshop),
                    revision: Some(catalog.revision.clone()),
                    mode: ProgramMode::Adaptation,
                    status: ProgramStatus::Active,
                    requirements: Vec::new(),
                    started_at: SimTime::EPOCH,
                    due_at: None,
                }),
            },
        },
    )?;
    let program = current_version::<TechnicalProgram>(&canwu, "program")?;
    let attempt_intent = authorize_intent(
        &mut canwu,
        operator,
        100,
        "attempt-intent",
        program.clone(),
        "reference-lab",
        TechnologyIntentRequest::Experiment {
            result_id: "attempt".to_owned(),
            revision: catalog.revision.clone(),
            operation: "operate".to_owned(),
            site: EntityRef::Territory(workshop),
            operator: Some(KnowledgeHolderRef::Person(operator)),
            required_assets: Vec::new(),
        },
    )?;

    let attempt_metrics = vec![MetricValue {
        metric: catalog.reliability.clone(),
        value: 820,
    }];
    let evaluation = evaluate_attempt(
        &catalog.revision_payload,
        &catalog.technique,
        &BTreeMap::from([(
            catalog.reliability.record.clone(),
            catalog.reliability_schema,
        )]),
        &MetricContext {
            values: BTreeMap::from([(catalog.reliability.record.clone(), 820)]),
        },
    )?;
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "provider-attempt".to_owned(),
            provider: "reference-lab".to_owned(),
            execution_intent: Some(attempt_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                    execution_intent: attempt_intent,
                    program,
                    revision: catalog.revision.clone(),
                    operator: KnowledgeHolderRef::Person(operator),
                    site: EntityRef::Territory(workshop),
                    operation: "operate".to_owned(),
                    inputs: Vec::new(),
                    environment: Vec::new(),
                    outputs: attempt_metrics,
                    assets: Vec::new(),
                    started_at: SimTime::EPOCH,
                    ended_at: SimTime::EPOCH,
                    evaluation,
                }),
            },
        },
    )?;
    let attempt = current_version::<ExperimentAttempt>(&canwu, "attempt")?;

    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "observe-fuel".to_owned(),
            provider: "meter-reader".to_owned(),
            execution_intent: None,
            change: TechnologyRecordChange::Create {
                id: "fuel-observation".to_owned(),
                value: TechnologyRecordPayload::AttemptObservation(AttemptObservationPayload {
                    attempt: attempt.clone(),
                    observer: KnowledgeHolderRef::Person(operator),
                    method: "metered fuel accounting".to_owned(),
                    values: vec![MetricValue {
                        metric: catalog.fuel_cost.clone(),
                        value: 400,
                    }],
                    uncertainty_per_mille: 50,
                    observed_at: SimTime::EPOCH,
                }),
            },
        },
    )?;
    let viability_evidence = current_version::<AttemptObservation>(&canwu, "fuel-observation")?;

    apply_command(
        &mut canwu,
        operator,
        2,
        TechnologyCommandEnvelope {
            id: "qualify-practice".to_owned(),
            subject: KnowledgeHolderRef::Person(operator),
            change: TechnologyRecordChange::Create {
                id: "capability".to_owned(),
                value: TechnologyRecordPayload::Capability(CapabilityQualificationPayload {
                    holder: KnowledgeHolderRef::Person(operator),
                    operator: Some(EntityRef::Person(operator)),
                    site: EntityRef::Territory(workshop),
                    revision: catalog.revision.clone(),
                    operation: "operate".to_owned(),
                    reliability_per_mille: 1_000,
                    attempts: vec![attempt],
                    last_practiced_at: SimTime::EPOCH,
                    valid_from: SimTime::EPOCH,
                    valid_until: None,
                    active: true,
                }),
            },
        },
    )?;
    let qualification = current_version::<CapabilityQualification>(&canwu, "capability")?;

    apply_command(
        &mut canwu,
        operator,
        3,
        TechnologyCommandEnvelope {
            id: "install".to_owned(),
            subject: KnowledgeHolderRef::Person(operator),
            change: TechnologyRecordChange::Create {
                id: "implementation".to_owned(),
                value: TechnologyRecordPayload::Implementation(ImplementationPayload {
                    owner: KnowledgeHolderRef::Person(operator),
                    site: EntityRef::Territory(workshop),
                    revision: catalog.revision.clone(),
                    qualification,
                    assets: Vec::new(),
                    installed_at: SimTime::EPOCH,
                    capacity: 10,
                    unit: "runs_per_month".to_owned(),
                    reliability_per_mille: 1_000,
                    maintenance_provider: Some(KnowledgeHolderRef::Person(operator)),
                    active: true,
                }),
            },
        },
    )?;
    let implementation = current_version::<ImplementationRecord>(&canwu, "implementation")?;

    let viability_metrics = vec![MetricValue {
        metric: catalog.fuel_cost.clone(),
        value: 400,
    }];
    let viability = evaluate_application(
        &catalog.application,
        &BTreeMap::from([(catalog.fuel_cost.record.clone(), catalog.fuel_schema)]),
        &MetricContext {
            values: BTreeMap::from([(catalog.fuel_cost.record.clone(), 400)]),
        },
    )?;
    let decision_evidence = EvidenceRef::Boundary(
        canwu
            .boundaries()
            .last()
            .ok_or("installation boundary evidence is missing")?
            .id,
    );
    apply_command(
        &mut canwu,
        operator,
        4,
        TechnologyCommandEnvelope {
            id: "adopt-use".to_owned(),
            subject: KnowledgeHolderRef::Person(operator),
            change: TechnologyRecordChange::Create {
                id: "adoption".to_owned(),
                value: TechnologyRecordPayload::Adoption(AdoptionPayload {
                    adopter: KnowledgeHolderRef::Person(operator),
                    site: EntityRef::Territory(workshop),
                    application: catalog.application_ref,
                    implementations: vec![implementation.clone()],
                    status: AdoptionStatus::Committed,
                    scale: 10,
                    decision_evidence,
                    viability_evidence: vec![viability_evidence],
                    viability_metrics,
                    viability,
                }),
            },
        },
    )?;

    apply_command(
        &mut canwu,
        learner,
        5,
        TechnologyCommandEnvelope {
            id: "open-learner-program".to_owned(),
            subject: KnowledgeHolderRef::Person(learner),
            change: TechnologyRecordChange::Create {
                id: "learner-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    sponsor: KnowledgeHolderRef::Person(learner),
                    site: EntityRef::Territory(destination),
                    revision: Some(catalog.revision.clone()),
                    mode: ProgramMode::Training,
                    status: ProgramStatus::Active,
                    requirements: Vec::new(),
                    started_at: SimTime::EPOCH,
                    due_at: None,
                }),
            },
        },
    )?;
    let learner_program = current_version::<TechnicalProgram>(&canwu, "learner-program")?;

    apply_command(
        &mut canwu,
        operator,
        6,
        TechnologyCommandEnvelope {
            id: "open-apprenticeship".to_owned(),
            subject: KnowledgeHolderRef::Person(operator),
            change: TechnologyRecordChange::Create {
                id: "teaching".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(operator)),
                    source_site: Some(EntityRef::Territory(workshop)),
                    source_capability: Some(implementation),
                    destination: KnowledgeHolderRef::Person(learner),
                    destination_site: EntityRef::Territory(destination),
                    revision: Some(catalog.revision),
                    mode: TransmissionMode::Apprenticeship,
                    evidence: Vec::new(),
                    resulting_program: Some(learner_program),
                    opened_at: SimTime::EPOCH,
                    active: true,
                }),
            },
        },
    )?;

    let adoption = canwu
        .domain_record(
            &canwu_api::TypedDomainRecordRef::<AdoptionRecord>::new("adoption").into_untyped(),
        )
        .ok_or("adoption did not commit")?
        .decode_payload::<AdoptionRecord>()?;
    let learner_knowledge = canwu
        .knowledge()
        .for_holder(&KnowledgeHolderRef::Person(learner))
        .map_or(0, BTreeMap::len);
    println!(
        "adoption={:?}, scale={}, learner_knowledge={} (opportunity is not automatic learning)",
        adoption.status, adoption.scale, learner_knowledge
    );

    let snapshot = canwu.snapshot_json()?;
    let restored = from_technology_snapshot_json(&snapshot, &[&plugin])?;
    let replayed = replay_technology_from_journal(scenario, &[&plugin], &canwu.replay_journal())?;
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    println!("snapshot restore and exact replay match");
    Ok(())
}

struct Catalog {
    reliability: DomainRecordVersionRef,
    fuel_cost: DomainRecordVersionRef,
    revision: DomainRecordVersionRef,
    application_ref: DomainRecordVersionRef,
    revision_payload: TechniqueRevisionPayload,
    technique: TechniqueSpecPayload,
    application: ApplicationSpecPayload,
    reliability_schema: MetricSchemaPayload,
    fuel_schema: MetricSchemaPayload,
}

type ExampleScenario = (
    Scenario,
    PersonId,
    PersonId,
    TerritoryId,
    TerritoryId,
    Catalog,
);

// Catalog construction stays together so its exact-version links are easy to audit.
#[allow(clippy::too_many_lines)]
fn scenario() -> Result<ExampleScenario, Box<dyn Error>> {
    let operator = PersonId::new(1);
    let learner = PersonId::new(2);
    let government = GovernmentId::new(1);
    let workshop = TerritoryId::new(1);
    let destination = TerritoryId::new(2);
    let reliability = initial_record_version::<MetricSchema>("reliability");
    let fuel_cost = initial_record_version::<MetricSchema>("fuel-cost");
    let technique_ref = initial_record_version::<TechniqueSpec>("technique");
    let revision = initial_record_version::<TechniqueRevision>("revision");
    let application_ref = initial_record_version::<ApplicationSpec>("application");
    let reliability_schema = MetricSchemaPayload {
        label: "reliability".to_owned(),
        unit: "permille".to_owned(),
        scale: 1_000,
        minimum: 0,
        maximum: 1_000,
    };
    let fuel_schema = MetricSchemaPayload {
        label: "fuel cost".to_owned(),
        unit: "cost_units".to_owned(),
        scale: 1,
        minimum: 0,
        maximum: 1_000,
    };
    let technique = TechniqueSpecPayload {
        label: "neutral pressure converter".to_owned(),
        function: "convert pressure into useful work".to_owned(),
        requirements: vec![RequirementGroup {
            id: "reliable_output".to_owned(),
            any_of: vec![MetricThreshold {
                id: "reliability_floor".to_owned(),
                metric: reliability.clone(),
                comparison: MetricComparison::AtLeast,
                value: 700,
            }],
        }],
        qualification_rules: vec![QualificationRule {
            operation: "operate".to_owned(),
            minimum_successful_attempts: 1,
            minimum_reliability_per_mille: 700,
            independent_reproduction_required: false,
        }],
    };
    let application = ApplicationSpecPayload {
        label: "drain a deep working".to_owned(),
        technique: technique_ref.clone(),
        viability: vec![RequirementGroup {
            id: "fuel_budget".to_owned(),
            any_of: vec![MetricThreshold {
                id: "affordable_fuel".to_owned(),
                metric: fuel_cost.clone(),
                comparison: MetricComparison::AtMost,
                value: 500,
            }],
        }],
    };
    let records = vec![
        TechnologyCatalogRecord::Metric(reliability_schema.clone())
            .into_initial_record("reliability")?,
        TechnologyCatalogRecord::Metric(fuel_schema.clone()).into_initial_record("fuel-cost")?,
        TechnologyCatalogRecord::Technique(technique.clone()).into_initial_record("technique")?,
        TechnologyCatalogRecord::Revision(TechniqueRevisionPayload {
            label: "neutral revision".to_owned(),
            spec: technique_ref.clone(),
            parents: Vec::new(),
            parameters: Vec::new(),
            evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
            produced_by: None,
            execution_intent: None,
            discovery_evidence: Vec::new(),
        })
        .into_initial_record("revision")?,
        TechnologyCatalogRecord::Application(application.clone())
            .into_initial_record("application")?,
    ];
    let scenario = Scenario {
        start_time: SimTime::EPOCH,
        world: WorldSnapshot {
            people: vec![
                Person {
                    id: operator,
                    name: "Operator".to_owned(),
                    government,
                    current_location: workshop,
                    roles: Vec::new(),
                    transit: None,
                },
                Person {
                    id: learner,
                    name: "Learner".to_owned(),
                    government,
                    current_location: destination,
                    roles: Vec::new(),
                    transit: None,
                },
            ],
            governments: vec![Government {
                id: government,
                name: "Workshop authority".to_owned(),
                capital: workshop,
            }],
            territories: vec![
                Territory {
                    id: workshop,
                    name: "Workshop".to_owned(),
                    controller: government,
                    position: MapPoint::default(),
                },
                Territory {
                    id: destination,
                    name: "Destination".to_owned(),
                    controller: government,
                    position: MapPoint { x: 1.0, y: 0.0 },
                },
            ],
            routes: Vec::new(),
            armies: Vec::new(),
            letters: Vec::new(),
        },
        knowledge: KnowledgeSnapshot::default(),
        domain_records: records,
    };
    Ok((
        scenario,
        operator,
        learner,
        workshop,
        destination,
        Catalog {
            reliability,
            fuel_cost,
            revision,
            application_ref,
            revision_payload: TechniqueRevisionPayload {
                label: "neutral revision".to_owned(),
                spec: technique_ref,
                parents: Vec::new(),
                parameters: Vec::new(),
                evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                produced_by: None,
                execution_intent: None,
                discovery_evidence: Vec::new(),
            },
            technique,
            application,
            reliability_schema,
            fuel_schema,
        },
    ))
}

fn apply_command(
    canwu: &mut Canwu,
    actor: PersonId,
    request_id: u64,
    envelope: TechnologyCommandEnvelope,
) -> Result<(), Box<dyn Error>> {
    canwu.enqueue_command(
        canwu.time(),
        0,
        CommandRequest::new(
            CommandRequestId::new(request_id),
            canwu.revision(),
            CommandEnvelope::new(
                Issuer::Actor(actor),
                Command::Plugin {
                    plugin: "canwu-technology".to_owned(),
                    command: TECHNOLOGY_COMMAND.to_owned(),
                    payload: serde_json::to_value(envelope)?,
                },
            )
            .at_time(canwu.time()),
        ),
    )?;
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    Ok(())
}

fn apply_result(
    canwu: &mut Canwu,
    envelope: TechnologyResultEnvelope,
) -> Result<(), Box<dyn Error>> {
    canwu.enqueue_plugin_ingress(PluginIngressRequest::new(
        "canwu-technology",
        TECHNOLOGY_RESULT_INGRESS,
        canwu.time(),
        serde_json::to_value(envelope)?,
    ))?;
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn authorize_intent(
    canwu: &mut Canwu,
    actor: PersonId,
    request_id: u64,
    id: &str,
    program: DomainRecordVersionRef,
    provider: &str,
    request: TechnologyIntentRequest,
) -> Result<DomainRecordVersionRef, Box<dyn Error>> {
    apply_command(
        canwu,
        actor,
        request_id,
        TechnologyCommandEnvelope {
            id: format!("authorize-{id}"),
            subject: KnowledgeHolderRef::Person(actor),
            change: TechnologyRecordChange::Create {
                id: id.to_owned(),
                value: TechnologyRecordPayload::ExecutionIntent(TechnologyExecutionIntentPayload {
                    authorized_by: KnowledgeHolderRef::Person(actor),
                    program,
                    provider: provider.to_owned(),
                    request,
                    not_before: canwu.time(),
                    expires_at: None,
                    state: TechnologyIntentState::Pending,
                }),
            },
        },
    )?;
    current_version::<TechnologyExecutionIntent>(canwu, id)
}

fn current_version<T: canwu_api::DomainRecordType>(
    canwu: &Canwu,
    id: &str,
) -> Result<DomainRecordVersionRef, Box<dyn Error>> {
    let reference = canwu_api::TypedDomainRecordRef::<T>::new(id).into_untyped();
    let record = canwu
        .domain_record(&reference)
        .ok_or_else(|| format!("technology record {reference} was not created"))?;
    for boundary in canwu.boundaries().iter().rev() {
        for (change_index, change) in boundary.record_changes.iter().enumerate().rev() {
            if change.current.reference == reference && change.current.version == record.version {
                return Ok(DomainRecordVersionRef {
                    record: reference,
                    version: record.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: u64::try_from(change_index)?,
                    },
                });
            }
        }
    }
    Err(format!("technology record {reference} lacks version evidence").into())
}
