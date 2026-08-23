#![allow(clippy::unnecessary_wraps)]

use super::*;

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
                    "content-addressed segment ID is already bound to different bytes",
                ))
            };
        }
        segments.insert(segment_id, segment.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct CacheFingerprint {
    journals: [String; 5],
    domains: [Option<String>; 7],
}

fn cache_fingerprint(simulation: &Simulation) -> CacheFingerprint {
    let cache = simulation
        .state
        .metadata
        .commitment_cache
        .as_ref()
        .expect("current runtimes should maintain a commitment cache");
    CacheFingerprint {
        journals: [
            cache.commands.root(),
            cache.attempts.root(),
            cache.events.root(),
            cache.ingress.root(),
            cache.random_draws.root(),
        ],
        domains: [
            cache.world.clone(),
            cache.knowledge.clone(),
            cache.plugin_components.clone(),
            cache.domain_records.clone(),
            cache.scheduler.clone(),
            cache.random_streams.clone(),
            cache.identity.clone(),
        ],
    }
}

macro_rules! test_plugin_identity {
    ($hash:literal) => {
        fn version(&self) -> &'static str {
            "test-v1"
        }

        fn semantic_hash(&self) -> &'static str {
            $hash
        }
    };
}

struct KnowledgePublicationPlugin;

fn fixture_knowledge_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new("fixture.knowledge", "notice"), 1)
}

fn fixture_knowledge_draft(value: i64) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema: fixture_knowledge_schema(),
        subjects: Vec::new(),
        payload: serde_json::json!({ "value": value }),
        as_of: None,
        confidence_per_mille: 900,
        origin: KnowledgeOrigin {
            method: "observation".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

fn phase4_publish_knowledge(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(1)),
                visibility: StateVisibility::SameBoundary,
                producer_correlation: Some("phase4-visible".to_owned()),
                records: vec![fixture_knowledge_draft(1)],
                summary: "Publish a same-boundary observation".to_owned(),
            },
            BoundaryDirective::PublishKnowledge {
                holder: KnowledgeHolderRef::Person(PersonId::new(1)),
                visibility: StateVisibility::NextBoundary,
                producer_correlation: Some("phase4-deferred".to_owned()),
                records: vec![fixture_knowledge_draft(2)],
                summary: "Publish a deferred observation".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

fn phase13_publish_knowledge(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("phase13-visible".to_owned()),
            records: vec![fixture_knowledge_draft(3)],
            summary: "Publish a same-boundary projection".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn expect_holder_knowledge(
    view: &SimulationView<'_>,
    expected: usize,
) -> Result<BoundaryProposal, CanwuError> {
    let result = view.knowledge_records(
        KnowledgeHolderRef::Person(PersonId::new(1)),
        &KnowledgeQuery {
            view: KnowledgeHistoryView::FullHistory,
            ..KnowledgeQuery::default()
        },
    )?;
    if result.records.len() != expected {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!(
                "knowledge phase view expected {expected} records but observed {}",
                result.records.len()
            ),
        ));
    }
    Ok(BoundaryProposal::default())
}

fn phase4_peer(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    expect_holder_knowledge(view, 0)
}

fn phase5_observer(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    expect_holder_knowledge(view, 1)
}

fn phase13_peer(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    expect_holder_knowledge(view, 1)
}

fn phase14_observer(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    expect_holder_knowledge(view, 2)
}

impl SimulationPlugin for KnowledgePublicationPlugin {
    fn name(&self) -> &'static str {
        "fixture-knowledge-publication"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000030");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: fixture_knowledge_schema(),
            schema_hash: "1000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
        })?;
        let grant = KnowledgeWriteGrant {
            schema: fixture_knowledge_schema(),
            visibilities: vec![StateVisibility::SameBoundary, StateVisibility::NextBoundary],
        };
        let mut phase4 = BoundarySystemContract::new(
            "a-phase4-publisher",
            BoundaryPhase::PerceptionAndAttentionRefresh,
            SystemCadence::Daily,
        );
        phase4.knowledge_writes = vec![grant.clone()];
        registrar.register_boundary_system(phase4, phase4_publish_knowledge)?;

        let mut phase4_peer_contract = BoundarySystemContract::new(
            "z-phase4-peer",
            BoundaryPhase::PerceptionAndAttentionRefresh,
            SystemCadence::Daily,
        );
        phase4_peer_contract.reads = vec![StateKey::core_knowledge()];
        registrar.register_boundary_system(phase4_peer_contract, phase4_peer)?;

        let mut phase5 = BoundarySystemContract::new(
            "phase5-observer",
            BoundaryPhase::DecisionAndAcceptedEffectIntake,
            SystemCadence::Daily,
        );
        phase5.reads = vec![StateKey::core_knowledge()];
        registrar.register_boundary_system(phase5, phase5_observer)?;

        let mut phase13 = BoundarySystemContract::new(
            "a-phase13-publisher",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        phase13.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: fixture_knowledge_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(phase13, phase13_publish_knowledge)?;

        let mut phase13_peer_contract = BoundarySystemContract::new(
            "z-phase13-peer",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        phase13_peer_contract.reads = vec![StateKey::core_knowledge()];
        registrar.register_boundary_system(phase13_peer_contract, phase13_peer)?;

        let mut diagnostic_observer = BoundarySystemContract::new(
            "phase14-observer",
            BoundaryPhase::SaveReplayAndDiagnosticHashing,
            SystemCadence::Daily,
        );
        diagnostic_observer.reads = vec![StateKey::core_knowledge()];
        registrar.register_boundary_system(diagnostic_observer, phase14_observer)
    }
}

struct PendingEvidencePublicationPlugin;

struct ArchivedEvidencePublicationPlugin;

fn publish_from_archived_boundary(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id != BoundaryId::new(2) {
        return Ok(BoundaryProposal::default());
    }
    let mut draft = fixture_knowledge_draft(8);
    draft.origin.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("archived-boundary-evidence".to_owned()),
            records: vec![draft],
            summary: "Publish from a verified archived boundary identity".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for ArchivedEvidencePublicationPlugin {
    fn name(&self) -> &'static str {
        "fixture-archived-evidence-publication"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000033");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: fixture_knowledge_schema(),
            schema_hash: "3000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
        })?;
        let mut phase13 = BoundarySystemContract::new(
            "publish-archived-evidence",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        phase13.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: fixture_knowledge_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(phase13, publish_from_archived_boundary)
    }
}

fn pending_evidence_record_ref() -> DomainRecordRef {
    DomainRecordRef {
        kind: DomainRecordKind::new("fixture.evidence", "marker"),
        id: "primary".to_owned(),
    }
}

fn phase7_create_pending_evidence(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: DomainRecordDraft {
                    reference: pending_evidence_record_ref(),
                    payload: serde_json::json!({ "code": "alpha" }),
                    references: Vec::new(),
                },
            },
            summary: "Create proposal-visible evidence".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn phase13_publish_pending_evidence(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let record = view
        .domain_record(&pending_evidence_record_ref())?
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::EvidenceUnavailable,
                "phase-7 evidence record is unavailable",
            )
        })?;
    let mut draft = fixture_knowledge_draft(7);
    draft.origin.evidence = vec![EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
        record: record.reference.clone(),
        version: record.version,
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: context.boundary_id,
            change_index: 0,
        },
    })];
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("pending-evidence".to_owned()),
            records: vec![draft],
            summary: "Publish from an earlier committed stage".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for PendingEvidencePublicationPlugin {
    fn name(&self) -> &'static str {
        "fixture-pending-evidence-publication"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000031");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            pending_evidence_record_ref().kind,
            DomainRecordClass::Record,
        ))?;
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: fixture_knowledge_schema(),
            schema_hash: "2000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
        })?;
        let mut phase7 = BoundarySystemContract::new(
            "phase7-create-evidence",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        phase7.writes = vec![records::record_state_key(
            &pending_evidence_record_ref().kind,
        )];
        phase7.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(phase7, phase7_create_pending_evidence)?;

        let mut phase13 = BoundarySystemContract::new(
            "phase13-publish-evidence",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        phase13.reads = vec![records::record_state_key(
            &pending_evidence_record_ref().kind,
        )];
        phase13.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: fixture_knowledge_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(phase13, phase13_publish_pending_evidence)
    }
}

struct KeyedRandomPlugin;

fn keyed_fixture_stream() -> RandomStreamKey {
    RandomStreamKey::new("fixture-keyed-random", "resolution", 1)
}

fn keyed_rollback_stream() -> RandomStreamKey {
    RandomStreamKey::new("fixture-keyed-rollback", "resolution", 1)
}

fn keyed_random_operations(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { payload, .. } = &record.payload else {
            continue;
        };
        let operation = payload
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CanwuError::new(ErrorCode::InvalidPayload, "operation code is missing")
            })?;
        let value = view.random_range_for_operation(
            &keyed_fixture_stream(),
            EvidenceRef::Ingress(*id),
            "resolve",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            10_000,
            "stable resolution",
        )?;
        let retry = view.random_range_for_operation(
            &keyed_fixture_stream(),
            EvidenceRef::Ingress(*id),
            "resolve",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            10_000,
            "stable resolution",
        )?;
        if value != retry {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "exact keyed retry changed its result",
            ));
        }
    }
    let _ = view.random_range(&keyed_fixture_stream(), 10_000, "sequential control draw")?;
    Ok(BoundaryProposal::default())
}

impl SimulationPlugin for KeyedRandomPlugin {
    fn name(&self) -> &'static str {
        "fixture-keyed-random"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000032");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "operation".to_owned(),
            description: "Admit one neutral keyed random operation".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut contract = BoundarySystemContract::new(
            "resolve-operations",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.random_streams = vec![keyed_fixture_stream()];
        registrar.register_boundary_system(contract, keyed_random_operations)
    }
}

struct KeyedRollbackPlugin;

fn stage_keyed_draw_before_later_failure(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let ingress = *context.admitted_ingress.first().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::EvidenceUnavailable,
            "keyed rollback fixture requires admitted ingress",
        )
    })?;
    let _ = view.random_range_for_operation(
        &keyed_rollback_stream(),
        EvidenceRef::Ingress(ingress),
        "resolve",
        "rollback-operation",
        RandomOperationTarget::CanonicalKey("rollback-target".to_owned()),
        0,
        10_000,
        "rollback fixture",
    )?;
    Ok(BoundaryProposal::default())
}

fn reject_after_keyed_draw(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("undeclared-after-keyed-draw".to_owned()),
            records: vec![fixture_knowledge_draft(99)],
            summary: "Trigger a later validation failure".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for KeyedRollbackPlugin {
    fn name(&self) -> &'static str {
        "fixture-keyed-rollback"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000034");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "operation".to_owned(),
            description: "Admit one rollback probe".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut draw = BoundarySystemContract::new(
            "a-stage-keyed-draw",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        draw.reads = vec![StateKey::core_ingress()];
        draw.random_streams = vec![keyed_rollback_stream()];
        registrar.register_boundary_system(draw, stage_keyed_draw_before_later_failure)?;

        registrar.register_boundary_system(
            BoundarySystemContract::new(
                "z-reject-after-keyed-draw",
                BoundaryPhase::PerspectiveAndReportMaterialization,
                SystemCadence::EventDriven,
            ),
            reject_after_keyed_draw,
        )
    }
}

struct AuthorityPlugin;
struct ChangedAuthorityPlugin;

fn authority_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let actor = PersonId::new(1);
    let army = ArmyId::new(1);
    if context.issuer != Issuer::Actor(actor) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "the command issuer does not own this test action",
        ));
    }
    if view.army(army)?.is_none() {
        return Err(CanwuError::new(
            ErrorCode::ArmyNotFound,
            "the test army does not exist",
        ));
    }
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("military", "stance"),
        entity: EntityRef::Army(army),
        component: "stance".to_owned(),
        value: Value::String("hold".to_owned()),
        summary: "The authorized actor changed the army stance".to_owned(),
    }])
}

fn register_authority(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    registrar.register_command(
        PluginActionDescriptor {
            name: "set_stance".to_owned(),
            description: "Set a test stance".to_owned(),
            payload_schema: PayloadSchema::Null,
            reads: vec![StateKey::core_armies()],
            writes: vec![StateKey::new("military", "stance")],
        },
        authority_command,
    )
}

impl SimulationPlugin for AuthorityPlugin {
    fn name(&self) -> &'static str {
        "authority-test"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000001");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_authority(registrar)
    }
}

impl SimulationPlugin for ChangedAuthorityPlugin {
    fn name(&self) -> &'static str {
        "authority-test"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000013");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_authority(registrar)
    }
}

struct MarkerPlugin {
    name: &'static str,
    writes: Vec<StateKey>,
}

fn marker_system(
    _view: &SimulationView<'_>,
    event: &SimEvent,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if !matches!(event.kind, EventKind::MoveOrdered { .. }) {
        return Ok(Vec::new());
    }
    Ok(vec![SystemDirective::Emit {
        event_type: "marker".to_owned(),
        summary: "movement marker".to_owned(),
        affected: Vec::new(),
    }])
}

impl SimulationPlugin for MarkerPlugin {
    fn name(&self) -> &str {
        self.name
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000002");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = SystemContract::event_driven(
            "movement-marker",
            BoundaryPhase::PerspectiveAndReportMaterialization,
        );
        contract.writes.clone_from(&self.writes);
        registrar.register_system(contract, marker_system)
    }
}

struct RecursivePlugin;

fn recursive_system(
    _view: &SimulationView<'_>,
    event: &SimEvent,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let should_recurse = match &event.kind {
        EventKind::MoveOrdered { .. } => true,
        EventKind::Plugin { plugin, event_type } => {
            plugin == "recursive-test" && event_type == "loop"
        }
        _ => false,
    };
    if should_recurse {
        Ok(vec![SystemDirective::Emit {
            event_type: "loop".to_owned(),
            summary: "recursive compatibility event".to_owned(),
            affected: Vec::new(),
        }])
    } else {
        Ok(Vec::new())
    }
}

impl SimulationPlugin for RecursivePlugin {
    fn name(&self) -> &'static str {
        "recursive-test"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000004");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_system(
            SystemContract::event_driven(
                "recursive-reactor",
                BoundaryPhase::PerspectiveAndReportMaterialization,
            ),
            recursive_system,
        )
    }
}

struct FailingPlugin;

fn failing_command(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let mutation = SystemDirective::SetComponent {
        state: StateKey::new("failure-fixture", "flag"),
        entity: EntityRef::Army(ArmyId::new(1)),
        component: "flag".to_owned(),
        value: Value::Bool(true),
        summary: "Set a flag before the injected failure".to_owned(),
    };
    if payload.get("scheduled").and_then(Value::as_bool) == Some(true) {
        Ok(vec![SystemDirective::Schedule {
            after: SimDuration::days(1),
            directive: Box::new(mutation),
        }])
    } else {
        Ok(vec![mutation])
    }
}

fn failing_event_system(
    _view: &SimulationView<'_>,
    event: &SimEvent,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if matches!(
        &event.kind,
        EventKind::Plugin { plugin, event_type }
            if plugin == "failing-test" && event_type == "flag_changed"
    ) {
        Ok(vec![SystemDirective::Schedule {
            after: SimDuration::ZERO,
            directive: Box::new(SystemDirective::Emit {
                event_type: "unreachable".to_owned(),
                summary: "This directive must be rejected".to_owned(),
                affected: Vec::new(),
            }),
        }])
    } else {
        Ok(Vec::new())
    }
}

fn panicking_command(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    panic!("injected plugin panic")
}

impl SimulationPlugin for FailingPlugin {
    fn name(&self) -> &'static str {
        "failing-test"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000003");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_system(
            SystemContract::event_driven("reject-flag-event", BoundaryPhase::InvariantValidation),
            failing_event_system,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "mutate".to_owned(),
                description: "Exercise transactional rollback".to_owned(),
                payload_schema: PayloadSchema::Object {
                    properties: BTreeMap::from([(
                        "scheduled".to_owned(),
                        PayloadProperty {
                            value_type: PayloadValueType::Boolean,
                            required: true,
                        },
                    )]),
                    allow_additional: false,
                },
                reads: Vec::new(),
                writes: vec![StateKey::new("failure-fixture", "flag")],
            },
            failing_command,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "panic".to_owned(),
                description: "Exercise the plugin panic boundary".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            panicking_command,
        )
    }
}

fn no_op_command(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(Vec::new())
}

fn no_op_boundary(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}

struct JournalCommandPlugin;

impl SimulationPlugin for JournalCommandPlugin {
    fn name(&self) -> &'static str {
        "journal-command"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000004");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "noop".to_owned(),
                description: "Append deterministic command evidence".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            no_op_command,
        )
    }
}

fn emit_archive_probe(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::Emit {
            event_type: "archive_probe".to_owned(),
            summary: "Emit evidence across the archive admission frontier".to_owned(),
            affected: vec![EntityRef::Person(PersonId::new(1))],
        }],
        ..BoundaryProposal::default()
    })
}

struct ArchiveEmissionPlugin;

impl SimulationPlugin for ArchiveEmissionPlugin {
    fn name(&self) -> &'static str {
        "archive-emission"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000031");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "emit",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        contract.emits = vec!["archive_probe".to_owned()];
        registrar.register_boundary_system(contract, emit_archive_probe)
    }
}

struct BoundaryGhostPlugin;

impl SimulationPlugin for BoundaryGhostPlugin {
    fn name(&self) -> &'static str {
        "boundary-ghost-test"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000005");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "seed".to_owned(),
                description: "Own immediate state for the conflict fixture".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![StateKey::new("boundary-conflict", "immediate")],
            },
            no_op_command,
        )?;
        let mut rejected = BoundarySystemContract::new(
            "rejected",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        rejected.writes = vec![
            StateKey::new("boundary-conflict", "immediate"),
            StateKey::new("boundary-ghost", "value"),
        ];
        if registrar
            .register_boundary_system(rejected, no_op_boundary)
            .is_ok()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "the boundary ghost fixture expected a writer-mode conflict",
            ));
        }
        Ok(())
    }
}

struct GhostPlugin;

impl SimulationPlugin for GhostPlugin {
    fn name(&self) -> &'static str {
        "ghost-test"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000006");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let ignored = registrar.register_command(
            PluginActionDescriptor {
                name: "ignored".to_owned(),
                description: "A deliberately rejected registration".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![
                    StateKey::new("fresh-domain", "value"),
                    StateKey::new("shared-domain", "balance"),
                ],
            },
            no_op_command,
        );
        if ignored.is_ok() {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "the ghost fixture expected an ownership conflict",
            ));
        }
        Ok(())
    }
}

fn seed_secret(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("secret-domain", "value"),
        entity: EntityRef::Army(ArmyId::new(1)),
        component: "value".to_owned(),
        value: Value::String("classified".to_owned()),
        summary: "Seed classified state".to_owned(),
    }])
}

struct SecretPlugin;

impl SimulationPlugin for SecretPlugin {
    fn name(&self) -> &'static str {
        "secret-owner"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000007");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "seed".to_owned(),
                description: "Seed owned state".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![StateKey::new("secret-domain", "value")],
            },
            seed_secret,
        )
    }
}

fn undeclared_read(
    view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let _ = view.component(
        &StateKey::new("secret-domain", "value"),
        &EntityRef::Army(ArmyId::new(1)),
        "value",
    )?;
    Ok(Vec::new())
}

fn undeclared_write(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("secret-domain", "value"),
        entity: EntityRef::Army(ArmyId::new(1)),
        component: "value".to_owned(),
        value: Value::String("overwritten".to_owned()),
        summary: "Attempt an undeclared write".to_owned(),
    }])
}

fn missing_entity_write(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("access-domain", "declared"),
        entity: EntityRef::Army(ArmyId::new(999)),
        component: "declared".to_owned(),
        value: Value::Bool(true),
        summary: "Attempt to write state for a missing entity".to_owned(),
    }])
}

struct UndeclaredAccessPlugin;

impl SimulationPlugin for UndeclaredAccessPlugin {
    fn name(&self) -> &'static str {
        "undeclared-access"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000008");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "missing".to_owned(),
                description: "Attempt to target a missing entity".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![StateKey::new("access-domain", "declared")],
            },
            missing_entity_write,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "read".to_owned(),
                description: "Attempt an undeclared read".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            undeclared_read,
        )?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "write".to_owned(),
                description: "Attempt an undeclared write".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![StateKey::new("access-domain", "declared")],
            },
            undeclared_write,
        )
    }
}

fn collision_a(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("collision-a", "b/person:1/c"),
        entity: EntityRef::Person(PersonId::new(1)),
        component: "b/person:1/c".to_owned(),
        value: Value::String("first".to_owned()),
        summary: "Write the first adversarial key".to_owned(),
    }])
}

fn collision_b(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("collision-b", "c"),
        entity: EntityRef::Person(PersonId::new(1)),
        component: "c".to_owned(),
        value: Value::String("second".to_owned()),
        summary: "Write the second adversarial key".to_owned(),
    }])
}

struct CollisionPluginA;

struct CollisionPluginB;

impl SimulationPlugin for CollisionPluginA {
    fn name(&self) -> &'static str {
        "a"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000009");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "write".to_owned(),
                description: "Write an adversarial component key".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![StateKey::new("collision-a", "b/person:1/c")],
            },
            collision_a,
        )
    }
}

impl SimulationPlugin for CollisionPluginB {
    fn name(&self) -> &'static str {
        "a/person:1/b"
    }

    test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000a");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "write".to_owned(),
                description: "Write a second adversarial component key".to_owned(),
                payload_schema: PayloadSchema::Null,
                reads: Vec::new(),
                writes: vec![StateKey::new("collision-b", "c")],
            },
            collision_b,
        )
    }
}

fn grain_pool() -> ReservationPoolKey {
    ReservationPoolKey::new(
        StateKey::new("logistics", "grain"),
        EntityRef::Territory(TerritoryId::new(1)),
        "grain",
    )
}

fn primary_random_stream() -> RandomStreamKey {
    RandomStreamKey::new("random-primary", "daily-roll", 1)
}

fn noise_random_stream() -> RandomStreamKey {
    RandomStreamKey::new("random-noise", "daily-noise", 1)
}

fn failure_random_stream() -> RandomStreamKey {
    RandomStreamKey::new("boundary-rollback", "rollback-proof", 1)
}

fn roll_primary(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let roll = view.random_range(&primary_random_stream(), 100, "daily primary roll")?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state: StateKey::new("random-primary", "roll"),
            entity: EntityRef::Territory(TerritoryId::new(1)),
            component: "value".to_owned(),
            value: Value::from(roll),
            summary: format!("Primary random stream rolled {roll}"),
        }],
        ..BoundaryProposal::default()
    })
}

fn draw_noise(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let _ = view.random_range(&noise_random_stream(), 10_000, "unrelated daily noise")?;
    Ok(BoundaryProposal::default())
}

struct PrimaryRandomPlugin;
struct ChangedPrimaryRandomPlugin;
struct NoiseRandomPlugin;

fn register_primary_random(registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
    let mut contract = BoundarySystemContract::new(
        "roll",
        BoundaryPhase::DomainDeltaProposal,
        SystemCadence::Daily,
    );
    contract.writes = vec![StateKey::new("random-primary", "roll")];
    contract.random_streams = vec![primary_random_stream()];
    registrar.register_boundary_system(contract, roll_primary)
}

impl SimulationPlugin for PrimaryRandomPlugin {
    fn name(&self) -> &'static str {
        "random-primary"
    }

    test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000b");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_primary_random(registrar)
    }
}

impl SimulationPlugin for ChangedPrimaryRandomPlugin {
    fn name(&self) -> &'static str {
        "random-primary"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000012");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_primary_random(registrar)
    }
}

impl SimulationPlugin for NoiseRandomPlugin {
    fn name(&self) -> &'static str {
        "random-noise"
    }

    test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000c");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "draw",
            BoundaryPhase::DerivedFieldSolve,
            SystemCadence::Daily,
        );
        contract.random_streams = vec![noise_random_stream()];
        registrar.register_boundary_system(contract, draw_noise)
    }
}

fn offer_grain(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        offers: vec![ReservationOffer {
            pool: grain_pool(),
            capacity: 10,
        }],
        ..BoundaryProposal::default()
    })
}

fn high_request(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        requests: vec![ReservationRequest {
            request: "grain".to_owned(),
            pool: grain_pool(),
            quantity: 7,
            priority: 10,
            tie_break: "high".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn low_request(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        requests: vec![ReservationRequest {
            request: "grain".to_owned(),
            pool: grain_pool(),
            quantity: 7,
            priority: 0,
            tie_break: "low".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn record_grant(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    plugin: &str,
    state: StateKey,
    component: &str,
) -> Result<BoundaryProposal, CanwuError> {
    let reservation = ReservationRef::new(plugin, "request", "grain");
    let allocation = view.reservation(&reservation)?.ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!(
                "{} could not find allocation {reservation:?}",
                context.system
            ),
        )
    })?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state,
            entity: EntityRef::Territory(TerritoryId::new(1)),
            component: component.to_owned(),
            value: Value::from(allocation.granted),
            summary: format!("Recorded a grant of {} grain", allocation.granted),
        }],
        ..BoundaryProposal::default()
    })
}

fn record_high_grant(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    record_grant(
        view,
        context,
        "high-claim",
        StateKey::new("allocation", "high"),
        "high",
    )
}

fn record_low_grant(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    record_grant(
        view,
        context,
        "low-claim",
        StateKey::new("allocation", "low"),
        "low",
    )
}

fn validate_visibility(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let entity = EntityRef::Territory(TerritoryId::new(1));
    let high = view
        .component(&StateKey::new("allocation", "high"), &entity, "high")?
        .and_then(Value::as_u64);
    let low = view
        .component(&StateKey::new("allocation", "low"), &entity, "low")?
        .and_then(Value::as_u64);
    let proposed_high = view
        .proposed_component(&StateKey::new("allocation", "high"), &entity, "high")?
        .and_then(Value::as_u64);
    let proposed_low = view
        .proposed_component(&StateKey::new("allocation", "low"), &entity, "low")?
        .and_then(Value::as_u64);
    let expected_current_low = (context.boundary_id.get() > 1).then_some(3);
    if high != Some(7)
        || low != expected_current_low
        || proposed_high != Some(7)
        || proposed_low != Some(3)
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "validators must see all proposals without exposing next-boundary state as current",
        ));
    }
    Ok(BoundaryProposal::default())
}

struct GrainSupplyPlugin;
struct HighClaimPlugin;
struct LowClaimPlugin;
struct VisibilityValidatorPlugin;

impl SimulationPlugin for GrainSupplyPlugin {
    fn name(&self) -> &'static str {
        "grain-supply"
    }

    test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000d");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "offer",
            BoundaryPhase::ReservationAndAllocation,
            SystemCadence::Daily,
        );
        contract.reservation_offers = vec![StateKey::new("logistics", "grain")];
        registrar.register_boundary_system(contract, offer_grain)
    }
}

impl SimulationPlugin for HighClaimPlugin {
    fn name(&self) -> &'static str {
        "high-claim"
    }

    test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000e");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut request = BoundarySystemContract::new(
            "request",
            BoundaryPhase::ReservationAndAllocation,
            SystemCadence::Daily,
        );
        request.reservation_requests = vec![StateKey::new("logistics", "grain")];
        registrar.register_boundary_system(request, high_request)?;
        let mut apply = BoundarySystemContract::new(
            "apply",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        apply.writes = vec![StateKey::new("allocation", "high")];
        apply.reservation_reads = vec![ReservationRef::new("high-claim", "request", "grain")];
        apply.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(apply, record_high_grant)
    }
}

impl SimulationPlugin for LowClaimPlugin {
    fn name(&self) -> &'static str {
        "low-claim"
    }

    test_plugin_identity!("000000000000000000000000000000000000000000000000000000000000000f");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut request = BoundarySystemContract::new(
            "request",
            BoundaryPhase::ReservationAndAllocation,
            SystemCadence::Daily,
        );
        request.reservation_requests = vec![StateKey::new("logistics", "grain")];
        registrar.register_boundary_system(request, low_request)?;
        let mut apply = BoundarySystemContract::new(
            "apply",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        apply.writes = vec![StateKey::new("allocation", "low")];
        apply.reservation_reads = vec![ReservationRef::new("low-claim", "request", "grain")];
        registrar.register_boundary_system(apply, record_low_grant)
    }
}

impl SimulationPlugin for VisibilityValidatorPlugin {
    fn name(&self) -> &'static str {
        "visibility-validator"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000010");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut contract = BoundarySystemContract::new(
            "validate",
            BoundaryPhase::InvariantValidation,
            SystemCadence::Daily,
        );
        contract.reads = vec![
            StateKey::new("allocation", "high"),
            StateKey::new("allocation", "low"),
        ];
        registrar.register_boundary_system(contract, validate_visibility)
    }
}

fn stage_boundary_rollback_mutations(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 2 {
        return Ok(BoundaryProposal::default());
    }
    let _ = view.random_range(&failure_random_stream(), 100, "rollback proof")?;
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::SetComponent {
                state: StateKey::new("boundary-rollback", "value"),
                entity: EntityRef::Army(ArmyId::new(1)),
                component: "value".to_owned(),
                value: Value::Bool(true),
                summary: "Stage a value before transaction failure".to_owned(),
            },
            BoundaryDirective::ScheduleIngress {
                after: SimDuration::hours(1),
                packet_type: "follow-up".to_owned(),
                priority: 0,
                payload: serde_json::json!({ "label": "rollback proof" }),
                affected: vec![EntityRef::Army(ArmyId::new(1))],
            },
        ],
        ..BoundaryProposal::default()
    })
}

struct BoundaryRollbackPlugin;

impl SimulationPlugin for BoundaryRollbackPlugin {
    fn name(&self) -> &'static str {
        "boundary-rollback"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000011");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "follow-up".to_owned(),
            description: "A rollback fixture packet".to_owned(),
            class: IngressClass::Information,
            payload_schema: object_payload_schema("label"),
        })?;
        let mut propose = BoundarySystemContract::new(
            "propose",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        propose.writes = vec![StateKey::new("boundary-rollback", "value")];
        propose.random_streams = vec![failure_random_stream()];
        propose.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(propose, stage_boundary_rollback_mutations)
    }
}

struct RecordLifecyclePlugin;
struct RecordDeleteOnlyPlugin;
struct RecordCyclePlugin;
struct RecordSeatDeletionPlugin;

fn office_kind() -> DomainRecordKind {
    DomainRecordKind::new("fixture.governance", "office")
}

fn obligation_kind() -> DomainRecordKind {
    DomainRecordKind::new("fixture.governance", "obligation")
}

fn office_reference(id: &str) -> DomainRecordRef {
    DomainRecordRef::new("fixture.governance", "office", id)
}

fn obligation_reference() -> DomainRecordRef {
    DomainRecordRef::new("fixture.governance", "obligation", "standing-order")
}

fn object_payload_schema(field: &str) -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([(
            field.to_owned(),
            PayloadProperty {
                value_type: PayloadValueType::String,
                required: true,
            },
        )]),
        allow_additional: false,
    }
}

fn office_draft(id: &str, name: &str) -> DomainRecordDraft {
    DomainRecordDraft {
        reference: office_reference(id),
        payload: serde_json::json!({ "name": name }),
        references: vec![DomainReference {
            role: "holder".to_owned(),
            target: DomainReferenceTarget::Core(EntityRef::Person(PersonId::new(1))),
        }],
    }
}

fn obligation_draft(office: &str, status: &str) -> DomainRecordDraft {
    DomainRecordDraft {
        reference: obligation_reference(),
        payload: serde_json::json!({ "status": status }),
        references: vec![DomainReference {
            role: "office".to_owned(),
            target: DomainReferenceTarget::Domain(office_reference(office)),
        }],
    }
}

fn initial_record(owner: &str, class: DomainRecordClass, draft: DomainRecordDraft) -> DomainRecord {
    DomainRecord {
        reference: draft.reference,
        owner: owner.to_owned(),
        class,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    }
}

fn try_rehash_tampered_snapshot(snapshot: &mut SimulationSnapshot) -> Result<(), CanwuError> {
    let mut previous_hash = GENESIS_BOUNDARY_HASH.to_owned();
    for boundary in &mut snapshot.boundaries {
        boundary.previous_hash.clone_from(&previous_hash);
        boundary.hash = compute_boundary_hash(boundary)?;
        previous_hash.clone_from(&boundary.hash);
    }
    if snapshot.commitment_format_version == COMMITMENT_FORMAT_VERSION {
        snapshot.commitment_roots = Some(snapshot_commitment_roots(snapshot)?);
    }
    snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)?;
    Ok(())
}

fn rehash_tampered_snapshot(snapshot: &mut SimulationSnapshot) {
    try_rehash_tampered_snapshot(snapshot).expect("tampered snapshot should still hash");
}

fn refresh_snapshot_commitments_and_checkpoint(snapshot: &mut SimulationSnapshot) {
    if snapshot.commitment_format_version == COMMITMENT_FORMAT_VERSION {
        snapshot.commitment_roots = Some(
            snapshot_commitment_roots(snapshot)
                .expect("snapshot domains should produce commitment roots"),
        );
    }
    snapshot.checkpoint_hash = snapshot_checkpoint_hash(snapshot)
        .expect("tampered snapshot should still have a coherent outer commitment");
}

fn downgrade_snapshot_commitments(snapshot: &mut SimulationSnapshot) {
    snapshot.commitment_format_version = 0;
    snapshot.commitment_roots = None;
}

fn record_lifecycle_proposal(context: &BoundaryContext, delete_only: bool) -> BoundaryProposal {
    let directives = match context.boundary_id.get() {
        1 => vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: office_draft("office-a", "Primary Office"),
                },
                summary: "Create the original office".to_owned(),
            },
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: office_draft("office-b", "Successor Office"),
                },
                summary: "Create the successor office".to_owned(),
            },
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: obligation_draft("office-a", "open"),
                },
                summary: "Create an obligation assigned to the original office".to_owned(),
            },
            BoundaryDirective::SetComponent {
                state: StateKey::new("fixture.governance", "marker"),
                entity: EntityRef::Domain(office_reference("office-b")),
                component: "status".to_owned(),
                value: Value::String("created".to_owned()),
                summary: "Mark the successor office as created".to_owned(),
            },
        ],
        2 => vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: office_draft("office-c", "Later Office"),
                },
                summary: "Create the later successor office".to_owned(),
            },
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Retire {
                    record: office_reference("office-a"),
                    expected_version: 1,
                    successor: Some(office_reference("office-b")),
                },
                summary: "Retire the original office with a stable successor".to_owned(),
            },
        ],
        3 if delete_only => vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Delete {
                record: office_reference("office-a"),
                expected_version: 2,
            },
            summary: "Attempt to delete a still-referenced office".to_owned(),
        }],
        3 => vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Retire {
                record: office_reference("office-b"),
                expected_version: 1,
                successor: Some(office_reference("office-c")),
            },
            summary: "Extend the persisted office succession chain".to_owned(),
        }],
        4 => vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: obligation_draft("office-c", "transferred"),
                    expected_version: 1,
                },
                summary: "Transfer the obligation to the successor office".to_owned(),
            },
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Delete {
                    record: office_reference("office-a"),
                    expected_version: 2,
                },
                summary: "Delete the unreferenced retired office".to_owned(),
            },
        ],
        5 => vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: office_draft("office-c", "Stale Office"),
                expected_version: 99,
            },
            summary: "Attempt a stale office update".to_owned(),
        }],
        _ => Vec::new(),
    };
    BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    }
}

fn apply_record_lifecycle(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(record_lifecycle_proposal(context, false))
}

fn apply_invalid_record_delete(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(record_lifecycle_proposal(context, true))
}

fn observe_record_proposal(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let directives = (context.boundary_id.get() == 1)
        .then(|| BoundaryDirective::Emit {
            event_type: "proposal_probe".to_owned(),
            affected: vec![EntityRef::Person(PersonId::new(1))],
            summary: "Observe the record proposal boundary".to_owned(),
        })
        .into_iter()
        .collect();
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn validate_record_lifecycle_view(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let original = view.domain_record(&office_reference("office-a"))?;
    let proposed_successor = view.proposed_domain_record(&office_reference("office-b"))?;
    let obligation = view.domain_record(&obligation_reference())?;
    let valid = match context.boundary_id.get() {
        1 => {
            original.is_some_and(DomainRecord::is_active)
                && obligation.is_some_and(DomainRecord::is_active)
        }
        2 => original.is_some_and(|record| {
            matches!(
                &record.lifecycle,
                DomainRecordLifecycle::Retired {
                    successor: Some(successor),
                    ..
                } if successor == &office_reference("office-b")
            )
        }),
        3 => {
            original.is_some_and(|record| {
                matches!(
                    &record.lifecycle,
                    DomainRecordLifecycle::Retired {
                        successor: Some(successor),
                        ..
                    } if successor == &office_reference("office-b")
                )
            }) && proposed_successor.is_some_and(|record| {
                matches!(
                    &record.lifecycle,
                    DomainRecordLifecycle::Retired {
                        successor: Some(successor),
                        ..
                    } if successor == &office_reference("office-c")
                )
            })
        }
        4 => {
            original.is_some_and(DomainRecord::is_deleted)
                && obligation.is_some_and(|record| {
                    record.references.iter().any(|reference| {
                        reference.target
                            == DomainReferenceTarget::Domain(office_reference("office-c"))
                    })
                })
        }
        _ => true,
    };
    if !valid {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "invariant systems did not receive the deterministic domain-record proposal",
        ));
    }
    Ok(BoundaryProposal::default())
}

fn register_record_fixture(
    registrar: &mut PluginRegistrar<'_>,
    handler: BoundarySystemHandler,
) -> Result<(), CanwuError> {
    let mut writes = register_record_schemas(registrar)?;
    writes.push(StateKey::new("fixture.governance", "marker"));

    let mut lifecycle = BoundarySystemContract::new(
        "lifecycle",
        BoundaryPhase::DomainDeltaProposal,
        SystemCadence::Daily,
    );
    lifecycle.writes.clone_from(&writes);
    lifecycle.emits = vec!["record_probe".to_owned()];
    lifecycle.visibility = StateVisibility::SameBoundary;
    registrar.register_boundary_system(lifecycle, handler)?;

    let mut observer = BoundarySystemContract::new(
        "observer",
        BoundaryPhase::DomainDeltaProposal,
        SystemCadence::Daily,
    );
    observer.emits = vec!["proposal_probe".to_owned()];
    observer.visibility = StateVisibility::SameBoundary;
    registrar.register_boundary_system(observer, observe_record_proposal)?;

    let mut invariant = BoundarySystemContract::new(
        "validate-lifecycle",
        BoundaryPhase::InvariantValidation,
        SystemCadence::Daily,
    );
    invariant.reads = writes;
    registrar.register_boundary_system(invariant, validate_record_lifecycle_view)
}

fn register_record_schemas(
    registrar: &mut PluginRegistrar<'_>,
) -> Result<Vec<StateKey>, CanwuError> {
    let mut office = DomainRecordSchema::new(office_kind(), DomainRecordClass::Entity);
    office.payload_schema = object_payload_schema("name");
    office.references = vec![DomainReferenceSchema {
        role: "holder".to_owned(),
        targets: vec![DomainReferenceTargetKind::Core(
            canwu_core::CoreEntityKind::Person,
        )],
        required: true,
        multiple: false,
        allow_retired: false,
    }];
    let office_state = office.state_key();
    registrar.register_record_schema(office)?;

    let mut obligation = DomainRecordSchema::new(obligation_kind(), DomainRecordClass::Record);
    obligation.payload_schema = object_payload_schema("status");
    obligation.references = vec![DomainReferenceSchema {
        role: "office".to_owned(),
        targets: vec![DomainReferenceTargetKind::Domain(office_kind())],
        required: true,
        multiple: false,
        allow_retired: true,
    }];
    let obligation_state = obligation.state_key();
    registrar.register_record_schema(obligation)?;
    Ok(vec![office_state, obligation_state])
}

impl SimulationPlugin for RecordLifecyclePlugin {
    fn name(&self) -> &'static str {
        "fixture-record-lifecycle"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000021");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_record_fixture(registrar, apply_record_lifecycle)
    }
}

impl SimulationPlugin for RecordDeleteOnlyPlugin {
    fn name(&self) -> &'static str {
        "fixture-record-delete-only"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000022");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        register_record_fixture(registrar, apply_invalid_record_delete)
    }
}

fn apply_record_cycle(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let directives = match context.boundary_id.get() {
        1 => vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: office_draft("office-a", "First Office"),
                },
                summary: "Create the first office".to_owned(),
            },
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create {
                    record: office_draft("office-b", "Second Office"),
                },
                summary: "Create the second office".to_owned(),
            },
        ],
        2 => vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Retire {
                    record: office_reference("office-a"),
                    expected_version: 1,
                    successor: Some(office_reference("office-b")),
                },
                summary: "Attempt the first half of a successor cycle".to_owned(),
            },
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Retire {
                    record: office_reference("office-b"),
                    expected_version: 1,
                    successor: Some(office_reference("office-a")),
                },
                summary: "Attempt the second half of a successor cycle".to_owned(),
            },
        ],
        _ => Vec::new(),
    };
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for RecordCyclePlugin {
    fn name(&self) -> &'static str {
        "fixture-record-cycle"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000023");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let Some(office_state) = register_record_schemas(registrar)?.into_iter().next() else {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "record cycle fixture is missing its office state",
            ));
        };
        let mut cycle = BoundarySystemContract::new(
            "cycle",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        cycle.writes = vec![office_state];
        cycle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(cycle, apply_record_cycle)
    }
}

fn apply_record_seat_deletion(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let directives = match context.boundary_id.get() {
        1 => vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Retire {
                record: office_reference("office-a"),
                expected_version: 1,
                successor: None,
            },
            summary: "Retire the institution-bound office".to_owned(),
        }],
        2 => vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Delete {
                record: office_reference("office-a"),
                expected_version: 2,
            },
            summary: "Delete the retired institution-bound office".to_owned(),
        }],
        _ => Vec::new(),
    };
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for RecordSeatDeletionPlugin {
    fn name(&self) -> &'static str {
        "fixture-record-seat-deletion"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000024");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let Some(office_state) = register_record_schemas(registrar)?.into_iter().next() else {
            return Err(CanwuError::new(
                ErrorCode::InvalidPluginRegistration,
                "seat deletion fixture is missing its office state",
            ));
        };
        let mut lifecycle = BoundarySystemContract::new(
            "seat-deletion",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        lifecycle.writes = vec![office_state];
        lifecycle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(lifecycle, apply_record_seat_deletion)
    }
}

struct CanonicalIngressPlugin;

fn ingress_class_name(class: IngressClass) -> &'static str {
    match class {
        IngressClass::Command => "command",
        IngressClass::Communication => "communication",
        IngressClass::Acknowledgement => "acknowledgement",
        IngressClass::Information => "information",
        IngressClass::ScheduledSystem => "scheduled_system",
        IngressClass::Decision => "decision",
    }
}

fn consume_canonical_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    for command_id in &context.admitted_commands {
        if view.command(*command_id)?.is_none() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "boundary systems must resolve every admitted command",
            ));
        }
    }
    for event_id in &context.admitted_events {
        if view.event(*event_id)?.is_none() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "boundary systems must resolve every admitted event",
            ));
        }
    }
    if !context.emitted_events.is_empty() {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "pre-commit boundary systems must not observe uncommitted emissions",
        ));
    }
    if context.boundary_id.get() == 1
        && view
            .component(
                &StateKey::new("ingress-fixture", "received"),
                &EntityRef::Person(PersonId::new(1)),
                "canonical-order",
            )?
            .is_some()
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "pre-commit boundary systems must read the stable current-state snapshot",
        ));
    }
    for value in 1..=32 {
        let id = IngressId::new(value);
        if !context.admitted_ingress.contains(&id) && view.ingress(id)?.is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "boundary systems must not observe ingress before admission",
            ));
        }
    }
    let mut order = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(record) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            ..
        } = &record.payload
        else {
            continue;
        };
        if plugin == "canonical-ingress" {
            order.push(format!(
                "{}:{packet_type}:{}",
                ingress_class_name(record.class),
                record.priority
            ));
        }
    }
    if order.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state: StateKey::new("ingress-fixture", "received"),
            entity: EntityRef::Person(PersonId::new(1)),
            component: "canonical-order".to_owned(),
            value: serde_json::json!(order),
            summary: "Record canonical ingress order".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn validate_committed_canonical_evidence(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let received = view.component(
        &StateKey::new("ingress-fixture", "received"),
        &EntityRef::Person(PersonId::new(1)),
        "canonical-order",
    )?;
    if received.is_none() {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "post-commit boundary systems must observe committed current state",
        ));
    }
    if context.emitted_events.is_empty() {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "post-commit boundary systems must observe committed emission identifiers",
        ));
    }
    for event_id in &context.emitted_events {
        if view.event(*event_id)?.is_none() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "post-commit boundary systems must resolve committed emissions",
            ));
        }
    }
    Ok(BoundaryProposal::default())
}

fn mark_daily_calendar(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SetComponent {
            state: StateKey::new("ingress-fixture", "calendar"),
            entity: EntityRef::Person(PersonId::new(1)),
            component: "daily".to_owned(),
            value: Value::Bool(true),
            summary: "Run the queued daily calendar boundary".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for CanonicalIngressPlugin {
    fn name(&self) -> &'static str {
        "canonical-ingress"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000025");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for (name, description, class) in [
            (
                "dispatch",
                "A command or communication packet in transit",
                IngressClass::Communication,
            ),
            (
                "ack",
                "A deterministic command acknowledgement",
                IngressClass::Acknowledgement,
            ),
            (
                "report",
                "A deterministic information packet",
                IngressClass::Information,
            ),
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: description.to_owned(),
                class,
                payload_schema: object_payload_schema("label"),
            })?;
        }
        let mut consumer = BoundarySystemContract::new(
            "consume-ingress",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        consumer.reads = vec![
            StateKey::core_commands(),
            StateKey::core_events(),
            StateKey::core_ingress(),
            StateKey::new("ingress-fixture", "received"),
        ];
        consumer.writes = vec![StateKey::new("ingress-fixture", "received")];
        consumer.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(consumer, consume_canonical_ingress)?;

        let mut committed = BoundarySystemContract::new(
            "validate-committed-evidence",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        committed.reads = vec![
            StateKey::core_events(),
            StateKey::new("ingress-fixture", "received"),
        ];
        registrar.register_boundary_system(committed, validate_committed_canonical_evidence)?;

        let mut calendar = BoundarySystemContract::new(
            "daily-calendar",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        calendar.writes = vec![StateKey::new("ingress-fixture", "calendar")];
        calendar.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(calendar, mark_daily_calendar)
    }
}

struct GeneratedIngressPlugin;

fn relay_generated_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(record) = view.ingress(*ingress_id)? else {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "generated-ingress context references a missing record",
            ));
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            affected_entities,
            ..
        } = &record.payload
        else {
            continue;
        };
        if plugin != "generated-ingress" {
            continue;
        }
        match packet_type.as_str() {
            "dispatch" => directives.push(BoundaryDirective::ScheduleIngress {
                after: SimDuration::ZERO,
                packet_type: "ack".to_owned(),
                priority: 5,
                payload: serde_json::json!({ "label": "automatic acknowledgement" }),
                affected: affected_entities.clone(),
            }),
            "ack" => directives.push(BoundaryDirective::SetComponent {
                state: StateKey::new("generated-ingress-fixture", "received"),
                entity: EntityRef::Person(PersonId::new(1)),
                component: "acknowledged".to_owned(),
                value: Value::Bool(true),
                summary: "Record the automatically generated acknowledgement".to_owned(),
            }),
            _ => {}
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for GeneratedIngressPlugin {
    fn name(&self) -> &'static str {
        "generated-ingress"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000026");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "dispatch".to_owned(),
            description: "A communication packet that requires acknowledgement".to_owned(),
            class: IngressClass::Communication,
            payload_schema: object_payload_schema("label"),
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: "ack".to_owned(),
            description: "A boundary-generated acknowledgement".to_owned(),
            class: IngressClass::Acknowledgement,
            payload_schema: object_payload_schema("label"),
        })?;
        let mut relay = BoundarySystemContract::new(
            "relay-ingress",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        relay.reads = vec![StateKey::core_ingress()];
        relay.writes = vec![StateKey::new("generated-ingress-fixture", "received")];
        relay.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(relay, relay_generated_ingress)
    }
}

struct CrossPluginProducer;
struct CrossPluginConsumer;

fn produce_cross_plugin_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(record) = view.ingress(*ingress_id)? else {
            continue;
        };
        if matches!(
            &record.payload,
            IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            } if plugin == "cross-producer" && packet_type == "start"
        ) {
            directives.push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: "cross-consumer".to_owned(),
                after: SimDuration::ZERO,
                packet_type: "accept".to_owned(),
                priority: 3,
                payload: serde_json::json!({ "label": "relayed" }),
                affected: vec![EntityRef::Person(PersonId::new(1))],
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn consume_cross_plugin_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(record) = view.ingress(*ingress_id)? else {
            continue;
        };
        if matches!(
            &record.payload,
            IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            } if plugin == "cross-consumer" && packet_type == "accept"
        ) {
            directives.push(BoundaryDirective::SetComponent {
                state: StateKey::new("cross-consumer", "received"),
                entity: EntityRef::Person(PersonId::new(1)),
                component: "relayed".to_owned(),
                value: Value::Bool(true),
                summary: "Record cross-plugin ingress delivery".to_owned(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

impl SimulationPlugin for CrossPluginProducer {
    fn name(&self) -> &'static str {
        "cross-producer"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000034");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "start".to_owned(),
            description: "Start one cross-plugin relay".to_owned(),
            class: IngressClass::Information,
            payload_schema: object_payload_schema("label"),
        })?;
        let mut contract = BoundarySystemContract::new(
            "produce-relay",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: "cross-consumer".to_owned(),
            packet_type: "accept".to_owned(),
        }];
        registrar.register_boundary_system(contract, produce_cross_plugin_ingress)
    }
}

impl SimulationPlugin for CrossPluginConsumer {
    fn name(&self) -> &'static str {
        "cross-consumer"
    }

    test_plugin_identity!("0000000000000000000000000000000000000000000000000000000000000035");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "accept".to_owned(),
            description: "Accept one cross-plugin relay".to_owned(),
            class: IngressClass::Information,
            payload_schema: object_payload_schema("label"),
        })?;
        let mut contract = BoundarySystemContract::new(
            "consume-relay",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.writes = vec![StateKey::new("cross-consumer", "received")];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, consume_cross_plugin_ingress)
    }
}

fn move_order(ids: &DemoIds) -> CommandEnvelope {
    CommandEnvelope::new(
        Issuer::Actor(ids.commander),
        Command::OrderMovement {
            subject: EntityRef::Army(ids.army),
            destination: ids.eastern_territory,
            cargo: Vec::new(),
        },
    )
}

fn manifest_for_configuration(
    scenario: &Scenario,
    configuration: &RunConfiguration,
) -> RunManifest {
    let scenario_manifest =
        ArtifactManifest::for_scenario("fixture", "policy-fixture", "1", scenario)
            .expect("scenario identity should hash");
    let configuration_manifest =
        ArtifactManifest::for_run_configuration("fixture", "run-configuration", "1", configuration)
            .expect("run configuration identity should hash");
    RunManifest::declared(scenario_manifest, configuration_manifest)
}

fn character_authority(
    actor: PersonId,
    army: ArmyId,
    seat_id: &str,
    permission_profile_id: &str,
) -> CommandAuthority {
    CommandAuthority {
        decision_origin: DecisionOrigin::Actor { actor },
        seat_id: Some(seat_id.to_owned()),
        permission_profile_id: Some(permission_profile_id.to_owned()),
        command_subject: Some(EntityRef::Army(army)),
    }
}

fn run_keyed_fixture(operations: &[&str]) -> (BTreeMap<String, u64>, u64, Simulation, Scenario) {
    let (scenario, _) = demo_scenario();
    let plugin = KeyedRandomPlugin;
    let mut simulation =
        Simulation::new(83, scenario.clone()).expect("keyed random fixture should initialize");
    simulation
        .register_plugin(&plugin)
        .expect("keyed random plugin should register");
    for operation in operations {
        simulation
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                plugin.name(),
                "operation",
                SimTime::EPOCH,
                serde_json::json!({ "operation": operation }),
            ))
            .expect("operation ingress should enqueue");
    }
    simulation
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("keyed operations should settle");
    let mut keyed = BTreeMap::new();
    let mut sequential = None;
    for draw in simulation.random_draws() {
        match &draw.address {
            RandomDrawAddress::OperationV1(address) => {
                keyed.insert(address.application_operation_id.clone(), draw.value);
            }
            RandomDrawAddress::Sequential { .. } => sequential = Some(draw.value),
        }
    }
    (
        keyed,
        sequential.expect("sequential control draw exists"),
        simulation,
        scenario,
    )
}

mod core;
mod ingress;
mod knowledge;
mod persistence_contracts;
mod plugins;
