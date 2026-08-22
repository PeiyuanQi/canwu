#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryId, BoundaryRequest, Canwu, Command, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordLifecycle, DomainRecordVersionRef, DomainRecordVersionSource,
    EntityRef, ErrorCode, EvidenceRef, Government, GovernmentId, IngressId, Issuer,
    KnowledgeHolderRef, KnowledgeQuery, KnowledgeSnapshot, MapPoint, PayloadSchema, Person,
    PersonId, PluginActionDescriptor, PluginRegistrar, Scenario, SimTime, SimulationPlugin,
    SimulationView, SystemDirective, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_api::{DomainReference, DomainReferenceTarget};
use canwu_information::{
    AUTHORITY_COMMAND_PRODUCER, AUTHORITY_COMMAND_TYPE, Access, AccessPayload, Audience,
    AudienceAccessEvidence, AudienceMembership, AudiencePayload, AuthorityAssignment,
    AuthorityAssignmentPayload, ClaimedSourceRef, Content, ContentPayload, ContentRelation,
    DELEGATED_AUTHORITY_GRANT, DelegationClaimV1, INFORMATION_COMMAND,
    INSTITUTIONAL_AUTHORITY_GRANT, InformationBody, InformationLimitsV1, InformationOperation,
    InformationOperationEnvelope, InformationOperationId, InformationOperationPayload,
    InformationOperationRecord, InformationOperationStatus, InformationOutputKind,
    InformationOutputSlot, InformationPlugin, Interpretation, InterpretationAuthority,
    InterpretationPayload, InterpretationStatus, LifecycleRequest, PLUGIN_NAME, RecordBinding,
    Release, ReleasePayload, ReleaseScope, ReleaseStatus, Representation, RepresentationPayload,
    audience_membership_root_v1, derive_operation_record_ref, derive_output_record_ref,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const TEST_NAMESPACE: &str = "fixture.runtime-information";

#[derive(Clone, Copy)]
struct AuthorityCommandPlugin {
    name: &'static str,
    semantic_hash: &'static str,
    consume_persisted_result_only: bool,
}

impl SimulationPlugin for AuthorityCommandPlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        self.semantic_hash
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), canwu_api::CanwuError> {
        let handler = if self.consume_persisted_result_only {
            consume_persisted_interpretation_result
        } else {
            retain_authority_command
        };
        registrar.register_command(
            PluginActionDescriptor {
                name: AUTHORITY_COMMAND_TYPE.to_owned(),
                description: "Persist one neutral interpretation authority claim".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            handler,
        )
    }
}

#[allow(clippy::unnecessary_wraps)]
fn retain_authority_command(
    _view: &SimulationView<'_>,
    _context: &canwu_api::CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, canwu_api::CanwuError> {
    Ok(Vec::new())
}

fn consume_persisted_interpretation_result(
    _view: &SimulationView<'_>,
    _context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, canwu_api::CanwuError> {
    if payload.pointer("/result/format").and_then(Value::as_str) == Some("neutral_result_v1") {
        Ok(Vec::new())
    } else {
        Err(canwu_api::CanwuError::new(
            ErrorCode::ReplayMismatch,
            "replay requires the persisted interpretation result",
        ))
    }
}

const AUTHORITY_PLUGIN: AuthorityCommandPlugin = AuthorityCommandPlugin {
    name: AUTHORITY_COMMAND_PRODUCER,
    semantic_hash: "0000000000000000000000000000000000000000000000000000000000000a01",
    consume_persisted_result_only: false,
};
const REPLAY_GUARD_AUTHORITY_PLUGIN: AuthorityCommandPlugin = AuthorityCommandPlugin {
    name: AUTHORITY_COMMAND_PRODUCER,
    semantic_hash: "0000000000000000000000000000000000000000000000000000000000000a01",
    consume_persisted_result_only: true,
};
const WRONG_AUTHORITY_PLUGIN: AuthorityCommandPlugin = AuthorityCommandPlugin {
    name: "fixture-wrong-authority",
    semantic_hash: "0000000000000000000000000000000000000000000000000000000000000a02",
    consume_persisted_result_only: false,
};

fn government() -> GovernmentId {
    GovernmentId::new(1)
}
fn territory() -> TerritoryId {
    TerritoryId::new(1)
}
fn person(index: u64) -> PersonId {
    PersonId::new(index)
}
fn holder(index: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(person(index))
}

fn world(holder_count: u64) -> WorldSnapshot {
    WorldSnapshot {
        people: (1..=holder_count.max(1))
            .map(|index| Person {
                id: person(index),
                name: format!("Holder {index:03}"),
                government: government(),
                current_location: territory(),
                roles: vec!["observer".to_owned()],
                transit: None,
            })
            .collect(),
        governments: vec![Government {
            id: government(),
            name: "Fixture Polity".to_owned(),
            capital: territory(),
        }],
        territories: vec![Territory {
            id: territory(),
            name: "Fixture Place".to_owned(),
            controller: government(),
            position: MapPoint { x: 0.0, y: 0.0 },
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    }
}

fn empty_scenario() -> Scenario {
    Scenario {
        start_time: SimTime::EPOCH,
        world: world(1),
        knowledge: KnowledgeSnapshot::default(),
        domain_records: Vec::new(),
    }
}

fn holder_reference(role: &str, holder: &KnowledgeHolderRef) -> DomainReference {
    let entity = match holder {
        KnowledgeHolderRef::Person(id) => EntityRef::Person(*id),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    };
    DomainReference {
        role: role.to_owned(),
        target: DomainReferenceTarget::Core(entity),
    }
}

fn initial_record<T>(
    reference: TypedDomainRecordRef<T>,
    payload: &T::Payload,
    mut references: Vec<DomainReference>,
) -> DomainRecord
where
    T: canwu_api::DomainRecordType,
    T::Payload: Serialize,
{
    references.sort();
    references.dedup();
    let mut draft =
        DomainRecordDraft::from_typed(reference, payload).expect("fixture payload should encode");
    draft.references = references;
    DomainRecord {
        reference: draft.reference,
        owner: PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    }
}

struct ReleaseFixture {
    scenario: Scenario,
    release: TypedDomainRecordRef<Release>,
    representation: TypedDomainRecordRef<Representation>,
}

fn release_scenario(holder_count: u64) -> ReleaseFixture {
    let content = TypedDomainRecordRef::<Content>::new("seed-content");
    let representation = TypedDomainRecordRef::<Representation>::new("seed-representation");
    let audience = TypedDomainRecordRef::<Audience>::new("seed-audience");
    let release = TypedDomainRecordRef::<Release>::new("seed-release");
    let content_record = initial_record(
        content.clone(),
        &ContentPayload {
            content_type: "structured_payload".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"code": "neutral-fixture"}),
            },
            created_at: SimTime::EPOCH,
            derivation: None,
        },
        vec![holder_reference("creator", &holder(1))],
    );
    let representation_record = initial_record(
        representation.clone(),
        &RepresentationPayload {
            format: "neutral_representation_v1".to_owned(),
            created_at: SimTime::EPOCH,
            operation: "encode".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: Some(ClaimedSourceRef {
                namespace: "fixture.claim".to_owned(),
                value: "source-a".to_owned(),
            }),
            interpretation_capability: None,
        },
        vec![DomainReference::from_typed("content", content)],
    );
    let member_holders = (1..=holder_count).map(holder).collect::<Vec<_>>();
    let members = member_holders
        .iter()
        .map(|member| holder_reference("member", member))
        .collect();
    let membership_root =
        audience_membership_root_v1(&member_holders, InformationLimitsV1::canonical())
            .expect("fixture audience root should encode");
    let audience_record = initial_record(
        audience.clone(),
        &AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            resolved_at: SimTime::EPOCH,
            resolution_version: 1,
            resolved_boundary: None,
            member_count: holder_count,
            membership_root,
        },
        members,
    );
    let release_record = initial_record(
        release.clone(),
        &ReleasePayload {
            status: ReleaseStatus::Prepared,
            scope: ReleaseScope::Audience,
            prepared_at: SimTime::EPOCH,
            active_at: None,
        },
        vec![
            DomainReference::from_typed("audience", audience),
            DomainReference::from_typed("representation", representation.clone()),
        ],
    );
    ReleaseFixture {
        scenario: Scenario {
            start_time: SimTime::EPOCH,
            world: world(holder_count),
            knowledge: KnowledgeSnapshot::default(),
            domain_records: vec![
                content_record,
                representation_record,
                audience_record,
                release_record,
            ],
        },
        release,
        representation,
    }
}

fn slot(index: u16, name: &str, kind: InformationOutputKind) -> InformationOutputSlot {
    InformationOutputSlot {
        index,
        name: name.to_owned(),
        kind,
    }
}

fn create_content_operation(
    value: &str,
) -> (InformationOperationEnvelope, TypedDomainRecordRef<Content>) {
    let id = InformationOperationId::new(TEST_NAMESPACE, "create-content");
    let output = slot(0, "result", InformationOutputKind::Content);
    let reference =
        TypedDomainRecordRef::<Content>::from_untyped(derive_output_record_ref(&id, &output))
            .expect("derived content kind");
    let envelope = InformationOperationEnvelope {
        id,
        operation_version: 1,
        operation_kind: "create_content".to_owned(),
        output_slots: vec![output],
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::CreateContent {
                binding: RecordBinding::new(reference.clone(), Vec::new()),
                payload: ContentPayload {
                    content_type: "structured_payload".to_owned(),
                    body: InformationBody::InlineJson {
                        value: json!({"value": value}),
                    },
                    created_at: SimTime::EPOCH,
                    derivation: None,
                },
            },
        },
    };
    (envelope, reference)
}

fn activate_release_operation(
    release: TypedDomainRecordRef<Release>,
    suffix: &str,
) -> InformationOperationEnvelope {
    transition_release_operation(release, suffix, 1, ReleaseStatus::Active)
}

fn transition_release_operation(
    release: TypedDomainRecordRef<Release>,
    suffix: &str,
    expected_version: u64,
    status: ReleaseStatus,
) -> InformationOperationEnvelope {
    InformationOperationEnvelope {
        id: InformationOperationId::new(TEST_NAMESPACE, format!("transition-release-{suffix}")),
        operation_version: 1,
        operation_kind: "transition_release".to_owned(),
        output_slots: Vec::new(),
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::TransitionRelease {
                record: release,
                expected_version,
                proposed: ReleasePayload {
                    status,
                    scope: ReleaseScope::Audience,
                    prepared_at: SimTime::EPOCH,
                    active_at: Some(SimTime::EPOCH),
                },
            },
        },
    }
}

fn interpretation_scenario() -> (
    Scenario,
    TypedDomainRecordRef<Access>,
    TypedDomainRecordRef<Representation>,
    TypedDomainRecordRef<Content>,
) {
    let content = TypedDomainRecordRef::<Content>::new("interpretation-source-content");
    let result_content = TypedDomainRecordRef::<Content>::new("interpretation-result-content");
    let representation =
        TypedDomainRecordRef::<Representation>::new("interpretation-representation");
    let access = TypedDomainRecordRef::<Access>::new("interpretation-access");
    let source_record = initial_record(
        content.clone(),
        &ContentPayload {
            content_type: "synthetic_source".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"code": "SYN-SOURCE"}),
            },
            created_at: SimTime::EPOCH,
            derivation: None,
        },
        vec![holder_reference("creator", &holder(1))],
    );
    let result_record = initial_record(
        result_content.clone(),
        &ContentPayload {
            content_type: "synthetic_result".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"code": "SYN-RESULT"}),
            },
            created_at: SimTime::EPOCH,
            derivation: None,
        },
        Vec::new(),
    );
    let representation_record = initial_record(
        representation.clone(),
        &RepresentationPayload {
            format: "symbol_groups_v1".to_owned(),
            created_at: SimTime::EPOCH,
            operation: "encode".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: Some("decode_fixture".to_owned()),
        },
        vec![
            DomainReference::from_typed("content", content),
            holder_reference("producer", &holder(1)),
        ],
    );
    let access_record = initial_record(
        access.clone(),
        &AccessPayload {
            accessed_at: SimTime::EPOCH,
            method: "nonexclusive_observation".to_owned(),
            extent_per_mille: 1_000,
        },
        vec![
            holder_reference("holder", &holder(2)),
            DomainReference::from_typed("representation", representation.clone()),
        ],
    );
    (
        Scenario {
            start_time: SimTime::EPOCH,
            world: world(4),
            knowledge: KnowledgeSnapshot::default(),
            domain_records: vec![
                source_record,
                result_record,
                representation_record,
                access_record,
            ],
        },
        access,
        representation,
        result_content,
    )
}

fn add_interpretation_review_release(
    scenario: &mut Scenario,
    result_content: TypedDomainRecordRef<Content>,
) -> TypedDomainRecordRef<Release> {
    let representation =
        TypedDomainRecordRef::<Representation>::new("interpretation-review-representation");
    let audience = TypedDomainRecordRef::<Audience>::new("interpretation-review-audience");
    let release = TypedDomainRecordRef::<Release>::new("interpretation-review-release");
    let members = vec![holder(3), holder(4)];
    scenario.domain_records.push(initial_record(
        representation.clone(),
        &RepresentationPayload {
            format: "neutral_result_v1".to_owned(),
            created_at: SimTime::EPOCH,
            operation: "render_result".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: None,
        },
        vec![
            DomainReference::from_typed("content", result_content),
            holder_reference("producer", &holder(2)),
        ],
    ));
    scenario.domain_records.push(initial_record(
        audience.clone(),
        &AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            resolved_at: SimTime::EPOCH,
            resolution_version: 1,
            resolved_boundary: None,
            member_count: 2,
            membership_root: audience_membership_root_v1(
                &members,
                InformationLimitsV1::canonical(),
            )
            .expect("review audience root should encode"),
        },
        members
            .iter()
            .map(|member| holder_reference("member", member))
            .collect(),
    ));
    scenario.domain_records.push(initial_record(
        release.clone(),
        &ReleasePayload {
            status: ReleaseStatus::Prepared,
            scope: ReleaseScope::Audience,
            prepared_at: SimTime::EPOCH,
            active_at: None,
        },
        vec![
            DomainReference::from_typed("audience", audience),
            holder_reference("publisher", &holder(3)),
            DomainReference::from_typed("representation", representation),
        ],
    ));
    release
}

fn record_release_access_operation(
    release: TypedDomainRecordRef<Release>,
    representation: TypedDomainRecordRef<Representation>,
    knowledge_holder: &KnowledgeHolderRef,
    suffix: &str,
) -> InformationOperationEnvelope {
    let id = InformationOperationId::new(TEST_NAMESPACE, format!("record-release-access-{suffix}"));
    let output = slot(0, "result", InformationOutputKind::Access);
    let access =
        TypedDomainRecordRef::<Access>::from_untyped(derive_output_record_ref(&id, &output))
            .expect("derived access kind");
    InformationOperationEnvelope {
        id,
        operation_version: 1,
        operation_kind: "record_access".to_owned(),
        output_slots: vec![output],
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::RecordAccess {
                binding: RecordBinding::new(
                    access,
                    vec![
                        holder_reference("holder", knowledge_holder),
                        DomainReference::from_typed("release", release),
                        DomainReference::from_typed("representation", representation),
                    ],
                ),
                payload: AccessPayload {
                    accessed_at: SimTime::EPOCH,
                    method: "fixture_release_access".to_owned(),
                    extent_per_mille: 1_000,
                },
                audience_evidence: Some(AudienceAccessEvidence::ListedMember),
            },
        },
    }
}

fn record_interpretation_operation(
    access: TypedDomainRecordRef<Access>,
    representation: TypedDomainRecordRef<Representation>,
    result_content: TypedDomainRecordRef<Content>,
    suffix: &str,
    authority: InterpretationAuthority,
) -> InformationOperationEnvelope {
    let id = InformationOperationId::new(TEST_NAMESPACE, format!("record-interpretation-{suffix}"));
    let output = slot(0, "result", InformationOutputKind::Interpretation);
    let reference = TypedDomainRecordRef::<Interpretation>::from_untyped(derive_output_record_ref(
        &id, &output,
    ))
    .expect("derived interpretation kind");
    InformationOperationEnvelope {
        id,
        operation_version: 1,
        operation_kind: "record_interpretation".to_owned(),
        output_slots: vec![output],
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::RecordInterpretation {
                binding: RecordBinding::new(
                    reference,
                    vec![
                        DomainReference::from_typed("input_access", access),
                        DomainReference::from_typed("input_representation", representation),
                        holder_reference("performed_by", &holder(2)),
                        holder_reference("performed_for", &holder(3)),
                        DomainReference::from_typed("result_content", result_content),
                    ],
                ),
                payload: InterpretationPayload {
                    interpreted_at: SimTime::EPOCH,
                    status: InterpretationStatus::Succeeded,
                    capability: "decode_fixture".to_owned(),
                    confidence_per_mille: 900,
                },
                authority,
            },
        },
    }
}

fn authority_claim() -> DelegationClaimV1 {
    DelegationClaimV1 {
        format_version: 1,
        performed_by: EntityRef::Person(person(2)),
        performed_for: holder(3),
        capabilities: vec!["decode_fixture".to_owned()],
        not_before: Some(SimTime::EPOCH),
        expires_at: None,
    }
}

fn queue_authority_command(
    canwu: &mut Canwu,
    request_id: u64,
    producer: &str,
    payload: Value,
) -> canwu_api::CommandId {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("runtime-authority-test".to_owned()),
                    Command::Plugin {
                        plugin: producer.to_owned(),
                        command: AUTHORITY_COMMAND_TYPE.to_owned(),
                        payload,
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("authority command should enter canonical ingress");
    settle(canwu);
    canwu
        .commands()
        .last()
        .expect("retained authority command")
        .id
}

fn settle_to_terminal(canwu: &mut Canwu, envelope: &InformationOperationEnvelope, request_id: u64) {
    queue(canwu, request_id, envelope);
    for _ in 0..5 {
        settle(canwu);
        if canwu
            .typed_domain_record(&derive_operation_record_ref(&envelope.id))
            .is_some_and(|record| {
                record
                    .decode_payload::<InformationOperationRecord>()
                    .is_ok_and(|payload| payload.status.is_terminal())
            })
        {
            break;
        }
    }
}

fn plugin_command(envelope: &InformationOperationEnvelope) -> CommandEnvelope {
    CommandEnvelope::new(
        Issuer::System("runtime-adapter-test".to_owned()),
        Command::Plugin {
            plugin: PLUGIN_NAME.to_owned(),
            command: INFORMATION_COMMAND.to_owned(),
            payload: serde_json::to_value(envelope).expect("operation envelope should encode"),
        },
    )
    .at_time(SimTime::EPOCH)
}

fn queue(canwu: &mut Canwu, request_id: u64, envelope: &InformationOperationEnvelope) {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                plugin_command(envelope),
            ),
        )
        .expect("information command should enter canonical ingress");
}

fn last_attempt_error(canwu: &Canwu) -> Option<ErrorCode> {
    canwu
        .command_attempts()
        .last()
        .and_then(|attempt| match &attempt.outcome {
            CommandAttemptOutcome::Rejected { error } => Some(error.code.clone()),
            CommandAttemptOutcome::Accepted { .. } => None,
        })
}

fn settle(canwu: &mut Canwu) {
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("information boundary should settle");
}

fn operation_payload(canwu: &Canwu, id: &InformationOperationId) -> InformationOperationPayload {
    canwu
        .typed_domain_record(&derive_operation_record_ref(id))
        .expect("operation record should exist")
        .decode_payload::<InformationOperationRecord>()
        .expect("operation payload should decode")
}

fn publication_changes<'a>(
    canwu: &'a Canwu,
    prefix: &str,
) -> Vec<&'a canwu_api::BoundaryKnowledgeChange> {
    canwu
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.knowledge_changes)
        .filter(|change| {
            change.plugin == PLUGIN_NAME
                && change
                    .producer_correlation
                    .as_deref()
                    .is_some_and(|value| value.starts_with(prefix))
        })
        .collect()
}

fn knowledge_schema_names(canwu: &Canwu, index: u64) -> Vec<String> {
    let mut schemas = canwu
        .admin_query_knowledge(holder(index), &KnowledgeQuery::default())
        .expect("authoritative holder query should succeed")
        .records
        .into_iter()
        .map(|record| record.schema.kind.name)
        .collect::<Vec<_>>();
    schemas.sort();
    schemas
}

fn run_delegated_authority_case<F>(
    seed: u64,
    producer: &'static AuthorityCommandPlugin,
    command_payload: Value,
    make_authority: F,
) -> InformationOperationPayload
where
    F: FnOnce(canwu_api::CommandId) -> InterpretationAuthority,
{
    let (scenario, access, representation, result_content) = interpretation_scenario();
    let plugin = InformationPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [producer, &plugin];
    let mut canwu = Canwu::new_with_plugins(seed, scenario, &plugins)
        .expect("authority rejection fixture should initialize");
    let command = queue_authority_command(&mut canwu, 1, producer.name(), command_payload);
    let envelope = record_interpretation_operation(
        access,
        representation,
        result_content,
        &format!("negative-{seed}"),
        make_authority(command),
    );
    settle_to_terminal(&mut canwu, &envelope, 2);
    operation_payload(&canwu, &envelope.id)
}
#[test]
fn public_command_uses_zero_delay_ingress_and_three_phase7_states_with_idempotency() {
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(41, empty_scenario(), &[&plugin])
        .expect("plugin runtime should initialize");
    let (envelope, content) = create_content_operation("alpha");
    queue(&mut canwu, 1, &envelope);
    assert!(
        canwu.typed_domain_record(&content).is_none(),
        "command admission must not mutate domain state"
    );
    settle(&mut canwu);
    assert!(
        canwu
            .typed_domain_record(&derive_operation_record_ref(&envelope.id))
            .is_none()
    );
    assert!(canwu.ingress_log().iter().any(|record| matches!(&record.payload, canwu_api::IngressPayload::Plugin { plugin, packet_type, .. } if plugin == PLUGIN_NAME && packet_type == canwu_information::INFORMATION_INGRESS)));

    settle(&mut canwu);
    assert_eq!(
        operation_payload(&canwu, &envelope.id).status,
        InformationOperationStatus::Accepted
    );
    assert!(canwu.typed_domain_record(&content).is_none());
    settle(&mut canwu);
    assert_eq!(
        operation_payload(&canwu, &envelope.id).status,
        InformationOperationStatus::ApplyingDomainChanges
    );
    assert!(canwu.typed_domain_record(&content).is_none());
    settle(&mut canwu);
    assert_eq!(
        operation_payload(&canwu, &envelope.id).status,
        InformationOperationStatus::Completed
    );
    assert!(canwu.typed_domain_record(&content).is_some());

    let information_ingress_before = canwu.ingress_log().iter().filter(|record| matches!(&record.payload, canwu_api::IngressPayload::Plugin { plugin, packet_type, .. } if plugin == PLUGIN_NAME && packet_type == canwu_information::INFORMATION_INGRESS)).count();
    let content_changes_before = canwu
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.record_changes)
        .filter(|change| change.current.reference == *content.as_untyped())
        .count();
    queue(&mut canwu, 2, &envelope);
    settle(&mut canwu);
    let information_ingress_after = canwu.ingress_log().iter().filter(|record| matches!(&record.payload, canwu_api::IngressPayload::Plugin { plugin, packet_type, .. } if plugin == PLUGIN_NAME && packet_type == canwu_information::INFORMATION_INGRESS)).count();
    assert_eq!(
        information_ingress_after, information_ingress_before,
        "exact retry must not enqueue another operation ingress"
    );
    let content_changes_after = canwu
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.record_changes)
        .filter(|change| change.current.reference == *content.as_untyped())
        .count();
    assert_eq!(
        content_changes_after, content_changes_before,
        "exact retry must not duplicate domain output"
    );

    let (conflict, _) = create_content_operation("different-input");
    queue(&mut canwu, 3, &conflict);
    settle(&mut canwu);
    assert_eq!(
        last_attempt_error(&canwu),
        Some(ErrorCode::IdempotencyConflict)
    );

    let mut direct = Canwu::new_with_plugins(44, empty_scenario(), &[&plugin])
        .expect("direct-command fixture should initialize");
    let error = direct
        .submit(plugin_command(&envelope))
        .expect_err("legacy-direct information commands must fail before scheduling ingress");
    assert_eq!(error.code, ErrorCode::MixedCommandIngress);
    assert!(direct.ingress_log().is_empty());
}

#[test]
fn release_publication_uses_unique_correlations_prefix_finalization_and_exact_evidence() {
    let fixture = release_scenario(2);
    let scenario = fixture.scenario.clone();
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(42, scenario, &[&plugin])
        .expect("release fixture should initialize");
    let envelope = activate_release_operation(fixture.release.clone(), "two");
    queue(&mut canwu, 1, &envelope);
    settle(&mut canwu);
    settle(&mut canwu);
    settle(&mut canwu);
    settle(&mut canwu);
    let awaiting = operation_payload(&canwu, &envelope.id);
    assert_eq!(
        awaiting.status,
        InformationOperationStatus::AwaitingFinalization
    );
    assert!(awaiting.continuation.is_none());
    let prefix = format!("information-operation:{}:", awaiting.canonical_input_hash);
    let changes = publication_changes(&canwu, &prefix);
    assert_eq!(changes.len(), 2);
    let correlations: BTreeSet<_> = changes
        .iter()
        .map(|change| change.producer_correlation.as_deref().expect("correlation"))
        .collect();
    assert_eq!(
        correlations.len(),
        2,
        "each holder batch needs a unique producer correlation"
    );
    assert!(correlations.iter().all(|value| value.starts_with(&prefix)));

    settle(&mut canwu);
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert_eq!(
        completed.publication_result_ids.len(),
        2,
        "prefix finalizer must collect both holder batches"
    );
    let operation_ref = derive_operation_record_ref(&envelope.id).into_untyped();
    for index in 1..=2 {
        assert_eq!(
            knowledge_schema_names(&canwu, index),
            vec!["release_available".to_owned()],
            "public knowledge query must expose exactly the released fact"
        );
        let records = canwu
            .knowledge()
            .records
            .get(&holder(index))
            .expect("holder knowledge ledger");
        assert_eq!(records.len(), 1);
        let record = records.values().next().expect("published knowledge record");
        let versions: Vec<&DomainRecordVersionRef> = record
            .origin
            .evidence
            .iter()
            .filter_map(|evidence| match evidence {
                EvidenceRef::DomainRecordVersion(version) => Some(version),
                _ => None,
            })
            .collect();
        assert!(
            versions
                .iter()
                .any(|version| version.record == *fixture.release.as_untyped()
                    && version.version == 2),
            "knowledge origin must cite exact release result version"
        );
        assert!(
            versions
                .iter()
                .any(|version| version.record == operation_ref && version.version == 3),
            "knowledge origin must cite the proposed operation version that entered publication"
        );
    }
}

#[test]
fn expired_release_retains_prior_access_and_knowledge_without_republication() {
    let fixture = release_scenario(2);
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(46, fixture.scenario, &[&plugin])
        .expect("expiration fixture should initialize");

    let activation = activate_release_operation(fixture.release.clone(), "before-expiration");
    settle_to_terminal(&mut canwu, &activation, 1);
    assert_eq!(
        operation_payload(&canwu, &activation.id).status,
        InformationOperationStatus::Completed
    );

    let access = record_release_access_operation(
        fixture.release.clone(),
        fixture.representation.clone(),
        &holder(1),
        "before-expiration",
    );
    let access_record = derive_output_record_ref(&access.id, &access.output_slots[0]);
    settle_to_terminal(&mut canwu, &access, 2);
    assert_eq!(
        operation_payload(&canwu, &access.id).status,
        InformationOperationStatus::Completed
    );
    assert_eq!(
        knowledge_schema_names(&canwu, 1),
        vec!["access_recorded".to_owned(), "release_available".to_owned(),]
    );
    assert_eq!(
        knowledge_schema_names(&canwu, 2),
        vec!["release_available".to_owned()]
    );
    let holder_one_before = canwu
        .admin_query_knowledge(holder(1), &KnowledgeQuery::default())
        .expect("holder one query before expiration")
        .records;
    let holder_two_before = canwu
        .admin_query_knowledge(holder(2), &KnowledgeQuery::default())
        .expect("holder two query before expiration")
        .records;

    let expiration = transition_release_operation(
        fixture.release.clone(),
        "expired",
        2,
        ReleaseStatus::Expired,
    );
    settle_to_terminal(&mut canwu, &expiration, 3);
    let expired = operation_payload(&canwu, &expiration.id);
    assert_eq!(expired.status, InformationOperationStatus::Completed);
    assert!(
        expired.publication_result_ids.is_empty(),
        "a terminal release transition must not publish a second availability fact"
    );
    assert_eq!(
        canwu
            .domain_record(&access_record)
            .expect("prior access must remain retained")
            .version,
        1
    );
    assert_eq!(
        canwu
            .admin_query_knowledge(holder(1), &KnowledgeQuery::default())
            .expect("holder one query after expiration")
            .records,
        holder_one_before
    );
    assert_eq!(
        canwu
            .admin_query_knowledge(holder(2), &KnowledgeQuery::default())
            .expect("holder two query after expiration")
            .records,
        holder_two_before
    );

    let late_access = record_release_access_operation(
        fixture.release,
        fixture.representation,
        &holder(2),
        "after-expiration",
    );
    settle_to_terminal(&mut canwu, &late_access, 4);
    assert_eq!(
        operation_payload(&canwu, &late_access.id).status,
        InformationOperationStatus::Rejected
    );
    assert_eq!(
        canwu
            .admin_query_knowledge(holder(1), &KnowledgeQuery::default())
            .expect("holder one query after rejected access")
            .records,
        holder_one_before
    );
    assert_eq!(
        canwu
            .admin_query_knowledge(holder(2), &KnowledgeQuery::default())
            .expect("holder two query after rejected access")
            .records,
        holder_two_before
    );
}

#[test]
fn publication_continues_across_sixty_four_holders_and_snapshot_replay_is_exact() {
    let fixture = release_scenario(65);
    let scenario = fixture.scenario.clone();
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(43, scenario.clone(), &[&plugin])
        .expect("large release fixture should initialize");
    let envelope = activate_release_operation(fixture.release, "chunked");
    queue(&mut canwu, 1, &envelope);
    settle(&mut canwu);
    settle(&mut canwu);
    settle(&mut canwu);
    settle(&mut canwu);
    let first = operation_payload(&canwu, &envelope.id);
    assert_eq!(
        first.status,
        InformationOperationStatus::AwaitingFinalization
    );
    let continuation = first
        .continuation
        .as_ref()
        .expect("one publication remains");
    assert_eq!(
        (
            continuation.cursor,
            continuation.remaining,
            continuation.chunk_size
        ),
        (64, 1, 1)
    );
    assert_eq!(canwu.knowledge().records.len(), 64);

    let operation_ingress_before = canwu.ingress_log().iter().filter(|record| matches!(&record.payload, canwu_api::IngressPayload::Plugin { plugin, packet_type, .. } if plugin == PLUGIN_NAME && packet_type == canwu_information::INFORMATION_INGRESS)).count();
    queue(&mut canwu, 2, &envelope);
    settle(&mut canwu);
    let operation_ingress_after = canwu.ingress_log().iter().filter(|record| matches!(&record.payload, canwu_api::IngressPayload::Plugin { plugin, packet_type, .. } if plugin == PLUGIN_NAME && packet_type == canwu_information::INFORMATION_INGRESS)).count();
    assert_eq!(
        operation_ingress_after, operation_ingress_before,
        "exact retry during continuation must not enqueue duplicate progress"
    );
    let second = operation_payload(&canwu, &envelope.id);
    assert_eq!(
        second.status,
        InformationOperationStatus::AwaitingFinalization
    );
    assert!(second.continuation.is_none());
    assert_eq!(canwu.knowledge().records.len(), 65);
    settle(&mut canwu);
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert_eq!(completed.publication_result_ids.len(), 65);
    let prefix = format!("information-operation:{}:", completed.canonical_input_hash);
    let changes = publication_changes(&canwu, &prefix);
    assert_eq!(changes.len(), 65);
    assert_eq!(
        changes
            .iter()
            .filter_map(|change| change.producer_correlation.as_ref())
            .collect::<BTreeSet<_>>()
            .len(),
        65
    );

    settle(&mut canwu);

    let snapshot_json = canwu.snapshot_json().expect("final snapshot should encode");
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot_json, &[&plugin])
        .expect("snapshot should restore with plugin");
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot JSON"),
        snapshot_json
    );

    let journal = canwu.replay_journal();
    let replayed = Canwu::replay_from_journal(scenario, &[&plugin], &journal)
        .expect("authoritative journal should replay exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot JSON"),
        snapshot_json
    );

    let expected = canwu.snapshot();
    let mut compact = canwu
        .into_compacted()
        .expect("release case should enter compact mode");
    let segment = compact
        .seal_evidence()
        .expect("release case evidence should seal")
        .expect("release case should produce one archive segment");
    let checkpoint = compact
        .checkpoint()
        .expect("release compact checkpoint should encode");
    assert_eq!(
        compact
            .snapshot_with_segments(vec![segment.clone()])
            .expect("release compact history should reconstruct"),
        expected
    );
    let restored_compact = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        checkpoint,
        vec![segment],
        &[&plugin],
    )
    .expect("release compact checkpoint should restore with the plugin");
    assert_eq!(
        restored_compact
            .snapshot_with_segments(Vec::new())
            .expect("restored release compact state should retain validated evidence"),
        expected
    );
}

#[test]
fn delegated_interpretation_publishes_only_to_holder_and_replays_without_external_work() {
    let (mut scenario, access, representation, result_content) = interpretation_scenario();
    let review_release = add_interpretation_review_release(&mut scenario, result_content.clone());
    let plugin = InformationPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&AUTHORITY_PLUGIN, &plugin];
    let mut canwu = Canwu::new_with_plugins(45, scenario.clone(), &plugins)
        .expect("interpretation fixture should initialize");
    let authority_command = queue_authority_command(
        &mut canwu,
        1,
        AUTHORITY_COMMAND_PRODUCER,
        json!({
            "claim": authority_claim(),
            "result": {"format": "neutral_result_v1"}
        }),
    );
    let envelope = record_interpretation_operation(
        access,
        representation,
        result_content,
        "delegated",
        InterpretationAuthority::Delegated {
            evidence: EvidenceRef::Command(authority_command),
            authority_grant: DELEGATED_AUTHORITY_GRANT.to_owned(),
        },
    );
    queue(&mut canwu, 2, &envelope);
    for _ in 0..5 {
        settle(&mut canwu);
    }
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert!(completed.authority_claim_hash.is_some());
    assert_eq!(completed.publication_result_ids.len(), 1);
    assert!(knowledge_schema_names(&canwu, 1).is_empty());
    assert!(knowledge_schema_names(&canwu, 2).is_empty());
    assert_eq!(
        knowledge_schema_names(&canwu, 3),
        vec!["interpretation_recorded".to_owned()]
    );
    assert!(knowledge_schema_names(&canwu, 4).is_empty());

    let review_activation = activate_release_operation(review_release, "interpretation-review");
    settle_to_terminal(&mut canwu, &review_activation, 3);
    let release_completed = operation_payload(&canwu, &review_activation.id);
    assert_eq!(
        release_completed.status,
        InformationOperationStatus::Completed
    );
    assert_eq!(release_completed.publication_result_ids.len(), 2);
    assert!(knowledge_schema_names(&canwu, 1).is_empty());
    assert!(knowledge_schema_names(&canwu, 2).is_empty());
    assert_eq!(
        knowledge_schema_names(&canwu, 3),
        vec![
            "interpretation_recorded".to_owned(),
            "release_available".to_owned(),
        ]
    );
    assert_eq!(
        knowledge_schema_names(&canwu, 4),
        vec!["release_available".to_owned()]
    );

    settle(&mut canwu);
    let snapshot = canwu.snapshot();
    let replay_plugins: [&dyn SimulationPlugin; 2] = [&REPLAY_GUARD_AUTHORITY_PLUGIN, &plugin];
    let replayed = Canwu::replay_from_journal(scenario, &replay_plugins, &canwu.replay_journal())
        .expect("interpretation replay must consume the persisted result without external work");
    assert_eq!(replayed.snapshot(), snapshot);

    let mut compact = canwu
        .into_compacted()
        .expect("interpretation case should enter compact mode");
    let segment = compact
        .seal_evidence()
        .expect("interpretation case evidence should seal")
        .expect("interpretation case should produce one archive segment");
    let checkpoint = compact
        .checkpoint()
        .expect("interpretation compact checkpoint should encode");
    assert_eq!(
        compact
            .snapshot_with_segments(vec![segment.clone()])
            .expect("interpretation compact history should reconstruct"),
        snapshot
    );
    let restored_compact = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        checkpoint,
        vec![segment],
        &plugins,
    )
    .expect("interpretation compact checkpoint should restore with the plugin");
    assert_eq!(
        restored_compact
            .snapshot_with_segments(Vec::new())
            .expect("restored interpretation compact state should retain validated evidence"),
        snapshot
    );
}

#[test]
fn institutional_interpretation_requires_exact_assignment_version_and_claim() {
    let (mut scenario, access, representation, result_content) = interpretation_scenario();
    let assignment = TypedDomainRecordRef::<AuthorityAssignment>::new("neutral-assignment");
    let claim = authority_claim();
    scenario.domain_records.push(initial_record(
        assignment.clone(),
        &AuthorityAssignmentPayload {
            claim: claim.clone(),
        },
        Vec::new(),
    ));
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(51, scenario, &[&plugin])
        .expect("institutional assignment fixture should initialize");
    let envelope = record_interpretation_operation(
        access,
        representation,
        result_content,
        "institutional",
        InterpretationAuthority::InstitutionalRole {
            assignment: DomainRecordVersionRef {
                record: assignment.as_untyped().clone(),
                version: 1,
                established_by: DomainRecordVersionSource::InitialScenario,
            },
            authority_grant: INSTITUTIONAL_AUTHORITY_GRANT.to_owned(),
        },
    );
    settle_to_terminal(&mut canwu, &envelope, 1);
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert_eq!(
        completed.authority_claim_hash.as_deref().map(str::len),
        Some(64)
    );

    let (mut scenario, access, representation, result_content) = interpretation_scenario();
    scenario.domain_records.push(initial_record(
        assignment.clone(),
        &AuthorityAssignmentPayload { claim },
        Vec::new(),
    ));
    let mut canwu = Canwu::new_with_plugins(52, scenario, &[&plugin])
        .expect("wrong-version assignment fixture should initialize");
    let wrong_version = record_interpretation_operation(
        access,
        representation,
        result_content,
        "institutional-wrong-version",
        InterpretationAuthority::InstitutionalRole {
            assignment: DomainRecordVersionRef {
                record: assignment.into_untyped(),
                version: 2,
                established_by: DomainRecordVersionSource::InitialScenario,
            },
            authority_grant: INSTITUTIONAL_AUTHORITY_GRANT.to_owned(),
        },
    );
    settle_to_terminal(&mut canwu, &wrong_version, 1);
    let rejected = operation_payload(&canwu, &wrong_version.id);
    assert_eq!(rejected.status, InformationOperationStatus::Rejected);
    assert_eq!(
        rejected.rejection_code.as_deref(),
        Some("invalid_authority")
    );
    assert!(rejected.authority_claim_hash.is_none());

    let (mut scenario, access, representation, result_content) = interpretation_scenario();
    let assignment = TypedDomainRecordRef::<AuthorityAssignment>::new("neutral-assignment");
    scenario.domain_records.push(initial_record(
        assignment.clone(),
        &AuthorityAssignmentPayload {
            claim: authority_claim(),
        },
        Vec::new(),
    ));
    let mut canwu = Canwu::new_with_plugins(53, scenario, &[&plugin])
        .expect("wrong-source assignment fixture should initialize");
    let wrong_source = record_interpretation_operation(
        access,
        representation,
        result_content,
        "institutional-wrong-source",
        InterpretationAuthority::InstitutionalRole {
            assignment: DomainRecordVersionRef {
                record: assignment.into_untyped(),
                version: 1,
                established_by: DomainRecordVersionSource::BoundaryChange {
                    boundary: BoundaryId::new(1),
                    change_index: 0,
                },
            },
            authority_grant: INSTITUTIONAL_AUTHORITY_GRANT.to_owned(),
        },
    );
    settle_to_terminal(&mut canwu, &wrong_source, 1);
    let rejected = operation_payload(&canwu, &wrong_source.id);
    assert_eq!(rejected.status, InformationOperationStatus::Rejected);
    assert_eq!(
        rejected.rejection_code.as_deref(),
        Some("invalid_authority")
    );
    assert!(rejected.authority_claim_hash.is_none());
}

#[test]
fn delegated_authority_fails_closed_for_every_bound_claim_dimension() {
    let correct = authority_claim();
    let delegated = |command| InterpretationAuthority::Delegated {
        evidence: EvidenceRef::Command(command),
        authority_grant: DELEGATED_AUTHORITY_GRANT.to_owned(),
    };

    let wrong_producer = run_delegated_authority_case(
        60,
        &WRONG_AUTHORITY_PLUGIN,
        json!({"claim": correct.clone()}),
        delegated,
    );
    assert_eq!(wrong_producer.status, InformationOperationStatus::Rejected);

    let wrong_kind = run_delegated_authority_case(
        61,
        &AUTHORITY_PLUGIN,
        json!({"claim": correct.clone()}),
        |_| InterpretationAuthority::Delegated {
            evidence: EvidenceRef::Ingress(IngressId::new(1)),
            authority_grant: DELEGATED_AUTHORITY_GRANT.to_owned(),
        },
    );
    assert_eq!(wrong_kind.status, InformationOperationStatus::Rejected);

    let wrong_path = run_delegated_authority_case(
        62,
        &AUTHORITY_PLUGIN,
        json!({"not_claim": correct.clone()}),
        delegated,
    );
    assert_eq!(wrong_path.status, InformationOperationStatus::Rejected);

    let mut wrong_performer = correct.clone();
    wrong_performer.performed_by = EntityRef::Person(person(1));
    let wrong_performer = run_delegated_authority_case(
        63,
        &AUTHORITY_PLUGIN,
        json!({"claim": wrong_performer}),
        delegated,
    );
    assert_eq!(wrong_performer.status, InformationOperationStatus::Rejected);

    let mut wrong_holder = correct.clone();
    wrong_holder.performed_for = holder(2);
    let wrong_holder = run_delegated_authority_case(
        64,
        &AUTHORITY_PLUGIN,
        json!({"claim": wrong_holder}),
        delegated,
    );
    assert_eq!(wrong_holder.status, InformationOperationStatus::Rejected);

    let mut wrong_capability = correct.clone();
    wrong_capability.capabilities = vec!["different_capability".to_owned()];
    let wrong_capability = run_delegated_authority_case(
        65,
        &AUTHORITY_PLUGIN,
        json!({"claim": wrong_capability}),
        delegated,
    );
    assert_eq!(
        wrong_capability.status,
        InformationOperationStatus::Rejected
    );

    let mut wrong_time = correct.clone();
    wrong_time.expires_at = Some(SimTime::EPOCH);
    let wrong_time = run_delegated_authority_case(
        66,
        &AUTHORITY_PLUGIN,
        json!({"claim": wrong_time}),
        delegated,
    );
    assert_eq!(wrong_time.status, InformationOperationStatus::Rejected);

    let mut wrong_version = correct;
    wrong_version.format_version = 2;
    let wrong_version = run_delegated_authority_case(
        67,
        &AUTHORITY_PLUGIN,
        json!({"claim": wrong_version}),
        delegated,
    );
    assert_eq!(wrong_version.status, InformationOperationStatus::Rejected);

    for rejected in [
        wrong_producer,
        wrong_kind,
        wrong_path,
        wrong_performer,
        wrong_holder,
        wrong_capability,
        wrong_time,
        wrong_version,
    ] {
        assert_eq!(
            rejected.rejection_code.as_deref(),
            Some("invalid_authority")
        );
        assert!(rejected.authority_claim_hash.is_none());
        assert!(rejected.domain_result_refs.is_empty());
    }
}
