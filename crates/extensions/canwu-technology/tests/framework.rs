use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, BoundaryRequest, Canwu, CanwuError,
    Command, CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu, DomainRecord,
    DomainRecordClass, DomainRecordLifecycle, DomainRecordRef, DomainRecordVersionRef,
    DomainRecordVersionSource, EntityRef, ErrorCode, EvidenceJournalSegment, EvidenceRef,
    Government, GovernmentId, Issuer, KnowledgeHolderRef, KnowledgeQuery, KnowledgeSnapshot,
    MapPoint, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadRequiredEvidenceContinuationV1,
    Person, PersonId, PluginIngressRequest, Scenario, SimTime, SimulationPlugin, Territory,
    TerritoryId, WorldSnapshot,
};
use canwu_technology::{
    AdoptionPayload, AdoptionStatus, ApplicationSpec, ApplicationSpecPayload, AssetBinding,
    AssetBindingPayload, AttemptObservation, AttemptObservationPayload, CapabilityQualification,
    CapabilityQualificationPayload, ClaimAssessment, ClaimAssessmentPayload, ExperimentAttempt,
    ExperimentAttemptPayload, ImplementationPayload, ImplementationRecord, MetricComparison,
    MetricContext, MetricSchema, MetricSchemaPayload, MetricThreshold, MetricValue, ProgramMode,
    ProgramStatus, ProviderRequirement, QualificationRule, REFERENCE_EVALUATOR_V1,
    RequirementGroup, TECHNOLOGY_COMMAND, TECHNOLOGY_RESULT_INGRESS, TechnicalClaimPayload,
    TechnicalProgram, TechnicalProgramPayload, TechniqueRevision, TechniqueRevisionPayload,
    TechniqueSpec, TechniqueSpecPayload, TechnologyCatalogRecord, TechnologyCommandEnvelope,
    TechnologyExecutionIntent, TechnologyExecutionIntentPayload, TechnologyIntentRequest,
    TechnologyIntentState, TechnologyLimitsV1, TechnologyOperation, TechnologyOperationPayload,
    TechnologyOperationStatus, TechnologyPlugin, TechnologyRecordChange, TechnologyRecordPayload,
    TechnologyRecordSet, TechnologyResultEnvelope, TransmissionMode, TransmissionOpportunity,
    TransmissionOpportunityPayload, evaluate_application, evaluate_attempt,
    from_technology_checkpoint_journal, from_technology_snapshot_json, initial_record_version,
    replay_technology_from_journal, validate_technology_runtime,
};
use std::cell::RefCell;
use std::collections::BTreeMap;

#[derive(Default)]
struct TestArchive {
    segments: RefCell<BTreeMap<String, EvidenceJournalSegment>>,
}

impl ArchiveProvider for TestArchive {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        Ok(self.segments.borrow().get(segment_id).cloned())
    }
}

impl ArchiveStore for TestArchive {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError> {
        let segment_id = segment
            .archive
            .as_ref()
            .ok_or_else(|| CanwuError::new(ErrorCode::InvalidArchive, "missing archive index"))?
            .header
            .segment_id
            .clone();
        let mut segments = self.segments.borrow_mut();
        if let Some(existing) = segments.get(&segment_id) {
            return if existing == segment {
                Ok(ArchiveStoreOutcome::AlreadyPresent)
            } else {
                Err(CanwuError::new(
                    ErrorCode::InvalidArchive,
                    "segment ID is already bound to different bytes",
                ))
            };
        }
        segments.insert(segment_id, segment.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

#[derive(Clone)]
struct Profile {
    label: &'static str,
    metric: &'static str,
    threshold: i64,
    success: i64,
    failure: i64,
}

#[test]
#[allow(clippy::too_many_lines)]
fn five_technology_profiles_use_one_evaluator_contract() {
    let profiles = [
        Profile {
            label: "papermaking",
            metric: "fiber_process",
            threshold: 700,
            success: 820,
            failure: 520,
        },
        Profile {
            label: "woodblock",
            metric: "edition_fit",
            threshold: 600,
            success: 900,
            failure: 200,
        },
        Profile {
            label: "movable_type",
            metric: "script_inventory",
            threshold: 800,
            success: 950,
            failure: 450,
        },
        Profile {
            label: "gunpowder",
            metric: "material_purity",
            threshold: 750,
            success: 880,
            failure: 300,
        },
        Profile {
            label: "steam_engine",
            metric: "sealing_reliability",
            threshold: 700,
            success: 810,
            failure: 480,
        },
    ];

    for (index, profile) in profiles.iter().enumerate() {
        let metric_id = format!("metric-{index}");
        let metric_ref = initial_record_version::<MetricSchema>(&metric_id);
        let spec = TechniqueSpecPayload {
            label: profile.label.to_owned(),
            function: "data-driven reference evaluation".to_owned(),
            requirements: vec![RequirementGroup {
                id: "local_condition".to_owned(),
                any_of: vec![MetricThreshold {
                    id: profile.metric.to_owned(),
                    metric: metric_ref.clone(),
                    comparison: MetricComparison::AtLeast,
                    value: profile.threshold,
                }],
            }],
            qualification_rules: vec![],
        };
        let schemas = BTreeMap::from([(
            metric_ref.record.clone(),
            MetricSchemaPayload {
                label: profile.metric.to_owned(),
                unit: "permille".to_owned(),
                scale: 1_000,
                minimum: 0,
                maximum: 1_000,
            },
        )]);
        let revision = TechniqueRevisionPayload {
            label: format!("{} revision", profile.label),
            spec: initial_record_version::<TechniqueSpec>(profile.label),
            parents: vec![],
            parameters: vec![],
            evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
            produced_by: None,
            execution_intent: None,
            discovery_evidence: vec![],
        };
        let succeeds = evaluate_attempt(
            &revision,
            &spec,
            &schemas,
            &MetricContext {
                values: BTreeMap::from([(metric_ref.record.clone(), profile.success)]),
            },
        )
        .expect("profile should evaluate");
        let fails = evaluate_attempt(
            &revision,
            &spec,
            &schemas,
            &MetricContext {
                values: BTreeMap::from([(metric_ref.record.clone(), profile.failure)]),
            },
        )
        .expect("profile should evaluate");
        assert!(succeeds.passed, "{} success profile", profile.label);
        assert!(!fails.passed, "{} counterfactual profile", profile.label);

        let actor = PersonId::new(1);
        let government = GovernmentId::new(1);
        let site = TerritoryId::new(1);
        let technique_id = format!("{}-technique", profile.label);
        let revision_id = format!("{}-revision", profile.label);
        let runtime_spec = initial_record_version::<TechniqueSpec>(&technique_id);
        let runtime_revision = initial_record_version::<TechniqueRevision>(&revision_id);
        let runtime_revision_payload = TechniqueRevisionPayload {
            spec: runtime_spec.clone(),
            ..revision.clone()
        };
        let scenario = Scenario {
            start_time: SimTime::EPOCH,
            world: WorldSnapshot {
                people: vec![Person {
                    id: actor,
                    name: "Operator".to_owned(),
                    government,
                    current_location: site,
                    roles: vec![],
                    transit: None,
                }],
                governments: vec![Government {
                    id: government,
                    name: "Workshop".to_owned(),
                    capital: site,
                }],
                territories: vec![Territory {
                    id: site,
                    name: "Site".to_owned(),
                    controller: government,
                    position: MapPoint::default(),
                }],
                routes: vec![],
                armies: vec![],
                letters: vec![],
            },
            knowledge: KnowledgeSnapshot::default(),
            domain_records: vec![
                TechnologyCatalogRecord::Metric(schemas[&metric_ref.record].clone())
                    .into_initial_record(&metric_id)
                    .expect("metric catalog record"),
                TechnologyCatalogRecord::Technique(spec.clone())
                    .into_initial_record(&technique_id)
                    .expect("technique catalog record"),
                TechnologyCatalogRecord::Revision(runtime_revision_payload.clone())
                    .into_initial_record(&revision_id)
                    .expect("revision catalog record"),
            ],
        };
        let mut canwu = Canwu::new_with_plugins(100 + index as u64, scenario, &[&TechnologyPlugin])
            .expect("profile runtime should initialize");
        apply_command(
            &mut canwu,
            actor,
            1,
            TechnologyCommandEnvelope {
                id: "program-operation".to_owned(),
                subject: KnowledgeHolderRef::Person(actor),
                change: TechnologyRecordChange::Create {
                    id: "program".to_owned(),
                    value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                        sponsor: KnowledgeHolderRef::Person(actor),
                        site: EntityRef::Territory(site),
                        revision: Some(runtime_revision.clone()),
                        mode: ProgramMode::Investigation,
                        status: ProgramStatus::Active,
                        requirements: vec![],
                        started_at: SimTime::EPOCH,
                        due_at: None,
                    }),
                },
            },
        );
        let program = current_version(
            &canwu,
            &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("program").into_untyped(),
        );
        let intent = authorize_intent(
            &mut canwu,
            actor,
            2,
            "attempt-intent",
            program.clone(),
            "reference-lab",
            TechnologyIntentRequest::Experiment {
                result_id: "attempt".to_owned(),
                revision: runtime_revision.clone(),
                operation: "test".to_owned(),
                site: EntityRef::Territory(site),
                operator: Some(KnowledgeHolderRef::Person(actor)),
                required_assets: vec![],
            },
        );
        apply_result(
            &mut canwu,
            TechnologyResultEnvelope {
                id: "attempt-operation".to_owned(),
                provider: "reference-lab".to_owned(),
                execution_intent: Some(intent.clone()),
                change: TechnologyRecordChange::Create {
                    id: "attempt".to_owned(),
                    value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                        execution_intent: intent,
                        program,
                        revision: runtime_revision,
                        operator: KnowledgeHolderRef::Person(actor),
                        site: EntityRef::Territory(site),
                        operation: "test".to_owned(),
                        inputs: vec![],
                        environment: vec![],
                        outputs: vec![MetricValue {
                            metric: metric_ref.clone(),
                            value: profile.success,
                        }],
                        assets: vec![],
                        started_at: SimTime::EPOCH,
                        ended_at: SimTime::EPOCH,
                        evaluation: evaluate_attempt(
                            &runtime_revision_payload,
                            &spec,
                            &schemas,
                            &MetricContext {
                                values: BTreeMap::from([(
                                    metric_ref.record.clone(),
                                    profile.success,
                                )]),
                            },
                        )
                        .expect("runtime attempt should evaluate"),
                    }),
                },
            },
        );
        assert!(
            canwu
                .typed_domain_record(&canwu_api::TypedDomainRecordRef::<ExperimentAttempt>::new(
                    "attempt"
                ))
                .is_some(),
            "{} must commit through command, ingress, and boundary state",
            profile.label
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn historical_profiles_reverse_outcomes_without_case_specific_solver_code() {
    let papermaking_technique = initial_record_version::<TechniqueSpec>("papermaking-technique");
    let woodblock_technique = initial_record_version::<TechniqueSpec>("woodblock-technique");
    let movable_type_technique = initial_record_version::<TechniqueSpec>("movable-type-technique");
    let gunpowder_technique = initial_record_version::<TechniqueSpec>("gunpowder-technique");
    let steam_technique = initial_record_version::<TechniqueSpec>("steam-technique");
    let metric = |id: &str| initial_record_version::<MetricSchema>(id);
    let schemas = [
        ("fiber", "quality", 0, 1_000),
        ("operator", "skill", 0, 1_000),
        ("edition", "copies", 0, 10_000),
        ("change", "permille", 0, 1_000),
        ("glyphs", "inventory_per_mille", 0, 1_000),
        ("composition", "labor_units", 0, 1_000),
        ("flame", "permille", 0, 1_000),
        ("impulse", "permille", 0, 1_000),
        ("fuel", "cost_units", 0, 1_000),
        ("head", "work_units", 0, 1_000),
        ("torque", "work_units", 0, 1_000),
    ]
    .into_iter()
    .map(|(id, unit, minimum, maximum)| {
        (
            metric(id).record,
            MetricSchemaPayload {
                label: id.to_owned(),
                unit: unit.to_owned(),
                scale: 1,
                minimum,
                maximum,
            },
        )
    })
    .collect::<BTreeMap<_, _>>();
    let threshold = |id: &str, comparison, value| MetricThreshold {
        id: format!("{id}-{value}"),
        metric: metric(id),
        comparison,
        value,
    };
    let group = |id: &str, threshold| RequirementGroup {
        id: id.to_owned(),
        any_of: vec![threshold],
    };
    let context = |values: &[(&str, i64)]| MetricContext {
        values: values
            .iter()
            .map(|(id, value)| (metric(id).record, *value))
            .collect(),
    };

    let papermaking = TechniqueSpecPayload {
        label: "papermaking".to_owned(),
        function: "form a durable sheet".to_owned(),
        requirements: vec![
            group("fiber", threshold("fiber", MetricComparison::AtLeast, 700)),
            group(
                "operator",
                threshold("operator", MetricComparison::AtLeast, 700),
            ),
        ],
        qualification_rules: vec![],
    };
    let papermaking_revision = TechniqueRevisionPayload {
        label: "papermaking cross-check revision".to_owned(),
        spec: papermaking_technique,
        parents: vec![],
        parameters: vec![],
        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
        produced_by: None,
        execution_intent: None,
        discovery_evidence: vec![],
    };
    assert!(
        evaluate_attempt(
            &papermaking_revision,
            &papermaking,
            &schemas,
            &context(&[("fiber", 800), ("operator", 800)])
        )
        .expect("papermaking should evaluate")
        .passed
    );
    assert!(
        !evaluate_attempt(
            &papermaking_revision,
            &papermaking,
            &schemas,
            &context(&[("fiber", 800), ("operator", 300)])
        )
        .expect("local operator counterfactual should evaluate")
        .passed
    );
    assert!(
        !evaluate_attempt(
            &papermaking_revision,
            &papermaking,
            &schemas,
            &context(&[("fiber", 300), ("operator", 800)])
        )
        .expect("poor-fiber counterfactual should evaluate")
        .passed
    );

    let woodblock_spec = TechniqueSpecPayload {
        label: "woodblock printing".to_owned(),
        function: "cut and print stable whole pages".to_owned(),
        requirements: vec![
            group(
                "edition",
                threshold("edition", MetricComparison::AtLeast, 700),
            ),
            group(
                "stability",
                threshold("change", MetricComparison::AtMost, 300),
            ),
        ],
        qualification_rules: vec![],
    };
    let movable_type_spec = TechniqueSpecPayload {
        label: "movable type printing".to_owned(),
        function: "compose and reuse individual glyphs".to_owned(),
        requirements: vec![
            group(
                "script_inventory",
                threshold("glyphs", MetricComparison::AtLeast, 700),
            ),
            group(
                "composition_labor",
                threshold("composition", MetricComparison::AtMost, 500),
            ),
        ],
        qualification_rules: vec![],
    };
    let woodblock_revision = TechniqueRevisionPayload {
        label: "woodblock cross-check revision".to_owned(),
        spec: woodblock_technique.clone(),
        parents: vec![],
        parameters: vec![],
        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
        produced_by: None,
        execution_intent: None,
        discovery_evidence: vec![],
    };
    let movable_type_revision = TechniqueRevisionPayload {
        label: "movable-type cross-check revision".to_owned(),
        spec: movable_type_technique.clone(),
        parents: vec![],
        parameters: vec![],
        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
        produced_by: None,
        execution_intent: None,
        discovery_evidence: vec![],
    };

    let woodblock = ApplicationSpecPayload {
        label: "stable long edition".to_owned(),
        technique: woodblock_technique,
        viability: vec![
            group(
                "edition",
                threshold("edition", MetricComparison::AtLeast, 700),
            ),
            group(
                "stability",
                threshold("change", MetricComparison::AtMost, 300),
            ),
        ],
    };
    let movable_type = ApplicationSpecPayload {
        label: "frequently revised edition".to_owned(),
        technique: movable_type_technique,
        viability: vec![
            group(
                "edition",
                threshold("edition", MetricComparison::AtLeast, 300),
            ),
            group(
                "revision_frequency",
                threshold("change", MetricComparison::AtLeast, 600),
            ),
        ],
    };
    assert_ne!(woodblock.technique, movable_type.technique);
    assert_ne!(woodblock_revision.spec, movable_type_revision.spec);
    let stable_long = context(&[
        ("edition", 900),
        ("change", 100),
        ("glyphs", 400),
        ("composition", 800),
    ]);
    let changing_short = context(&[
        ("edition", 400),
        ("change", 800),
        ("glyphs", 900),
        ("composition", 300),
    ]);
    assert!(
        evaluate_attempt(&woodblock_revision, &woodblock_spec, &schemas, &stable_long)
            .unwrap()
            .passed
    );
    assert!(
        !evaluate_attempt(
            &movable_type_revision,
            &movable_type_spec,
            &schemas,
            &stable_long
        )
        .unwrap()
        .passed
    );
    assert!(
        !evaluate_attempt(
            &woodblock_revision,
            &woodblock_spec,
            &schemas,
            &changing_short
        )
        .unwrap()
        .passed
    );
    assert!(
        evaluate_attempt(
            &movable_type_revision,
            &movable_type_spec,
            &schemas,
            &changing_short
        )
        .unwrap()
        .passed
    );
    assert!(
        evaluate_application(&woodblock, &schemas, &stable_long)
            .unwrap()
            .passed
    );
    assert!(
        !evaluate_application(&movable_type, &schemas, &stable_long)
            .unwrap()
            .passed
    );
    assert!(
        !evaluate_application(&woodblock, &schemas, &changing_short)
            .unwrap()
            .passed
    );
    assert!(
        evaluate_application(&movable_type, &schemas, &changing_short)
            .unwrap()
            .passed
    );

    let pyrotechnics = ApplicationSpecPayload {
        label: "pyrotechnic effect".to_owned(),
        technique: gunpowder_technique.clone(),
        viability: vec![group(
            "visible_flame",
            threshold("flame", MetricComparison::AtLeast, 700),
        )],
    };
    let propulsion = ApplicationSpecPayload {
        label: "propulsion".to_owned(),
        technique: gunpowder_technique,
        viability: vec![group(
            "impulse",
            threshold("impulse", MetricComparison::AtLeast, 700),
        )],
    };
    let weak_powder = context(&[("flame", 900), ("impulse", 200)]);
    assert!(
        evaluate_application(&pyrotechnics, &schemas, &weak_powder)
            .unwrap()
            .passed
    );
    assert!(
        !evaluate_application(&propulsion, &schemas, &weak_powder)
            .unwrap()
            .passed
    );

    let pumping = ApplicationSpecPayload {
        label: "mine pumping".to_owned(),
        technique: steam_technique.clone(),
        viability: vec![
            group("fuel", threshold("fuel", MetricComparison::AtMost, 500)),
            group("head", threshold("head", MetricComparison::AtLeast, 700)),
        ],
    };
    let rotary = ApplicationSpecPayload {
        label: "rotary workshop power".to_owned(),
        technique: steam_technique,
        viability: vec![
            group("fuel", threshold("fuel", MetricComparison::AtMost, 300)),
            group(
                "torque",
                threshold("torque", MetricComparison::AtLeast, 700),
            ),
        ],
    };
    let coal_mine = context(&[("fuel", 300), ("head", 800), ("torque", 300)]);
    let costly_workshop = context(&[("fuel", 800), ("head", 800), ("torque", 800)]);
    assert!(
        evaluate_application(&pumping, &schemas, &coal_mine)
            .unwrap()
            .passed
    );
    assert!(
        !evaluate_application(&rotary, &schemas, &coal_mine)
            .unwrap()
            .passed
    );
    assert!(
        !evaluate_application(&pumping, &schemas, &costly_workshop)
            .unwrap()
            .passed
    );

    assert_ne!(
        TransmissionMode::DocumentAccess,
        TransmissionMode::Apprenticeship
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn playable_flow_is_authoritative_private_and_exactly_restorable() {
    let plugin = TechnologyPlugin;
    let (scenario, ids, catalog) = scenario();
    let mut canwu = Canwu::new_with_plugins(41, scenario.clone(), &[&plugin])
        .expect("technology scenario should initialize");

    let program = TechnicalProgramPayload {
        sponsor: KnowledgeHolderRef::Person(ids.0),
        site: EntityRef::Territory(ids.2),
        revision: Some(catalog.revision.clone()),
        mode: ProgramMode::Adaptation,
        status: ProgramStatus::Active,
        requirements: vec![],
        started_at: SimTime::EPOCH,
        due_at: None,
    };
    apply_command(
        &mut canwu,
        ids.0,
        1,
        TechnologyCommandEnvelope {
            id: "op-program".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(program),
            },
        },
    );
    let program_ref = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("program").into_untyped(),
    );

    let metrics = BTreeMap::from([(
        catalog.reliability.record.clone(),
        MetricSchemaPayload {
            label: "reliability".to_owned(),
            unit: "permille".to_owned(),
            scale: 1_000,
            minimum: 0,
            maximum: 1_000,
        },
    )]);
    let outputs = vec![MetricValue {
        metric: catalog.reliability.clone(),
        value: 820,
    }];
    let evaluation = evaluate_attempt(
        &catalog.revision_payload,
        &catalog.spec_payload,
        &metrics,
        &MetricContext {
            values: BTreeMap::from([(catalog.reliability.record.clone(), 820)]),
        },
    )
    .expect("attempt should evaluate");
    let attempt = ExperimentAttemptPayload {
        execution_intent: authorize_intent(
            &mut canwu,
            ids.0,
            100,
            "attempt-intent",
            program_ref.clone(),
            "reference-lab",
            TechnologyIntentRequest::Experiment {
                result_id: "attempt".to_owned(),
                revision: catalog.revision.clone(),
                operation: "operate".to_owned(),
                site: EntityRef::Territory(ids.2),
                operator: Some(KnowledgeHolderRef::Person(ids.0)),
                required_assets: vec![],
            },
        ),
        program: program_ref,
        revision: catalog.revision.clone(),
        operator: KnowledgeHolderRef::Person(ids.0),
        site: EntityRef::Territory(ids.2),
        operation: "operate".to_owned(),
        inputs: vec![],
        environment: vec![],
        outputs,
        assets: vec![],
        started_at: SimTime::EPOCH,
        ended_at: SimTime::EPOCH,
        evaluation,
    };
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "op-attempt".to_owned(),
            provider: "reference-lab".to_owned(),
            execution_intent: Some(attempt.execution_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(attempt),
            },
        },
    );
    let attempt_ref = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<ExperimentAttempt>::new("attempt").into_untyped(),
    );

    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "op-observation".to_owned(),
            provider: "meter-reader".to_owned(),
            execution_intent: None,
            change: TechnologyRecordChange::Create {
                id: "fuel-observation".to_owned(),
                value: TechnologyRecordPayload::AttemptObservation(AttemptObservationPayload {
                    attempt: attempt_ref.clone(),
                    observer: KnowledgeHolderRef::Person(ids.0),
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
    );
    let viability_evidence = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<AttemptObservation>::new("fuel-observation")
            .into_untyped(),
    );

    let knowledge_before_rejection = canwu
        .knowledge()
        .for_holder(&KnowledgeHolderRef::Person(ids.0))
        .map_or(0, BTreeMap::len);
    apply_command(
        &mut canwu,
        ids.0,
        2,
        TechnologyCommandEnvelope {
            id: "op-capability-duplicate".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "capability-duplicate".to_owned(),
                value: TechnologyRecordPayload::Capability(CapabilityQualificationPayload {
                    holder: KnowledgeHolderRef::Person(ids.0),
                    operator: Some(EntityRef::Person(ids.0)),
                    site: EntityRef::Territory(ids.2),
                    revision: catalog.revision.clone(),
                    operation: "operate".to_owned(),
                    reliability_per_mille: 1_000,
                    attempts: vec![attempt_ref.clone(), attempt_ref.clone()],
                    last_practiced_at: SimTime::EPOCH,
                    valid_from: SimTime::EPOCH,
                    valid_until: None,
                    active: true,
                }),
            },
        },
    );
    let duplicate_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("op-capability-duplicate"),
        )
        .expect("duplicate qualification operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("operation should decode");
    assert_eq!(
        duplicate_outcome.status,
        TechnologyOperationStatus::Rejected
    );
    assert_eq!(
        canwu
            .knowledge()
            .for_holder(&KnowledgeHolderRef::Person(ids.0))
            .map_or(0, BTreeMap::len),
        knowledge_before_rejection,
        "a rejected phase-7 operation must not publish phase-13 knowledge",
    );

    apply_command(
        &mut canwu,
        ids.0,
        3,
        TechnologyCommandEnvelope {
            id: "op-capability".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "capability".to_owned(),
                value: TechnologyRecordPayload::Capability(CapabilityQualificationPayload {
                    holder: KnowledgeHolderRef::Person(ids.0),
                    operator: Some(EntityRef::Person(ids.0)),
                    site: EntityRef::Territory(ids.2),
                    revision: catalog.revision.clone(),
                    operation: "operate".to_owned(),
                    reliability_per_mille: 1_000,
                    attempts: vec![attempt_ref],
                    last_practiced_at: SimTime::EPOCH,
                    valid_from: SimTime::EPOCH,
                    valid_until: None,
                    active: true,
                }),
            },
        },
    );
    let capability_ref = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<CapabilityQualification>::new("capability")
            .into_untyped(),
    );
    let capability = canwu
        .domain_record(
            &canwu_api::TypedDomainRecordRef::<CapabilityQualification>::new("capability")
                .into_untyped(),
        )
        .expect("current capability should exist")
        .decode_payload::<CapabilityQualification>()
        .expect("capability should decode");

    let asset_evidence = EvidenceRef::Boundary(
        canwu
            .boundaries()
            .last()
            .expect("qualification boundary should exist")
            .id,
    );
    apply_command(
        &mut canwu,
        ids.0,
        20,
        TechnologyCommandEnvelope {
            id: "op-asset".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "asset".to_owned(),
                value: TechnologyRecordPayload::AssetBinding(AssetBindingPayload {
                    owner: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    provider_asset: asset_evidence,
                    capabilities: vec!["operate".to_owned()],
                    condition_per_mille: 1_000,
                    active: true,
                }),
            },
        },
    );
    let asset_ref = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<AssetBinding>::new("asset").into_untyped(),
    );

    let implementation = ImplementationPayload {
        owner: KnowledgeHolderRef::Person(ids.0),
        site: EntityRef::Territory(ids.2),
        revision: catalog.revision.clone(),
        qualification: capability_ref.clone(),
        assets: vec![asset_ref.clone()],
        installed_at: SimTime::EPOCH,
        capacity: 10,
        unit: "runs_per_month".to_owned(),
        reliability_per_mille: 1_000,
        maintenance_provider: Some(KnowledgeHolderRef::Person(ids.0)),
        active: true,
    };

    apply_command(
        &mut canwu,
        ids.0,
        4,
        TechnologyCommandEnvelope {
            id: "op-implementation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "implementation".to_owned(),
                value: TechnologyRecordPayload::Implementation(implementation.clone()),
            },
        },
    );
    let implementation_ref = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<ImplementationRecord>::new("implementation")
            .into_untyped(),
    );

    let viability_metrics = vec![MetricValue {
        metric: catalog.fuel_cost.clone(),
        value: 400,
    }];
    let viability = evaluate_application(
        &catalog.application_payload,
        &BTreeMap::from([(
            catalog.fuel_cost.record.clone(),
            MetricSchemaPayload {
                label: "fuel cost".to_owned(),
                unit: "cost_units".to_owned(),
                scale: 1,
                minimum: 0,
                maximum: 1_000,
            },
        )]),
        &MetricContext {
            values: BTreeMap::from([(catalog.fuel_cost.record.clone(), 400)]),
        },
    )
    .expect("application should evaluate");
    let prior_boundary = canwu.boundaries().last().expect("boundary evidence").id;
    apply_command(
        &mut canwu,
        ids.0,
        5,
        TechnologyCommandEnvelope {
            id: "op-adoption".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "adoption".to_owned(),
                value: TechnologyRecordPayload::Adoption(AdoptionPayload {
                    adopter: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    application: catalog.application.clone(),
                    implementations: vec![implementation_ref.clone()],
                    status: AdoptionStatus::Committed,
                    scale: 10,
                    decision_evidence: EvidenceRef::Boundary(prior_boundary),
                    viability_evidence: vec![viability_evidence],
                    viability_metrics,
                    viability,
                }),
            },
        },
    );

    apply_command(
        &mut canwu,
        ids.1,
        6,
        TechnologyCommandEnvelope {
            id: "op-learner-program".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.1),
            change: TechnologyRecordChange::Create {
                id: "learner-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    sponsor: KnowledgeHolderRef::Person(ids.1),
                    site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision.clone()),
                    mode: ProgramMode::Training,
                    status: ProgramStatus::Active,
                    requirements: vec![],
                    started_at: SimTime::EPOCH,
                    due_at: None,
                }),
            },
        },
    );
    let learner_program = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("learner-program").into_untyped(),
    );

    apply_command(
        &mut canwu,
        ids.0,
        7,
        TechnologyCommandEnvelope {
            id: "op-backdated-teaching".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "backdated-teaching".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(ids.0)),
                    source_site: Some(EntityRef::Territory(ids.2)),
                    source_capability: Some(implementation_ref.clone()),
                    destination: KnowledgeHolderRef::Person(ids.1),
                    destination_site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision.clone()),
                    mode: TransmissionMode::Demonstration,
                    evidence: vec![],
                    resulting_program: None,
                    opened_at: SimTime::from_minutes(-1),
                    active: true,
                }),
            },
        },
    );
    let backdated_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("op-backdated-teaching"),
        )
        .expect("backdated transmission operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("backdated transmission outcome should decode");
    assert_eq!(
        backdated_outcome.status,
        TechnologyOperationStatus::Rejected
    );
    assert!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TransmissionOpportunity>::new(
                    "backdated-teaching"
                )
            )
            .is_none(),
        "an installation cannot support a transmission before it was installed"
    );
    apply_command(
        &mut canwu,
        ids.0,
        13,
        TechnologyCommandEnvelope {
            id: "op-premature-demonstration".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "premature-demonstration".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(ids.0)),
                    source_site: Some(EntityRef::Territory(ids.2)),
                    source_capability: Some(capability_ref.clone()),
                    destination: KnowledgeHolderRef::Person(ids.1),
                    destination_site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision.clone()),
                    mode: TransmissionMode::Demonstration,
                    evidence: vec![],
                    resulting_program: None,
                    opened_at: SimTime::from_minutes(-1),
                    active: true,
                }),
            },
        },
    );
    let premature_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "op-premature-demonstration",
            ),
        )
        .expect("premature qualification operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("premature qualification outcome should decode");
    assert_eq!(
        premature_outcome.status,
        TechnologyOperationStatus::Rejected
    );

    apply_command(
        &mut canwu,
        ids.0,
        8,
        TechnologyCommandEnvelope {
            id: "op-teaching".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "teaching".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(ids.0)),
                    source_site: Some(EntityRef::Territory(ids.2)),
                    source_capability: Some(implementation_ref.clone()),
                    destination: KnowledgeHolderRef::Person(ids.1),
                    destination_site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision.clone()),
                    mode: TransmissionMode::Apprenticeship,
                    evidence: vec![],
                    resulting_program: Some(learner_program.clone()),
                    opened_at: SimTime::EPOCH,
                    active: true,
                }),
            },
        },
    );

    apply_command(
        &mut canwu,
        ids.0,
        9,
        TechnologyCommandEnvelope {
            id: "op-direct-demonstration".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "direct-demonstration".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(ids.0)),
                    source_site: Some(EntityRef::Territory(ids.2)),
                    source_capability: Some(capability_ref.clone()),
                    destination: KnowledgeHolderRef::Person(ids.1),
                    destination_site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision.clone()),
                    mode: TransmissionMode::Demonstration,
                    evidence: vec![],
                    resulting_program: Some(learner_program),
                    opened_at: SimTime::EPOCH,
                    active: true,
                }),
            },
        },
    );

    let mut inactive_asset = canwu
        .typed_domain_record(&canwu_api::TypedDomainRecordRef::<AssetBinding>::new(
            "asset",
        ))
        .expect("asset should exist")
        .decode_payload::<AssetBinding>()
        .expect("asset should decode");
    inactive_asset.active = false;
    apply_command(
        &mut canwu,
        ids.0,
        21,
        TechnologyCommandEnvelope {
            id: "op-asset-stop".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "asset".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::AssetBinding(inactive_asset),
            },
        },
    );
    apply_command(
        &mut canwu,
        ids.0,
        22,
        TechnologyCommandEnvelope {
            id: "op-stale-asset-implementation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "stale-asset-implementation".to_owned(),
                value: TechnologyRecordPayload::Implementation(implementation.clone()),
            },
        },
    );
    assert_eq!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                    "op-stale-asset-implementation"
                )
            )
            .expect("stale asset operation should be terminal")
            .decode_payload::<TechnologyOperation>()
            .expect("stale asset outcome")
            .status,
        TechnologyOperationStatus::Rejected
    );

    let mut inactive_implementation = implementation.clone();
    inactive_implementation.active = false;
    apply_command(
        &mut canwu,
        ids.0,
        10,
        TechnologyCommandEnvelope {
            id: "op-implementation-stop".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "implementation".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::Implementation(inactive_implementation),
            },
        },
    );
    let current_implementation = canwu
        .domain_record(
            &canwu_api::TypedDomainRecordRef::<ImplementationRecord>::new("implementation")
                .into_untyped(),
        )
        .expect("current implementation should exist")
        .decode_payload::<ImplementationRecord>()
        .expect("implementation should decode");
    assert!(!current_implementation.active);

    let mut inactive_capability = capability;
    inactive_capability.active = false;
    apply_command(
        &mut canwu,
        ids.0,
        11,
        TechnologyCommandEnvelope {
            id: "op-capability-stop".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "capability".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::Capability(inactive_capability),
            },
        },
    );
    let mut stale_qualification_implementation = implementation.clone();
    stale_qualification_implementation.assets.clear();
    apply_command(
        &mut canwu,
        ids.0,
        23,
        TechnologyCommandEnvelope {
            id: "op-stale-qualification-implementation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "stale-qualification-implementation".to_owned(),
                value: TechnologyRecordPayload::Implementation(stale_qualification_implementation),
            },
        },
    );
    assert_eq!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                    "op-stale-qualification-implementation"
                )
            )
            .expect("stale qualification operation should be terminal")
            .decode_payload::<TechnologyOperation>()
            .expect("stale qualification outcome")
            .status,
        TechnologyOperationStatus::Rejected
    );

    let adoption = canwu
        .typed_domain_record(&canwu_api::TypedDomainRecordRef::<
            canwu_technology::AdoptionRecord,
        >::new("adoption"))
        .expect("adoption should exist")
        .decode_payload::<canwu_technology::AdoptionRecord>()
        .expect("adoption should decode");
    apply_command(
        &mut canwu,
        ids.0,
        24,
        TechnologyCommandEnvelope {
            id: "op-stale-implementation-adoption".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "stale-implementation-adoption".to_owned(),
                value: TechnologyRecordPayload::Adoption(adoption.clone()),
            },
        },
    );
    assert_eq!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                    "op-stale-implementation-adoption"
                )
            )
            .expect("stale adoption operation should be terminal")
            .decode_payload::<TechnologyOperation>()
            .expect("stale adoption outcome")
            .status,
        TechnologyOperationStatus::Rejected
    );
    let mut suspended_adoption = adoption;
    suspended_adoption.status = AdoptionStatus::Suspended;
    apply_command(
        &mut canwu,
        ids.0,
        25,
        TechnologyCommandEnvelope {
            id: "op-suspend-adoption".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "adoption".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::Adoption(suspended_adoption.clone()),
            },
        },
    );
    suspended_adoption.status = AdoptionStatus::Committed;
    apply_command(
        &mut canwu,
        ids.0,
        26,
        TechnologyCommandEnvelope {
            id: "op-recommit-stale-adoption".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "adoption".to_owned(),
                expected_version: 2,
                value: TechnologyRecordPayload::Adoption(suspended_adoption),
            },
        },
    );
    assert_eq!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                    "op-recommit-stale-adoption"
                )
            )
            .expect("recommit operation should be terminal")
            .decode_payload::<TechnologyOperation>()
            .expect("recommit outcome")
            .status,
        TechnologyOperationStatus::Rejected
    );
    let mut closed_teaching = canwu
        .domain_record(
            &canwu_api::TypedDomainRecordRef::<TransmissionOpportunity>::new("teaching")
                .into_untyped(),
        )
        .expect("existing teaching opportunity should remain available")
        .decode_payload::<TransmissionOpportunity>()
        .expect("teaching opportunity should decode");
    closed_teaching.active = false;
    apply_command(
        &mut canwu,
        ids.0,
        15,
        TechnologyCommandEnvelope {
            id: "op-close-teaching".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "teaching".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::Transmission(closed_teaching),
            },
        },
    );
    let mut reopened_teaching = canwu
        .domain_record(
            &canwu_api::TypedDomainRecordRef::<TransmissionOpportunity>::new("teaching")
                .into_untyped(),
        )
        .expect("closed teaching opportunity should remain available")
        .decode_payload::<TransmissionOpportunity>()
        .expect("closed teaching opportunity should decode");
    reopened_teaching.active = true;
    apply_command(
        &mut canwu,
        ids.0,
        16,
        TechnologyCommandEnvelope {
            id: "op-reopen-teaching".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "teaching".to_owned(),
                expected_version: 2,
                value: TechnologyRecordPayload::Transmission(reopened_teaching),
            },
        },
    );
    let reopen_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("op-reopen-teaching"),
        )
        .expect("reopen operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("reopen outcome should decode");
    assert_eq!(reopen_outcome.status, TechnologyOperationStatus::Rejected);
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::from_minutes(2)))
        .expect("runtime should advance beyond capability deactivation");
    apply_command(
        &mut canwu,
        ids.0,
        12,
        TechnologyCommandEnvelope {
            id: "op-stale-capability-demonstration".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "stale-capability-demonstration".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(ids.0)),
                    source_site: Some(EntityRef::Territory(ids.2)),
                    source_capability: Some(capability_ref),
                    destination: KnowledgeHolderRef::Person(ids.1),
                    destination_site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision.clone()),
                    mode: TransmissionMode::Demonstration,
                    evidence: vec![],
                    resulting_program: None,
                    opened_at: SimTime::from_minutes(2),
                    active: true,
                }),
            },
        },
    );
    let stale_capability_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "op-stale-capability-demonstration",
            ),
        )
        .expect("stale qualification operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("stale qualification outcome should decode");
    assert_eq!(
        stale_capability_outcome.status,
        TechnologyOperationStatus::Rejected
    );
    assert!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TransmissionOpportunity>::new(
                    "stale-capability-demonstration"
                )
            )
            .is_none(),
        "a superseded qualification cannot support a later transmission"
    );

    apply_command(
        &mut canwu,
        ids.0,
        14,
        TechnologyCommandEnvelope {
            id: "op-stale-implementation-demonstration".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "stale-implementation-demonstration".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: Some(KnowledgeHolderRef::Person(ids.0)),
                    source_site: Some(EntityRef::Territory(ids.2)),
                    source_capability: Some(implementation_ref),
                    destination: KnowledgeHolderRef::Person(ids.1),
                    destination_site: EntityRef::Territory(ids.3),
                    revision: Some(catalog.revision),
                    mode: TransmissionMode::Demonstration,
                    evidence: vec![],
                    resulting_program: None,
                    opened_at: SimTime::from_minutes(2),
                    active: true,
                }),
            },
        },
    );
    let stale_implementation_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "op-stale-implementation-demonstration",
            ),
        )
        .expect("stale implementation operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("stale implementation outcome should decode");
    assert_eq!(
        stale_implementation_outcome.status,
        TechnologyOperationStatus::Rejected
    );
    assert!(
        canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TransmissionOpportunity>::new(
                    "stale-implementation-demonstration"
                )
            )
            .is_none(),
        "a rejected stale implementation must not create a transmission"
    );

    validate_technology_runtime(&canwu).expect("technology runtime should validate");
    let operator_knowledge = canwu
        .knowledge()
        .for_holder(&KnowledgeHolderRef::Person(ids.0))
        .map_or(0, BTreeMap::len);
    assert!(
        operator_knowledge >= 3,
        "operator should receive at least three technology records, got {operator_knowledge}"
    );
    let actor_records = canwu
        .viewer_for_actor(ids.0)
        .expect("legacy actor viewer should be available")
        .query_knowledge(&KnowledgeQuery::default())
        .expect("actor should read their own technology records");
    assert!(actor_records.records.iter().all(|record| {
        serde_json::from_value::<TechnologyRecordPayload>(record.payload["record"].clone()).is_ok()
    }));
    assert_eq!(
        canwu
            .knowledge()
            .for_holder(&KnowledgeHolderRef::Person(ids.1))
            .map_or(0, BTreeMap::len),
        0,
        "a teaching opportunity must not grant destination capability or knowledge"
    );

    let snapshot = canwu.snapshot_json().expect("snapshot should serialize");
    let restored = from_technology_snapshot_json(&snapshot, &[&plugin])
        .expect("technology snapshot should restore");
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let checkpoint_restored = from_technology_checkpoint_journal(
        canwu
            .checkpoint_journal()
            .expect("checkpoint journal should serialize"),
        &[&plugin],
    )
    .expect("technology checkpoint should restore");
    assert_eq!(checkpoint_restored.snapshot(), canwu.snapshot());
    validate_technology_runtime(&canwu.fork()).expect("fork should validate");
    let replayed = replay_technology_from_journal(scenario, &[&plugin], &canwu.replay_journal())
        .expect("journal should replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

#[test]
#[allow(clippy::too_many_lines)]
fn investigation_can_produce_a_new_runtime_revision() {
    let (scenario, ids, catalog) = scenario();
    let mut canwu = Canwu::new_with_plugins(31, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    apply_command(
        &mut canwu,
        ids.0,
        1,
        TechnologyCommandEnvelope {
            id: "open-investigation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "investigation".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    sponsor: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    revision: None,
                    mode: ProgramMode::Investigation,
                    status: ProgramStatus::Active,
                    requirements: vec![ProviderRequirement {
                        provider: "reference-lab".to_owned(),
                        capability: "invention-result".to_owned(),
                        quantity: 1,
                        unit: "result".to_owned(),
                        evidence: None,
                    }],
                    started_at: SimTime::EPOCH,
                    due_at: None,
                }),
            },
        },
    );
    let program = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("investigation").into_untyped(),
    );
    let discovery_boundary = canwu
        .boundaries()
        .last()
        .expect("program boundary should exist")
        .id;
    let intent = authorize_intent(
        &mut canwu,
        ids.0,
        100,
        "invention-intent",
        program.clone(),
        "reference-lab",
        TechnologyIntentRequest::Invention {
            result_id: "invented-revision".to_owned(),
            spec: catalog.revision_payload.spec.clone(),
            parent: None,
            site: EntityRef::Territory(ids.2),
        },
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "invention-result".to_owned(),
            provider: "reference-lab".to_owned(),
            execution_intent: Some(intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "invented-revision".to_owned(),
                value: TechnologyRecordPayload::TechniqueRevision(TechniqueRevisionPayload {
                    label: "runtime invention".to_owned(),
                    spec: catalog.revision_payload.spec,
                    parents: vec![],
                    parameters: vec![],
                    evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                    produced_by: Some(program),
                    execution_intent: Some(intent),
                    discovery_evidence: vec![EvidenceRef::Boundary(discovery_boundary)],
                }),
            },
        },
    );
    let invented_ref = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechniqueRevision>::new("invented-revision")
            .into_untyped(),
    );
    let invented = canwu
        .typed_domain_record(&canwu_api::TypedDomainRecordRef::<TechniqueRevision>::new(
            "invented-revision",
        ))
        .expect("provider result should create a runtime revision")
        .decode_payload::<TechniqueRevision>()
        .expect("revision should decode");
    assert_eq!(invented.label, "runtime invention");
    assert_eq!(
        canwu
            .knowledge()
            .for_holder(&KnowledgeHolderRef::Person(ids.0))
            .map_or(0, BTreeMap::len),
        0,
        "inventing a revision must not automatically publish it as holder knowledge"
    );
    let consumed = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new("invention-intent"),
        )
        .expect("invention intent should remain auditable")
        .decode_payload::<TechnologyExecutionIntent>()
        .expect("intent should decode");
    assert!(matches!(
        consumed.state,
        TechnologyIntentState::Consumed { .. }
    ));
    let claimed_at = canwu.time();
    apply_command(
        &mut canwu,
        ids.0,
        101,
        TechnologyCommandEnvelope {
            id: "claim-invention".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "invented-revision-claim".to_owned(),
                value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                    asserted_by: KnowledgeHolderRef::Person(ids.0),
                    proposition: "the new revision produced the observed result".to_owned(),
                    scope: vec![invented_ref.record.clone()],
                    source_evidence: vec![EvidenceRef::DomainRecordVersion(invented_ref)],
                    relations: vec![],
                    asserted_at: claimed_at,
                }),
            },
        },
    );
    assert_eq!(
        canwu
            .knowledge()
            .for_holder(&KnowledgeHolderRef::Person(ids.0))
            .map_or(0, BTreeMap::len),
        1,
        "a later explicit claim should establish holder-relative knowledge"
    );
    validate_technology_runtime(&canwu).expect("runtime invention must restore safely");
}

#[test]
#[allow(clippy::too_many_lines)]
fn provider_requirements_accept_a_match_and_reject_a_nonmatching_authorized_provider() {
    let (scenario, ids, catalog) = scenario();
    let mut canwu = Canwu::new_with_plugins(33, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    apply_command(
        &mut canwu,
        ids.0,
        1,
        TechnologyCommandEnvelope {
            id: "open-provider-test".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "provider-test-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    sponsor: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    revision: Some(catalog.revision.clone()),
                    mode: ProgramMode::Investigation,
                    status: ProgramStatus::Active,
                    requirements: vec![ProviderRequirement {
                        provider: "authorized-lab".to_owned(),
                        capability: "experiment-result".to_owned(),
                        quantity: 1,
                        unit: "result".to_owned(),
                        evidence: None,
                    }],
                    started_at: SimTime::EPOCH,
                    due_at: None,
                }),
            },
        },
    );
    let program = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("provider-test-program")
            .into_untyped(),
    );
    let matching_intent = authorize_intent(
        &mut canwu,
        ids.0,
        2,
        "matching-provider-intent",
        program.clone(),
        "authorized-lab",
        TechnologyIntentRequest::Experiment {
            result_id: "matching-attempt".to_owned(),
            revision: catalog.revision.clone(),
            operation: "operate".to_owned(),
            site: EntityRef::Territory(ids.2),
            operator: Some(KnowledgeHolderRef::Person(ids.0)),
            required_assets: vec![],
        },
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "provider-match".to_owned(),
            provider: "authorized-lab".to_owned(),
            execution_intent: Some(matching_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "matching-attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                    execution_intent: matching_intent,
                    program: program.clone(),
                    revision: catalog.revision.clone(),
                    operator: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    operation: "operate".to_owned(),
                    inputs: vec![],
                    environment: vec![],
                    outputs: vec![],
                    assets: vec![],
                    started_at: SimTime::EPOCH,
                    ended_at: SimTime::EPOCH,
                    evaluation: canwu_technology::EvaluationResult {
                        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                        passed: false,
                        satisfied_groups: vec![],
                        failed_groups: vec!["reliable_output".to_owned()],
                    },
                }),
            },
        },
    );
    let matching_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("provider-match"),
        )
        .expect("matching provider result must be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("operation should decode");
    assert_eq!(matching_outcome.status, TechnologyOperationStatus::Applied);

    let mismatching_intent = authorize_intent(
        &mut canwu,
        ids.0,
        3,
        "requirement-mismatch-intent",
        program.clone(),
        "other-lab",
        TechnologyIntentRequest::Experiment {
            result_id: "requirement-mismatch-attempt".to_owned(),
            revision: catalog.revision.clone(),
            operation: "operate".to_owned(),
            site: EntityRef::Territory(ids.2),
            operator: Some(KnowledgeHolderRef::Person(ids.0)),
            required_assets: vec![],
        },
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "provider-requirement-mismatch".to_owned(),
            provider: "other-lab".to_owned(),
            execution_intent: Some(mismatching_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "requirement-mismatch-attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                    execution_intent: mismatching_intent,
                    program,
                    revision: catalog.revision,
                    operator: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    operation: "operate".to_owned(),
                    inputs: vec![],
                    environment: vec![],
                    outputs: vec![],
                    assets: vec![],
                    started_at: SimTime::EPOCH,
                    ended_at: SimTime::EPOCH,
                    evaluation: canwu_technology::EvaluationResult {
                        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                        passed: false,
                        satisfied_groups: vec![],
                        failed_groups: vec!["reliable_output".to_owned()],
                    },
                }),
            },
        },
    );
    let outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "provider-requirement-mismatch",
            ),
        )
        .expect("provider requirement mismatch must be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("operation should decode");
    assert_eq!(outcome.status, TechnologyOperationStatus::Rejected);
    assert!(
        canwu
            .typed_domain_record(&canwu_api::TypedDomainRecordRef::<ExperimentAttempt>::new(
                "requirement-mismatch-attempt"
            ))
            .is_none()
    );
    let pending = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(
                "requirement-mismatch-intent",
            ),
        )
        .expect("rejected provider requirement must not consume intent")
        .decode_payload::<TechnologyExecutionIntent>()
        .expect("intent should decode");
    assert_eq!(pending.state, TechnologyIntentState::Pending);
    validate_technology_runtime(&canwu).expect("rejected result must restore safely");
}

#[test]
fn operation_collision_reduction_is_order_and_duplicate_independent() {
    let (scenario, _, _) = scenario();
    let run = |order: &[&str]| {
        let mut canwu = Canwu::new_with_plugins(43, scenario.clone(), &[&TechnologyPlugin])
            .expect("technology plugin should initialize");
        for label in order {
            let envelope = collision_envelope(label);
            canwu
                .enqueue_plugin_ingress(PluginIngressRequest::new(
                    "canwu-technology",
                    TECHNOLOGY_RESULT_INGRESS,
                    canwu.time(),
                    serde_json::to_value(envelope).expect("collision result payload"),
                ))
                .expect("collision result ingress should enqueue");
        }
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("collision boundary should settle");
        validate_technology_runtime(&canwu).expect("collision state should restore safely");
        let outcome = canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("collision"),
            )
            .expect("collision should have one terminal operation")
            .decode_payload::<TechnologyOperation>()
            .expect("collision operation should decode");
        let replayed = replay_technology_from_journal(
            scenario.clone(),
            &[&TechnologyPlugin],
            &canwu.replay_journal(),
        )
        .expect("collision should replay exactly");
        assert_eq!(replayed.snapshot(), canwu.snapshot());
        let domain = canwu.domain_records().cloned().collect::<Vec<_>>();
        (outcome, domain)
    };
    let (ab, ab_domain) = run(&["a", "b"]);
    let (ba, ba_domain) = run(&["b", "a"]);
    let (with_duplicate, duplicate_domain) = run(&["a", "b", "a"]);
    assert_eq!(ab.status, TechnologyOperationStatus::Rejected);
    assert_eq!(ab.rejection_code.as_deref(), Some("idempotency_conflict"));
    assert_eq!(ab.canonical_input_hashes.len(), 2);
    assert_eq!(ab, ba);
    assert_eq!(ab, with_duplicate);
    assert_eq!(ab_domain, ba_domain);
    assert_eq!(ab_domain, duplicate_domain);
}

#[test]
#[allow(clippy::too_many_lines)]
fn technology_total_and_boundary_caps_persist_rejections_without_poisoning() {
    let mut records = TechnologyRecordSet::default();
    for index in 0..=TechnologyLimitsV1::canonical().max_total_records {
        records.insert(DomainRecord {
            reference: canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(format!(
                "over-cap-{index:05}"
            ))
            .into_untyped(),
            owner: "canwu-technology".to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::Value::Null,
            references: vec![],
        });
    }
    assert!(records.validate(SimTime::EPOCH).is_err());

    let (mut near_cap_scenario, near_cap_ids, near_cap_catalog) = scenario();
    while near_cap_scenario.domain_records.len()
        < TechnologyLimitsV1::canonical().max_total_records - 1
    {
        let index = near_cap_scenario.domain_records.len();
        near_cap_scenario.domain_records.push(
            TechnologyCatalogRecord::Metric(MetricSchemaPayload {
                label: format!("capacity fixture {index}"),
                unit: "count".to_owned(),
                scale: 1,
                minimum: 0,
                maximum: 1,
            })
            .into_initial_record(format!("capacity-metric-{index:05}"))
            .expect("capacity fixture record"),
        );
    }
    let mut near_cap = Canwu::new_with_plugins(46, near_cap_scenario, &[&TechnologyPlugin])
        .expect("near-cap technology runtime should initialize");
    apply_command(
        &mut near_cap,
        near_cap_ids.0,
        1,
        program_command(
            "near-cap-create",
            "near-cap-program",
            near_cap_ids.0,
            TechnologyRecordChange::Create {
                id: "ignored".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(test_program(
                    near_cap_ids.0,
                    near_cap_ids.2,
                    near_cap_catalog.revision.clone(),
                    ProgramMode::Investigation,
                )),
            },
        ),
    );
    assert!(
        near_cap
            .typed_domain_record(&canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new(
                "near-cap-program"
            ))
            .is_none(),
        "the candidate record must be rejected because its terminal outcome also needs capacity"
    );
    let outcome = near_cap
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("near-cap-create"),
        )
        .expect("capacity rejection must have a terminal outcome")
        .decode_payload::<TechnologyOperation>()
        .expect("capacity outcome");
    assert_eq!(outcome.status, TechnologyOperationStatus::Rejected);
    assert_eq!(
        near_cap.domain_records().count(),
        TechnologyLimitsV1::canonical().max_total_records
    );
    apply_command(
        &mut near_cap,
        near_cap_ids.0,
        2,
        program_command(
            "full-cap-create",
            "full-cap-program",
            near_cap_ids.0,
            TechnologyRecordChange::Create {
                id: "ignored".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(test_program(
                    near_cap_ids.0,
                    near_cap_ids.2,
                    near_cap_catalog.revision.clone(),
                    ProgramMode::Investigation,
                )),
            },
        ),
    );
    assert!(
        near_cap
            .typed_domain_record(&canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new(
                "full-cap-program"
            ))
            .is_none()
    );
    assert!(near_cap.events().iter().any(|event| {
        event.kind.plugin_identity()
            == Some((
                "canwu-technology",
                "technology_operation_rejected_capacity_v1",
            ))
    }));
    near_cap
        .settle_boundary(BoundaryRequest::at(near_cap.time()))
        .expect("the saturated runtime must continue after rejection");
    validate_technology_runtime(&near_cap).expect("near-cap runtime must remain valid");

    let (scenario, _, _) = scenario();
    let mut canwu = Canwu::new_with_plugins(47, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    for index in 0..33 {
        let mut envelope = collision_envelope(&format!("cap-{index:02}"));
        envelope.id = format!("cap-operation-{index:02}");
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canwu-technology",
                TECHNOLOGY_RESULT_INGRESS,
                canwu.time(),
                serde_json::to_value(envelope).expect("bounded result payload"),
            ))
            .expect("bounded result ingress should enqueue");
    }
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("over-budget operations must settle as persisted capacity rejections");
    assert_eq!(
        canwu
            .events()
            .iter()
            .filter(|event| {
                event.kind.plugin_identity()
                    == Some((
                        "canwu-technology",
                        "technology_operation_rejected_capacity_v1",
                    ))
            })
            .count(),
        33
    );
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("capacity rejection must not poison a later boundary");
}

#[test]
fn idempotent_retries_do_not_consume_boundary_mutation_budget() {
    let (scenario, _, _) = scenario();
    let mut canwu = Canwu::new_with_plugins(48, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    let mut retries = Vec::new();
    for index in 0..33 {
        let mut envelope = collision_envelope(&format!("retry-{index:02}"));
        envelope.id = format!("retry-operation-{index:02}");
        apply_result(&mut canwu, envelope.clone());
        retries.push(envelope);
    }

    for envelope in &retries {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                "canwu-technology",
                TECHNOLOGY_RESULT_INGRESS,
                canwu.time(),
                serde_json::to_value(envelope).expect("idempotent retry payload"),
            ))
            .expect("idempotent retry ingress should enqueue");
    }
    let mut fresh = collision_envelope("fresh-after-retries");
    fresh.id = "fresh-after-retries".to_owned();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canwu-technology",
            TECHNOLOGY_RESULT_INGRESS,
            canwu.time(),
            serde_json::to_value(fresh).expect("fresh result payload"),
        ))
        .expect("fresh operation ingress should enqueue");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("idempotent retries should not exhaust mutation budget");

    let outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("fresh-after-retries"),
        )
        .expect("fresh operation should receive a terminal outcome")
        .decode_payload::<TechnologyOperation>()
        .expect("fresh operation should decode");
    assert_eq!(outcome.status, TechnologyOperationStatus::Rejected);
    assert!(
        canwu.events().iter().all(|event| !matches!(
            &event.kind,
            EventKind::Plugin { plugin, event_type }
                if plugin == "canwu-technology"
                    && event_type == "technology_operation_rejected_capacity_v1"
        )),
        "idempotent retries must not produce a capacity rejection"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn future_facts_and_oversized_collections_are_terminally_rejected() {
    let (scenario, ids, catalog) = scenario();
    let mut canwu = Canwu::new_with_plugins(49, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    let evidence = EvidenceRef::DomainRecordVersion(catalog.revision.clone());
    for (request_id, id, source_evidence, asserted_at) in [
        (
            1,
            "future-claim",
            vec![evidence.clone()],
            SimTime::from_minutes(1),
        ),
        (
            2,
            "oversized-claim",
            vec![evidence.clone(); TechnologyLimitsV1::canonical().max_collection_entries + 1],
            SimTime::EPOCH,
        ),
    ] {
        apply_command(
            &mut canwu,
            ids.0,
            request_id,
            TechnologyCommandEnvelope {
                id: id.to_owned(),
                subject: KnowledgeHolderRef::Person(ids.0),
                change: TechnologyRecordChange::Create {
                    id: format!("{id}-record"),
                    value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                        asserted_by: KnowledgeHolderRef::Person(ids.0),
                        proposition: "bounded claim".to_owned(),
                        scope: vec![],
                        source_evidence,
                        relations: vec![],
                        asserted_at,
                    }),
                },
            },
        );
        let outcome = canwu
            .typed_domain_record(&canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(id))
            .expect("invalid claim should have a terminal outcome")
            .decode_payload::<TechnologyOperation>()
            .expect("claim outcome");
        assert_eq!(outcome.status, TechnologyOperationStatus::Rejected);
    }

    apply_command(
        &mut canwu,
        ids.0,
        3,
        TechnologyCommandEnvelope {
            id: "future-fact-program-operation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "future-fact-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(test_program(
                    ids.0,
                    ids.2,
                    catalog.revision.clone(),
                    ProgramMode::Investigation,
                )),
            },
        },
    );
    let program = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("future-fact-program")
            .into_untyped(),
    );
    let evaluation = evaluate_attempt(
        &catalog.revision_payload,
        &catalog.spec_payload,
        &BTreeMap::from([(
            catalog.reliability.record.clone(),
            MetricSchemaPayload {
                label: "reliability".to_owned(),
                unit: "permille".to_owned(),
                scale: 1_000,
                minimum: 0,
                maximum: 1_000,
            },
        )]),
        &MetricContext {
            values: BTreeMap::from([(catalog.reliability.record.clone(), 800)]),
        },
    )
    .expect("future fact evaluation");
    let experiment_intent = authorize_intent(
        &mut canwu,
        ids.0,
        4,
        "future-attempt-intent",
        program.clone(),
        "future-provider",
        TechnologyIntentRequest::Experiment {
            result_id: "future-attempt".to_owned(),
            revision: catalog.revision.clone(),
            operation: "operate".to_owned(),
            site: EntityRef::Territory(ids.2),
            operator: Some(KnowledgeHolderRef::Person(ids.0)),
            required_assets: vec![],
        },
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "future-attempt-operation".to_owned(),
            provider: "future-provider".to_owned(),
            execution_intent: Some(experiment_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "future-attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                    execution_intent: experiment_intent,
                    program: program.clone(),
                    revision: catalog.revision.clone(),
                    operator: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    operation: "operate".to_owned(),
                    inputs: vec![],
                    environment: vec![],
                    outputs: vec![MetricValue {
                        metric: catalog.reliability.clone(),
                        value: 800,
                    }],
                    assets: vec![],
                    started_at: SimTime::EPOCH,
                    ended_at: SimTime::from_minutes(1),
                    evaluation: evaluation.clone(),
                }),
            },
        },
    );
    let production_intent = authorize_intent(
        &mut canwu,
        ids.0,
        5,
        "future-production-intent",
        program,
        "future-provider",
        TechnologyIntentRequest::Production {
            result_id: "future-production".to_owned(),
            revision: catalog.revision.clone(),
            application: Some(catalog.application.clone()),
            site: EntityRef::Territory(ids.2),
            operator: Some(KnowledgeHolderRef::Person(ids.0)),
            required_assets: vec![],
        },
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "future-production-operation".to_owned(),
            provider: "future-provider".to_owned(),
            execution_intent: Some(production_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "future-production".to_owned(),
                value: TechnologyRecordPayload::ProductionRun(
                    canwu_technology::ProductionRunPayload {
                        execution_intent: production_intent,
                        revision: catalog.revision,
                        application: Some(catalog.application),
                        operator: KnowledgeHolderRef::Person(ids.0),
                        site: EntityRef::Territory(ids.2),
                        assets: vec![],
                        inputs: vec![],
                        outputs: vec![MetricValue {
                            metric: catalog.reliability,
                            value: 800,
                        }],
                        started_at: SimTime::EPOCH,
                        ended_at: SimTime::from_minutes(1),
                        successful: evaluation.passed,
                        evaluation,
                    },
                ),
            },
        },
    );
    for operation in ["future-attempt-operation", "future-production-operation"] {
        let outcome = canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(operation),
            )
            .expect("future fact should have a terminal outcome")
            .decode_payload::<TechnologyOperation>()
            .expect("future fact outcome");
        assert_eq!(outcome.status, TechnologyOperationStatus::Rejected);
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn causal_time_cuts_reject_backdated_results_observations_and_assessments() {
    let (scenario, ids, catalog) = scenario();
    let mut canwu = Canwu::new_with_plugins(50, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    apply_command(
        &mut canwu,
        ids.0,
        1,
        TechnologyCommandEnvelope {
            id: "causal-program-operation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "causal-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(test_program(
                    ids.0,
                    ids.2,
                    catalog.revision.clone(),
                    ProgramMode::Investigation,
                )),
            },
        },
    );
    let program = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("causal-program").into_untyped(),
    );
    apply_command(
        &mut canwu,
        ids.0,
        2,
        TechnologyCommandEnvelope {
            id: "old-claim-operation".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "old-claim".to_owned(),
                value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                    asserted_by: KnowledgeHolderRef::Person(ids.0),
                    proposition: "evidence available at epoch".to_owned(),
                    scope: vec![],
                    source_evidence: vec![],
                    relations: vec![],
                    asserted_at: SimTime::EPOCH,
                }),
            },
        },
    );
    let old_claim = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<canwu_technology::TechnicalClaim>::new("old-claim")
            .into_untyped(),
    );

    let delayed_at = SimTime::from_minutes(1);
    for (request_id, id, request) in [
        (
            3,
            "backdated-attempt-intent",
            TechnologyIntentRequest::Experiment {
                result_id: "backdated-attempt".to_owned(),
                revision: catalog.revision.clone(),
                operation: "operate".to_owned(),
                site: EntityRef::Territory(ids.2),
                operator: Some(KnowledgeHolderRef::Person(ids.0)),
                required_assets: vec![],
            },
        ),
        (
            4,
            "valid-attempt-intent",
            TechnologyIntentRequest::Experiment {
                result_id: "valid-causal-attempt".to_owned(),
                revision: catalog.revision.clone(),
                operation: "operate".to_owned(),
                site: EntityRef::Territory(ids.2),
                operator: Some(KnowledgeHolderRef::Person(ids.0)),
                required_assets: vec![],
            },
        ),
        (
            5,
            "backdated-production-intent",
            TechnologyIntentRequest::Production {
                result_id: "backdated-production".to_owned(),
                revision: catalog.revision.clone(),
                application: Some(catalog.application.clone()),
                site: EntityRef::Territory(ids.2),
                operator: Some(KnowledgeHolderRef::Person(ids.0)),
                required_assets: vec![],
            },
        ),
    ] {
        apply_command(
            &mut canwu,
            ids.0,
            request_id,
            TechnologyCommandEnvelope {
                id: format!("authorize-{id}"),
                subject: KnowledgeHolderRef::Person(ids.0),
                change: TechnologyRecordChange::Create {
                    id: id.to_owned(),
                    value: TechnologyRecordPayload::ExecutionIntent(
                        TechnologyExecutionIntentPayload {
                            authorized_by: KnowledgeHolderRef::Person(ids.0),
                            program: program.clone(),
                            provider: "causal-provider".to_owned(),
                            request,
                            not_before: delayed_at,
                            expires_at: Some(delayed_at),
                            state: TechnologyIntentState::Pending,
                        },
                    ),
                },
            },
        );
    }
    canwu
        .settle_boundary(BoundaryRequest::at(delayed_at))
        .expect("runtime should reach the delayed authorization cut");

    let metrics = BTreeMap::from([
        (
            catalog.reliability.record.clone(),
            MetricSchemaPayload {
                label: "reliability".to_owned(),
                unit: "permille".to_owned(),
                scale: 1_000,
                minimum: 0,
                maximum: 1_000,
            },
        ),
        (
            catalog.fuel_cost.record.clone(),
            MetricSchemaPayload {
                label: "fuel cost".to_owned(),
                unit: "cost_units".to_owned(),
                scale: 1,
                minimum: 0,
                maximum: 1_000,
            },
        ),
    ]);
    let evaluation = evaluate_attempt(
        &catalog.revision_payload,
        &catalog.spec_payload,
        &metrics,
        &MetricContext {
            values: BTreeMap::from([(catalog.reliability.record.clone(), 800)]),
        },
    )
    .expect("causal fixture should evaluate");
    let backdated_attempt_intent = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(
            "backdated-attempt-intent",
        )
        .into_untyped(),
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "backdated-attempt-operation".to_owned(),
            provider: "causal-provider".to_owned(),
            execution_intent: Some(backdated_attempt_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "backdated-attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                    execution_intent: backdated_attempt_intent,
                    program: program.clone(),
                    revision: catalog.revision.clone(),
                    operator: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    operation: "operate".to_owned(),
                    inputs: vec![],
                    environment: vec![],
                    outputs: vec![MetricValue {
                        metric: catalog.reliability.clone(),
                        value: 800,
                    }],
                    assets: vec![],
                    started_at: SimTime::EPOCH,
                    ended_at: delayed_at,
                    evaluation: evaluation.clone(),
                }),
            },
        },
    );

    let production_intent = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(
            "backdated-production-intent",
        )
        .into_untyped(),
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "backdated-production-operation".to_owned(),
            provider: "causal-provider".to_owned(),
            execution_intent: Some(production_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "backdated-production".to_owned(),
                value: TechnologyRecordPayload::ProductionRun(
                    canwu_technology::ProductionRunPayload {
                        execution_intent: production_intent,
                        revision: catalog.revision.clone(),
                        application: Some(catalog.application.clone()),
                        operator: KnowledgeHolderRef::Person(ids.0),
                        site: EntityRef::Territory(ids.2),
                        assets: vec![],
                        inputs: vec![],
                        outputs: vec![MetricValue {
                            metric: catalog.reliability.clone(),
                            value: 800,
                        }],
                        started_at: SimTime::EPOCH,
                        ended_at: delayed_at,
                        successful: evaluation.passed,
                        evaluation: evaluation.clone(),
                    },
                ),
            },
        },
    );

    let valid_intent = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new("valid-attempt-intent")
            .into_untyped(),
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "valid-causal-attempt-operation".to_owned(),
            provider: "causal-provider".to_owned(),
            execution_intent: Some(valid_intent.clone()),
            change: TechnologyRecordChange::Create {
                id: "valid-causal-attempt".to_owned(),
                value: TechnologyRecordPayload::ExperimentAttempt(ExperimentAttemptPayload {
                    execution_intent: valid_intent,
                    program,
                    revision: catalog.revision,
                    operator: KnowledgeHolderRef::Person(ids.0),
                    site: EntityRef::Territory(ids.2),
                    operation: "operate".to_owned(),
                    inputs: vec![],
                    environment: vec![],
                    outputs: vec![MetricValue {
                        metric: catalog.reliability,
                        value: 800,
                    }],
                    assets: vec![],
                    started_at: delayed_at,
                    ended_at: delayed_at,
                    evaluation,
                }),
            },
        },
    );
    let valid_attempt = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<ExperimentAttempt>::new("valid-causal-attempt")
            .into_untyped(),
    );
    apply_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "backdated-observation-operation".to_owned(),
            provider: "causal-provider".to_owned(),
            execution_intent: None,
            change: TechnologyRecordChange::Create {
                id: "backdated-observation".to_owned(),
                value: TechnologyRecordPayload::AttemptObservation(AttemptObservationPayload {
                    attempt: valid_attempt.clone(),
                    observer: KnowledgeHolderRef::Person(ids.0),
                    method: "backdated observation".to_owned(),
                    values: vec![],
                    uncertainty_per_mille: 100,
                    observed_at: SimTime::EPOCH,
                }),
            },
        },
    );

    for (request_id, id, claim, supporting_evidence, as_of) in [
        (
            6,
            "future-support-assessment",
            old_claim,
            vec![EvidenceRef::DomainRecordVersion(valid_attempt)],
            SimTime::EPOCH,
        ),
        (
            7,
            "pre-claim-assessment",
            current_version(
                &canwu,
                &canwu_api::TypedDomainRecordRef::<canwu_technology::TechnicalClaim>::new(
                    "old-claim",
                )
                .into_untyped(),
            ),
            vec![],
            SimTime::from_minutes(-1),
        ),
    ] {
        apply_command(
            &mut canwu,
            ids.0,
            request_id,
            TechnologyCommandEnvelope {
                id: format!("{id}-operation"),
                subject: KnowledgeHolderRef::Person(ids.0),
                change: TechnologyRecordChange::Create {
                    id: id.to_owned(),
                    value: TechnologyRecordPayload::ClaimAssessment(ClaimAssessmentPayload {
                        claim,
                        assessor: KnowledgeHolderRef::Person(ids.0),
                        confidence_per_mille: 500,
                        method: "temporal audit".to_owned(),
                        supporting_evidence,
                        contradicting_evidence: vec![],
                        as_of,
                    }),
                },
            },
        );
    }

    for operation in [
        "backdated-attempt-operation",
        "backdated-production-operation",
        "backdated-observation-operation",
        "future-support-assessment-operation",
        "pre-claim-assessment-operation",
    ] {
        assert_eq!(
            canwu
                .typed_domain_record(
                    &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(operation,)
                )
                .expect("causal rejection should be terminal")
                .decode_payload::<TechnologyOperation>()
                .expect("causal rejection should decode")
                .status,
            TechnologyOperationStatus::Rejected,
            "{operation}"
        );
    }
    assert!(
        canwu
            .typed_domain_record(&canwu_api::TypedDomainRecordRef::<ClaimAssessment>::new(
                "future-support-assessment"
            ))
            .is_none()
    );
    validate_technology_runtime(&canwu).expect("causal rejections must restore exactly");
}

#[test]
#[allow(clippy::too_many_lines)]
fn compact_archive_declares_old_payload_dependencies_and_restores_for_continuation() {
    let (scenario, ids, catalog) = scenario();
    let plugins: [&dyn SimulationPlugin; 1] = [&TechnologyPlugin];
    let mut canwu = Canwu::new_with_plugins(51, scenario, &plugins)
        .expect("technology plugin should initialize");
    let original = test_program(
        ids.0,
        ids.2,
        catalog.revision.clone(),
        ProgramMode::Investigation,
    );
    apply_command(
        &mut canwu,
        ids.0,
        1,
        TechnologyCommandEnvelope {
            id: "archive-program-create".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "archive-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(original.clone()),
            },
        },
    );
    let program_v1 = current_version(
        &canwu,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("archive-program").into_untyped(),
    );
    apply_command(
        &mut canwu,
        ids.0,
        2,
        TechnologyCommandEnvelope {
            id: "archive-transmission-create".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "archive-transmission".to_owned(),
                value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                    source: None,
                    source_site: None,
                    source_capability: None,
                    destination: KnowledgeHolderRef::Person(ids.0),
                    destination_site: EntityRef::Territory(ids.2),
                    revision: Some(catalog.revision),
                    mode: TransmissionMode::IndependentInvestigation,
                    evidence: vec![EvidenceRef::DomainRecordVersion(program_v1.clone())],
                    resulting_program: Some(program_v1.clone()),
                    opened_at: SimTime::EPOCH,
                    active: true,
                }),
            },
        },
    );
    let mut updated = original;
    updated.status = ProgramStatus::Paused;
    apply_command(
        &mut canwu,
        ids.0,
        3,
        TechnologyCommandEnvelope {
            id: "archive-program-update".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Update {
                id: "archive-program".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::TechnicalProgram(updated),
            },
        },
    );
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("generated evidence should enter a completed causal prefix");

    let mut compact = canwu.into_compacted().expect("enter compact mode");
    assert_eq!(
        compact
            .seal_evidence()
            .expect_err("payload-required evidence needs provider-backed sealing")
            .code,
        ErrorCode::ArchiveNotReady
    );
    let prepared = compact
        .prepare_evidence_seal()
        .expect("prepare archive")
        .expect("technology run should have a retained evidence tail");
    let archive = TestArchive::default();
    assert_eq!(
        archive
            .store_evidence_segment(&prepared.segment)
            .expect("store archive segment"),
        ArchiveStoreOutcome::Stored
    );
    compact
        .commit_evidence_seal(&prepared.token, &archive)
        .expect("commit provider-backed archive");
    let checkpoint = compact.checkpoint().expect("compact checkpoint");
    assert!(checkpoint.evidence_dependencies.iter().any(|dependency| {
        dependency.reference == EvidenceRef::DomainRecordVersion(program_v1.clone())
    }));

    let mut restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        checkpoint,
        vec![prepared.segment],
        &plugins,
    )
    .expect("archived technology runtime should restore with exact payload evidence");
    let command = TechnologyCommandEnvelope {
        id: "post-archive-claim-operation".to_owned(),
        subject: KnowledgeHolderRef::Person(ids.0),
        change: TechnologyRecordChange::Create {
            id: "post-archive-claim".to_owned(),
            value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                asserted_by: KnowledgeHolderRef::Person(ids.0),
                proposition: "archive continuation remains executable".to_owned(),
                scope: vec![],
                source_evidence: vec![EvidenceRef::DomainRecordVersion(program_v1)],
                relations: vec![],
                asserted_at: restored.time(),
            }),
        },
    };
    restored
        .enqueue_command(
            restored.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(4),
                restored.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(ids.0),
                    Command::Plugin {
                        plugin: "canwu-technology".to_owned(),
                        command: TECHNOLOGY_COMMAND.to_owned(),
                        payload: serde_json::to_value(command).expect("post-archive command"),
                    },
                )
                .at_time(restored.time()),
            ),
        )
        .expect("post-archive command should enqueue");
    restored
        .settle_boundary(BoundaryRequest::at(restored.time()))
        .expect("post-archive command should admit");
    restored
        .settle_boundary(BoundaryRequest::at(restored.time()))
        .expect("technology should continue after reconstruction");
    let outcome = restored
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "post-archive-claim-operation",
            ),
        )
        .expect("post-archive operation")
        .decode_payload::<TechnologyOperation>()
        .expect("post-archive operation payload");
    assert_eq!(outcome.status, TechnologyOperationStatus::Applied);
}

#[test]
fn operation_records_cannot_bypass_admitted_provenance() {
    let (mut scenario, _, _) = scenario();
    let payload = TechnologyOperationPayload {
        id: "injected-operation".to_owned(),
        canonical_input_hash: "0".repeat(64),
        canonical_input_hashes: vec!["0".repeat(64)],
        causes: vec![EvidenceRef::DomainRecordVersion(initial_record_version::<
            TechnicalProgram,
        >("missing-cause"))],
        provider: None,
        execution_intent: None,
        status: TechnologyOperationStatus::Rejected,
        result: None,
        rejection_code: Some("invalid_domain_record".to_owned()),
    };
    let draft = initial_operation_draft(&payload);
    scenario.domain_records.push(DomainRecord {
        reference: draft.reference,
        owner: "canwu-technology".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    });
    let canwu = Canwu::new_with_plugins(53, scenario, &[&TechnologyPlugin])
        .expect("kernel accepts plugin-owned initial payload shape");
    let error = validate_technology_runtime(&canwu)
        .expect_err("module restore validation must reject injected operations");
    assert_eq!(error.code, canwu_api::ErrorCode::InvalidDomainRecord);
}

#[test]
fn public_snapshot_wrapper_rejects_kernel_valid_domain_invalid_state() {
    let (mut scenario, _, _) = scenario();
    let payload = TechnologyOperationPayload {
        id: "injected-operation".to_owned(),
        canonical_input_hash: "0".repeat(64),
        canonical_input_hashes: vec!["0".repeat(64)],
        causes: vec![EvidenceRef::DomainRecordVersion(initial_record_version::<
            TechnicalProgram,
        >("missing-cause"))],
        provider: None,
        execution_intent: None,
        status: TechnologyOperationStatus::Rejected,
        result: None,
        rejection_code: Some("invalid_domain_record".to_owned()),
    };
    let draft = initial_operation_draft(&payload);
    scenario.domain_records.push(DomainRecord {
        reference: draft.reference,
        owner: "canwu-technology".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    });
    let canwu = Canwu::new_with_plugins(59, scenario, &[&TechnologyPlugin])
        .expect("the kernel should accept the schema-valid plugin record");
    let json = serde_json::to_string(&canwu.snapshot()).expect("snapshot should encode");
    let Err(error) = from_technology_snapshot_json(&json, &[&TechnologyPlugin]) else {
        panic!("the public wrapper must run technology domain validation");
    };
    assert_eq!(error.code, canwu_api::ErrorCode::InvalidDomainRecord);
    assert!(
        error.message.contains("technology evidence"),
        "the failure must come from technology validation, not kernel commitments: {error:?}"
    );
}

#[test]
fn public_validation_replays_future_exact_evidence_as_unavailable_at_the_original_cut() {
    let (oracle_scenario, oracle_ids, oracle_catalog) = scenario();
    let destination_program = test_program(
        oracle_ids.1,
        oracle_ids.3,
        oracle_catalog.revision.clone(),
        ProgramMode::Training,
    );
    let transmission = |resulting_program| TechnologyCommandEnvelope {
        id: "future-exact-transmission".to_owned(),
        subject: KnowledgeHolderRef::Person(oracle_ids.1),
        change: TechnologyRecordChange::Create {
            id: "future-exact-link".to_owned(),
            value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                source: None,
                source_site: None,
                source_capability: None,
                destination: KnowledgeHolderRef::Person(oracle_ids.1),
                destination_site: EntityRef::Territory(oracle_ids.3),
                revision: Some(oracle_catalog.revision.clone()),
                mode: TransmissionMode::IndependentInvestigation,
                evidence: vec![],
                resulting_program: Some(resulting_program),
                opened_at: SimTime::EPOCH,
                active: true,
            }),
        },
    };
    let create_program = program_command(
        "create-future-program",
        "future-program",
        oracle_ids.1,
        TechnologyRecordChange::Create {
            id: String::new(),
            value: TechnologyRecordPayload::TechnicalProgram(destination_program),
        },
    );
    let mut oracle = Canwu::new_with_plugins(61, oracle_scenario, &[&TechnologyPlugin])
        .expect("oracle runtime should initialize");
    apply_command(
        &mut oracle,
        oracle_ids.1,
        1,
        transmission(initial_record_version::<TechnicalProgram>(
            "missing-program",
        )),
    );
    apply_command(&mut oracle, oracle_ids.1, 2, create_program.clone());
    let future_program = current_version(
        &oracle,
        &canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new("future-program").into_untyped(),
    );

    let (scenario, ids, _) = scenario();
    let mut canwu = Canwu::new_with_plugins(61, scenario, &[&TechnologyPlugin])
        .expect("regression runtime should initialize");
    apply_command(&mut canwu, ids.1, 1, transmission(future_program));
    let early_outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "future-exact-transmission",
            ),
        )
        .expect("early future-exact operation must be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("operation should decode");
    assert_eq!(early_outcome.status, TechnologyOperationStatus::Rejected);
    apply_command(&mut canwu, ids.1, 2, create_program);
    let json = serde_json::to_string(&canwu.snapshot()).expect("snapshot should encode");
    from_technology_snapshot_json(&json, &[&TechnologyPlugin])
        .expect("restoration must preserve the earlier rejection despite later exact evidence");
}

#[test]
fn restoration_makes_current_command_and_generated_ingress_visible() {
    let (oracle_scenario, oracle_ids, _) = scenario();
    let claim = |source_evidence| TechnologyCommandEnvelope {
        id: "current-evidence-claim-operation".to_owned(),
        subject: KnowledgeHolderRef::Person(oracle_ids.0),
        change: TechnologyRecordChange::Create {
            id: "current-evidence-claim".to_owned(),
            value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                asserted_by: KnowledgeHolderRef::Person(oracle_ids.0),
                proposition: "current admission evidence is visible".to_owned(),
                scope: vec![],
                source_evidence,
                relations: vec![],
                asserted_at: SimTime::EPOCH,
            }),
        },
    };
    let mut oracle = Canwu::new_with_plugins(67, oracle_scenario, &[&TechnologyPlugin])
        .expect("oracle runtime should initialize");
    apply_command(
        &mut oracle,
        oracle_ids.0,
        1,
        claim(vec![EvidenceRef::Command(canwu_api::CommandId::new(999))]),
    );
    let current_command = oracle.commands().last().expect("admitted command").id;
    let current_ingress = oracle
        .boundaries()
        .last()
        .and_then(|boundary| boundary.admitted_ingress.first())
        .copied()
        .expect("technology operation boundary should admit command ingress");

    let (scenario, ids, _) = scenario();
    let mut canwu = Canwu::new_with_plugins(67, scenario, &[&TechnologyPlugin])
        .expect("current-evidence runtime should initialize");
    apply_command(
        &mut canwu,
        ids.0,
        1,
        claim(vec![
            EvidenceRef::Command(current_command),
            EvidenceRef::Ingress(current_ingress),
        ]),
    );
    let outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                "current-evidence-claim-operation",
            ),
        )
        .expect("current-evidence operation should be terminal")
        .decode_payload::<TechnologyOperation>()
        .expect("operation should decode");
    assert_eq!(outcome.status, TechnologyOperationStatus::Applied);
    validate_technology_runtime(&canwu)
        .expect("the current command and pre-phase-7 generated ingress must be visible");
}

#[test]
fn restoration_rejects_current_boundary_and_future_command_or_ingress_evidence() {
    let (oracle_scenario, oracle_ids, oracle_catalog) = scenario();
    let claim = |evidence| TechnologyCommandEnvelope {
        id: "cut-evidence-claim-operation".to_owned(),
        subject: KnowledgeHolderRef::Person(oracle_ids.0),
        change: TechnologyRecordChange::Create {
            id: "cut-evidence-claim".to_owned(),
            value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                asserted_by: KnowledgeHolderRef::Person(oracle_ids.0),
                proposition: "evidence visibility follows its restoration cut".to_owned(),
                scope: vec![],
                source_evidence: vec![evidence],
                relations: vec![],
                asserted_at: SimTime::EPOCH,
            }),
        },
    };
    let later_command = program_command(
        "create-future-cut-program",
        "future-cut-program",
        oracle_ids.0,
        TechnologyRecordChange::Create {
            id: String::new(),
            value: TechnologyRecordPayload::TechnicalProgram(test_program(
                oracle_ids.0,
                oracle_ids.2,
                oracle_catalog.revision,
                ProgramMode::Training,
            )),
        },
    );
    let mut oracle = Canwu::new_with_plugins(71, oracle_scenario, &[&TechnologyPlugin])
        .expect("oracle runtime should initialize");
    apply_command(
        &mut oracle,
        oracle_ids.0,
        1,
        claim(EvidenceRef::Command(canwu_api::CommandId::new(999))),
    );
    let current_boundary = oracle.boundaries().last().expect("claim boundary").id;
    apply_command(&mut oracle, oracle_ids.0, 2, later_command.clone());
    let future_command = oracle.commands().last().expect("future command").id;
    let future_ingress = oracle
        .boundaries()
        .last()
        .and_then(|boundary| boundary.admitted_ingress.first())
        .copied()
        .expect("future command boundary should admit command ingress");

    for (label, evidence) in [
        ("current-boundary", EvidenceRef::Boundary(current_boundary)),
        ("future-command", EvidenceRef::Command(future_command)),
        ("future-ingress", EvidenceRef::Ingress(future_ingress)),
    ] {
        let (scenario, ids, _) = scenario();
        let mut canwu = Canwu::new_with_plugins(71, scenario, &[&TechnologyPlugin])
            .expect("future-evidence runtime should initialize");
        apply_command(&mut canwu, ids.0, 1, claim(evidence));
        let outcome = canwu
            .typed_domain_record(
                &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(
                    "cut-evidence-claim-operation",
                ),
            )
            .expect("cut-evidence operation should be terminal")
            .decode_payload::<TechnologyOperation>()
            .expect("operation should decode");
        assert_eq!(
            outcome.status,
            TechnologyOperationStatus::Rejected,
            "{label}"
        );
        if label != "current-boundary" {
            apply_command(&mut canwu, ids.0, 2, later_command.clone());
        }
        validate_technology_runtime(&canwu)
            .unwrap_or_else(|error| panic!("{label} must replay as unavailable: {error:?}"));
    }
}

fn collision_envelope(label: &str) -> TechnologyResultEnvelope {
    TechnologyResultEnvelope {
        id: "collision".to_owned(),
        provider: "passive-reader".to_owned(),
        execution_intent: None,
        change: TechnologyRecordChange::Create {
            id: format!("observation-{label}"),
            value: TechnologyRecordPayload::AttemptObservation(AttemptObservationPayload {
                attempt: initial_record_version::<ExperimentAttempt>("unresolved-attempt"),
                observer: KnowledgeHolderRef::Person(PersonId::new(1)),
                method: format!("method-{label}"),
                values: vec![],
                uncertainty_per_mille: 100,
                observed_at: SimTime::EPOCH,
            }),
        },
    }
}

fn initial_operation_draft(payload: &TechnologyOperationPayload) -> canwu_api::DomainRecordDraft {
    let mut draft = canwu_api::DomainRecordDraft::from_typed(
        canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new(&payload.id),
        payload,
    )
    .expect("operation draft should encode");
    draft
        .payload
        .as_object_mut()
        .expect("operation payload should be an object")
        .insert(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            serde_json::to_value(PayloadRequiredEvidenceContinuationV1::completed())
                .expect("completed continuation should encode"),
        );
    draft
}

#[test]
fn normal_update_cannot_transfer_technology_record_ownership() {
    let (scenario, ids, catalog) = scenario();
    let mut canwu = Canwu::new_with_plugins(37, scenario, &[&TechnologyPlugin])
        .expect("technology plugin should initialize");
    let original = TechnicalProgramPayload {
        sponsor: KnowledgeHolderRef::Person(ids.0),
        site: EntityRef::Territory(ids.2),
        revision: Some(catalog.revision),
        mode: ProgramMode::Adaptation,
        status: ProgramStatus::Active,
        requirements: vec![],
        started_at: SimTime::EPOCH,
        due_at: None,
    };
    apply_command(
        &mut canwu,
        ids.0,
        1,
        TechnologyCommandEnvelope {
            id: "create-owned-program".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.0),
            change: TechnologyRecordChange::Create {
                id: "owned-program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(original.clone()),
            },
        },
    );
    let mut takeover = original;
    takeover.sponsor = KnowledgeHolderRef::Person(ids.1);
    apply_command(
        &mut canwu,
        ids.1,
        2,
        TechnologyCommandEnvelope {
            id: "takeover".to_owned(),
            subject: KnowledgeHolderRef::Person(ids.1),
            change: TechnologyRecordChange::Update {
                id: "owned-program".to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::TechnicalProgram(takeover),
            },
        },
    );
    let outcome = canwu
        .typed_domain_record(
            &canwu_api::TypedDomainRecordRef::<TechnologyOperation>::new("takeover"),
        )
        .expect("takeover should have a terminal outcome")
        .decode_payload::<TechnologyOperation>()
        .expect("operation should decode");
    assert_eq!(outcome.status, TechnologyOperationStatus::Rejected);
    let program = canwu
        .typed_domain_record(&canwu_api::TypedDomainRecordRef::<TechnicalProgram>::new(
            "owned-program",
        ))
        .expect("original program should remain")
        .decode_payload::<TechnicalProgram>()
        .expect("program should decode");
    assert_eq!(program.sponsor, KnowledgeHolderRef::Person(ids.0));
}

struct Catalog {
    reliability: DomainRecordVersionRef,
    fuel_cost: DomainRecordVersionRef,
    revision: DomainRecordVersionRef,
    application: DomainRecordVersionRef,
    revision_payload: TechniqueRevisionPayload,
    spec_payload: TechniqueSpecPayload,
    application_payload: ApplicationSpecPayload,
}

#[allow(clippy::too_many_lines)]
fn scenario() -> (
    Scenario,
    (PersonId, PersonId, TerritoryId, TerritoryId),
    Catalog,
) {
    let actor = PersonId::new(1);
    let learner = PersonId::new(2);
    let government = GovernmentId::new(1);
    let first = TerritoryId::new(1);
    let second = TerritoryId::new(2);
    let reliability = initial_record_version::<MetricSchema>("reliability");
    let fuel_cost = initial_record_version::<MetricSchema>("fuel-cost");
    let technique = initial_record_version::<TechniqueSpec>("technique");
    let revision = initial_record_version::<TechniqueRevision>("revision");
    let application = initial_record_version::<ApplicationSpec>("application");
    let spec_payload = TechniqueSpecPayload {
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
    let application_payload = ApplicationSpecPayload {
        label: "drain a deep working".to_owned(),
        technique: technique.clone(),
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
        TechnologyCatalogRecord::Metric(MetricSchemaPayload {
            label: "reliability".to_owned(),
            unit: "permille".to_owned(),
            scale: 1_000,
            minimum: 0,
            maximum: 1_000,
        })
        .into_initial_record("reliability")
        .expect("metric"),
        TechnologyCatalogRecord::Metric(MetricSchemaPayload {
            label: "fuel cost".to_owned(),
            unit: "cost_units".to_owned(),
            scale: 1,
            minimum: 0,
            maximum: 1_000,
        })
        .into_initial_record("fuel-cost")
        .expect("metric"),
        TechnologyCatalogRecord::Technique(spec_payload.clone())
            .into_initial_record("technique")
            .expect("technique"),
        TechnologyCatalogRecord::Revision(TechniqueRevisionPayload {
            label: "neutral revision".to_owned(),
            spec: technique.clone(),
            parents: vec![],
            parameters: vec![],
            evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
            produced_by: None,
            execution_intent: None,
            discovery_evidence: vec![],
        })
        .into_initial_record("revision")
        .expect("revision"),
        TechnologyCatalogRecord::Application(application_payload.clone())
            .into_initial_record("application")
            .expect("application"),
    ];
    let world = WorldSnapshot {
        people: vec![
            Person {
                id: actor,
                name: "Operator".to_owned(),
                government,
                current_location: first,
                roles: vec![],
                transit: None,
            },
            Person {
                id: learner,
                name: "Learner".to_owned(),
                government,
                current_location: second,
                roles: vec![],
                transit: None,
            },
        ],
        governments: vec![Government {
            id: government,
            name: "Workshop authority".to_owned(),
            capital: first,
        }],
        territories: vec![
            Territory {
                id: first,
                name: "First site".to_owned(),
                controller: government,
                position: MapPoint::default(),
            },
            Territory {
                id: second,
                name: "Second site".to_owned(),
                controller: government,
                position: MapPoint { x: 1.0, y: 0.0 },
            },
        ],
        routes: vec![],
        armies: vec![],
        letters: vec![],
    };
    (
        Scenario {
            start_time: SimTime::EPOCH,
            world,
            knowledge: KnowledgeSnapshot::default(),
            domain_records: records,
        },
        (actor, learner, first, second),
        Catalog {
            reliability,
            fuel_cost,
            revision,
            application,
            revision_payload: TechniqueRevisionPayload {
                label: "neutral revision".to_owned(),
                spec: technique,
                parents: vec![],
                parameters: vec![],
                evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                produced_by: None,
                execution_intent: None,
                discovery_evidence: vec![],
            },
            spec_payload,
            application_payload,
        },
    )
}

fn test_program(
    sponsor: PersonId,
    site: TerritoryId,
    revision: DomainRecordVersionRef,
    mode: ProgramMode,
) -> TechnicalProgramPayload {
    TechnicalProgramPayload {
        sponsor: KnowledgeHolderRef::Person(sponsor),
        site: EntityRef::Territory(site),
        revision: Some(revision),
        mode,
        status: ProgramStatus::Active,
        requirements: vec![],
        started_at: SimTime::EPOCH,
        due_at: None,
    }
}

fn program_command(
    operation: &str,
    program: &str,
    subject: PersonId,
    change: TechnologyRecordChange,
) -> TechnologyCommandEnvelope {
    TechnologyCommandEnvelope {
        id: operation.to_owned(),
        subject: KnowledgeHolderRef::Person(subject),
        change: match change {
            TechnologyRecordChange::Create { value, .. } => TechnologyRecordChange::Create {
                id: program.to_owned(),
                value,
            },
            TechnologyRecordChange::Update {
                expected_version,
                value,
                ..
            } => TechnologyRecordChange::Update {
                id: program.to_owned(),
                expected_version,
                value,
            },
        },
    }
}

fn enqueue_technology_command(
    canwu: &mut Canwu,
    actor: PersonId,
    request_id: u64,
    envelope: TechnologyCommandEnvelope,
) {
    canwu
        .enqueue_command(
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
                        payload: serde_json::to_value(envelope).expect("command payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("command should enqueue");
}

fn apply_command(
    canwu: &mut Canwu,
    actor: PersonId,
    request_id: u64,
    envelope: TechnologyCommandEnvelope,
) {
    enqueue_technology_command(canwu, actor, request_id, envelope);
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("technology command should be admitted");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("technology command boundary should settle");
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
) -> DomainRecordVersionRef {
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
    );
    current_version(
        canwu,
        &canwu_api::TypedDomainRecordRef::<TechnologyExecutionIntent>::new(id).into_untyped(),
    )
}

fn apply_result(canwu: &mut Canwu, envelope: TechnologyResultEnvelope) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canwu-technology",
            TECHNOLOGY_RESULT_INGRESS,
            canwu.time(),
            serde_json::to_value(envelope).expect("result payload"),
        ))
        .expect("result ingress should enqueue");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("technology result boundary should settle");
}

fn current_version(canwu: &Canwu, reference: &DomainRecordRef) -> DomainRecordVersionRef {
    let record = canwu.domain_record(reference).expect("record should exist");
    for boundary in canwu.boundaries().iter().rev() {
        for (change_index, change) in boundary.record_changes.iter().enumerate().rev() {
            if change.current.reference == *reference && change.current.version == record.version {
                return DomainRecordVersionRef {
                    record: reference.clone(),
                    version: record.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: change_index as u64,
                    },
                };
            }
        }
    }
    DomainRecordVersionRef {
        record: reference.clone(),
        version: record.version,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}
