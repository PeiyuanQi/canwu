#![allow(clippy::too_many_lines)]

mod support;

use canwu_api::{DomainRecordKind, DomainReference, TypedDomainRecordRef};
use canwu_information::{
    Access, AccessPayload, Audience, AudienceAccessEvidence, AudienceMembership, AudiencePayload,
    Channel, ChannelCapability, ChannelPayload, Content, ContentDigest, ContentPayload,
    ContentRelation, DeliveryAttempt, DeliveryAttemptPayload, DeliveryAttemptStatus,
    DigestAlgorithm, Dispatch, DispatchPayload, DispatchStatus, DispatchTarget,
    GenericInformationPublicationDraft, InformationBody, InformationLifecycle, InformationLimitsV1,
    InformationQuery, Instance, LifecycleRequest, RecordBinding, Release, ReleasePayload,
    ReleaseScope, ReleaseStatus, Representation, RepresentationPayload,
    audience_membership_root_v1,
};
use serde_json::json;
use support::{DetachedLedger, assert_stable_profile_id, holder_reference, minute, person};

const EPHEMERAL_MULTI_OBSERVER: &str = "fixture.information.ephemeral-multi-observer";
const PARTIAL_MULTI_RECIPIENT: &str = "fixture.information.partial-multi-recipient";
const OPEN_FANOUT_RESOURCE: &str = "fixture.information.open-fanout-resource";

fn seed_inline_representation(
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
            interpretation_capability: None,
        },
    })?;
    Ok((content, representation))
}

#[test]
fn fixture_information_ephemeral_multi_observer() -> Result<(), String> {
    assert_stable_profile_id(EPHEMERAL_MULTI_OBSERVER);
    let mut ledger = DetachedLedger::default();
    let (_, representation) = seed_inline_representation(&mut ledger, "EPH-01")?;
    let channel = TypedDomainRecordRef::<Channel>::new("channel-ephemeral");
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "ephemeral_shared_field".to_owned(),
            capabilities: vec![
                ChannelCapability::SimultaneousAccess,
                ChannelCapability::OpenReception,
            ],
        },
    })?;
    let dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-ephemeral");
    let prepared = DispatchPayload {
        status: DispatchStatus::Prepared,
        target: DispatchTarget::Open,
        prepared_at: minute(9),
        dispatched_at: None,
        completed_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(
            dispatch.clone(),
            vec![
                DomainReference::from_typed("channel", channel),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
        payload: prepared.clone(),
    })?;
    ledger.plan_and_apply(&LifecycleRequest::TransitionDispatch {
        record: dispatch.clone(),
        expected_version: 1,
        proposed: DispatchPayload {
            status: DispatchStatus::Active,
            dispatched_at: Some(minute(10)),
            ..prepared
        },
    })?;

    for observer in 1..=4 {
        let holder = person(1_000 + observer);
        let plan = ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
            binding: RecordBinding::new(
                TypedDomainRecordRef::<Access>::new(format!("access-observer-{observer}")),
                vec![
                    DomainReference::from_typed("dispatch", dispatch.clone()),
                    holder_reference("holder", &holder),
                    DomainReference::from_typed("representation", representation.clone()),
                ],
            ),
            payload: AccessPayload {
                accessed_at: minute(10),
                method: "simultaneous_observation".to_owned(),
                extent_per_mille: 1_000,
            },
            audience_evidence: None,
        })?;
        assert!(matches!(
            plan.publications.as_slice(),
            [GenericInformationPublicationDraft::AccessRecorded {
                holder: published,
                ..
            }] if published == &holder
        ));
    }

    let records = ledger.record_set()?;
    let accesses = records.query(&InformationQuery {
        kinds: vec![DomainRecordKind::for_type::<Access>()],
        active_only: true,
        reference_role: None,
        reference_target: None,
    })?;
    assert_eq!(accesses.len(), 4);
    assert!(accesses.iter().all(|record| {
        record
            .decode_payload::<Access>()
            .is_ok_and(|payload| payload.accessed_at == minute(10))
    }));
    assert!(
        records
            .query(&InformationQuery {
                kinds: vec![DomainRecordKind::for_type::<Instance>()],
                active_only: true,
                reference_role: None,
                reference_target: None,
            })?
            .is_empty()
    );
    assert_eq!(ledger.record(dispatch.as_untyped()).version, 2);
    Ok(())
}

#[test]
fn fixture_information_partial_multi_recipient() -> Result<(), String> {
    assert_stable_profile_id(PARTIAL_MULTI_RECIPIENT);
    let mut ledger = DetachedLedger::default();
    let (_, representation) = seed_inline_representation(&mut ledger, "PMR-02")?;
    let channel = TypedDomainRecordRef::<Channel>::new("channel-multi-recipient");
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "addressed_queue".to_owned(),
            capabilities: vec![ChannelCapability::AddressedDelivery],
        },
    })?;
    let recipients = vec![person(2_001), person(2_002), person(2_003)];
    let dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-multi-recipient");
    let prepared_dispatch = DispatchPayload {
        status: DispatchStatus::Prepared,
        target: DispatchTarget::Addressed(recipients.clone()),
        prepared_at: minute(2),
        dispatched_at: None,
        completed_at: None,
    };
    let mut dispatch_refs = vec![
        DomainReference::from_typed("channel", channel),
        DomainReference::from_typed("representation", representation),
    ];
    dispatch_refs.extend(
        recipients
            .iter()
            .map(|holder| holder_reference("intended_recipient", holder)),
    );
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(dispatch.clone(), dispatch_refs),
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

    let first = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-recipient-1-1");
    create_attempt(
        &mut ledger,
        &dispatch,
        &recipients[0],
        &first,
        1,
        None,
        3,
        8,
    )?;
    transition_attempt(
        &mut ledger,
        &first,
        1,
        DeliveryAttemptStatus::Delivered,
        4,
        Some(5),
        8,
    )?;

    let second_first = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-recipient-2-1");
    create_attempt(
        &mut ledger,
        &dispatch,
        &recipients[1],
        &second_first,
        1,
        None,
        3,
        8,
    )?;
    transition_attempt(
        &mut ledger,
        &second_first,
        1,
        DeliveryAttemptStatus::Failed,
        4,
        Some(6),
        8,
    )?;
    let second_retry = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-recipient-2-2");
    create_attempt(
        &mut ledger,
        &dispatch,
        &recipients[1],
        &second_retry,
        2,
        Some(second_first.clone()),
        7,
        12,
    )?;
    transition_attempt(
        &mut ledger,
        &second_retry,
        1,
        DeliveryAttemptStatus::Delivered,
        8,
        Some(10),
        12,
    )?;

    let third = TypedDomainRecordRef::<DeliveryAttempt>::new("attempt-recipient-3-1");
    create_attempt(
        &mut ledger,
        &dispatch,
        &recipients[2],
        &third,
        1,
        None,
        3,
        20,
    )?;
    transition_attempt(
        &mut ledger,
        &third,
        1,
        DeliveryAttemptStatus::InTransit,
        4,
        None,
        20,
    )?;

    let records = ledger.record_set()?;
    assert_eq!(
        records
            .delivery_attempts(&dispatch, Some(&recipients[0]))?
            .len(),
        1
    );
    assert_eq!(
        records
            .delivery_attempts(&dispatch, Some(&recipients[1]))?
            .len(),
        2
    );
    assert_eq!(
        records
            .delivery_attempts(&dispatch, Some(&recipients[2]))?
            .len(),
        1
    );
    assert_eq!(
        records.decode::<DeliveryAttempt>(&first)?.status,
        DeliveryAttemptStatus::Delivered
    );
    assert_eq!(
        records.decode::<DeliveryAttempt>(&second_first)?.status,
        DeliveryAttemptStatus::Failed
    );
    assert_eq!(
        records.decode::<DeliveryAttempt>(&second_retry)?.status,
        DeliveryAttemptStatus::Delivered
    );
    assert_eq!(
        records.decode::<DeliveryAttempt>(&third)?.status,
        DeliveryAttemptStatus::InTransit
    );

    let completion = LifecycleRequest::TransitionDispatch {
        record: dispatch.clone(),
        expected_version: 2,
        proposed: DispatchPayload {
            status: DispatchStatus::Completed,
            dispatched_at: Some(minute(3)),
            completed_at: Some(minute(21)),
            ..prepared_dispatch
        },
    };
    assert!(ledger.plan(&completion).is_err());
    assert_eq!(ledger.record(dispatch.as_untyped()).version, 2);
    Ok(())
}

#[test]
fn release_withdrawal_retains_prior_access_and_publication_facts() -> Result<(), String> {
    let mut ledger = DetachedLedger::default();
    let (_, representation) = seed_inline_representation(&mut ledger, "REL-03")?;
    let first_holder = person(3_001);
    let second_holder = person(3_002);
    let audience = TypedDomainRecordRef::<Audience>::new("audience-release");
    ledger.plan_and_apply(&LifecycleRequest::CreateAudience {
        binding: RecordBinding::new(
            audience.clone(),
            vec![
                holder_reference("member", &first_holder),
                holder_reference("member", &second_holder),
            ],
        ),
        payload: AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            resolved_at: minute(2),
            resolution_version: 1,
            resolved_boundary: None,
            member_count: 2,
            membership_root: audience_membership_root_v1(
                &[first_holder.clone(), second_holder.clone()],
                InformationLimitsV1::canonical(),
            )?,
        },
    })?;
    let release = TypedDomainRecordRef::<Release>::new("release-withdrawal");
    let prepared = ReleasePayload {
        status: ReleaseStatus::Prepared,
        scope: ReleaseScope::Audience,
        prepared_at: minute(3),
        active_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::CreateRelease {
        binding: RecordBinding::new(
            release.clone(),
            vec![
                DomainReference::from_typed("audience", audience),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
        payload: prepared.clone(),
    })?;
    let active_plan = ledger.plan_and_apply(&LifecycleRequest::TransitionRelease {
        record: release.clone(),
        expected_version: 1,
        proposed: ReleasePayload {
            status: ReleaseStatus::Active,
            active_at: Some(minute(4)),
            ..prepared.clone()
        },
    })?;
    assert_eq!(active_plan.publications.len(), 2);

    let access = TypedDomainRecordRef::<Access>::new("access-before-withdrawal");
    let access_plan = ledger.plan_and_apply(&LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            access.clone(),
            vec![
                holder_reference("holder", &first_holder),
                DomainReference::from_typed("release", release.clone()),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(5),
            method: "release_read".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: Some(AudienceAccessEvidence::ListedMember),
    })?;
    let prior_facts = [active_plan.publications, access_plan.publications].concat();

    let withdrawal = ledger.plan_and_apply(&LifecycleRequest::TransitionRelease {
        record: release.clone(),
        expected_version: 2,
        proposed: ReleasePayload {
            status: ReleaseStatus::Withdrawn,
            active_at: Some(minute(4)),
            ..prepared
        },
    })?;
    assert!(withdrawal.publications.is_empty());
    assert_eq!(ledger.record(access.as_untyped()).version, 1);
    assert_eq!(prior_facts.len(), 3);

    let late_access = LifecycleRequest::RecordAccess {
        binding: RecordBinding::new(
            TypedDomainRecordRef::<Access>::new("access-after-withdrawal"),
            vec![
                holder_reference("holder", &second_holder),
                DomainReference::from_typed("release", release),
                DomainReference::from_typed("representation", representation),
            ],
        ),
        payload: AccessPayload {
            accessed_at: minute(7),
            method: "late_release_read".to_owned(),
            extent_per_mille: 1_000,
        },
        audience_evidence: Some(AudienceAccessEvidence::ListedMember),
    };
    assert!(ledger.plan(&late_access).is_err());
    Ok(())
}

#[test]
fn fixture_information_open_fanout_resource() -> Result<(), String> {
    assert_stable_profile_id(OPEN_FANOUT_RESOURCE);
    let mut ledger = DetachedLedger::default();
    let content = TypedDomainRecordRef::<Content>::new("content-open-resource");
    ledger.plan_and_apply(&LifecycleRequest::CreateContent {
        binding: RecordBinding::new(content.clone(), Vec::new()),
        payload: ContentPayload {
            content_type: "digest_resource".to_owned(),
            body: InformationBody::Resource {
                digest: ContentDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    value: "ab".repeat(32),
                },
                media_type: "application_fixture".to_owned(),
                byte_length: 4_096,
            },
            created_at: minute(0),
            derivation: None,
        },
    })?;
    let representation =
        TypedDomainRecordRef::<Representation>::new("representation-open-resource");
    ledger.plan_and_apply(&LifecycleRequest::CreateRepresentation {
        binding: RecordBinding::new(
            representation.clone(),
            vec![DomainReference::from_typed("content", content.clone())],
        ),
        payload: RepresentationPayload {
            format: "resource_notice_v1".to_owned(),
            created_at: minute(1),
            operation: "publish_digest".to_owned(),
            content_relation: ContentRelation::SameContent,
            sources: Vec::new(),
            claimed_source: None,
            interpretation_capability: None,
        },
    })?;
    let channel = TypedDomainRecordRef::<Channel>::new("channel-open-resource");
    ledger.plan_and_apply(&LifecycleRequest::CreateChannel {
        binding: RecordBinding::new(channel.clone(), Vec::new()),
        payload: ChannelPayload {
            profile: "open_resource_field".to_owned(),
            capabilities: vec![ChannelCapability::OpenReception],
        },
    })?;
    let dispatch = TypedDomainRecordRef::<Dispatch>::new("dispatch-open-resource");
    let prepared_dispatch = DispatchPayload {
        status: DispatchStatus::Prepared,
        target: DispatchTarget::Open,
        prepared_at: minute(2),
        dispatched_at: None,
        completed_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::BeginDispatch {
        binding: RecordBinding::new(
            dispatch.clone(),
            vec![
                DomainReference::from_typed("channel", channel),
                DomainReference::from_typed("representation", representation.clone()),
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
            ..prepared_dispatch
        },
    })?;

    let release = TypedDomainRecordRef::<Release>::new("release-open-resource");
    let prepared_release = ReleasePayload {
        status: ReleaseStatus::Prepared,
        scope: ReleaseScope::OpenAvailability,
        prepared_at: minute(3),
        active_at: None,
    };
    ledger.plan_and_apply(&LifecycleRequest::CreateRelease {
        binding: RecordBinding::new(
            release.clone(),
            vec![DomainReference::from_typed(
                "representation",
                representation.clone(),
            )],
        ),
        payload: prepared_release.clone(),
    })?;
    let release_plan = ledger.plan_and_apply(&LifecycleRequest::TransitionRelease {
        record: release.clone(),
        expected_version: 1,
        proposed: ReleasePayload {
            status: ReleaseStatus::Active,
            active_at: Some(minute(4)),
            ..prepared_release
        },
    })?;
    assert!(release_plan.publications.is_empty());

    let validation_cut = ledger.record_set()?;
    let mut chunks = Vec::new();
    for chunk in 0..4_u64 {
        let mut ids = Vec::with_capacity(2_500);
        for offset in 0..2_500_u64 {
            let ordinal = chunk * 2_500 + offset;
            let holder = person(10_000 + ordinal);
            let access = TypedDomainRecordRef::<Access>::new(format!("access-open-{ordinal:05}"));
            let plan = InformationLifecycle::plan(
                &validation_cut,
                &LifecycleRequest::RecordAccess {
                    binding: RecordBinding::new(
                        access.clone(),
                        vec![
                            DomainReference::from_typed("dispatch", dispatch.clone()),
                            holder_reference("holder", &holder),
                            DomainReference::from_typed("release", release.clone()),
                            DomainReference::from_typed("representation", representation.clone()),
                        ],
                    ),
                    payload: AccessPayload {
                        accessed_at: minute(10 + i64::try_from(chunk).expect("small chunk")),
                        method: "open_resource_access".to_owned(),
                        extent_per_mille: 1_000,
                    },
                    audience_evidence: None,
                },
                InformationLimitsV1::canonical(),
            )?;
            assert_eq!(plan.mutations.len(), 1);
            assert!(matches!(
                plan.publications.as_slice(),
                [GenericInformationPublicationDraft::AccessRecorded {
                    holder: published,
                    ..
                }] if published == &holder
            ));
            ids.push(access.as_untyped().id.clone());
        }
        chunks.push(ids);
    }
    assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 10_000);
    assert_eq!(
        chunks[0].first().map(String::as_str),
        Some("access-open-00000")
    );
    assert_eq!(
        chunks[3].last().map(String::as_str),
        Some("access-open-09999")
    );
    let persisted_chunks = serde_json::to_vec(&chunks).map_err(|error| error.to_string())?;
    let restored_chunks: Vec<Vec<String>> =
        serde_json::from_slice(&persisted_chunks).map_err(|error| error.to_string())?;
    assert_eq!(restored_chunks, chunks);

    let body = &ledger.record(content.as_untyped()).payload["body"];
    assert!(body.get("locator").is_none());
    assert_eq!(body["byte_length"], json!(4_096));
    assert!(
        ledger
            .record_set()?
            .query(&InformationQuery {
                kinds: vec![DomainRecordKind::for_type::<Access>()],
                active_only: true,
                reference_role: None,
                reference_target: None,
            })?
            .is_empty()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_attempt(
    ledger: &mut DetachedLedger,
    dispatch: &TypedDomainRecordRef<Dispatch>,
    recipient: &canwu_api::KnowledgeHolderRef,
    attempt: &TypedDomainRecordRef<DeliveryAttempt>,
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
fn transition_attempt(
    ledger: &mut DetachedLedger,
    attempt: &TypedDomainRecordRef<DeliveryAttempt>,
    expected_version: u64,
    status: DeliveryAttemptStatus,
    dispatched_at: i64,
    completed_at: Option<i64>,
    due_at: i64,
) -> Result<(), String> {
    let prepared_at = if attempt.as_untyped().id.ends_with("-2") {
        7
    } else {
        3
    };
    ledger.plan_and_apply(&LifecycleRequest::TransitionDeliveryAttempt {
        record: attempt.clone(),
        expected_version,
        proposed: DeliveryAttemptPayload {
            status,
            attempt_number: if attempt.as_untyped().id.ends_with("-2") {
                2
            } else {
                1
            },
            prepared_at: minute(prepared_at),
            dispatched_at: Some(minute(dispatched_at)),
            due_at: minute(due_at),
            completed_at: completed_at.map(minute),
        },
    })?;
    Ok(())
}
