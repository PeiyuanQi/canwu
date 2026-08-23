mod support;

use canwu_api::{DomainReference, KnowledgeHolderRef, PersonId, SimTime, TypedDomainRecordRef};
use canwu_information::{
    Access, AccessPayload, Audience, AudienceAccessEvidence, AudienceMembership, AudiencePayload,
    Channel, ChannelCapability, ChannelPayload, Content, ContentDerivation, ContentPayload,
    ContentRelation, ContentSourceEdge, ContentSourceRole, DeliveryAttempt, DeliveryAttemptPayload,
    DeliveryAttemptStatus, Dispatch, DispatchPayload, DispatchStatus, DispatchTarget,
    GenericInformationPublicationDraft, InformationBody, InformationLimitsV1, Instance,
    InstancePayload, InstanceStatus, LifecycleRequest, RecordBinding, Release, ReleasePayload,
    ReleaseScope, ReleaseStatus, Representation, RepresentationPayload, RepresentationSourceEdge,
    audience_membership_root_v1,
};
use serde_json::json;
use support::{
    DetachedCaseLedger, assert_authoritative_knowledge, holder_reference,
    verify_authoritative_operation_roundtrip,
};

const CONTENT_CODE: &str = "SYN-C4-17";
const COPY_CODE: &str = "SYN-C4-17-X";

fn minute(value: i64) -> SimTime {
    SimTime::from_minutes(value)
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), String> {
    let origin_holder = KnowledgeHolderRef::Person(PersonId::new(101));
    let destination_holder = KnowledgeHolderRef::Person(PersonId::new(202));
    let relay_holder = KnowledgeHolderRef::Person(PersonId::new(303));
    let audience_member_a = KnowledgeHolderRef::Person(PersonId::new(404));
    let audience_member_b = KnowledgeHolderRef::Person(PersonId::new(505));
    let unrelated_holder = KnowledgeHolderRef::Person(PersonId::new(606));
    let mut ledger = DetachedCaseLedger::default();

    let content = TypedDomainRecordRef::<Content>::new("content-primary");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(
            content.clone(),
            vec![holder_reference("creator", &origin_holder)],
        ),
        payload: ContentPayload {
            content_type: "synthetic_code".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"content_code": CONTENT_CODE}),
            },
            created_at: minute(0),
            derivation: None,
        },
    })?;

    let representation = TypedDomainRecordRef::<Representation>::new("representation-primary");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            representation.clone(),
            vec![
                DomainReference::from_typed("content", content.clone()),
                holder_reference("producer", &origin_holder),
            ],
        ),
        payload: RepresentationPayload {
            format: "sealed_text_v1".to_owned(),
            created_at: minute(1),
            operation: "render".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: Some("plain_reading".to_owned()),
        },
    })?;

    let instance = TypedDomainRecordRef::<Instance>::new("instance-primary");
    ledger.plan_and_apply(&LifecycleRequest::CreateInstance {
        binding: RecordBinding::new(
            instance.clone(),
            vec![
                DomainReference::from_typed("representation", representation.clone()),
                holder_reference("custodian", &origin_holder),
            ],
        ),
        payload: InstancePayload {
            created_at: minute(2),
            status: InstanceStatus::Available,
        },
    })?;

    let channel = TypedDomainRecordRef::<Channel>::new("channel-carrier");
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "bounded_carrier".to_owned(),
            capabilities: vec![
                ChannelCapability::PersistentInstance,
                ChannelCapability::AddressedDelivery,
            ],
        },
    })?;

    let dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-primary");
    let prepared_dispatch = DispatchPayload {
        status: DispatchStatus::Prepared,
        target: DispatchTarget::Addressed(vec![destination_holder.clone()]),
        prepared_at: minute(5),
        dispatched_at: None,
        completed_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(
            dispatch.clone(),
            vec![
                DomainReference::from_typed("channel", channel),
                holder_reference("intended_recipient", &destination_holder),
                DomainReference::from_typed("representation", representation.clone()),
                holder_reference("sender", &origin_holder),
                DomainReference::from_typed("source_instance", instance.clone()),
            ],
        ),
        payload: prepared_dispatch.clone(),
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch.clone(),
        expected_version: 1,
        proposed: DispatchPayload {
            status: DispatchStatus::Active,
            dispatched_at: Some(minute(6)),
            ..prepared_dispatch.clone()
        },
    })?;

    let attempt = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-primary-1");
    let prepared_attempt = DeliveryAttemptPayload {
        status: DeliveryAttemptStatus::Prepared,
        attempt_number: 1,
        prepared_at: minute(6),
        dispatched_at: None,
        due_at: minute(30),
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

    let relay_access = TypedDomainRecordRef::<Access>::new("access-relay");
    let relay_access_plan = ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            relay_access.clone(),
            vec![
                holder_reference("holder", &relay_holder),
                DomainReference::from_typed("instance", instance),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(12),
            method: "temporary_read".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: None,
    })?;

    let copied_content = TypedDomainRecordRef::<Content>::new("content-selected-copy");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(
            copied_content.clone(),
            vec![
                holder_reference("creator", &relay_holder),
                DomainReference::from_typed("source_content", content.clone()),
            ],
        ),
        payload: ContentPayload {
            content_type: "synthetic_code_excerpt".to_owned(),
            body: InformationBody::InlineJson {
                value: json!({"content_code": COPY_CODE}),
            },
            created_at: minute(13),
            derivation: Some(ContentDerivation {
                operation: "select_and_copy".to_owned(),
                sources: vec![ContentSourceEdge {
                    source: content,
                    role: ContentSourceRole::Quotation,
                    completeness_per_mille: 700,
                    fidelity_per_mille: 970,
                }],
            }),
        },
    })?;

    let copied_representation =
        TypedDomainRecordRef::<Representation>::new("representation-selected-copy");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            copied_representation.clone(),
            vec![
                DomainReference::from_typed("content", copied_content.clone()),
                DomainReference::from_typed("parent_representation", representation.clone()),
                holder_reference("producer", &relay_holder),
            ],
        ),
        payload: RepresentationPayload {
            format: "excerpt_sheet_v1".to_owned(),
            created_at: minute(14),
            operation: "copy_selected_units".to_owned(),
            content_relation: ContentRelation::DerivedContent,
            sources: vec![RepresentationSourceEdge {
                parent: representation.clone(),
                completeness_per_mille: 700,
                fidelity_per_mille: 970,
            }],
            claimed_source: None,
            interpretation_capability: Some("plain_reading".to_owned()),
        },
    })?;

    ledger.plan_and_apply(&LifecycleRequest::TransitionDeliveryAttempt {
        record: attempt.clone(),
        expected_version: 1,
        proposed: DeliveryAttemptPayload {
            status: DeliveryAttemptStatus::Delivered,
            dispatched_at: Some(minute(7)),
            completed_at: Some(minute(25)),
            ..prepared_attempt
        },
    })?;

    let destination_access = TypedDomainRecordRef::<Access>::new("access-destination");
    let destination_access_plan = ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            destination_access.clone(),
            vec![
                DomainReference::from_typed("delivery_attempt", attempt),
                DomainReference::from_typed("dispatch", dispatch.clone()),
                holder_reference("holder", &destination_holder),
                DomainReference::from_typed("representation", representation),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(26),
            method: "received_read".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: None,
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch,
        expected_version: 2,
        proposed: DispatchPayload {
            status: DispatchStatus::Completed,
            dispatched_at: Some(minute(6)),
            completed_at: Some(minute(27)),
            ..prepared_dispatch
        },
    })?;

    let audience = TypedDomainRecordRef::<Audience>::new("audience-selected");
    ledger.plan_and_apply(&LifecycleRequest::CreateAudience {
        binding: RecordBinding::new(
            audience.clone(),
            vec![
                holder_reference("member", &audience_member_a),
                holder_reference("member", &audience_member_b),
            ],
        ),
        payload: AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            resolved_at: minute(40),
            resolution_version: 1,
            resolved_boundary: None,
            member_count: 2,
            membership_root: audience_membership_root_v1(
                &[audience_member_a.clone(), audience_member_b.clone()],
                InformationLimitsV1::canonical(),
            )?,
        },
    })?;

    let release = TypedDomainRecordRef::<Release>::new("release-selected");
    let prepared_release = ReleasePayload {
        status: ReleaseStatus::Prepared,
        scope: ReleaseScope::Audience,
        prepared_at: minute(41),
        active_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::CreateRelease {
        binding: RecordBinding::new(
            release.clone(),
            vec![
                DomainReference::from_typed("audience", audience),
                holder_reference("publisher", &relay_holder),
                DomainReference::from_typed("representation", copied_representation.clone()),
            ],
        ),
        payload: prepared_release.clone(),
    })?;
    let activation = LifecycleRequest::TransitionRelease {
        record: release.clone(),
        expected_version: 1,
        proposed: ReleasePayload {
            status: ReleaseStatus::Active,
            active_at: Some(minute(42)),
            ..prepared_release.clone()
        },
    };
    let authoritative_activation_seed = ledger.clone();
    let activation_plan = ledger.plan_and_apply(&activation)?;
    verify_authoritative_operation_roundtrip(
        "fixture.case.confidential-copy-release.activation",
        &authoritative_activation_seed,
        &activation,
        &ledger,
        None,
        |canwu| {
            assert_authoritative_knowledge(canwu, &audience_member_a, &["release_available"])?;
            assert_authoritative_knowledge(canwu, &audience_member_b, &["release_available"])?;
            assert_authoritative_knowledge(canwu, &origin_holder, &[])?;
            assert_authoritative_knowledge(canwu, &unrelated_holder, &[])?;
            assert_authoritative_knowledge(canwu, &relay_holder, &[])
        },
    )?;

    let audience_access = TypedDomainRecordRef::<Access>::new("access-audience-a");
    let audience_access_plan = ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            audience_access.clone(),
            vec![
                holder_reference("holder", &audience_member_a),
                DomainReference::from_typed("release", release.clone()),
                DomainReference::from_typed("representation", copied_representation.clone()),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(43),
            method: "audience_release_read".to_owned(),
            extent_per_mille: 700,
        },
        audience_evidence: Some(AudienceAccessEvidence::ListedMember),
    })?;

    assert!(matches!(
        relay_access_plan.publications.as_slice(),
        [GenericInformationPublicationDraft::AccessRecorded { holder, .. }]
            if holder == &relay_holder
    ));
    assert!(matches!(
        destination_access_plan.publications.as_slice(),
        [GenericInformationPublicationDraft::AccessRecorded { holder, .. }]
            if holder == &destination_holder
    ));
    assert_eq!(activation_plan.publications.len(), 2);
    assert!(matches!(
        audience_access_plan.publications.as_slice(),
        [GenericInformationPublicationDraft::AccessRecorded { holder, .. }]
            if holder == &audience_member_a
    ));

    let authoritative_seed = ledger.clone();
    let withdrawal = LifecycleRequest::TransitionRelease {
        record: release.clone(),
        expected_version: 2,
        proposed: ReleasePayload {
            status: ReleaseStatus::Withdrawn,
            active_at: Some(minute(42)),
            ..prepared_release
        },
    };
    ledger.plan_and_apply(&withdrawal)?;
    verify_authoritative_operation_roundtrip(
        "fixture.case.confidential-copy-release",
        &authoritative_seed,
        &withdrawal,
        &ledger,
        None,
        |canwu| {
            assert_authoritative_knowledge(canwu, &audience_member_a, &[])?;
            assert_authoritative_knowledge(canwu, &audience_member_b, &[])?;
            assert_authoritative_knowledge(canwu, &origin_holder, &[])?;
            assert_authoritative_knowledge(canwu, &unrelated_holder, &[])
        },
    )?;

    let access_after_withdrawal = LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            TypedDomainRecordRef::<Access>::new("access-after-withdrawal"),
            vec![
                holder_reference("holder", &audience_member_b),
                DomainReference::from_typed("release", release.clone()),
                DomainReference::from_typed("representation", copied_representation),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(51),
            method: "late_release_read".to_owned(),
            extent_per_mille: 700,
        },
        audience_evidence: Some(AudienceAccessEvidence::ListedMember),
    };
    assert!(ledger.plan(&access_after_withdrawal).is_err());

    let records = ledger.record_set()?;
    let copied_payload = records.decode::<Content>(&copied_content)?;
    assert!(copied_payload.derivation.is_some());
    assert_eq!(
        records.decode::<Release>(&release)?.status,
        ReleaseStatus::Withdrawn
    );
    assert_eq!(ledger.record(release.as_untyped()).version, 3);
    assert_eq!(
        ledger
            .record(copied_content.as_untyped())
            .payload
            .get("body")
            .and_then(|body| body.get("value"))
            .and_then(|value| value.get("content_code"))
            .and_then(|value| value.as_str()),
        Some(COPY_CODE)
    );
    let snapshot_json = ledger.snapshot_json()?;
    let restored = DetachedCaseLedger::from_snapshot_json(&snapshot_json)?;
    assert_eq!(restored, ledger);
    assert_eq!(ledger.replay()?, ledger);

    println!(
        "confidential_copy_release: hidden-operation holder isolation, derived copy, restricted release, withdrawal, retained knowledge, authoritative save/load, exact replay, and compact reconstruction verified"
    );
    Ok(())
}
