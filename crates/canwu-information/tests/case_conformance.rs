#![allow(clippy::too_many_lines)]

mod support;

use canwu_api::{
    BoundaryId, DomainRecordKind, DomainRecordRef, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainReference, EvidenceRef, HolderKnowledgeRecordId, IngressId,
    KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeQuery, KnowledgeRecord,
    KnowledgeRecordId, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSnapshot,
    TypedDomainRecordRef,
};
use canwu_information::{
    Access, AccessPayload, Audience, AudienceMembership, AudiencePayload, Channel,
    ChannelCapability, ChannelPayload, ClaimedSourceRef, Content, ContentDerivation,
    ContentPayload, ContentRelation, ContentSourceEdge, ContentSourceRole, DeliveryAttempt,
    DeliveryAttemptPayload, DeliveryAttemptStatus, Dispatch, DispatchPayload, DispatchStatus,
    DispatchTarget, GenericInformationPublicationDraft, InformationAdmissionRef, InformationBody,
    InformationContinuation, InformationLimitsV1, InformationOperationId,
    InformationOperationPayload, InformationOperationStatus, InformationQuery, Instance,
    InstancePayload, InstanceStatus, Interpretation, InterpretationAuthority,
    InterpretationPayload, InterpretationStatus, LifecycleRequest, RecordBinding, Release,
    ReleasePayload, ReleaseScope, ReleaseStatus, Representation, RepresentationPayload,
    RepresentationSourceEdge, audience_membership_root_v1, neutral_knowledge_schemas,
    validate_operation_transition,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use support::{
    DetachedLedger, assert_stable_profile_id, holder_reference, institution, minute, person,
};

const INSTITUTIONAL_HOLDER: &str = "fixture.information.institutional-holder";
const DELEGATED_INTERPRETATION: &str = "fixture.information.delegated-interpretation";
const MULTI_HOP_RELAY: &str = "fixture.information.multi-hop-relay";
const CLAIMED_SOURCE_DIVERGENCE: &str = "fixture.information.claimed-source-divergence";
const CONTRADICTORY_MULTI_SOURCE: &str = "fixture.information.contradictory-multi-source-history";
const PERSISTED_TEN_THOUSAND_FANOUT: &str = "fixture.information.persisted-ten-thousand-fanout";
const INDEPENDENT_SHARED_CACHE_CHANNEL: &str =
    "fixture.information.independent-shared-cache-channel";

fn knowledge_record(
    holder: KnowledgeHolderRef,
    id: u64,
    schema_name: &str,
    learned_at: i64,
    payload: Value,
    supersedes: Vec<KnowledgeRecordId>,
    evidence: EvidenceRef,
) -> KnowledgeRecord {
    KnowledgeRecord {
        id: KnowledgeRecordId::new(id),
        holder,
        schema: KnowledgeSchemaId::new(
            KnowledgeRecordKind::new("fixture.information", schema_name),
            1,
        ),
        subjects: Vec::new(),
        payload,
        as_of: None,
        learned_at: minute(learned_at),
        confidence_per_mille: 900,
        origin: KnowledgeOrigin {
            method: "fixture_observation".to_owned(),
            evidence: vec![evidence],
        },
        supersedes,
        contradicts: Vec::new(),
    }
}

fn seed_representation(
    ledger: &mut DetachedLedger,
    suffix: &str,
) -> Result<
    (
        TypedDomainRecordRef<Content>,
        TypedDomainRecordRef<Representation>,
    ),
    String,
> {
    let content = TypedDomainRecordRef::<Content>::new(format!("content-{suffix}"));
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(content.clone(), Vec::new()),
        payload: ContentPayload {
            content_type: "synthetic_code".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"content_code": format!("SYN-{suffix}")}),
            },
            created_at: minute(0),
            derivation: None,
        },
    })?;
    let representation =
        TypedDomainRecordRef::<Representation>::new(format!("representation-{suffix}"));
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            representation.clone(),
            vec![DomainReference::from_typed("content", content.clone())],
        ),
        payload: RepresentationPayload {
            format: "neutral_signal_v1".to_owned(),
            created_at: minute(1),
            operation: "render".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: Some("decode_fixture".to_owned()),
        },
    })?;
    Ok((content, representation))
}

#[test]
fn fixture_information_institutional_holder() -> Result<(), String> {
    assert_stable_profile_id(INSTITUTIONAL_HOLDER);
    let holder = institution("institution-01");
    let predecessor = person(5_001);
    let successor = person(5_002);
    let record = knowledge_record(
        holder.clone(),
        1,
        "institution_fact",
        4,
        json!({"claim_code": "SYN-IH-01"}),
        Vec::new(),
        EvidenceRef::Boundary(BoundaryId::new(1)),
    );
    let snapshot = KnowledgeSnapshot {
        actors: BTreeMap::new(),
        records: BTreeMap::from([(holder.clone(), BTreeMap::from([(record.id, record)]))]),
    };

    let before = snapshot
        .query_current(holder.clone(), &KnowledgeQuery::default(), None)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(before.records.len(), 1);
    assert!(
        snapshot
            .query_current(predecessor.clone(), &KnowledgeQuery::default(), None)
            .map_err(|error| format!("{error:?}"))?
            .records
            .is_empty()
    );

    let seat_holder_before = (predecessor, holder.clone());
    let seat_holder_after = (successor.clone(), holder.clone());
    assert_ne!(seat_holder_before.0, seat_holder_after.0);
    assert_eq!(seat_holder_before.1, seat_holder_after.1);

    let after = snapshot
        .query_current(seat_holder_after.1, &KnowledgeQuery::default(), None)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(after.records, before.records);
    assert!(
        snapshot
            .query_current(successor, &KnowledgeQuery::default(), None)
            .map_err(|error| format!("{error:?}"))?
            .records
            .is_empty()
    );
    Ok(())
}

#[test]
fn fixture_information_delegated_interpretation() -> Result<(), String> {
    assert_stable_profile_id(DELEGATED_INTERPRETATION);
    let mut ledger = DetachedLedger::default();
    let (source_content, representation) = seed_representation(&mut ledger, "DI-02")?;
    let collector = person(6_001);
    let performer = person(6_002);
    let commissioning_holder = institution("institution-02");

    let instance = TypedDomainRecordRef::<Instance>::new("instance-collected");
    ledger.plan_and_apply(&LifecycleRequest::CreateInstance {
        binding: RecordBinding::new(
            instance.clone(),
            vec![
                holder_reference("custodian", &collector),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
        payload: InstancePayload {
            created_at: minute(2),
            status: InstanceStatus::Available,
        },
    })?;

    let access = TypedDomainRecordRef::<Access>::new("access-commissioning-holder");
    let access_plan = ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            access.clone(),
            vec![
                holder_reference("holder", &commissioning_holder),
                DomainReference::from_typed("instance", instance),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(3),
            method: "collected_for_holder".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: None,
    })?;
    assert!(matches!(
        access_plan.publications.as_slice(),
        [GenericInformationPublicationDraft::AccessRecorded { holder, .. }]
            if holder == &commissioning_holder
    ));

    let result_content = TypedDomainRecordRef::<Content>::new("content-interpretation-result");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(
            result_content.clone(),
            vec![DomainReference::from_typed(
                "source_content",
                source_content.clone(),
            )],
        ),
        payload: ContentPayload {
            content_type: "synthetic_result".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"content_code": "SYN-DI-02-R"}),
            },
            created_at: minute(4),
            derivation: Some(ContentDerivation {
                operation: "interpret".to_owned(),
                sources: vec![ContentSourceEdge {
                    source: source_content,
                    role: ContentSourceRole::Contribution,
                    completeness_per_mille: 1_000,
                    fidelity_per_mille: 1_000,
                }],
            }),
        },
    })?;

    let interpretation = TypedDomainRecordRef::<Interpretation>::new("interpretation-delegated");
    let interpretation_plan = ledger.plan_and_apply(&LifecycleRequest::RecordInterpretation {
        binding: RecordBinding::new(
            interpretation.clone(),
            vec![
                DomainReference::from_typed("input_access", access),
                DomainReference::from_typed("input_representation", representation),
                holder_reference("performed_by", &performer),
                holder_reference("performed_for", &commissioning_holder),
                DomainReference::from_typed("result_content", result_content),
            ],
        ),
        payload: InterpretationPayload {
            interpreted_at: minute(5),
            status: InterpretationStatus::Succeeded,
            capability: "decode_fixture".to_owned(),
            confidence_per_mille: 950,
        },
        authority: InterpretationAuthority::InstitutionalRole {
            assignment: DomainRecordVersionRef {
                record: DomainRecordRef::new("fixture.information", "assignment", "assignment-01"),
                version: 1,
                established_by: DomainRecordVersionSource::InitialScenario,
            },
            authority_grant: "interpret_for_holder".to_owned(),
        },
    })?;
    assert!(matches!(
        interpretation_plan.publications.as_slice(),
        [GenericInformationPublicationDraft::InterpretationRecorded { holder, .. }]
            if holder == &commissioning_holder
    ));
    assert_ne!(collector, performer);
    assert_ne!(collector, commissioning_holder);
    assert_ne!(performer, commissioning_holder);
    assert!(interpretation_plan.publications.iter().all(|publication| {
        matches!(
            publication,
            GenericInformationPublicationDraft::InterpretationRecorded { holder, .. }
                if holder == &commissioning_holder
        )
    }));
    assert_eq!(ledger.record(interpretation.as_untyped()).version, 1);
    Ok(())
}

#[test]
fn fixture_information_multi_hop_relay() -> Result<(), String> {
    assert_stable_profile_id(MULTI_HOP_RELAY);
    let mut ledger = DetachedLedger::default();
    let (content, original_representation) = seed_representation(&mut ledger, "MHR-03")?;
    let source_holder = person(7_001);
    let relay_holder = person(7_002);
    let destination_holder = person(7_003);
    let channel = TypedDomainRecordRef::<Channel>::new("channel-multi-hop");
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "persistent_relay".to_owned(),
            capabilities: vec![
                ChannelCapability::PersistentInstance,
                ChannelCapability::AddressedDelivery,
            ],
        },
    })?;
    let original_instance = TypedDomainRecordRef::<Instance>::new("instance-original");
    ledger.plan_and_apply(&LifecycleRequest::CreateInstance {
        binding: RecordBinding::new(
            original_instance.clone(),
            vec![
                holder_reference("custodian", &source_holder),
                DomainReference::from_typed("representation", original_representation.clone()),
            ],
        ),
        payload: InstancePayload {
            created_at: minute(2),
            status: InstanceStatus::Available,
        },
    })?;

    let first_dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-hop-1");
    let first_prepared = begin_active_dispatch(
        &mut ledger,
        &first_dispatch,
        &channel,
        &original_representation,
        Some(&original_instance),
        &relay_holder,
        3,
    )?;
    let first_attempt = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-hop-1-1");
    begin_attempt(
        &mut ledger,
        &first_attempt,
        &first_dispatch,
        &relay_holder,
        1,
        None,
        4,
        10,
    )?;
    finish_attempt(
        &mut ledger,
        &first_attempt,
        1,
        1,
        DeliveryAttemptStatus::Delivered,
        4,
        5,
        Some(6),
        10,
    )?;
    complete_dispatch(&mut ledger, &first_dispatch, first_prepared, 3, 7)?;

    let copied_representation =
        TypedDomainRecordRef::<Representation>::new("representation-hop-copy");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            copied_representation.clone(),
            vec![
                DomainReference::from_typed("content", content),
                DomainReference::from_typed(
                    "parent_representation",
                    original_representation.clone(),
                ),
                holder_reference("producer", &relay_holder),
            ],
        ),
        payload: RepresentationPayload {
            format: "relay_copy_v1".to_owned(),
            created_at: minute(7),
            operation: "copy".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: vec![RepresentationSourceEdge {
                parent: original_representation.clone(),
                completeness_per_mille: 1_000,
                fidelity_per_mille: 990,
            }],
            claimed_source: None,
            interpretation_capability: Some("decode_fixture".to_owned()),
        },
    })?;
    let copied_instance = TypedDomainRecordRef::<Instance>::new("instance-hop-copy");
    ledger.plan_and_apply(&LifecycleRequest::CreateInstance {
        binding: RecordBinding::new(
            copied_instance.clone(),
            vec![
                holder_reference("custodian", &relay_holder),
                DomainReference::from_typed("representation", copied_representation.clone()),
            ],
        ),
        payload: InstancePayload {
            created_at: minute(8),
            status: InstanceStatus::Available,
        },
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionInstance {
        record: original_instance.clone(),
        expected_version: 1,
        status: InstanceStatus::Destroyed,
        custodian: None,
        location: None,
    })?;

    let second_dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-hop-2");
    let second_prepared = begin_active_dispatch(
        &mut ledger,
        &second_dispatch,
        &channel,
        &copied_representation,
        Some(&copied_instance),
        &destination_holder,
        9,
    )?;
    let second_attempt = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-hop-2-1");
    begin_attempt(
        &mut ledger,
        &second_attempt,
        &second_dispatch,
        &destination_holder,
        1,
        None,
        10,
        15,
    )?;
    finish_attempt(
        &mut ledger,
        &second_attempt,
        1,
        1,
        DeliveryAttemptStatus::Failed,
        10,
        11,
        Some(12),
        15,
    )?;
    let retry = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-hop-2-2");
    begin_attempt(
        &mut ledger,
        &retry,
        &second_dispatch,
        &destination_holder,
        2,
        Some(second_attempt),
        13,
        20,
    )?;
    finish_attempt(
        &mut ledger,
        &retry,
        1,
        2,
        DeliveryAttemptStatus::Delivered,
        13,
        14,
        Some(16),
        20,
    )?;
    complete_dispatch(&mut ledger, &second_dispatch, second_prepared, 9, 17)?;

    let records = ledger.record_set()?;
    assert_eq!(
        records.decode::<Instance>(&original_instance)?.status,
        InstanceStatus::Destroyed
    );
    assert_eq!(
        records.decode::<Instance>(&copied_instance)?.status,
        InstanceStatus::Available
    );
    let copied = records.decode::<Representation>(&copied_representation)?;
    assert_eq!(copied.sources[0].parent, original_representation);
    assert_eq!(
        records
            .query(&InformationQuery {
                kinds: vec![DomainRecordKind::for_type::<Dispatch>()],
                active_only: true,
                reference_role: None,
                reference_target: None,
            })?
            .len(),
        2
    );
    assert_eq!(
        records
            .query(&InformationQuery {
                kinds: vec![DomainRecordKind::for_type::<DeliveryAttempt>()],
                active_only: true,
                reference_role: None,
                reference_target: None,
            })?
            .len(),
        3
    );
    assert_eq!(ledger.history(first_dispatch.as_untyped()).len(), 3);
    assert_eq!(ledger.history(second_dispatch.as_untyped()).len(), 3);

    let history_holder = destination_holder;
    let first_fact = knowledge_record(
        history_holder.clone(),
        20,
        "relay_state",
        16,
        json!({"state_code": "SYN-MHR-PREV"}),
        Vec::new(),
        EvidenceRef::Boundary(BoundaryId::new(20)),
    );
    let second_fact = knowledge_record(
        history_holder.clone(),
        21,
        "relay_state",
        17,
        json!({"state_code": "SYN-MHR-CURRENT"}),
        vec![first_fact.id],
        EvidenceRef::Boundary(BoundaryId::new(21)),
    );
    let snapshot = KnowledgeSnapshot {
        actors: BTreeMap::new(),
        records: BTreeMap::from([(
            history_holder.clone(),
            BTreeMap::from([(first_fact.id, first_fact), (second_fact.id, second_fact)]),
        )]),
    };
    let current = snapshot
        .query_current(history_holder.clone(), &KnowledgeQuery::default(), None)
        .map_err(|error| format!("{error:?}"))?;
    let full = snapshot
        .query_current(
            history_holder,
            &KnowledgeQuery {
                view: KnowledgeHistoryView::FullHistory,
                ..KnowledgeQuery::default()
            },
            None,
        )
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(current.records.len(), 1);
    assert_eq!(full.records.len(), 2);
    Ok(())
}

#[test]
fn fixture_information_claimed_source_divergence_separates_claim_from_audit_origin()
-> Result<(), String> {
    assert_stable_profile_id(CLAIMED_SOURCE_DIVERGENCE);
    let actual_producer = person(8_000);
    let representation_payload = RepresentationPayload {
        format: "neutral_claim_carrier_v1".to_owned(),
        created_at: minute(1),
        operation: "render_claim".to_owned(),
        content_relation: ContentRelation::SameContent,
        sources: Vec::new(),
        claimed_source: Some(ClaimedSourceRef {
            namespace: "synthetic-source".to_owned(),
            value: "source-a".to_owned(),
        }),
        interpretation_capability: None,
    };
    let representation_wire =
        serde_json::to_value(&representation_payload).map_err(|error| error.to_string())?;
    assert_eq!(
        representation_wire["claimed_source"],
        json!({"namespace": "synthetic-source", "value": "source-a"})
    );
    assert!(
        neutral_knowledge_schemas()
            .iter()
            .all(|schema| !schema.name.contains("audit_origin"))
    );

    let mut ledger = DetachedLedger::default();
    let content = TypedDomainRecordRef::<Content>::new("content-claimed-source");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(content.clone(), Vec::new()),
        payload: ContentPayload {
            content_type: "synthetic_claim".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"code": "SYN-CLAIM"}),
            },
            created_at: minute(0),
            derivation: None,
        },
    })?;
    let representation = TypedDomainRecordRef::<Representation>::new("representation-claim");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            representation.clone(),
            vec![
                DomainReference::from_typed("content", content),
                holder_reference("producer", &actual_producer),
            ],
        ),
        payload: representation_payload.clone(),
    })?;
    let persisted = ledger.record(representation.as_untyped());
    assert_eq!(
        persisted
            .decode_payload::<Representation>()
            .map_err(|error| format!("{error:?}"))?
            .claimed_source,
        representation_payload.claimed_source
    );

    let accepting_holder = person(8_001);
    let disputing_holder = person(8_002);
    let accepted = knowledge_record(
        accepting_holder.clone(),
        30,
        "source_claim",
        2,
        json!({"claim_code": "SYN-SRC-A"}),
        Vec::new(),
        EvidenceRef::Boundary(BoundaryId::new(30)),
    );
    let disputed = knowledge_record(
        disputing_holder.clone(),
        31,
        "source_attribution",
        3,
        json!({"claim_code": "SYN-SRC-B", "contradicts_code": "SYN-SRC-A"}),
        Vec::new(),
        EvidenceRef::Boundary(BoundaryId::new(31)),
    );
    let snapshot = KnowledgeSnapshot {
        actors: BTreeMap::new(),
        records: BTreeMap::from([
            (
                accepting_holder.clone(),
                BTreeMap::from([(accepted.id, accepted)]),
            ),
            (
                disputing_holder.clone(),
                BTreeMap::from([(disputed.id, disputed)]),
            ),
        ]),
    };
    let accepted_view = snapshot
        .query_current(accepting_holder, &KnowledgeQuery::default(), None)
        .map_err(|error| format!("{error:?}"))?;
    let disputed_view = snapshot
        .query_current(disputing_holder, &KnowledgeQuery::default(), None)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(accepted_view.records[0].payload["claim_code"], "SYN-SRC-A");
    assert_eq!(disputed_view.records[0].payload["claim_code"], "SYN-SRC-B");
    assert!(
        serde_json::to_value(&accepted_view.records[0])
            .map_err(|error| error.to_string())?
            .get("origin")
            .is_none()
    );
    assert!(
        snapshot.records[&person(8_001)][&KnowledgeRecordId::new(30)]
            .origin
            .evidence
            .contains(&EvidenceRef::Boundary(BoundaryId::new(30)))
    );
    Ok(())
}

fn begin_active_dispatch(
    ledger: &mut DetachedLedger,
    dispatch: &TypedDomainRecordRef<Dispatch>,
    channel: &TypedDomainRecordRef<Channel>,
    representation: &TypedDomainRecordRef<Representation>,
    source_instance: Option<&TypedDomainRecordRef<Instance>>,
    recipient: &KnowledgeHolderRef,
    prepared_at: i64,
) -> Result<DispatchPayload, String> {
    let prepared = DispatchPayload {
        status: DispatchStatus::Prepared,
        target: DispatchTarget::Addressed(vec![recipient.clone()]),
        prepared_at: minute(prepared_at),
        dispatched_at: None,
        completed_at: None,
    };
    let mut references = vec![
        DomainReference::from_typed("channel", channel.clone()),
        holder_reference("intended_recipient", recipient),
        DomainReference::from_typed("representation", representation.clone()),
    ];
    if let Some(instance) = source_instance {
        references.push(DomainReference::from_typed(
            "source_instance",
            instance.clone(),
        ));
    }
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(dispatch.clone(), references),
        payload: prepared.clone(),
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch.clone(),
        expected_version: 1,
        proposed: DispatchPayload {
            status: DispatchStatus::Active,
            dispatched_at: Some(minute(prepared_at + 1)),
            ..prepared.clone()
        },
    })?;
    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
fn begin_attempt(
    ledger: &mut DetachedLedger,
    attempt: &TypedDomainRecordRef<DeliveryAttempt>,
    dispatch: &TypedDomainRecordRef<Dispatch>,
    recipient: &KnowledgeHolderRef,
    number: u32,
    previous: Option<TypedDomainRecordRef<DeliveryAttempt>>,
    prepared_at: i64,
    due_at: i64,
) -> Result<(), String> {
    let mut references = vec![
        DomainReference::from_typed("dispatch", dispatch.clone()),
        holder_reference("recipient", recipient),
    ];
    if let Some(previous) = previous {
        references.push(DomainReference::from_typed("previous_attempt", previous));
    }
    ledger.plan_and_apply(&LifecycleRequest::BeginDeliveryAttempt {
        binding: RecordBinding::new(attempt.clone(), references),
        payload: DeliveryAttemptPayload {
            status: DeliveryAttemptStatus::Prepared,
            attempt_number: number,
            prepared_at: minute(prepared_at),
            dispatched_at: None,
            due_at: minute(due_at),
            completed_at: None,
        },
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_attempt(
    ledger: &mut DetachedLedger,
    attempt: &TypedDomainRecordRef<DeliveryAttempt>,
    expected_version: u64,
    number: u32,
    status: DeliveryAttemptStatus,
    prepared_at: i64,
    dispatched_at: i64,
    completed_at: Option<i64>,
    due_at: i64,
) -> Result<(), String> {
    ledger.plan_and_apply(&LifecycleRequest::TransitionDeliveryAttempt {
        record: attempt.clone(),
        expected_version,
        proposed: DeliveryAttemptPayload {
            status,
            attempt_number: number,
            prepared_at: minute(prepared_at),
            dispatched_at: Some(minute(dispatched_at)),
            due_at: minute(due_at),
            completed_at: completed_at.map(minute),
        },
    })?;
    Ok(())
}

fn complete_dispatch(
    ledger: &mut DetachedLedger,
    dispatch: &TypedDomainRecordRef<Dispatch>,
    prepared: DispatchPayload,
    dispatched_at: i64,
    completed_at: i64,
) -> Result<(), String> {
    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch.clone(),
        expected_version: 2,
        proposed: DispatchPayload {
            status: DispatchStatus::Completed,
            dispatched_at: Some(minute(dispatched_at + 1)),
            completed_at: Some(minute(completed_at)),
            ..prepared
        },
    })?;
    Ok(())
}

#[test]
fn fixture_information_contradictory_multi_source_history_views() -> Result<(), String> {
    assert_stable_profile_id(CONTRADICTORY_MULTI_SOURCE);
    let holder = person(9_001);
    let first = knowledge_record(
        holder.clone(),
        101,
        "multi_source_claim",
        1,
        json!({"claim_code": "SYN-MS-A", "source_code": "SYN-SOURCE-1"}),
        Vec::new(),
        EvidenceRef::Boundary(BoundaryId::new(101)),
    );
    let mut second = knowledge_record(
        holder.clone(),
        102,
        "multi_source_claim",
        2,
        json!({"claim_code": "SYN-MS-B", "source_code": "SYN-SOURCE-2"}),
        Vec::new(),
        EvidenceRef::Boundary(BoundaryId::new(102)),
    );
    second.origin.method = "fixture_independent_observation".to_owned();
    second.contradicts = vec![first.id];
    let mut synthesis = knowledge_record(
        holder.clone(),
        103,
        "multi_source_claim",
        3,
        json!({"claim_code": "SYN-MS-C", "source_code": "SYN-SOURCE-3"}),
        vec![first.id, second.id],
        EvidenceRef::Boundary(BoundaryId::new(103)),
    );
    synthesis.origin.method = "fixture_reconciled_observation".to_owned();
    let first_id = first.id;
    let second_id = second.id;
    let synthesis_id = synthesis.id;
    let snapshot = KnowledgeSnapshot {
        actors: BTreeMap::new(),
        records: BTreeMap::from([(
            holder.clone(),
            BTreeMap::from([
                (first.id, first),
                (second.id, second),
                (synthesis.id, synthesis),
            ]),
        )]),
    };

    let current = snapshot
        .query_current(holder.clone(), &KnowledgeQuery::default(), None)
        .map_err(|error| format!("{error:?}"))?;
    let full = snapshot
        .query_current(
            holder.clone(),
            &KnowledgeQuery {
                view: KnowledgeHistoryView::FullHistory,
                ..KnowledgeQuery::default()
            },
            None,
        )
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(current.records.len(), 1);
    assert_eq!(current.records[0].id, HolderKnowledgeRecordId::new(3));
    assert_eq!(current.records[0].payload["claim_code"], "SYN-MS-C");
    assert_eq!(full.records.len(), 3);
    assert_eq!(
        full.records
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec![
            HolderKnowledgeRecordId::new(1),
            HolderKnowledgeRecordId::new(2),
            HolderKnowledgeRecordId::new(3),
        ]
    );
    assert_eq!(
        full.records[1].contradicts,
        vec![HolderKnowledgeRecordId::new(1)]
    );
    assert_eq!(full.records[0].payload["source_code"], "SYN-SOURCE-1");
    assert_eq!(full.records[1].payload["source_code"], "SYN-SOURCE-2");
    assert_eq!(full.records[2].payload["source_code"], "SYN-SOURCE-3");
    let stored = &snapshot.records[&holder];
    assert_ne!(
        stored[&first_id].origin.evidence,
        stored[&second_id].origin.evidence
    );
    assert_ne!(
        stored[&first_id].origin.method,
        stored[&second_id].origin.method
    );
    assert_eq!(stored[&synthesis_id].supersedes, vec![first_id, second_id]);
    Ok(())
}

#[test]
fn fixture_information_persists_ten_thousand_fanout_continuation_chunks() -> Result<(), String> {
    const MEMBER_COUNT: u64 = 10_000;
    const CHUNK_SIZE: u32 = 64;
    assert_stable_profile_id(PERSISTED_TEN_THOUSAND_FANOUT);
    let member_count = usize::try_from(MEMBER_COUNT)
        .map_err(|_| "fan-out member count is not representable".to_owned())?;
    let mut ledger = DetachedLedger::default();
    let (_, representation) = seed_representation(&mut ledger, "TF-10K")?;
    let audience = TypedDomainRecordRef::<Audience>::new("audience-ten-thousand");
    let member_holders = (0..MEMBER_COUNT)
        .map(|index| person(10_000 + index))
        .collect::<Vec<_>>();
    let members = member_holders
        .iter()
        .map(|member| holder_reference("member", member))
        .collect();
    let membership_root =
        audience_membership_root_v1(&member_holders, InformationLimitsV1::canonical())?;
    ledger.plan_and_apply(&LifecycleRequest::CreateAudience {
        binding: RecordBinding::new(audience.clone(), members),
        payload: AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            resolved_at: minute(1),
            resolution_version: 1,
            resolved_boundary: None,
            member_count: MEMBER_COUNT,
            membership_root,
        },
    })?;
    let release = TypedDomainRecordRef::<Release>::new("release-ten-thousand");
    let prepared = ReleasePayload {
        status: ReleaseStatus::Prepared,
        scope: ReleaseScope::Audience,
        prepared_at: minute(2),
        active_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::CreateRelease {
        binding: RecordBinding::new(
            release.clone(),
            vec![
                DomainReference::from_typed("audience", audience),
                DomainReference::from_typed("representation", representation),
            ],
        ),
        payload: prepared.clone(),
    })?;
    let plan = ledger.plan_and_apply(&LifecycleRequest::TransitionRelease {
        record: release.clone(),
        expected_version: 1,
        proposed: ReleasePayload {
            status: ReleaseStatus::Active,
            active_at: Some(minute(3)),
            ..prepared
        },
    })?;
    assert_eq!(plan.publications.len(), member_count);

    let mut persisted = InformationOperationPayload {
        id: InformationOperationId::new("fixture.information", "ten-thousand-fanout"),
        operation_version: 1,
        operation_kind: "transition_release".to_owned(),
        canonical_input_hash: "d".repeat(64),
        output_slots: Vec::new(),
        status: InformationOperationStatus::AwaitingPublication,
        admitted_at: minute(0),
        accepted_cause: InformationAdmissionRef::Ingress(IngressId::new(1)),
        authority_claim_hash: None,
        domain_result_refs: vec![release.into_untyped()],
        domain_result_evidence: Vec::new(),
        publication_result_ids: Vec::new(),
        continuation: Some(InformationContinuation {
            cursor: 0,
            remaining: MEMBER_COUNT,
            chunk_size: CHUNK_SIZE,
        }),
        completed_at: None,
        rejection_code: None,
    };
    let mut chunk_count = 0_u64;
    loop {
        persisted = serde_json::from_slice(
            &serde_json::to_vec(&persisted).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let continuation = persisted
            .continuation
            .clone()
            .ok_or_else(|| "publication continuation ended before its last chunk".to_owned())?;
        let published = continuation
            .remaining
            .min(u64::from(continuation.chunk_size));
        let end = continuation.cursor + published;
        let remaining = continuation.remaining - published;
        let mut awaiting_finalization = persisted.clone();
        awaiting_finalization.status = InformationOperationStatus::AwaitingFinalization;
        awaiting_finalization
            .publication_result_ids
            .extend((continuation.cursor + 1..=end).map(KnowledgeRecordId::new));
        awaiting_finalization.continuation = (remaining > 0).then_some(InformationContinuation {
            cursor: end,
            remaining,
            chunk_size: u32::try_from(remaining.min(u64::from(CHUNK_SIZE)))
                .map_err(|_| "fan-out chunk size is not representable".to_owned())?,
        });
        validate_operation_transition(&persisted, &awaiting_finalization)?;
        persisted = serde_json::from_slice(
            &serde_json::to_vec(&awaiting_finalization).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        chunk_count += 1;
        if remaining == 0 {
            let mut completed = persisted.clone();
            completed.status = InformationOperationStatus::Completed;
            completed.completed_at = Some(minute(4));
            validate_operation_transition(&persisted, &completed)?;
            persisted = completed;
            break;
        }
        let mut next = persisted.clone();
        next.status = InformationOperationStatus::AwaitingPublication;
        validate_operation_transition(&persisted, &next)?;
        persisted = next;
    }
    let final_wire = serde_json::to_vec(&persisted).map_err(|error| error.to_string())?;
    let restored: InformationOperationPayload =
        serde_json::from_slice(&final_wire).map_err(|error| error.to_string())?;
    assert!(
        final_wire.len() <= 65_536,
        "the fully finalized 10k publication ID ledger must remain compactly persistable"
    );
    assert_eq!(restored, persisted);
    assert_eq!(chunk_count, 157);
    assert_eq!(restored.status, InformationOperationStatus::Completed);
    assert_eq!(restored.publication_result_ids.len(), member_count);
    assert!(restored.continuation.is_none());
    Ok(())
}

#[test]
fn fixture_information_independent_shared_cache_channel() -> Result<(), String> {
    assert_stable_profile_id(INDEPENDENT_SHARED_CACHE_CHANNEL);
    let mut ledger = DetachedLedger::default();
    let (_, representation) = seed_representation(&mut ledger, "SC-04")?;
    let channel = TypedDomainRecordRef::<Channel>::new("channel-independent-shared-cache");
    let capabilities = vec![
        ChannelCapability::PersistentInstance,
        ChannelCapability::NonexclusiveAccess,
        ChannelCapability::OpenReception,
    ];
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "independent_shared_cache".to_owned(),
            capabilities: capabilities.clone(),
        },
    })?;
    let instance = TypedDomainRecordRef::<Instance>::new("instance-shared-cache");
    ledger.plan_and_apply(&LifecycleRequest::CreateInstance {
        binding: RecordBinding::new(
            instance.clone(),
            vec![DomainReference::from_typed(
                "representation",
                representation.clone(),
            )],
        ),
        payload: InstancePayload {
            created_at: minute(2),
            status: InstanceStatus::Available,
        },
    })?;
    let dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-shared-cache-open");
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(
            dispatch.clone(),
            vec![
                DomainReference::from_typed("channel", channel.clone()),
                DomainReference::from_typed("representation", representation),
                DomainReference::from_typed("source_instance", instance),
            ],
        ),
        payload: DispatchPayload {
            status: DispatchStatus::Prepared,
            target: DispatchTarget::Open,
            prepared_at: minute(3),
            dispatched_at: None,
            completed_at: None,
        },
    })?;
    let records = ledger.record_set()?;
    let channel_payload = records.decode::<Channel>(&channel)?;
    assert_eq!(channel_payload.profile, "independent_shared_cache");
    assert_eq!(channel_payload.capabilities, capabilities);
    assert_eq!(
        records.decode::<Dispatch>(&dispatch)?.target,
        DispatchTarget::Open
    );
    Ok(())
}
