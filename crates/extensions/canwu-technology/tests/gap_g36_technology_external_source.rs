//! Downstream gap set §28 (gap G-36): an off-map external source admits a
//! practice-mode transmission opportunity.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, Command, CommandEnvelope, CommandRequest, CommandRequestId,
    DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle, DomainRecordSchema,
    DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource, DomainValueKindClass,
    EntityRef, EvidenceRef, Government, GovernmentId, Issuer, KnowledgeHolderRef,
    KnowledgeSnapshot, MapPoint, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, Person, PersonId,
    PluginRegistrar, Scenario, SimulationPlugin, Territory, TerritoryId, TypedDomainRecordRef,
    WorldSnapshot,
};
use canwu_technology::{
    ExternalTransmissionSourceV1, MetricComparison, MetricSchema, MetricSchemaPayload,
    MetricThreshold, REFERENCE_EVALUATOR_V1, RequirementGroup, TECHNOLOGY_COMMAND,
    TechniqueRevision, TechniqueRevisionPayload, TechniqueSpec, TechniqueSpecPayload,
    TechnologyCatalogRecord, TechnologyCommandEnvelope, TechnologyOperation,
    TechnologyOperationStatus, TechnologyPlugin, TechnologyRecordChange, TechnologyRecordPayload,
    TechnologyRecordSet, TransmissionMode, TransmissionOpportunity, TransmissionOpportunityPayload,
    from_technology_snapshot_json, initial_record_version, replay_technology_from_journal,
};
use serde::{Deserialize, Serialize};

const TEACHER: PersonId = PersonId::new(1);
const LEARNER: PersonId = PersonId::new(2);
const WORKSHOP: TerritoryId = TerritoryId::new(1);
const TRAINING_GROUND: TerritoryId = TerritoryId::new(2);

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g36_technology_external_source() {
    let plugins: [&dyn SimulationPlugin; 2] = [&TechnologyPlugin, &ContentPlugin];
    let mut canwu = Canwu::new_with_plugins(36, scenario(), &plugins).expect("technology run");
    let revision = initial_record_version::<TechniqueRevision>("revision");
    let content = DomainRecordVersionRef {
        record: content_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let external = ExternalTransmissionSourceV1 {
        evidence: EvidenceRef::DomainRecordVersion(content.clone()),
        declared_reliability_per_mille: 650,
    };
    let opening = TransmissionOpportunityPayload {
        source: None,
        source_site: None,
        source_capability: None,
        external_source: Some(external.clone()),
        destination: KnowledgeHolderRef::Person(LEARNER),
        destination_site: EntityRef::Territory(TRAINING_GROUND),
        revision: Some(revision.clone()),
        mode: TransmissionMode::Demonstration,
        evidence: Vec::new(),
        resulting_program: None,
        opened_at: canwu.time(),
        active: true,
    };

    // An off-map source admits a practice transmission without a live
    // capability; the destination opens it.
    let mut request = 0;
    assert_eq!(
        create(&mut canwu, &mut request, LEARNER, "foreign-demo", &opening),
        TechnologyOperationStatus::Applied
    );
    let stored = canwu
        .typed_domain_record(&TypedDomainRecordRef::<TransmissionOpportunity>::new(
            "foreign-demo",
        ))
        .expect("stored transmission")
        .decode_payload::<TransmissionOpportunity>()
        .expect("transmission payload");
    assert_eq!(stored.external_source, Some(external.clone()));

    // Both or neither of the live capability and the external source are
    // rejected, as are an external source for a document mode and evidence
    // that is not manifest-bound content.
    let both = TransmissionOpportunityPayload {
        source_capability: Some(revision.clone()),
        ..opening.clone()
    };
    let neither = TransmissionOpportunityPayload {
        source: Some(KnowledgeHolderRef::Person(TEACHER)),
        source_site: Some(EntityRef::Territory(WORKSHOP)),
        external_source: None,
        ..opening.clone()
    };
    let document = TransmissionOpportunityPayload {
        mode: TransmissionMode::DocumentAccess,
        ..opening.clone()
    };
    let runtime_evidence = TransmissionOpportunityPayload {
        external_source: Some(ExternalTransmissionSourceV1 {
            evidence: EvidenceRef::DomainRecordVersion(revision.clone()),
            declared_reliability_per_mille: 650,
        }),
        ..opening.clone()
    };
    let overconfident = TransmissionOpportunityPayload {
        external_source: Some(ExternalTransmissionSourceV1 {
            declared_reliability_per_mille: 1_001,
            ..external.clone()
        }),
        ..opening.clone()
    };
    let with_holder = TransmissionOpportunityPayload {
        source: Some(KnowledgeHolderRef::Person(TEACHER)),
        source_site: Some(EntityRef::Territory(WORKSHOP)),
        ..opening.clone()
    };
    let missing_evidence = TransmissionOpportunityPayload {
        external_source: Some(ExternalTransmissionSourceV1 {
            evidence: EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
                record: TypedDomainRecordRef::<ExternalSourceContentRecord>::new("missing")
                    .into_untyped(),
                ..content.clone()
            }),
            declared_reliability_per_mille: 650,
        }),
        ..opening.clone()
    };
    for (subject, id, payload) in [
        (LEARNER, "both-sources", &both),
        (TEACHER, "no-source", &neither),
        (LEARNER, "document-external", &document),
        (LEARNER, "technology-evidence", &runtime_evidence),
        (LEARNER, "overconfident-source", &overconfident),
        (TEACHER, "external-with-holder", &with_holder),
        (LEARNER, "missing-evidence", &missing_evidence),
    ] {
        assert_eq!(
            create(&mut canwu, &mut request, subject, id, payload),
            TechnologyOperationStatus::Rejected,
            "{id} must be rejected"
        );
    }
    let mut records = TechnologyRecordSet::load_host(&canwu).expect("technology records");
    for (payload, message) in [
        (
            &both,
            "cites both a live source capability and an external source",
        ),
        (
            &neither,
            "requires an exact source capability or an external source",
        ),
        (&document, "accepted only for demonstration"),
        (&runtime_evidence, "manifest-bound content record"),
    ] {
        let mut record = records
            .records
            .get(
                &TypedDomainRecordRef::<TransmissionOpportunity>::new("foreign-demo")
                    .into_untyped(),
            )
            .expect("stored record")
            .clone();
        let continuation = record.payload[PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD].clone();
        record.payload = serde_json::to_value(payload).expect("candidate payload");
        record.payload[PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD] = continuation;
        records.insert(record);
        let error = records
            .validate(canwu.time())
            .expect_err("invalid source combination");
        assert!(error.message.contains(message), "{}", error.message);
    }

    // The external source keeps the same open/close immutability: it can be
    // closed, not changed or reopened.
    let changed = TransmissionOpportunityPayload {
        external_source: Some(ExternalTransmissionSourceV1 {
            declared_reliability_per_mille: 900,
            ..external
        }),
        ..opening.clone()
    };
    assert_eq!(
        update(&mut canwu, &mut request, "foreign-demo", 1, &changed),
        TechnologyOperationStatus::Rejected
    );
    let closed = TransmissionOpportunityPayload {
        active: false,
        ..opening.clone()
    };
    assert_eq!(
        update(&mut canwu, &mut request, "foreign-demo", 1, &closed),
        TechnologyOperationStatus::Applied
    );
    assert_eq!(
        update(&mut canwu, &mut request, "foreign-demo", 2, &opening),
        TechnologyOperationStatus::Rejected
    );

    // Save/load and exact replay revalidate the external source.
    let snapshot = canwu.snapshot();
    let restored = from_technology_snapshot_json(
        &serde_json::to_string(&snapshot).expect("encode snapshot"),
        &plugins,
    )
    .expect("restore technology snapshot");
    assert_eq!(restored.snapshot(), snapshot);
    let replayed = replay_technology_from_journal(&plugins, &canwu.replay_journal())
        .expect("replay technology history");
    assert_eq!(replayed.snapshot(), snapshot);
    assert_eq!(
        replayed
            .typed_domain_record(&TypedDomainRecordRef::<TransmissionOpportunity>::new(
                "foreign-demo",
            ))
            .expect("replayed transmission")
            .decode_payload::<TransmissionOpportunity>()
            .expect("replayed payload"),
        closed
    );
}

fn create(
    canwu: &mut Canwu,
    request: &mut u64,
    subject: PersonId,
    id: &str,
    payload: &TransmissionOpportunityPayload,
) -> TechnologyOperationStatus {
    submit(
        canwu,
        request,
        subject,
        TechnologyRecordChange::Create {
            id: id.to_owned(),
            value: TechnologyRecordPayload::Transmission(payload.clone()),
        },
    )
}

fn update(
    canwu: &mut Canwu,
    request: &mut u64,
    id: &str,
    expected_version: u64,
    payload: &TransmissionOpportunityPayload,
) -> TechnologyOperationStatus {
    submit(
        canwu,
        request,
        LEARNER,
        TechnologyRecordChange::Update {
            id: id.to_owned(),
            expected_version,
            value: TechnologyRecordPayload::Transmission(payload.clone()),
        },
    )
}

fn submit(
    canwu: &mut Canwu,
    request: &mut u64,
    subject: PersonId,
    change: TechnologyRecordChange,
) -> TechnologyOperationStatus {
    *request += 1;
    let operation = format!("operation-{request}");
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(*request),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(subject),
                    Command::Plugin {
                        plugin: "canwu-technology".to_owned(),
                        command: TECHNOLOGY_COMMAND.to_owned(),
                        payload: serde_json::to_value(TechnologyCommandEnvelope {
                            id: operation.clone(),
                            subject: KnowledgeHolderRef::Person(subject),
                            change,
                        })
                        .expect("command payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("queue technology command");
    for _ in 0..2 {
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("technology boundary");
    }
    canwu
        .typed_domain_record(&TypedDomainRecordRef::<TechnologyOperation>::new(operation))
        .expect("terminal operation outcome")
        .decode_payload::<TechnologyOperation>()
        .expect("operation payload")
        .status
}

fn scenario() -> Scenario {
    let government = GovernmentId::new(1);
    let reliability = initial_record_version::<MetricSchema>("reliability");
    let technique = initial_record_version::<TechniqueSpec>("technique");
    let world = WorldSnapshot {
        people: vec![
            Person {
                id: TEACHER,
                name: "Teacher".to_owned(),
                government,
                current_location: WORKSHOP,
                roles: vec![],
                transit: None,
            },
            Person {
                id: LEARNER,
                name: "Learner".to_owned(),
                government,
                current_location: TRAINING_GROUND,
                roles: vec![],
                transit: None,
            },
        ],
        governments: vec![Government {
            id: government,
            name: "Workshop authority".to_owned(),
            capital: WORKSHOP,
        }],
        territories: vec![
            Territory {
                id: WORKSHOP,
                name: "Workshop".to_owned(),
                controller: government,
                position: MapPoint::default(),
            },
            Territory {
                id: TRAINING_GROUND,
                name: "Training ground".to_owned(),
                controller: government,
                position: MapPoint { x: 1.0, y: 0.0 },
            },
        ],
        routes: vec![],
        armies: vec![],
        letters: vec![],
    };
    Scenario {
        start_time: canwu_api::SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![
            TechnologyCatalogRecord::Metric(MetricSchemaPayload {
                label: "reliability".to_owned(),
                unit: "permille".to_owned(),
                scale: 1_000,
                minimum: 0,
                maximum: 1_000,
            })
            .into_initial_record("reliability")
            .expect("metric"),
            TechnologyCatalogRecord::Technique(TechniqueSpecPayload {
                label: "neutral pressure converter".to_owned(),
                function: "convert pressure into useful work".to_owned(),
                requirements: vec![RequirementGroup {
                    id: "reliable_output".to_owned(),
                    any_of: vec![MetricThreshold {
                        id: "reliability_floor".to_owned(),
                        metric: reliability,
                        comparison: MetricComparison::AtLeast,
                        value: 700,
                    }],
                }],
                qualification_rules: vec![],
            })
            .into_initial_record("technique")
            .expect("technique"),
            TechnologyCatalogRecord::Revision(TechniqueRevisionPayload {
                label: "neutral revision".to_owned(),
                spec: technique,
                parents: vec![],
                parameters: vec![],
                evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
                produced_by: None,
                execution_intent: None,
                discovery_evidence: vec![],
            })
            .into_initial_record("revision")
            .expect("revision"),
            content_record(),
        ],
    }
}

/// Application content describing an off-map source.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct ExternalSourceContent {
    label: String,
}

struct ExternalSourceContentRecord;

impl DomainRecordType for ExternalSourceContentRecord {
    type Payload = ExternalSourceContent;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "test.content";
    const NAME: &'static str = "external-source";
}

fn content_reference() -> TypedDomainRecordRef<ExternalSourceContentRecord> {
    TypedDomainRecordRef::new("distant-foundry")
}

fn content_record() -> DomainRecord {
    let draft = DomainRecordDraft::from_typed(
        content_reference(),
        &ExternalSourceContent {
            label: "a distant foundry".to_owned(),
        },
    )
    .expect("content draft");
    DomainRecord {
        reference: draft.reference,
        owner: "test-content".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: Vec::new(),
    }
}

struct ContentPlugin;

impl SimulationPlugin for ContentPlugin {
    fn name(&self) -> &'static str {
        "test-content"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "3b5d7f9102a4c6e8f0b2d4f6a8c0e2f4b6d8f0a2c4e6f8a0b2c4d6e8f0a2b4c6"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar
            .register_record_schema(DomainRecordSchema::for_record::<ExternalSourceContentRecord>())
    }
}
