use canwu_core::{
    BoundaryId, DomainRecordKind, KnowledgeHolderPolicy, KnowledgeHolderRef, KnowledgeRecordKind,
    KnowledgeSchemaId, PersonId, RandomDrawId,
};
use canwu_event::{CauseRef, EventAudience, EventKind};
use canwu_sim::{
    BoundaryDirective, BoundaryEmissionKind, BoundaryPhase, BoundaryReceipt,
    BoundarySystemContract, DomainRecordClass, DomainRecordMutationPolicy, DomainRecordSchema,
    DomainReferenceTargetKind, ErrorCode, KnowledgeWriteGrant, PluginDescriptor,
    PluginIngressTarget, PluginKnowledgeSchema, RandomDrawAddress, RandomDrawProducer,
    RandomDrawRecord, RandomStreamKey, SimulationSnapshot, StateVisibility, SystemCadence,
};
use canwu_time::SimTime;

#[cfg(not(feature = "legacy-v4-source-shape"))]
fn construct_record() -> RandomDrawRecord {
    RandomDrawRecord {
        id: RandomDrawId::new(1),
        at: SimTime::EPOCH,
        stream: RandomStreamKey::new("fixture", "compile", 1),
        address: RandomDrawAddress::Sequential { position: 0 },
        operation_evidence: None,
        upper_exclusive: 10,
        value: 4,
        purpose: "compile the format-5 shape".to_owned(),
        producer: RandomDrawProducer::CoreSystem {
            system: "api-delta".to_owned(),
        },
        outcome: None,
        cause: CauseRef::System("api-delta".to_owned()),
        correlation_id: 1,
    }
}

#[cfg(not(feature = "legacy-v4-source-shape"))]
fn exercise_information_flow_api_delta() {
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let schema_id = KnowledgeSchemaId {
        kind: KnowledgeRecordKind::new("fixture", "knowledge"),
        version: 1,
    };
    let publication = BoundaryDirective::PublishKnowledge {
        holder: holder.clone(),
        visibility: StateVisibility::SameBoundary,
        producer_correlation: Some("fixture-correlation".to_owned()),
        records: Vec::new(),
        summary: "exercise the format-5 publication shape".to_owned(),
    };
    let cross_plugin_ingress = BoundaryDirective::SchedulePluginIngress {
        target_plugin: "fixture-target".to_owned(),
        after: canwu_time::SimDuration::ZERO,
        packet_type: "continue".to_owned(),
        priority: 0,
        payload: serde_json::Value::Null,
        affected: Vec::new(),
    };

    let mut boundary_contract = BoundarySystemContract::new(
        "publish",
        BoundaryPhase::PerspectiveAndReportMaterialization,
        SystemCadence::Daily,
    );
    boundary_contract.knowledge_writes.push(KnowledgeWriteGrant {
        schema: schema_id.clone(),
        visibilities: vec![StateVisibility::SameBoundary],
    });
    boundary_contract
        .plugin_ingress_targets
        .push(PluginIngressTarget {
            target_plugin: "fixture-target".to_owned(),
            packet_type: "continue".to_owned(),
        });

    let mut record_schema = DomainRecordSchema::new(
        DomainRecordKind::new("fixture", "record"),
        DomainRecordClass::Entity,
    );
    record_schema.holder_policy = KnowledgeHolderPolicy::Allowed;
    record_schema.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    let descriptor = PluginDescriptor {
        name: "fixture".to_owned(),
        version: "0.5.0".to_owned(),
        semantic_hash: "0".repeat(64),
        boundary_systems: vec![boundary_contract],
        record_schemas: vec![record_schema],
        knowledge_schemas: vec![PluginKnowledgeSchema {
            id: schema_id,
            schema_hash: "1".repeat(64),
            writable: true,
            payload_schema: canwu_sim::PayloadSchema::Any,
            subjects: Vec::new(),
        }],
        ..PluginDescriptor::default()
    };
    let receipt = BoundaryReceipt {
        boundary_id: BoundaryId::new(1),
        settled_at: SimTime::EPOCH,
        emitted_events: Vec::new(),
        generated_ingress: Vec::new(),
        random_draws: Vec::new(),
        boundary_hash: "2".repeat(64),
        change_count: 0,
        record_change_count: 0,
        knowledge_batch_count: 1,
        knowledge_record_count: 0,
        allocations: Vec::new(),
    };
    let event = EventKind::KnowledgePublished {
        holder: holder.clone(),
        record_count: 0,
    };
    let audience = EventAudience::KnowledgeHolder(holder);
    let emission = BoundaryEmissionKind::KnowledgeChange { change_index: 0 };
    let target = DomainReferenceTargetKind::AnyEntity;
    let knowledge_errors = [
        ErrorCode::InvalidKnowledgeHolder,
        ErrorCode::InvalidKnowledgeRecord,
        ErrorCode::InvalidKnowledgeSchema,
        ErrorCode::InvalidKnowledgeAuthority,
        ErrorCode::KnowledgeLimitExceeded,
        ErrorCode::KnowledgeReadCutUnavailable,
        ErrorCode::KnowledgeRecordNotFound,
        ErrorCode::UndeclaredKnowledgeWrite,
        ErrorCode::EvidenceUnavailable,
        ErrorCode::EvidenceContentUnavailable,
        ErrorCode::DuplicateKnowledgeRecordKind,
        ErrorCode::InvalidRandomOperationEvidence,
        ErrorCode::RandomOperationConflict,
        ErrorCode::LegacyReplayUnavailable,
        ErrorCode::UnsupportedRandomDrawAddress,
    ];

    let _ = (
        publication,
        cross_plugin_ingress,
        descriptor,
        receipt,
        event,
        audience,
        emission,
        target,
        knowledge_errors,
    );
}

#[cfg(not(feature = "legacy-v4-source-shape"))]
fn inspect_snapshot_delta(snapshot: &SimulationSnapshot) -> usize {
    snapshot.knowledge.records.len()
}

#[cfg(not(feature = "legacy-v4-source-shape"))]
fn inspect_viewer_context(context: &canwu_api::ViewerContext) {
    let _ = (context.principal(), context.actor(), context.observation());
}

#[cfg(feature = "legacy-v4-source-shape")]
fn construct_record() -> RandomDrawRecord {
    RandomDrawRecord {
        id: RandomDrawId::new(1),
        at: SimTime::EPOCH,
        stream: RandomStreamKey::new("fixture", "compile", 1),
        position: 0,
        upper_exclusive: 10,
        value: 4,
        purpose: "compile the format-4 shape".to_owned(),
        producer: RandomDrawProducer::CoreSystem {
            system: "api-delta".to_owned(),
        },
        outcome: None,
        cause: CauseRef::System("api-delta".to_owned()),
        correlation_id: 1,
    }
}

#[cfg(feature = "viewer-admin-leak")]
fn leak_admin_snapshot(canwu: &canwu_api::Canwu) {
    let viewer = canwu
        .viewer()
        .expect("fixture only checks the public type surface");
    let _ = viewer.snapshot();
}

#[cfg(feature = "viewer-journal-leak")]
fn leak_admin_journal(canwu: &canwu_api::Canwu) {
    let viewer = canwu
        .viewer()
        .expect("fixture only checks the public type surface");
    let _ = viewer.replay_journal();
}

#[cfg(feature = "viewer-domain-leak")]
fn leak_admin_domain_records(canwu: &canwu_api::Canwu) {
    let viewer = canwu
        .viewer()
        .expect("fixture only checks the public type surface");
    let _ = viewer.domain_records();
}

#[cfg(feature = "viewer-boundary-leak")]
fn leak_admin_boundaries(canwu: &canwu_api::Canwu) {
    let viewer = canwu
        .viewer()
        .expect("fixture only checks the public type surface");
    let _ = viewer.boundaries();
}

fn main() {
    let record = construct_record();
    assert_eq!(record.upper_exclusive, 10);

    #[cfg(not(feature = "legacy-v4-source-shape"))]
    {
        exercise_information_flow_api_delta();
        let _ = inspect_snapshot_delta;
        let _ = inspect_viewer_context;
    }

    #[cfg(feature = "viewer-admin-leak")]
    {
        let _ = leak_admin_snapshot;
    }
    #[cfg(feature = "viewer-journal-leak")]
    {
        let _ = leak_admin_journal;
    }
    #[cfg(feature = "viewer-domain-leak")]
    {
        let _ = leak_admin_domain_records;
    }
    #[cfg(feature = "viewer-boundary-leak")]
    {
        let _ = leak_admin_boundaries;
    }
}
