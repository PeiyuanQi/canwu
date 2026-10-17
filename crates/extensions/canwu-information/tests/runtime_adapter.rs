#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProposal,
    BoundaryRequest, BoundarySystemContract, Canwu, Command, CommandAttemptOutcome,
    CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu, CustodyState, DomainRecord,
    DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle, DomainRecordVersionRef,
    DomainRecordVersionSource, EntityRef, ErrorCode, EvidenceRef, Government, GovernmentId,
    IngressClass, IngressId, IngressPayload, Issuer, KnowledgeHolderRef, KnowledgeQuery,
    KnowledgeSnapshot, LifeState, MapPoint, PayloadSchema, Person, PersonAvailability, PersonId,
    PluginActionDescriptor, PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar,
    Scenario, SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    SystemDirective, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_api::{DomainReference, DomainReferenceTarget};
use canwu_information::{
    AUTHORITY_COMMAND_PRODUCER, AUTHORITY_COMMAND_TYPE, Access, AccessPayload, Audience,
    AudienceAccessEvidence, AudienceMembership, AudiencePayload, AuthorityAssignment,
    AuthorityAssignmentPayload, Channel, ChannelCapability, ChannelPayload, ClaimedSourceRef,
    Content, ContentPayload, ContentRelation, DELEGATED_AUTHORITY_GRANT, DelegatedAuthority,
    DelegationClaimV1, DelegationExpiry, DeliveryAttempt, DeliveryAttemptPayload,
    DeliveryAttemptStatus, Dispatch, DispatchPayload, DispatchStatus, DispatchTarget,
    INFORMATION_COMMAND, INFORMATION_INGRESS, INFORMATION_REJECTION_EVENT,
    INSTITUTIONAL_AUTHORITY_GRANT, InformationBody, InformationLimitsV1, InformationOperation,
    InformationOperationEnvelope, InformationOperationId, InformationOperationPayload,
    InformationOperationRecord, InformationOperationStatus, InformationOutputKind,
    InformationOutputSlot, InformationPlugin, Interpretation, InterpretationAuthority,
    InterpretationPayload, InterpretationStatus, LifecycleRequest, PLUGIN_NAME, RecordBinding,
    Release, ReleasePayload, ReleaseScope, ReleaseStatus, Representation, RepresentationPayload,
    audience_membership_root_v1, delegated_authority_reference, derive_operation_record_ref,
    derive_output_record_ref,
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
        entities: world(1).entities(),
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
    release_scenario_with_members(holder_count, (1..=holder_count).map(holder).collect())
}

/// A release fixture whose audience lists `members`, in a world of
/// `holder_count` persons.
fn release_scenario_with_members(
    holder_count: u64,
    mut member_holders: Vec<KnowledgeHolderRef>,
) -> ReleaseFixture {
    member_holders.sort();
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
            member_count: member_holders.len() as u64,
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
            entities: world(holder_count).entities(),
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
            entities: world(4).entities(),
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
                    authenticity: None,
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

const LIFE_PLUGIN: &str = "fixture-life";

/// Marks a person dead for every `die` packet it admits.
fn record_deaths(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, canwu_api::CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin, payload, ..
            } = &ingress.payload
            && plugin == LIFE_PLUGIN
        {
            directives.push(BoundaryDirective::SetPersonAvailability {
                person: PersonId::new(payload["person"].as_u64().expect("person")),
                availability: PersonAvailability::new(
                    LifeState::Dead,
                    CustodyState::Free,
                    view.time(),
                ),
                summary: "A person dies".to_owned(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct LifePlugin;

impl SimulationPlugin for LifePlugin {
    fn name(&self) -> &'static str {
        LIFE_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000c03"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), canwu_api::CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "die".to_owned(),
            description: "A person dies".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut contract = BoundarySystemContract::new(
            "record-deaths",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        contract.writes = vec![StateKey::core_person_availability()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, record_deaths)
    }
}

/// A dead person's ledger accepts no knowledge: a release to an audience
/// with a dead member publishes to the living members only, the operation
/// completes with their records, and the boundary never fails.
#[test]
fn a_release_to_an_audience_with_a_dead_member_publishes_to_the_living() {
    let fixture = release_scenario(3);
    let plugin = InformationPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&LifePlugin, &plugin];
    let mut canwu = Canwu::new_with_plugins(43, fixture.scenario.clone(), &plugins)
        .expect("release fixture should initialize");
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            LIFE_PLUGIN,
            "die",
            canwu.time(),
            json!({"person": 2}),
        ))
        .expect("queue death");
    settle(&mut canwu);
    let envelope = activate_release_operation(fixture.release.clone(), "with-dead-member");
    settle_to_terminal(&mut canwu, &envelope, 1);
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert_eq!(completed.publication_result_ids.len(), 2);
    assert_eq!(
        knowledge_schema_names(&canwu, 1),
        vec!["release_available".to_owned()]
    );
    assert!(knowledge_schema_names(&canwu, 2).is_empty());
    assert_eq!(
        knowledge_schema_names(&canwu, 3),
        vec!["release_available".to_owned()]
    );
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("the run replays exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// Host-authored operation packets that a boundary cannot use are refused
/// with a recorded event, and two operations whose plans change the same
/// record in one boundary do not make it invalid: the later one, in
/// operation-ID order, is planned again at the next boundary, against the
/// record the first one changed.
#[test]
fn unusable_or_conflicting_operations_in_one_boundary_do_not_fail_it() {
    let fixture = release_scenario(1);
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(44, fixture.scenario.clone(), &[&plugin])
        .expect("release fixture should initialize");
    let first = activate_release_operation(fixture.release.clone(), "first");
    let second = activate_release_operation(fixture.release.clone(), "second");
    let mut invalid = activate_release_operation(fixture.release.clone(), "invalid");
    invalid.operation_version = 2;
    let mut conflicting = first.clone();
    conflicting.operation = transition_release_operation(
        fixture.release.clone(),
        "first",
        1,
        ReleaseStatus::Withdrawn,
    )
    .operation;
    for envelope in [&first, &second, &invalid, &conflicting] {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                INFORMATION_INGRESS,
                canwu.time(),
                serde_json::to_value(envelope).expect("operation envelope should encode"),
            ))
            .expect("queue information operation");
    }
    for _ in 0..6 {
        settle(&mut canwu);
    }
    let rejections = canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, INFORMATION_REJECTION_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect::<Vec<_>>();
    assert_eq!(rejections.len(), 2, "{rejections:?}");
    assert!(
        rejections[0].contains("version must equal one"),
        "{rejections:?}"
    );
    assert!(rejections[1].contains("conflicting"), "{rejections:?}");
    assert!(
        canwu
            .typed_domain_record(&derive_operation_record_ref(&invalid.id))
            .is_none()
    );
    assert_eq!(
        operation_payload(&canwu, &first.id).status,
        InformationOperationStatus::Completed
    );
    let second = operation_payload(&canwu, &second.id);
    assert_eq!(second.status, InformationOperationStatus::Rejected);
    assert_eq!(second.rejection_code.as_deref(), Some("invalid_lifecycle"));
    let replayed = Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("the run replays exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
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
    let replayed = Canwu::replay_from_journal(&[&plugin], &journal)
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
    let replayed = Canwu::replay_from_journal(&replay_plugins, &canwu.replay_journal())
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

/// Queues one information operation command and settles enough boundaries to
/// finish it, on a full or a compact run.
macro_rules! settle_operation {
    ($run:expr, $request_id:expr, $envelope:expr) => {{
        let at = $run.time();
        let revision = $run.revision();
        $run.enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new($request_id),
                revision,
                plugin_command($envelope),
            ),
        )
        .expect("information command should enter canonical ingress");
        for _ in 0..5 {
            $run.settle_boundary(BoundaryRequest::at(at))
                .expect("information boundary should settle");
        }
    }};
}

/// A delegation is checked against the record the information plugin made
/// when the delegation command was admitted, never against the command, so an
/// interpretation citing a delegation whose command a seal removed is decided
/// as an unsealed fork, a restore from the sealed segments, and exact replay
/// decide it.
#[test]
fn delegated_interpretation_decides_alike_after_its_delegation_is_sealed() {
    let (scenario, access, representation, result_content) = interpretation_scenario();
    let plugin = InformationPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&AUTHORITY_PLUGIN, &plugin];
    let mut base = Canwu::new_with_plugins(46, scenario, &plugins)
        .expect("interpretation fixture should initialize");
    let authority_command = queue_authority_command(
        &mut base,
        1,
        AUTHORITY_COMMAND_PRODUCER,
        json!({"claim": authority_claim()}),
    );
    let ledger = base
        .typed_domain_record(&delegated_authority_reference(DELEGATED_AUTHORITY_GRANT))
        .expect("the admitted delegation is recorded")
        .decode_payload::<DelegatedAuthority>()
        .expect("the delegation ledger decodes");
    let recorded = ledger
        .delegation(authority_command)
        .expect("the ledger holds the delegation");
    assert_eq!(recorded.claim, authority_claim());
    assert_eq!(recorded.command, authority_command);
    settle(&mut base);

    let mut unsealed = base.fork();
    let mut sealed = base
        .into_compacted()
        .expect("interpretation case should enter compact mode");
    let segment = sealed
        .seal_evidence()
        .expect("the settled history seals")
        .expect("the settled history holds evidence");
    assert!(
        segment
            .commands
            .iter()
            .any(|command| command.id == authority_command),
        "the seal removes the delegation command"
    );
    let mut restored = CompactedCanwu::from_checkpoint_and_journal_with_plugins(
        sealed.checkpoint().expect("compact checkpoint"),
        vec![segment.clone()],
        &plugins,
    )
    .expect("the sealed segments restore the compact run");

    let envelope = record_interpretation_operation(
        access,
        representation,
        result_content,
        "delegated-after-seal",
        InterpretationAuthority::Delegated {
            evidence: EvidenceRef::Command(authority_command),
            authority_grant: DELEGATED_AUTHORITY_GRANT.to_owned(),
        },
    );
    settle_operation!(unsealed, 2, &envelope);
    settle_operation!(sealed, 2, &envelope);
    settle_operation!(restored, 2, &envelope);

    let completed = operation_payload(&unsealed, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert!(completed.authority_claim_hash.is_some());
    assert_eq!(completed.publication_result_ids.len(), 1);
    for run in [&sealed, &restored] {
        assert_eq!(
            run.typed_domain_record(&derive_operation_record_ref(&envelope.id))
                .expect("operation record")
                .decode_payload::<InformationOperationRecord>()
                .expect("operation payload"),
            completed
        );
    }
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());
    assert_eq!(restored.checkpoint_hash(), unsealed.checkpoint_hash());

    let journal = sealed
        .replay_journal_with_segments(vec![segment.clone()])
        .expect("the sealed run produces an exact replay journal");
    let replayed =
        Canwu::replay_from_journal(&plugins, &journal).expect("the sealed run replays exactly");
    assert_eq!(replayed.snapshot(), unsealed.snapshot());
    assert_eq!(
        replayed.snapshot(),
        sealed
            .snapshot_with_segments(vec![segment])
            .expect("the sealed run reconstructs a full snapshot")
    );
}

/// A delegation stays in its grant's ledger only while it can authorize: the
/// lifecycle system removes it at the first boundary it runs at or after the
/// claim's `expires_at`, without a wake of its own, and from that time the
/// delegation authorizes nothing, not even an interpretation dated inside its
/// window. Exact replay reproduces the run.
#[test]
fn an_expired_delegation_leaves_its_ledger_and_authorizes_nothing() {
    let (scenario, access, representation, result_content) = interpretation_scenario();
    let plugin = InformationPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&AUTHORITY_PLUGIN, &plugin];
    let mut canwu = Canwu::new_with_plugins(47, scenario, &plugins)
        .expect("interpretation fixture should initialize");
    let mut claim = authority_claim();
    claim.expires_at = Some(SimTime::from_minutes(10));
    let command = queue_authority_command(
        &mut canwu,
        1,
        AUTHORITY_COMMAND_PRODUCER,
        json!({ "claim": claim }),
    );
    let ledger = |canwu: &Canwu| {
        canwu
            .typed_domain_record(&delegated_authority_reference(DELEGATED_AUTHORITY_GRANT))
            .expect("the delegation ledger exists")
            .decode_payload::<DelegatedAuthority>()
            .expect("the delegation ledger decodes")
    };
    assert_eq!(
        ledger(&canwu).expiring,
        [DelegationExpiry {
            expires_at: SimTime::from_minutes(10),
            command,
        }]
    );
    let delegated = || InterpretationAuthority::Delegated {
        evidence: EvidenceRef::Command(command),
        authority_grant: DELEGATED_AUTHORITY_GRANT.to_owned(),
    };
    let in_force = record_interpretation_operation(
        access.clone(),
        representation.clone(),
        result_content.clone(),
        "in-force",
        delegated(),
    );
    settle_to_terminal(&mut canwu, &in_force, 2);
    assert_eq!(
        operation_payload(&canwu, &in_force.id).status,
        InformationOperationStatus::Completed
    );

    // From the expiry on, the delegation authorizes nothing, and the first
    // lifecycle boundary at or after it removes it from the ledger.
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::from_minutes(10)))
        .expect("a boundary at the expiry");
    let expired = record_interpretation_operation(
        access,
        representation,
        result_content,
        "expired",
        delegated(),
    );
    let at = canwu.time();
    let revision = canwu.revision();
    canwu
        .enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(3),
                revision,
                plugin_command(&expired).at_time(at),
            ),
        )
        .expect("information command should enter canonical ingress");
    for _ in 0..5 {
        settle(&mut canwu);
    }
    let expired = operation_payload(&canwu, &expired.id);
    assert_eq!(expired.status, InformationOperationStatus::Rejected);
    assert_eq!(expired.rejection_code.as_deref(), Some("invalid_authority"));
    let retired = ledger(&canwu);
    assert!(retired.delegations.is_empty() && retired.expiring.is_empty());

    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("the run replays exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
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

/// A release to an audience that names a holder the engine does not accept
/// (a territory holds no knowledge) publishes to the persons and leaves the
/// territory unpublished; the boundary never fails.
#[test]
fn a_release_to_ineligible_entity_holders_publishes_to_the_eligible() {
    let fixture = release_scenario_with_members(
        2,
        vec![
            holder(1),
            holder(2),
            KnowledgeHolderRef::Entity(EntityRef::Territory(territory())),
        ],
    );
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(46, fixture.scenario.clone(), &[&plugin])
        .expect("release fixture should initialize");
    let envelope = activate_release_operation(fixture.release.clone(), "ineligible-holders");
    settle_to_terminal(&mut canwu, &envelope, 1);
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert_eq!(completed.publication_result_ids.len(), 2);
    for index in [1, 2] {
        assert_eq!(
            knowledge_schema_names(&canwu, index),
            vec!["release_available".to_owned()]
        );
    }
    let replayed = Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("the run replays exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// Two deliveries of one representation to one holder in one boundary name
/// the same holder, knowledge kind, and subjects. The publication system
/// publishes that record once; the second operation relies on it, and both
/// operations record its ID.
#[test]
fn two_deliveries_of_one_representation_to_one_holder_publish_once() {
    let fixture = release_scenario(1);
    let mut scenario = fixture.scenario.clone();
    let channel = TypedDomainRecordRef::<Channel>::new("seed-channel");
    scenario.domain_records.push(initial_record(
        channel.clone(),
        &ChannelPayload {
            profile: "courier".to_owned(),
            capabilities: vec![ChannelCapability::AddressedDelivery],
        },
        Vec::new(),
    ));
    let deliver = |suffix: &str| {
        let attempt =
            TypedDomainRecordRef::<DeliveryAttempt>::new(format!("seed-attempt-{suffix}"));
        InformationOperationEnvelope {
            id: InformationOperationId::new(TEST_NAMESPACE, format!("deliver-{suffix}")),
            operation_version: 1,
            operation_kind: "transition_delivery_attempt".to_owned(),
            output_slots: Vec::new(),
            lineage: Vec::new(),
            operation: InformationOperation {
                request: LifecycleRequest::TransitionDeliveryAttempt {
                    record: attempt,
                    expected_version: 1,
                    proposed: DeliveryAttemptPayload {
                        status: DeliveryAttemptStatus::Delivered,
                        attempt_number: 1,
                        prepared_at: SimTime::EPOCH,
                        dispatched_at: Some(SimTime::EPOCH),
                        due_at: SimTime::from_minutes(10),
                        completed_at: Some(SimTime::EPOCH),
                    },
                },
            },
        }
    };
    for suffix in ["a", "b"] {
        let dispatch = TypedDomainRecordRef::<Dispatch>::new(format!("seed-dispatch-{suffix}"));
        scenario.domain_records.push(initial_record(
            dispatch.clone(),
            &DispatchPayload {
                status: DispatchStatus::Active,
                target: DispatchTarget::Addressed(vec![holder(1)]),
                prepared_at: SimTime::EPOCH,
                dispatched_at: Some(SimTime::EPOCH),
                completed_at: None,
            },
            vec![
                DomainReference::from_typed("channel", channel.clone()),
                DomainReference::from_typed("representation", fixture.representation.clone()),
                holder_reference("intended_recipient", &holder(1)),
            ],
        ));
        scenario.domain_records.push(initial_record(
            TypedDomainRecordRef::<DeliveryAttempt>::new(format!("seed-attempt-{suffix}")),
            &DeliveryAttemptPayload {
                status: DeliveryAttemptStatus::InTransit,
                attempt_number: 1,
                prepared_at: SimTime::EPOCH,
                dispatched_at: Some(SimTime::EPOCH),
                due_at: SimTime::from_minutes(10),
                completed_at: None,
            },
            vec![
                DomainReference::from_typed("dispatch", dispatch),
                holder_reference("recipient", &holder(1)),
            ],
        ));
    }
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(47, scenario, &[&plugin])
        .expect("delivery fixture should initialize");
    let first = deliver("a");
    let second = deliver("b");
    for envelope in [&first, &second] {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                INFORMATION_INGRESS,
                canwu.time(),
                serde_json::to_value(envelope).expect("operation envelope should encode"),
            ))
            .expect("queue delivery operation");
    }
    for _ in 0..6 {
        settle(&mut canwu);
    }
    let first = operation_payload(&canwu, &first.id);
    let second = operation_payload(&canwu, &second.id);
    assert_eq!(first.status, InformationOperationStatus::Completed);
    assert_eq!(second.status, InformationOperationStatus::Completed);
    assert_eq!(first.publication_result_ids.len(), 1);
    assert_eq!(first.publication_result_ids, second.publication_result_ids);
    assert_eq!(
        knowledge_schema_names(&canwu, 1),
        vec!["representation_available".to_owned()]
    );
    let replayed = Canwu::replay_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("the run replays exactly");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}

/// A host-authored packet that reuses an operation ID with other content, at
/// a higher priority than the operation's own progress packet, is refused;
/// the operation keeps its own input and completes.
#[test]
fn a_crafted_packet_for_a_persisted_operation_does_not_displace_its_progress() {
    let fixture = release_scenario(1);
    let plugin = InformationPlugin;
    let mut canwu = Canwu::new_with_plugins(48, fixture.scenario.clone(), &[&plugin])
        .expect("release fixture should initialize");
    let envelope = activate_release_operation(fixture.release.clone(), "own-progress");
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            INFORMATION_INGRESS,
            canwu.time(),
            serde_json::to_value(&envelope).expect("operation envelope should encode"),
        ))
        .expect("queue operation");
    settle(&mut canwu);
    assert_eq!(
        operation_payload(&canwu, &envelope.id).status,
        InformationOperationStatus::Accepted
    );
    // The operation's progress packet is queued; a crafted packet with the
    // same ID and other content outranks it at the next boundary.
    let mut crafted = envelope.clone();
    crafted.operation = transition_release_operation(
        fixture.release.clone(),
        "own-progress",
        1,
        ReleaseStatus::Withdrawn,
    )
    .operation;
    canwu
        .enqueue_plugin_ingress(
            PluginIngressRequest::new(
                PLUGIN_NAME,
                INFORMATION_INGRESS,
                canwu.time(),
                serde_json::to_value(&crafted).expect("crafted envelope should encode"),
            )
            .with_priority(1),
        )
        .expect("queue crafted operation");
    for _ in 0..6 {
        settle(&mut canwu);
    }
    let rejections = canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, INFORMATION_REJECTION_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect::<Vec<_>>();
    assert_eq!(rejections.len(), 1, "{rejections:?}");
    assert!(rejections[0].contains("conflicting"), "{rejections:?}");
    assert_eq!(
        operation_payload(&canwu, &envelope.id).status,
        InformationOperationStatus::Completed
    );
    assert_eq!(
        knowledge_schema_names(&canwu, 1),
        vec!["release_available".to_owned()]
    );
}

const HOG_PLUGIN: &str = "a-hog";

fn hog_schema() -> canwu_api::KnowledgeSchemaId {
    canwu_api::KnowledgeSchemaId::new(
        canwu_api::KnowledgeRecordKind::new("fixture.hog", "spent"),
        1,
    )
}

/// Spends every holder entry of the boundary's publication budget: eight
/// directives, each publishing 64 bodies, one per slot, to persons 1 to 64.
fn spend_the_budget(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, canwu_api::CanwuError> {
    let holders: Vec<_> = (1..=64).map(holder).collect();
    let mut budget = view.knowledge_publication_budget();
    let mut directives = Vec::new();
    for batch in 0..8_u64 {
        let records: Vec<_> = (0..64_u64)
            .map(|index| canwu_api::KnowledgeRecordDraft {
                schema: hog_schema(),
                subjects: vec![canwu_api::KnowledgeSubject {
                    role: "slot".to_owned(),
                    target: canwu_api::KnowledgeSubjectTarget::Event(canwu_api::EventId::new(
                        batch * 64 + index + 1,
                    )),
                }],
                payload: json!({ "batch": batch, "index": index }),
                as_of: None,
                confidence_per_mille: 1_000,
                origin: canwu_api::KnowledgeOrigin {
                    method: "fixture.hog".to_owned(),
                    evidence: Vec::new(),
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            })
            .collect();
        let mut payload_bytes = 0;
        for record in &records {
            payload_bytes += record.payload_bytes()?;
        }
        assert!(
            budget.take(64, 4_096, payload_bytes),
            "each of the hog's directives fits what is left"
        );
        directives.push(BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: canwu_api::KnowledgeSupersession::CurrentOnSubjects,
            groups: vec![canwu_api::KnowledgePublicationGroup {
                holders: holders.clone(),
                records,
            }],
            summary: "Spend the publication budget".to_owned(),
        });
    }
    assert!(!budget.admits(1, 1, 0), "no holder entry is left");
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// A plugin whose name sorts before `canwu-information` and whose Daily
/// phase-13 system runs before the information publisher and spends the
/// boundary's whole holder-entry budget.
struct HogPlugin;

impl SimulationPlugin for HogPlugin {
    fn name(&self) -> &'static str {
        HOG_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000c04"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), canwu_api::CanwuError> {
        registrar.register_knowledge_schema(canwu_api::PluginKnowledgeSchema {
            id: hog_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000000c05"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![canwu_api::KnowledgeSubjectSchema {
                role: "slot".to_owned(),
                targets: vec![canwu_api::KnowledgeSubjectTargetKind::Event],
                required: true,
                multiple: false,
            }],
            retention: canwu_api::KnowledgeRetention::Keep,
        })?;
        let mut contract = BoundarySystemContract::new(
            "spend-the-budget",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        contract.reads = vec![StateKey::core_knowledge()];
        contract.knowledge_writes = vec![canwu_api::KnowledgeWriteGrant {
            schema: hog_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, spend_the_budget)
    }
}

fn information_changes(boundary: &canwu_api::BoundaryRecord) -> usize {
    boundary
        .knowledge_changes
        .iter()
        .filter(|change| change.plugin == PLUGIN_NAME)
        .count()
}

/// A publisher that sorts before `canwu-information` spends the boundary's
/// whole publication budget, so not even the first record of the release's
/// chunk fits: the information publisher defers the whole chunk, the
/// boundary commits, the chunk's finalization returns the operation to
/// publication, and the operation completes with every record at later
/// boundaries.
#[test]
fn a_chunk_whose_first_record_does_not_fit_the_budget_is_deferred_whole() {
    let fixture = release_scenario_with_members(64, vec![holder(1), holder(2)]);
    let information = InformationPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&HogPlugin, &information];
    let envelope = activate_release_operation(fixture.release.clone(), "deferred");
    // A twin run without the hog finds the boundary that publishes.
    let publishing = {
        let mut twin = Canwu::new_with_plugins(44, fixture.scenario.clone(), &plugins)
            .expect("release fixture should initialize");
        queue(&mut twin, 1, &envelope);
        (1..=8)
            .find(|_| {
                settle(&mut twin);
                information_changes(twin.boundaries().last().expect("a boundary")) > 0
            })
            .expect("the release publishes within eight boundaries")
    };

    let mut canwu = Canwu::new_with_plugins(44, fixture.scenario.clone(), &plugins)
        .expect("release fixture should initialize");
    queue(&mut canwu, 1, &envelope);
    for _ in 1..publishing {
        settle(&mut canwu);
    }
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()).with_cadence(SystemCadence::Daily))
        .expect("a boundary whose chunk is deferred commits");
    let boundary = canwu.boundaries().last().expect("the deferring boundary");
    assert_eq!(
        boundary
            .knowledge_changes
            .iter()
            .filter(|change| change.plugin == HOG_PLUGIN)
            .map(canwu_api::BoundaryKnowledgeChange::entry_count)
            .sum::<usize>(),
        32_768
    );
    assert_eq!(information_changes(boundary), 0);
    let deferred = operation_payload(&canwu, &envelope.id);
    assert_eq!(
        deferred.status,
        InformationOperationStatus::AwaitingFinalization
    );
    let continuation = deferred
        .continuation
        .as_ref()
        .expect("the whole chunk is left");
    assert_eq!((continuation.cursor, continuation.remaining), (0, 2));
    assert!(deferred.publication_result_ids.is_empty());

    // The empty chunk's finalization returns the operation to publication,
    // and the same boundary publishes the chunk.
    settle(&mut canwu);
    assert_eq!(
        information_changes(canwu.boundaries().last().expect("a boundary")),
        2
    );
    let published = operation_payload(&canwu, &envelope.id);
    assert_eq!(
        published.status,
        InformationOperationStatus::AwaitingFinalization
    );
    assert!(published.continuation.is_none());

    settle(&mut canwu);
    let completed = operation_payload(&canwu, &envelope.id);
    assert_eq!(completed.status, InformationOperationStatus::Completed);
    assert_eq!(completed.publication_result_ids.len(), 2);
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())
        .expect("the run replays exactly");
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
}
