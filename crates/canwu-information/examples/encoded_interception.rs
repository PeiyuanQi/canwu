mod support;

use canwu_api::{
    CommandId, DomainReference, EntityRef, EvidenceRef, KnowledgeHolderRef, PersonId, SimTime,
    TypedDomainRecordRef,
};
use canwu_information::{
    Access, AccessPayload, Audience, AudienceMembership, AudiencePayload, Channel,
    ChannelCapability, ChannelPayload, Content, ContentDerivation, ContentPayload, ContentRelation,
    ContentSourceEdge, ContentSourceRole, DELEGATED_AUTHORITY_GRANT, DelegationClaimV1,
    DeliveryAttempt, DeliveryAttemptPayload, DeliveryAttemptStatus, Dispatch, DispatchPayload,
    DispatchStatus, DispatchTarget, GenericInformationPublicationDraft, InformationBody,
    InformationLimitsV1, InformationOperationId, InformationOutputKind, InformationOutputSlot,
    Instance, InstancePayload, InstanceStatus, Interpretation, InterpretationAuthority,
    InterpretationPayload, InterpretationStatus, LifecycleRequest, RecordBinding, Release,
    ReleasePayload, ReleaseScope, ReleaseStatus, Representation, RepresentationPayload,
    RepresentationSourceEdge, audience_membership_root_v1, derive_output_record_ref,
};
use serde_json::json;
use support::{
    DetachedCaseLedger, assert_authoritative_knowledge, holder_reference,
    verify_authoritative_operation_roundtrip,
};

const SOURCE_CODE: &str = "SYN-E8-42";
const RESULT_CODE: &str = "SYN-E8-42-R";

fn minute(value: i64) -> SimTime {
    SimTime::from_minutes(value)
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), String> {
    let source_holder = KnowledgeHolderRef::Person(PersonId::new(601));
    let destination_holder = KnowledgeHolderRef::Person(PersonId::new(602));
    let relay_holder = KnowledgeHolderRef::Person(PersonId::new(603));
    let performer_holder = KnowledgeHolderRef::Person(PersonId::new(604));
    let review_holder = KnowledgeHolderRef::Person(PersonId::new(605));
    let mut ledger = DetachedCaseLedger::default();

    let content = TypedDomainRecordRef::<Content>::new("content-encoded-source");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(
            content.clone(),
            vec![holder_reference("creator", &source_holder)],
        ),
        payload: ContentPayload {
            content_type: "synthetic_code".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"content_code": SOURCE_CODE}),
            },
            created_at: minute(0),
            derivation: None,
        },
    })?;

    let encoded = TypedDomainRecordRef::<Representation>::new("representation-encoded");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            encoded.clone(),
            vec![
                DomainReference::from_typed("content", content.clone()),
                holder_reference("producer", &source_holder),
            ],
        ),
        payload: RepresentationPayload {
            format: "grouped_symbols_v1".to_owned(),
            created_at: minute(1),
            operation: "encode_k2".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: Some("decode_k2".to_owned()),
        },
    })?;

    let channel = TypedDomainRecordRef::<Channel>::new("channel-relay-link");
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "relay_link".to_owned(),
            capabilities: vec![
                ChannelCapability::NonexclusiveAccess,
                ChannelCapability::AddressedDelivery,
            ],
        },
    })?;

    let dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-encoded");
    let prepared_dispatch = DispatchPayload {
        status: DispatchStatus::Prepared,
        target: DispatchTarget::Addressed(vec![destination_holder.clone()]),
        prepared_at: minute(2),
        dispatched_at: None,
        completed_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(
            dispatch.clone(),
            vec![
                DomainReference::from_typed("channel", channel),
                holder_reference("intended_recipient", &destination_holder),
                DomainReference::from_typed("representation", encoded.clone()),
                holder_reference("sender", &source_holder),
            ],
        ),
        payload: prepared_dispatch.clone(),
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch.clone(),
        expected_version: 1,
        proposed: DispatchPayload {
            status: DispatchStatus::Active,
            dispatched_at: Some(minute(3)),
            ..prepared_dispatch.clone()
        },
    })?;

    let attempt = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-encoded-1");
    let prepared_attempt = DeliveryAttemptPayload {
        status: DeliveryAttemptStatus::Prepared,
        attempt_number: 1,
        prepared_at: minute(3),
        dispatched_at: None,
        due_at: minute(20),
        completed_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::BeginDeliveryAttempt {
        binding: RecordBinding::new(
            attempt.clone(),
            vec![
                DomainReference::from_typed("dispatch", dispatch.clone()),
                holder_reference("recipient", &destination_holder),
                holder_reference("relay", &relay_holder),
            ],
        ),
        payload: prepared_attempt.clone(),
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionDeliveryAttempt {
        record: attempt.clone(),
        expected_version: 1,
        proposed: DeliveryAttemptPayload {
            status: DeliveryAttemptStatus::InTransit,
            dispatched_at: Some(minute(4)),
            ..prepared_attempt.clone()
        },
    })?;

    let captured = TypedDomainRecordRef::<Representation>::new("representation-captured");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            captured.clone(),
            vec![
                DomainReference::from_typed("content", content.clone()),
                DomainReference::from_typed("parent_representation", encoded.clone()),
                holder_reference("producer", &relay_holder),
            ],
        ),
        payload: RepresentationPayload {
            format: "captured_symbols_v1".to_owned(),
            created_at: minute(5),
            operation: "relay_capture".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: vec![RepresentationSourceEdge {
                parent: encoded.clone(),
                completeness_per_mille: 1_000,
                fidelity_per_mille: 1_000,
            }],
            claimed_source: None,
            interpretation_capability: Some("decode_k2".to_owned()),
        },
    })?;

    let capture_instance = TypedDomainRecordRef::<Instance>::new("instance-captured");
    ledger.plan_and_apply(&LifecycleRequest::CreateInstance {
        binding: RecordBinding::new(
            capture_instance.clone(),
            vec![
                holder_reference("custodian", &relay_holder),
                DomainReference::from_typed("representation", captured.clone()),
            ],
        ),
        payload: InstancePayload {
            created_at: minute(5),
            status: InstanceStatus::Available,
        },
    })?;

    let relay_access = TypedDomainRecordRef::<Access>::new("access-captured");
    ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            relay_access.clone(),
            vec![
                holder_reference("holder", &relay_holder),
                DomainReference::from_typed("instance", capture_instance),
                DomainReference::from_typed("representation", captured.clone()),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(6),
            method: "relay_capture_read".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: None,
    })?;

    let failed_interpretation =
        TypedDomainRecordRef::<Interpretation>::new("interpretation-relay-failed");
    ledger.plan_and_apply(&LifecycleRequest::RecordInterpretation {
        binding: RecordBinding::new(
            failed_interpretation.clone(),
            vec![
                DomainReference::from_typed("input_access", relay_access.clone()),
                DomainReference::from_typed("input_representation", captured.clone()),
                holder_reference("performed_by", &relay_holder),
                holder_reference("performed_for", &relay_holder),
            ],
        ),
        payload: InterpretationPayload {
            interpreted_at: minute(7),
            status: InterpretationStatus::Failed,
            capability: "decode_k1".to_owned(),
            confidence_per_mille: 0,
        },
        authority: InterpretationAuthority::HolderSelf,
    })?;

    ledger.plan_and_apply(&LifecycleRequest::TransitionDeliveryAttempt {
        record: attempt.clone(),
        expected_version: 2,
        proposed: DeliveryAttemptPayload {
            status: DeliveryAttemptStatus::Delivered,
            dispatched_at: Some(minute(4)),
            completed_at: Some(minute(12)),
            ..prepared_attempt
        },
    })?;

    let destination_access = TypedDomainRecordRef::<Access>::new("access-encoded-destination");
    ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            destination_access.clone(),
            vec![
                DomainReference::from_typed("delivery_attempt", attempt),
                DomainReference::from_typed("dispatch", dispatch.clone()),
                holder_reference("holder", &destination_holder),
                DomainReference::from_typed("representation", encoded.clone()),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(13),
            method: "received_symbol_read".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: None,
    })?;

    let decoded_content = TypedDomainRecordRef::<Content>::new("content-decoded-result");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(
            decoded_content.clone(),
            vec![
                holder_reference("creator", &destination_holder),
                DomainReference::from_typed("source_content", content.clone()),
            ],
        ),
        payload: ContentPayload {
            content_type: "synthetic_decode_result".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"content_code": RESULT_CODE}),
            },
            created_at: minute(14),
            derivation: Some(ContentDerivation {
                operation: "decode_result".to_owned(),
                sources: vec![ContentSourceEdge {
                    source: content,
                    role: ContentSourceRole::Contribution,
                    completeness_per_mille: 1_000,
                    fidelity_per_mille: 1_000,
                }],
            }),
        },
    })?;

    let invalid_success_without_result = LifecycleRequest::RecordInterpretation {
        binding: RecordBinding::new(
            TypedDomainRecordRef::<Interpretation>::new("interpretation-invalid-no-result"),
            vec![
                DomainReference::from_typed("input_access", destination_access.clone()),
                DomainReference::from_typed("input_representation", encoded.clone()),
                holder_reference("performed_by", &destination_holder),
                holder_reference("performed_for", &destination_holder),
            ],
        ),
        payload: InterpretationPayload {
            interpreted_at: minute(15),
            status: InterpretationStatus::Succeeded,
            capability: "decode_k2".to_owned(),
            confidence_per_mille: 990,
        },
        authority: InterpretationAuthority::HolderSelf,
    };
    assert!(ledger.plan(&invalid_success_without_result).is_err());

    let authoritative_namespace = "fixture.case.encoded-interception";
    let authoritative_operation_id =
        InformationOperationId::new(authoritative_namespace, "authoritative-operation");
    let interpretation_output = InformationOutputSlot {
        index: 0,
        name: "result".to_owned(),
        kind: InformationOutputKind::Interpretation,
    };
    let successful_interpretation = TypedDomainRecordRef::<Interpretation>::from_untyped(
        derive_output_record_ref(&authoritative_operation_id, &interpretation_output),
    )
    .map_err(|error| error.to_string())?;
    let successful_interpretation_request = LifecycleRequest::RecordInterpretation {
        binding: RecordBinding::new(
            successful_interpretation.clone(),
            vec![
                DomainReference::from_typed("input_access", destination_access.clone()),
                DomainReference::from_typed("input_representation", encoded.clone()),
                holder_reference("performed_by", &performer_holder),
                holder_reference("performed_for", &destination_holder),
                DomainReference::from_typed("result_content", decoded_content.clone()),
            ],
        ),
        payload: InterpretationPayload {
            interpreted_at: minute(15),
            status: InterpretationStatus::Succeeded,
            capability: "decode_k2".to_owned(),
            confidence_per_mille: 990,
        },
        authority: InterpretationAuthority::Delegated {
            evidence: EvidenceRef::Command(CommandId::new(1)),
            authority_grant: DELEGATED_AUTHORITY_GRANT.to_owned(),
        },
    };
    let authoritative_seed = ledger.clone();
    let successful_interpretation_plan =
        ledger.plan_and_apply(&successful_interpretation_request)?;
    verify_authoritative_operation_roundtrip(
        authoritative_namespace,
        &authoritative_seed,
        &successful_interpretation_request,
        &ledger,
        Some(DelegationClaimV1 {
            format_version: 1,
            performed_by: EntityRef::Person(PersonId::new(604)),
            performed_for: destination_holder.clone(),
            capabilities: vec!["decode_k2".to_owned()],
            not_before: Some(SimTime::EPOCH),
            expires_at: None,
        }),
        |canwu| {
            assert_authoritative_knowledge(
                canwu,
                &destination_holder,
                &["interpretation_recorded"],
            )?;
            assert_authoritative_knowledge(canwu, &review_holder, &[])?;
            assert_authoritative_knowledge(canwu, &source_holder, &[])?;
            assert_authoritative_knowledge(canwu, &performer_holder, &[])
        },
    )?;

    let decoded_representation =
        TypedDomainRecordRef::<Representation>::new("representation-decoded-result");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            decoded_representation.clone(),
            vec![
                DomainReference::from_typed("content", decoded_content.clone()),
                holder_reference("producer", &performer_holder),
            ],
        ),
        payload: RepresentationPayload {
            format: "review_text_v1".to_owned(),
            created_at: minute(16),
            operation: "render_decode_result".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: Some("plain_review".to_owned()),
        },
    })?;

    let review_audience = TypedDomainRecordRef::<Audience>::new("audience-decode-review");
    ledger.plan_and_apply(&LifecycleRequest::CreateAudience {
        binding: RecordBinding::new(
            review_audience.clone(),
            vec![
                holder_reference("member", &destination_holder),
                holder_reference("member", &review_holder),
            ],
        ),
        payload: AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            resolved_at: minute(17),
            resolution_version: 1,
            resolved_boundary: None,
            member_count: 2,
            membership_root: audience_membership_root_v1(
                &[destination_holder.clone(), review_holder.clone()],
                InformationLimitsV1::canonical(),
            )?,
        },
    })?;

    let review_release = TypedDomainRecordRef::<Release>::new("release-decode-review");
    let prepared_review_release = ReleasePayload {
        status: ReleaseStatus::Prepared,
        scope: ReleaseScope::Audience,
        prepared_at: minute(18),
        active_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::CreateRelease {
        binding: RecordBinding::new(
            review_release.clone(),
            vec![
                DomainReference::from_typed("audience", review_audience),
                holder_reference("publisher", &destination_holder),
                DomainReference::from_typed("representation", decoded_representation),
            ],
        ),
        payload: prepared_review_release.clone(),
    })?;
    let review_activation = LifecycleRequest::TransitionRelease {
        record: review_release.clone(),
        expected_version: 1,
        proposed: ReleasePayload {
            status: ReleaseStatus::Active,
            active_at: Some(minute(19)),
            ..prepared_review_release.clone()
        },
    };
    let authoritative_review_seed = ledger.clone();
    let review_release_plan = ledger.plan_and_apply(&review_activation)?;
    verify_authoritative_operation_roundtrip(
        "fixture.case.encoded-interception.review-release",
        &authoritative_review_seed,
        &review_activation,
        &ledger,
        None,
        |canwu| {
            assert_authoritative_knowledge(canwu, &destination_holder, &["release_available"])?;
            assert_authoritative_knowledge(canwu, &review_holder, &["release_available"])?;
            assert_authoritative_knowledge(canwu, &source_holder, &[])?;
            assert_authoritative_knowledge(canwu, &performer_holder, &[])
        },
    )?;

    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch,
        expected_version: 2,
        proposed: DispatchPayload {
            status: DispatchStatus::Completed,
            dispatched_at: Some(minute(3)),
            completed_at: Some(minute(16)),
            ..prepared_dispatch
        },
    })?;

    let records = ledger.record_set()?;
    let encoded_payload = records.decode::<Representation>(&encoded)?;
    let captured_payload = records.decode::<Representation>(&captured)?;
    let failed = records.decode::<Interpretation>(&failed_interpretation)?;
    let succeeded = records.decode::<Interpretation>(&successful_interpretation)?;

    assert_eq!(
        encoded_payload.interpretation_capability.as_deref(),
        Some("decode_k2")
    );
    assert_eq!(
        captured_payload.content_relation,
        ContentRelation::SameContent
    );
    assert_eq!(captured_payload.sources[0].parent, encoded);
    assert_eq!(failed.status, InterpretationStatus::Failed);
    assert_eq!(failed.capability, "decode_k1");
    assert!(
        !ledger
            .record(failed_interpretation.as_untyped())
            .references
            .iter()
            .any(|reference| reference.role == "result_content")
    );
    assert_eq!(succeeded.status, InterpretationStatus::Succeeded);
    assert_eq!(succeeded.capability, "decode_k2");
    assert!(
        ledger
            .record(successful_interpretation.as_untyped())
            .references
            .iter()
            .any(|reference| reference.role == "result_content")
    );
    assert_ne!(performer_holder, destination_holder);
    assert!(matches!(
        successful_interpretation_plan.publications.as_slice(),
        [GenericInformationPublicationDraft::InterpretationRecorded { holder, .. }]
            if holder == &destination_holder
    ));
    assert!(matches!(
        review_release_plan.publications.as_slice(),
        [
            GenericInformationPublicationDraft::ReleaseAvailable { holder: first, .. },
            GenericInformationPublicationDraft::ReleaseAvailable { holder: second, .. },
        ] if first == &destination_holder && second == &review_holder
    ));
    assert_eq!(
        ledger
            .record(decoded_content.as_untyped())
            .payload
            .get("body")
            .and_then(|body| body.get("value"))
            .and_then(|value| value.get("content_code"))
            .and_then(|value| value.as_str()),
        Some(RESULT_CODE)
    );
    let snapshot_json = ledger.snapshot_json()?;
    let restored = DetachedCaseLedger::from_snapshot_json(&snapshot_json)?;
    assert_eq!(restored, ledger);
    assert_eq!(ledger.replay()?, ledger);

    println!(
        "encoded_interception: in-transit copy, failed access interpretation, delegated decode, restricted review release, authoritative save/load, exact replay without external decoding, and compact reconstruction verified"
    );
    Ok(())
}
