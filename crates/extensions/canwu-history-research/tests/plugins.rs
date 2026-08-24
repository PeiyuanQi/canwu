use canwu_api::{
    BoundaryRequest, Canwu, Command, CommandAttemptOutcome, CommandEnvelope, CommandId,
    CommandRequest, CommandRequestId, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainReference, DomainReferenceTarget, EntityRef, ErrorCode, EvidenceRef, Government,
    GovernmentId, Issuer, KnowledgeHolderRef, KnowledgeSnapshot, MapPoint, Person, PersonId,
    PluginIngressRequest, Scenario, SimTime, Territory, TerritoryId, TypedDomainRecordRef,
    WorldSnapshot,
};
use canwu_history_research::{
    ASSESSMENT_COMMAND, ASSESSMENT_INGRESS, AssessmentCore, AssessmentRecord, HistoricalAnalysis,
    HistoricalAssessmentCommand, HistoricalPracticeAssessment, HistoricalPracticeAssessmentPayload,
    HistoricalPracticePlugin, HistoricalResearchSuite, HistoricalSourcesAssessment,
    HistoricalSourcesAssessmentPayload, HistoricalSourcesPlugin, ProductionArchaeologyAssessment,
    ProductionArchaeologyAssessmentPayload, ProductionArchaeologyPlugin,
    from_historical_research_snapshot_json, validate_historical_research_runtime,
};
use canwu_technology::{
    MetricSchema, MetricSchemaPayload, ProgramMode, ProgramStatus, TECHNOLOGY_COMMAND,
    TechnicalProgramPayload, TechnologyCatalogRecord, TechnologyCommandEnvelope, TechnologyPlugin,
    TechnologyRecordChange, TechnologyRecordPayload, initial_record_version,
};

#[test]
#[allow(clippy::too_many_lines)]
fn assessments_are_separately_selectable_and_cross_reference_exact_evidence() {
    let technology = TechnologyPlugin;
    let plugins = HistoricalResearchSuite::plugins();
    let actor = PersonId::new(1);
    let subject = initial_record_version::<MetricSchema>("pressure");
    let initial_scenario = scenario(actor);
    let mut canwu = Canwu::new_with_plugins(
        17,
        initial_scenario,
        &[&technology, plugins[0], plugins[1], plugins[2]],
    )
    .expect("research suite should initialize");

    enqueue_assessment(
        &mut canwu,
        actor,
        1,
        HistoricalSourcesAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: "sources-pressure".to_owned(),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalSourcesAssessmentPayload {
                core: core(actor, subject.clone()),
                earliest_date: SimTime::EPOCH,
                latest_date: SimTime::EPOCH,
                authenticity_per_mille: 900,
                reliability_per_mille: 750,
                provenance_digest: digest('b'),
            },
        },
    );
    let sources = current_version(
        &canwu,
        &TypedDomainRecordRef::<HistoricalSourcesAssessment>::new("sources-pressure")
            .into_untyped(),
    );

    let mut practice_core = core(actor, subject.clone());
    practice_core.contradicts.push(sources.clone());
    enqueue_assessment(
        &mut canwu,
        actor,
        2,
        HistoricalPracticeAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: "practice-pressure".to_owned(),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalPracticeAssessmentPayload {
                core: practice_core,
                participants: vec![EntityRef::Person(actor)],
                relation: "failed controlled reconstruction".to_owned(),
                notebook_digest: Some(digest('c')),
                negative_result: true,
            },
        },
    );

    let mut archaeology_core = core(actor, subject.clone());
    archaeology_core.citations.push(EvidenceRef::Boundary(
        canwu.boundaries().last().expect("practice boundary").id,
    ));
    enqueue_assessment(
        &mut canwu,
        actor,
        3,
        ProductionArchaeologyAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: "archaeology-pressure".to_owned(),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: ProductionArchaeologyAssessmentPayload {
                core: archaeology_core,
                observed_kind: "cylinder wear pattern".to_owned(),
                observed_digest: digest('d'),
                inferred_process_digest: digest('e'),
                earliest_date: SimTime::EPOCH,
                latest_date: SimTime::EPOCH,
            },
        },
    );

    let assessments = HistoricalAnalysis::for_subject(&canwu, &subject.record)
        .expect("historical analysis should remain a host-side read");
    assert_eq!(assessments.len(), 3);
    assert_eq!(assessments[0].subject, subject);
    assert!(
        HistoricalAnalysis::for_subject(&canwu, &sources.record)
            .expect("relation targets must not become assessment subjects")
            .is_empty()
    );

    let snapshot = canwu.snapshot_json().expect("snapshot should serialize");
    let Err(error) = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&technology]) else {
        panic!("omitting active research plugins must fail closed");
    };
    assert!(matches!(
        error.code,
        ErrorCode::PluginNotActive | ErrorCode::PluginManifestMismatch
    ));

    let restored = from_historical_research_snapshot_json(
        &snapshot,
        &[&technology, plugins[0], plugins[1], plugins[2]],
    )
    .expect("full research suite should restore");
    assert_eq!(restored.snapshot(), canwu.snapshot());
    validate_historical_research_runtime(&restored)
        .expect("restored assessments should satisfy deep semantics");

    Canwu::new_with_plugins(
        17,
        scenario(actor),
        &[&technology, &HistoricalSourcesPlugin],
    )
    .expect("sources plugin should be selectable alone");
    Canwu::new_with_plugins(
        17,
        scenario(actor),
        &[&technology, &HistoricalPracticePlugin],
    )
    .expect("practice plugin should be selectable alone");
    Canwu::new_with_plugins(
        17,
        scenario(actor),
        &[&technology, &ProductionArchaeologyPlugin],
    )
    .expect("production archaeology plugin should be selectable alone");
}

#[test]
fn contradiction_and_supersession_require_an_assessment_of_the_same_subject() {
    let technology = TechnologyPlugin;
    let sources_plugin = HistoricalSourcesPlugin;
    let practice_plugin = HistoricalPracticePlugin;
    let actor = PersonId::new(1);
    let technology_subject = initial_record_version::<MetricSchema>("pressure");
    let mut canwu = Canwu::new_with_plugins(
        19,
        scenario(actor),
        &[&technology, &sources_plugin, &practice_plugin],
    )
    .expect("plugins should initialize");
    enqueue_assessment(
        &mut canwu,
        actor,
        1,
        HistoricalSourcesAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: "sources-pressure".to_owned(),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalSourcesAssessmentPayload {
                core: core(actor, technology_subject),
                earliest_date: SimTime::EPOCH,
                latest_date: SimTime::EPOCH,
                authenticity_per_mille: 900,
                reliability_per_mille: 750,
                provenance_digest: digest('b'),
            },
        },
    );
    let sources = current_version(
        &canwu,
        &TypedDomainRecordRef::<HistoricalSourcesAssessment>::new("sources-pressure")
            .into_untyped(),
    );
    let mut mismatched = core(actor, sources.clone());
    mismatched.contradicts.push(sources);
    enqueue_rejected_assessment(
        &mut canwu,
        actor,
        2,
        HistoricalPracticeAssessment::PLUGIN_NAME,
        HistoricalAssessmentCommand {
            id: "mismatched-contradiction".to_owned(),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalPracticeAssessmentPayload {
                core: mismatched,
                participants: vec![],
                relation: "mismatched subject".to_owned(),
                notebook_digest: None,
                negative_result: true,
            },
        },
    );
    assert!(
        canwu
            .typed_domain_record(&TypedDomainRecordRef::<HistoricalPracticeAssessment>::new(
                "mismatched-contradiction"
            ))
            .is_none()
    );
}

#[test]
fn idle_historical_plugins_do_not_change_technology_records_or_outcomes() {
    let actor = PersonId::new(1);
    let initial = scenario(actor);
    let technology = TechnologyPlugin;
    let history = HistoricalResearchSuite::plugins();
    let mut technology_only =
        Canwu::new_with_plugins(20, initial.clone(), &[&technology]).expect("technology runtime");
    let mut with_history = Canwu::new_with_plugins(
        20,
        initial,
        &[&technology, history[0], history[1], history[2]],
    )
    .expect("technology plus history runtime");

    for canwu in [&mut technology_only, &mut with_history] {
        let command = TechnologyCommandEnvelope {
            id: "program-operation".to_owned(),
            subject: KnowledgeHolderRef::Person(actor),
            change: TechnologyRecordChange::Create {
                id: "program".to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    sponsor: KnowledgeHolderRef::Person(actor),
                    site: EntityRef::Territory(TerritoryId::new(1)),
                    revision: None,
                    mode: ProgramMode::Investigation,
                    status: ProgramStatus::Active,
                    requirements: vec![],
                    started_at: SimTime::EPOCH,
                    due_at: None,
                }),
            },
        };
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                CommandRequest::new(
                    CommandRequestId::new(1),
                    canwu.revision(),
                    CommandEnvelope::new(
                        Issuer::Actor(actor),
                        Command::Plugin {
                            plugin: "canwu-technology".to_owned(),
                            command: TECHNOLOGY_COMMAND.to_owned(),
                            payload: serde_json::to_value(command).expect("technology command"),
                        },
                    )
                    .at_time(canwu.time()),
                ),
            )
            .expect("technology command should enqueue");
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("technology command should admit");
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("technology operation should settle");
    }

    let technology_records = |canwu: &Canwu| {
        canwu
            .domain_records()
            .filter(|record| record.owner == "canwu-technology")
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        technology_records(&technology_only),
        technology_records(&with_history)
    );
}

#[test]
fn assessment_ingress_without_an_authorized_command_is_rejected() {
    let technology = TechnologyPlugin;
    let history = HistoricalSourcesPlugin;
    let actor = PersonId::new(1);
    let mut canwu = Canwu::new_with_plugins(23, scenario(actor), &[&technology, &history])
        .expect("plugins should initialize");
    let envelope = HistoricalAssessmentCommand {
        id: "forged-source".to_owned(),
        subject: KnowledgeHolderRef::Person(actor),
        assessment: HistoricalSourcesAssessmentPayload {
            core: core(actor, initial_record_version::<MetricSchema>("pressure")),
            earliest_date: SimTime::EPOCH,
            latest_date: SimTime::EPOCH,
            authenticity_per_mille: 900,
            reliability_per_mille: 900,
            provenance_digest: digest('f'),
        },
    };
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            HistoricalSourcesAssessment::PLUGIN_NAME,
            ASSESSMENT_INGRESS,
            canwu.time(),
            serde_json::to_value(envelope).expect("assessment payload"),
        ))
        .expect("provider ingress should enqueue before authorization is checked");
    let error = canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect_err("direct assessment ingress must fail closed");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
}

#[test]
fn assessment_with_missing_citation_is_rejected() {
    let technology = TechnologyPlugin;
    let history = HistoricalSourcesPlugin;
    let actor = PersonId::new(1);
    let mut canwu = Canwu::new_with_plugins(29, scenario(actor), &[&technology, &history])
        .expect("plugins should initialize");
    let mut assessment_core = core(actor, initial_record_version::<MetricSchema>("pressure"));
    assessment_core
        .citations
        .push(EvidenceRef::Command(CommandId::new(999)));
    let envelope = HistoricalAssessmentCommand {
        id: "missing-citation".to_owned(),
        subject: KnowledgeHolderRef::Person(actor),
        assessment: HistoricalSourcesAssessmentPayload {
            core: assessment_core,
            earliest_date: SimTime::EPOCH,
            latest_date: SimTime::EPOCH,
            authenticity_per_mille: 900,
            reliability_per_mille: 900,
            provenance_digest: digest('f'),
        },
    };
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(actor),
                    Command::Plugin {
                        plugin: HistoricalSourcesAssessment::PLUGIN_NAME.to_owned(),
                        command: ASSESSMENT_COMMAND.to_owned(),
                        payload: serde_json::to_value(envelope).expect("assessment payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("command should enqueue");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("missing citation should become a persisted command rejection");
    assert!(matches!(
        &canwu.command_attempts().last().expect("command attempt").outcome,
        CommandAttemptOutcome::Rejected { error }
            if error.code == ErrorCode::InvalidPayload
    ));
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a rejected assessment must not poison later settlement");
}

#[test]
fn historical_dating_ranges_cannot_extend_beyond_the_assessment_cut() {
    let actor = PersonId::new(1);
    let technology = TechnologyPlugin;
    let sources = HistoricalSourcesPlugin;
    let archaeology = ProductionArchaeologyPlugin;
    let mut canwu =
        Canwu::new_with_plugins(29, scenario(actor), &[&technology, &sources, &archaeology])
            .expect("plugins should initialize");
    let subject = initial_record_version::<MetricSchema>("pressure");

    let source = HistoricalAssessmentCommand {
        id: "future-dated-source".to_owned(),
        subject: KnowledgeHolderRef::Person(actor),
        assessment: HistoricalSourcesAssessmentPayload {
            core: core(actor, subject.clone()),
            earliest_date: SimTime::EPOCH,
            latest_date: SimTime::from_minutes(1),
            authenticity_per_mille: 900,
            reliability_per_mille: 900,
            provenance_digest: digest('f'),
        },
    };
    enqueue_rejected_assessment(
        &mut canwu,
        actor,
        1,
        HistoricalSourcesAssessment::PLUGIN_NAME,
        source,
    );

    let production = HistoricalAssessmentCommand {
        id: "future-dated-production".to_owned(),
        subject: KnowledgeHolderRef::Person(actor),
        assessment: ProductionArchaeologyAssessmentPayload {
            core: core(actor, subject),
            observed_kind: "workshop residue".to_owned(),
            observed_digest: digest('b'),
            inferred_process_digest: digest('c'),
            earliest_date: SimTime::EPOCH,
            latest_date: SimTime::from_minutes(1),
        },
    };
    enqueue_rejected_assessment(
        &mut canwu,
        actor,
        2,
        ProductionArchaeologyAssessment::PLUGIN_NAME,
        production,
    );

    assert_eq!(
        canwu
            .command_attempts()
            .iter()
            .filter(|attempt| matches!(&attempt.outcome, CommandAttemptOutcome::Rejected { .. }))
            .count(),
        2
    );
    assert!(
        canwu
            .typed_domain_record(&TypedDomainRecordRef::<HistoricalSourcesAssessment>::new(
                "future-dated-source"
            ))
            .is_none()
    );
    assert!(
        canwu
            .typed_domain_record(
                &TypedDomainRecordRef::<ProductionArchaeologyAssessment>::new(
                    "future-dated-production"
                )
            )
            .is_none()
    );
}

fn enqueue_rejected_assessment<T: serde::Serialize>(
    canwu: &mut Canwu,
    actor: PersonId,
    request_id: u64,
    plugin: &str,
    envelope: T,
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
                        plugin: plugin.to_owned(),
                        command: ASSESSMENT_COMMAND.to_owned(),
                        payload: serde_json::to_value(envelope).expect("assessment payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("assessment command should enqueue");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("invalid assessment should become a persisted command rejection");
}

#[test]
fn historical_boundary_cap_persists_overflow_rejection_without_poisoning() {
    let technology = TechnologyPlugin;
    let history = HistoricalSourcesPlugin;
    let actor = PersonId::new(1);
    let mut accepted = Canwu::new_with_plugins(30, scenario(actor), &[&technology, &history])
        .expect("plugins should initialize");
    enqueue_assessment_batch(&mut accepted, actor, 21);
    accepted
        .settle_boundary(BoundaryRequest::at(accepted.time()))
        .expect("assessment commands should be admitted together");
    accepted
        .settle_boundary(BoundaryRequest::at(accepted.time()))
        .expect("21 assessments should fit the per-plugin boundary cap");
    assert!((0..21).all(|index| {
        accepted
            .typed_domain_record(&TypedDomainRecordRef::<HistoricalSourcesAssessment>::new(
                format!("bounded-source-{index:02}"),
            ))
            .is_some()
    }));

    let mut canwu = Canwu::new_with_plugins(31, scenario(actor), &[&technology, &history])
        .expect("plugins should initialize");
    enqueue_assessment_batch(&mut canwu, actor, 22);
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("assessment commands should be admitted together");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("the overflow assessment must be rejected without poisoning settlement");
    assert!((0..21).all(|index| {
        canwu
            .typed_domain_record(&TypedDomainRecordRef::<HistoricalSourcesAssessment>::new(
                format!("bounded-source-{index:02}"),
            ))
            .is_some()
    }));
    assert!(
        canwu
            .typed_domain_record(&TypedDomainRecordRef::<HistoricalSourcesAssessment>::new(
                "bounded-source-21"
            ))
            .is_none()
    );
    assert!(has_capacity_rejection(&canwu));
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a later boundary must remain usable after capacity rejection");
}

#[test]
fn historical_total_cap_persists_rejection_without_poisoning() {
    let technology = TechnologyPlugin;
    let history = HistoricalSourcesPlugin;
    let actor = PersonId::new(1);
    let mut initial = scenario(actor);
    for index in 0..1_000 {
        initial
            .domain_records
            .push(initial_source_assessment(actor, index));
    }
    let mut canwu = Canwu::new_with_plugins(37, initial, &[&technology, &history])
        .expect("1,000 initial assessments should fit the plugin cap");
    validate_historical_research_runtime(&canwu)
        .expect("the exact 1,000-record boundary should validate");

    enqueue_assessment_batch(&mut canwu, actor, 1);
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("the 1,001st command should be admitted before the plugin cap is checked");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("the 1,001st assessment must become a persisted rejection");
    assert!(
        canwu
            .typed_domain_record(&TypedDomainRecordRef::<HistoricalSourcesAssessment>::new(
                "bounded-source-00"
            ))
            .is_none()
    );
    assert!(has_capacity_rejection(&canwu));
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("capacity rejection must not poison the next boundary");
    validate_historical_research_runtime(&canwu)
        .expect("capacity rejection must preserve a valid runtime");
}

fn has_capacity_rejection(canwu: &Canwu) -> bool {
    canwu.events().iter().any(|event| {
        event.kind.plugin_identity()
            == Some((
                HistoricalSourcesAssessment::PLUGIN_NAME,
                "historical_assessment_rejected_capacity_v1",
            ))
    })
}

fn initial_source_assessment(actor: PersonId, index: u64) -> DomainRecord {
    let subject = initial_record_version::<MetricSchema>("pressure");
    let payload = HistoricalSourcesAssessmentPayload {
        core: core(actor, subject.clone()),
        earliest_date: SimTime::EPOCH,
        latest_date: SimTime::EPOCH,
        authenticity_per_mille: 900,
        reliability_per_mille: 900,
        provenance_digest: digest('f'),
    };
    let mut draft = DomainRecordDraft::from_typed(
        TypedDomainRecordRef::<HistoricalSourcesAssessment>::new(format!(
            "initial-source-{index:04}"
        )),
        &payload,
    )
    .expect("initial assessment should encode");
    draft.references = vec![
        DomainReference {
            role: "core".to_owned(),
            target: DomainReferenceTarget::Core(EntityRef::Person(actor)),
        },
        DomainReference {
            role: "subject".to_owned(),
            target: DomainReferenceTarget::Domain(subject.record),
        },
    ];
    DomainRecord {
        reference: draft.reference,
        owner: HistoricalSourcesAssessment::PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    }
}

fn enqueue_assessment_batch(canwu: &mut Canwu, actor: PersonId, count: u64) {
    for index in 0..count {
        let envelope = HistoricalAssessmentCommand {
            id: format!("bounded-source-{index:02}"),
            subject: KnowledgeHolderRef::Person(actor),
            assessment: HistoricalSourcesAssessmentPayload {
                core: core(actor, initial_record_version::<MetricSchema>("pressure")),
                earliest_date: SimTime::EPOCH,
                latest_date: SimTime::EPOCH,
                authenticity_per_mille: 900,
                reliability_per_mille: 900,
                provenance_digest: digest('f'),
            },
        };
        canwu
            .enqueue_command(
                canwu.time(),
                0,
                CommandRequest::new(
                    CommandRequestId::new(index + 1),
                    canwu.revision() + index,
                    CommandEnvelope::new(
                        Issuer::Actor(actor),
                        Command::Plugin {
                            plugin: HistoricalSourcesAssessment::PLUGIN_NAME.to_owned(),
                            command: ASSESSMENT_COMMAND.to_owned(),
                            payload: serde_json::to_value(envelope)
                                .expect("assessment payload should encode"),
                        },
                    )
                    .at_time(canwu.time()),
                ),
            )
            .expect("assessment command should enqueue");
    }
}

fn enqueue_assessment<T: serde::Serialize>(
    canwu: &mut Canwu,
    actor: PersonId,
    request_id: u64,
    plugin: &str,
    envelope: T,
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
                        plugin: plugin.to_owned(),
                        command: ASSESSMENT_COMMAND.to_owned(),
                        payload: serde_json::to_value(envelope).expect("assessment payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("assessment command should enqueue");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("assessment command should be admitted");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("assessment ingress should settle");
}

fn core(actor: PersonId, subject: DomainRecordVersionRef) -> AssessmentCore {
    AssessmentCore {
        assessor: KnowledgeHolderRef::Person(actor),
        subject,
        method: "bounded source comparison".to_owned(),
        method_version: "1".to_owned(),
        as_of: SimTime::EPOCH,
        uncertainty_per_mille: 250,
        summary_digest: digest('a'),
        citations: vec![],
        contradicts: vec![],
        supersedes: vec![],
    }
}

fn digest(character: char) -> String {
    std::iter::repeat_n(character, 64).collect()
}

fn scenario(actor: PersonId) -> Scenario {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    Scenario {
        start_time: SimTime::EPOCH,
        entities: vec![
            EntityRef::Government(government),
            EntityRef::Person(actor),
            EntityRef::Territory(territory),
        ],
        world: WorldSnapshot {
            people: vec![Person {
                id: actor,
                name: "Researcher".to_owned(),
                government,
                current_location: territory,
                roles: vec![],
                transit: None,
            }],
            governments: vec![Government {
                id: government,
                name: "Research institution".to_owned(),
                capital: territory,
            }],
            territories: vec![Territory {
                id: territory,
                name: "Archive".to_owned(),
                controller: government,
                position: MapPoint::default(),
            }],
            routes: vec![],
            armies: vec![],
            letters: vec![],
        },
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![
            TechnologyCatalogRecord::Metric(MetricSchemaPayload {
                label: "pressure".to_owned(),
                unit: "pascal".to_owned(),
                scale: 1,
                minimum: 0,
                maximum: 1_000,
            })
            .into_initial_record("pressure")
            .expect("metric should encode"),
        ],
    }
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
