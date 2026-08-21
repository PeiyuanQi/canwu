use crate::model::{
    AccessContext, AccessPayload, AudienceAccessEvidence, AudienceMembership, AudiencePayload,
    ChannelCapability, ChannelPayload, ContentPayload, ContentRelation, DelegationAuthorityGrant,
    DelegationClaimV1, DelegationEvidenceSelector, DeliveryAttemptPayload, DeliveryAttemptStatus,
    DispatchPayload, DispatchStatus, DispatchTarget, InformationBody, InformationLimitsV1,
    InstancePayload, InstanceStatus, InterpretationAuthority, InterpretationPayload,
    InterpretationStatus, ReleasePayload, ReleaseScope, ReleaseStatus, RepresentationPayload,
};
use crate::query::InformationRecordSet;
use crate::schema::{
    Access, Audience, Channel, Content, DeliveryAttempt, Dispatch, Instance, Interpretation,
    Release, Representation,
};
use canwu_api::{
    DomainRecordDraft, DomainRecordMutation, DomainRecordType, DomainReference,
    DomainReferenceTarget, EntityRef, KnowledgeHolderRef, TypedDomainRecordRef,
    canonical_byte_hash, canonical_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::marker::PhantomData;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct RecordBinding<T: DomainRecordType> {
    pub reference: TypedDomainRecordRef<T>,
    pub references: Vec<DomainReference>,
    #[serde(skip)]
    marker: PhantomData<fn() -> T>,
}

impl<T: DomainRecordType> RecordBinding<T> {
    #[must_use]
    pub fn new(reference: TypedDomainRecordRef<T>, mut references: Vec<DomainReference>) -> Self {
        references.sort();
        references.dedup();
        Self {
            reference,
            references,
            marker: PhantomData,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleRequest {
    CreateChannel {
        binding: RecordBinding<Channel>,
        payload: ChannelPayload,
    },
    CreateContent {
        binding: RecordBinding<Content>,
        payload: ContentPayload,
    },
    CreateRepresentation {
        binding: RecordBinding<Representation>,
        payload: RepresentationPayload,
    },
    CreateInstance {
        binding: RecordBinding<Instance>,
        payload: InstancePayload,
    },
    TransitionInstance {
        record: TypedDomainRecordRef<Instance>,
        expected_version: u64,
        status: InstanceStatus,
        custodian: Option<EntityRef>,
        location: Option<EntityRef>,
    },
    BeginDispatch {
        binding: RecordBinding<Dispatch>,
        payload: DispatchPayload,
    },
    TransitionDispatch {
        record: TypedDomainRecordRef<Dispatch>,
        expected_version: u64,
        proposed: DispatchPayload,
    },
    BeginDeliveryAttempt {
        binding: RecordBinding<DeliveryAttempt>,
        payload: DeliveryAttemptPayload,
    },
    TransitionDeliveryAttempt {
        record: TypedDomainRecordRef<DeliveryAttempt>,
        expected_version: u64,
        proposed: DeliveryAttemptPayload,
    },
    RecordAccess {
        binding: RecordBinding<Access>,
        payload: AccessPayload,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audience_evidence: Option<AudienceAccessEvidence>,
    },
    RecordInterpretation {
        binding: RecordBinding<Interpretation>,
        payload: InterpretationPayload,
        authority: InterpretationAuthority,
    },
    CreateAudience {
        binding: RecordBinding<Audience>,
        payload: AudiencePayload,
    },
    CreateRelease {
        binding: RecordBinding<Release>,
        payload: ReleasePayload,
    },
    TransitionRelease {
        record: TypedDomainRecordRef<Release>,
        expected_version: u64,
        proposed: ReleasePayload,
    },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct InformationMutationPlan {
    pub mutations: Vec<DomainRecordMutation>,
    pub publications: Vec<GenericInformationPublicationDraft>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GenericInformationPublicationDraft {
    RepresentationAvailable {
        holder: KnowledgeHolderRef,
        representation: TypedDomainRecordRef<Representation>,
        delivery_attempt: TypedDomainRecordRef<DeliveryAttempt>,
        record_version: u64,
    },
    AccessRecorded {
        holder: KnowledgeHolderRef,
        access: TypedDomainRecordRef<Access>,
        record_version: u64,
    },
    InterpretationRecorded {
        holder: KnowledgeHolderRef,
        interpretation: TypedDomainRecordRef<Interpretation>,
        record_version: u64,
    },
    ReleaseAvailable {
        holder: KnowledgeHolderRef,
        release: TypedDomainRecordRef<Release>,
        record_version: u64,
    },
}

pub struct InformationLifecycle;

impl InformationLifecycle {
    pub fn plan(
        records: &InformationRecordSet,
        request: &LifecycleRequest,
        limits: InformationLimitsV1,
    ) -> Result<InformationMutationPlan, String> {
        match request {
            LifecycleRequest::CreateChannel { binding, payload } => {
                validate_channel(payload)?;
                create(binding, payload)
            }
            LifecycleRequest::CreateContent { binding, payload } => {
                validate_content_lineage(payload, binding)?;
                if payload.derivation.as_ref().is_some_and(|derivation| {
                    derivation.sources.len() > limits.max_parent_edges as usize
                }) {
                    return Err("content exceeds the parent-edge limit".to_owned());
                }
                validate_body(&payload.body, limits)?;
                create(binding, payload)
            }
            LifecycleRequest::CreateRepresentation { binding, payload } => {
                validate_representation_lineage(records, payload, binding, limits)?;
                create(binding, payload)
            }
            LifecycleRequest::CreateInstance { binding, payload } => {
                require_single_role(binding, "representation")?;
                create(binding, payload)
            }
            LifecycleRequest::TransitionInstance {
                record,
                expected_version,
                status,
                custodian,
                location,
            } => {
                let previous = records.required(record)?;
                if previous.version != *expected_version {
                    return Err(
                        "instance expected version does not match current record".to_owned()
                    );
                }
                let previous_payload = previous.decode_payload::<Instance>().map_err(stringify)?;
                let proposed = InstancePayload {
                    created_at: previous_payload.created_at,
                    status: *status,
                };
                validate_instance_transition(&previous_payload, &proposed)?;
                let mut references = retain_role(&previous.references, "representation");
                push_entity_role(&mut references, "custodian", custodian.clone());
                push_entity_role(&mut references, "location", location.clone());
                update(record, &proposed, references, *expected_version)
            }
            LifecycleRequest::BeginDispatch { binding, payload } => {
                validate_dispatch_shape(records, payload, binding, limits)?;
                create(binding, payload)
            }
            LifecycleRequest::TransitionDispatch {
                record,
                expected_version,
                proposed,
            } => {
                let previous = records.required(record)?;
                if previous.version != *expected_version {
                    return Err(
                        "dispatch expected version does not match current record".to_owned()
                    );
                }
                let previous_payload = previous.decode_payload::<Dispatch>().map_err(stringify)?;
                validate_dispatch_transition(&previous_payload, proposed)?;
                if proposed.status == DispatchStatus::Completed {
                    validate_dispatch_completion(records, record, &previous_payload)?;
                }
                update(
                    record,
                    proposed,
                    previous.references.clone(),
                    *expected_version,
                )
            }
            LifecycleRequest::BeginDeliveryAttempt { binding, payload } => {
                validate_delivery_attempt_shape(records, payload, binding, limits)?;
                create(binding, payload)
            }
            LifecycleRequest::TransitionDeliveryAttempt {
                record,
                expected_version,
                proposed,
            } => {
                let previous = records.required(record)?;
                if previous.version != *expected_version {
                    return Err(
                        "delivery-attempt expected version does not match current record"
                            .to_owned(),
                    );
                }
                let previous_payload = previous
                    .decode_payload::<DeliveryAttempt>()
                    .map_err(stringify)?;
                validate_delivery_attempt_transition(&previous_payload, proposed)?;
                let mut plan = update(
                    record,
                    proposed,
                    previous.references.clone(),
                    *expected_version,
                )?;
                if proposed.status == DeliveryAttemptStatus::Delivered {
                    let holder = single_holder_role(&previous.references, "recipient")?;
                    let dispatch = single_typed_role::<Dispatch>(&previous.references, "dispatch")?;
                    let dispatch_record = records.required(&dispatch)?;
                    let representation = single_typed_role::<Representation>(
                        &dispatch_record.references,
                        "representation",
                    )?;
                    let representation_record = records.required(&representation)?;
                    plan.publications.push(
                        GenericInformationPublicationDraft::RepresentationAvailable {
                            holder,
                            representation,
                            delivery_attempt: record.clone(),
                            record_version: representation_record.version,
                        },
                    );
                }
                Ok(plan)
            }
            LifecycleRequest::RecordAccess {
                binding,
                payload,
                audience_evidence,
            } => {
                validate_access_context(
                    records,
                    payload,
                    binding,
                    audience_evidence.as_ref(),
                    limits,
                )?;
                let mut plan = create(binding, payload)?;
                plan.publications
                    .push(GenericInformationPublicationDraft::AccessRecorded {
                        holder: single_holder_role(&binding.references, "holder")?,
                        access: binding.reference.clone(),
                        record_version: 1,
                    });
                Ok(plan)
            }
            LifecycleRequest::RecordInterpretation {
                binding,
                payload,
                authority,
            } => {
                validate_interpretation(records, payload, binding, authority)?;
                let mut plan = create(binding, payload)?;
                plan.publications.push(
                    GenericInformationPublicationDraft::InterpretationRecorded {
                        holder: single_holder_role(&binding.references, "performed_for")?,
                        interpretation: binding.reference.clone(),
                        record_version: 1,
                    },
                );
                Ok(plan)
            }
            LifecycleRequest::CreateAudience { binding, payload } => {
                validate_audience(payload, binding, limits)?;
                create(binding, payload)
            }
            LifecycleRequest::CreateRelease { binding, payload } => {
                validate_release_shape(payload, binding)?;
                create(binding, payload)
            }
            LifecycleRequest::TransitionRelease {
                record,
                expected_version,
                proposed,
            } => {
                let previous = records.required(record)?;
                if previous.version != *expected_version {
                    return Err("release expected version does not match current record".to_owned());
                }
                let previous_payload = previous.decode_payload::<Release>().map_err(stringify)?;
                validate_release_transition(&previous_payload, proposed)?;
                let mut plan = update(
                    record,
                    proposed,
                    previous.references.clone(),
                    *expected_version,
                )?;
                if proposed.status == ReleaseStatus::Active
                    && previous_payload.status != ReleaseStatus::Active
                    && proposed.scope == ReleaseScope::Audience
                {
                    let audience = single_typed_role::<Audience>(&previous.references, "audience")?;
                    let audience_record = records.required(&audience)?;
                    let mut holders: Vec<_> = role_targets(&audience_record.references, "member")
                        .into_iter()
                        .map(holder_from_target)
                        .collect();
                    holders.sort();
                    holders.dedup();
                    if holders.len() > limits.max_explicit_audience_members as usize {
                        return Err("release audience exceeds the explicit-member limit".to_owned());
                    }
                    plan.publications.extend(holders.into_iter().map(|holder| {
                        GenericInformationPublicationDraft::ReleaseAvailable {
                            holder,
                            release: record.clone(),
                            record_version: expected_version + 1,
                        }
                    }));
                }
                Ok(plan)
            }
        }
    }
}

pub fn validate_content_lineage(
    payload: &ContentPayload,
    binding: &RecordBinding<Content>,
) -> Result<(), String> {
    validate_canonical_text(&payload.content_type, "content type")?;
    let references = role_domain_refs(&binding.references, "source_content")?;
    match &payload.derivation {
        None if references.is_empty() => Ok(()),
        None => Err("original content cannot carry source-content references".to_owned()),
        Some(derivation) => {
            validate_canonical_text(&derivation.operation, "content derivation operation")?;
            if derivation.sources.is_empty() {
                return Err("derived content requires at least one source edge".to_owned());
            }
            let edge_refs: Vec<_> = derivation
                .sources
                .iter()
                .map(|edge| edge.source.as_untyped().clone())
                .collect();
            validate_unique_refs(&edge_refs, "content source edge")?;
            for edge in &derivation.sources {
                validate_per_mille(edge.completeness_per_mille, "content completeness")?;
                validate_per_mille(edge.fidelity_per_mille, "content fidelity")?;
            }
            if as_set(edge_refs) != as_set(references) {
                return Err(
                    "content source references must exactly match derivation source edges"
                        .to_owned(),
                );
            }
            Ok(())
        }
    }
}

pub fn validate_representation_lineage(
    records: &InformationRecordSet,
    payload: &RepresentationPayload,
    binding: &RecordBinding<Representation>,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    validate_canonical_text(&payload.format, "representation format")?;
    validate_canonical_text(&payload.operation, "representation operation")?;
    if let Some(claimed_source) = &payload.claimed_source {
        validate_canonical_text(&claimed_source.namespace, "claimed-source namespace")?;
        validate_canonical_text(&claimed_source.value, "claimed-source value")?;
    }
    if let Some(capability) = &payload.interpretation_capability {
        validate_canonical_text(capability, "interpretation capability")?;
    }
    if payload.sources.len() > limits.max_parent_edges as usize {
        return Err("representation exceeds the parent-edge limit".to_owned());
    }
    let child_content = single_typed_role::<Content>(&binding.references, "content")?;
    let parents = role_typed_refs::<Representation>(&binding.references, "parent_representation")?;
    let edge_parents: Vec<_> = payload
        .sources
        .iter()
        .map(|edge| edge.parent.clone())
        .collect();
    validate_unique_refs(&edge_parents, "representation parent edge")?;
    if as_set(parents.clone()) != as_set(edge_parents) {
        return Err("representation parent references must exactly match source edges".to_owned());
    }
    for edge in &payload.sources {
        validate_per_mille(edge.completeness_per_mille, "representation completeness")?;
        validate_per_mille(edge.fidelity_per_mille, "representation fidelity")?;
    }
    if parents.is_empty() {
        return Ok(());
    }

    let child_content_record = records.required(&child_content)?;
    let child_payload = child_content_record
        .decode_payload::<Content>()
        .map_err(stringify)?;
    let child_sources =
        child_payload
            .derivation
            .as_ref()
            .map_or_else(BTreeSet::new, |derivation| {
                derivation
                    .sources
                    .iter()
                    .map(|edge| edge.source.as_untyped().clone())
                    .collect()
            });

    for parent in parents {
        let parent_record = records.required(&parent)?;
        let parent_content = single_typed_role::<Content>(&parent_record.references, "content")?;
        match payload.content_relation {
            ContentRelation::SameContent if parent_content != child_content => {
                return Err(
                    "same-content representation lineage must retain parent content".to_owned(),
                );
            }
            ContentRelation::DerivedContent
                if !child_sources.contains(parent_content.as_untyped()) =>
            {
                return Err(
                    "derived representation content must name every parent content".to_owned(),
                );
            }
            ContentRelation::SameContent | ContentRelation::DerivedContent => {}
        }
    }
    Ok(())
}

pub fn validate_instance_transition(
    previous: &InstancePayload,
    proposed: &InstancePayload,
) -> Result<(), String> {
    if previous.created_at != proposed.created_at {
        return Err("instance transition changed immutable creation time".to_owned());
    }
    if previous.status.is_terminal() {
        return Err("terminal instance state cannot reopen".to_owned());
    }
    let allowed = previous.status == proposed.status
        || matches!(
            (previous.status, proposed.status),
            (
                InstanceStatus::Available | InstanceStatus::LocationUnknown,
                InstanceStatus::Unavailable,
            ) | (
                InstanceStatus::Available | InstanceStatus::Unavailable,
                InstanceStatus::LocationUnknown,
            ) | (
                InstanceStatus::Available
                    | InstanceStatus::Unavailable
                    | InstanceStatus::LocationUnknown,
                InstanceStatus::Destroyed | InstanceStatus::Consumed,
            ) | (
                InstanceStatus::Unavailable | InstanceStatus::LocationUnknown,
                InstanceStatus::Available,
            )
        );
    if !allowed {
        return Err("invalid instance status transition".to_owned());
    }
    Ok(())
}

pub fn validate_dispatch_transition(
    previous: &DispatchPayload,
    proposed: &DispatchPayload,
) -> Result<(), String> {
    if previous.target != proposed.target || previous.prepared_at != proposed.prepared_at {
        return Err("dispatch transition changed immutable fields".to_owned());
    }
    if previous.status.is_terminal() {
        return Err("terminal dispatch cannot reopen".to_owned());
    }
    let allowed = matches!(
        (previous.status, proposed.status),
        (
            DispatchStatus::Prepared,
            DispatchStatus::Active | DispatchStatus::Cancelled
        ) | (
            DispatchStatus::Active,
            DispatchStatus::Completed | DispatchStatus::Cancelled
        )
    );
    if !allowed {
        return Err("invalid dispatch status transition".to_owned());
    }
    validate_dispatch_times(proposed)
}

pub fn validate_delivery_attempt_transition(
    previous: &DeliveryAttemptPayload,
    proposed: &DeliveryAttemptPayload,
) -> Result<(), String> {
    if previous.attempt_number != proposed.attempt_number
        || previous.prepared_at != proposed.prepared_at
        || previous.due_at != proposed.due_at
    {
        return Err("delivery-attempt transition changed immutable fields".to_owned());
    }
    if previous.status.is_terminal() {
        return Err("terminal delivery attempt cannot reopen".to_owned());
    }
    let allowed = matches!(
        (previous.status, proposed.status),
        (
            DeliveryAttemptStatus::Prepared,
            DeliveryAttemptStatus::InTransit
                | DeliveryAttemptStatus::Delivered
                | DeliveryAttemptStatus::Failed
                | DeliveryAttemptStatus::Cancelled
        ) | (
            DeliveryAttemptStatus::InTransit,
            DeliveryAttemptStatus::Delivered
                | DeliveryAttemptStatus::Failed
                | DeliveryAttemptStatus::Cancelled
        )
    );
    if !allowed {
        return Err("invalid delivery-attempt status transition".to_owned());
    }
    validate_delivery_attempt_times(proposed)
}

pub fn validate_access_context(
    records: &InformationRecordSet,
    payload: &AccessPayload,
    binding: &RecordBinding<Access>,
    audience_evidence: Option<&AudienceAccessEvidence>,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    validate_canonical_text(&payload.method, "access method")?;
    validate_per_mille(payload.extent_per_mille, "access extent")?;
    let representation =
        single_typed_role::<Representation>(&binding.references, "representation")?;
    let holder = single_holder_role(&binding.references, "holder")?;
    let instance = optional_typed_role::<Instance>(&binding.references, "instance")?;
    let dispatch = optional_typed_role::<Dispatch>(&binding.references, "dispatch")?;
    let attempt = optional_typed_role::<DeliveryAttempt>(&binding.references, "delivery_attempt")?;
    let release = optional_typed_role::<Release>(&binding.references, "release")?;
    if instance.is_none() && dispatch.is_none() && attempt.is_none() && release.is_none() {
        return Err("access requires instance, dispatch, attempt, or release context".to_owned());
    }
    let context = AccessContext {
        representation: representation.clone(),
        instance: instance.clone(),
        dispatch: dispatch.clone(),
        delivery_attempt: attempt.clone(),
        release: release.clone(),
    };
    validate_context_representation(records, &context)?;

    if let Some(dispatch_ref) = &dispatch {
        let dispatch_record = records.required(dispatch_ref)?;
        let dispatch_payload = dispatch_record
            .decode_payload::<Dispatch>()
            .map_err(stringify)?;
        if let Some(attempt_ref) = &attempt {
            let attempt_record = records.required(attempt_ref)?;
            let owning_dispatch =
                single_typed_role::<Dispatch>(&attempt_record.references, "dispatch")?;
            if &owning_dispatch != dispatch_ref {
                return Err("access attempt does not belong to its dispatch".to_owned());
            }
        } else if !matches!(
            dispatch_payload.target,
            DispatchTarget::Audience(_) | DispatchTarget::Open
        ) || !matches!(
            dispatch_payload.status,
            DispatchStatus::Active | DispatchStatus::Completed
        ) {
            return Err(
                "direct dispatch access requires an active audience or open dispatch".to_owned(),
            );
        }
    }
    validate_access_audience(
        records,
        &holder,
        dispatch.as_ref(),
        release.as_ref(),
        audience_evidence,
        limits,
    )?;
    Ok(())
}

fn validate_access_audience(
    records: &InformationRecordSet,
    holder: &KnowledgeHolderRef,
    dispatch: Option<&TypedDomainRecordRef<Dispatch>>,
    release: Option<&TypedDomainRecordRef<Release>>,
    evidence: Option<&AudienceAccessEvidence>,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    let mut audiences = BTreeSet::new();
    if let Some(dispatch) = dispatch {
        let record = records.required(dispatch)?;
        let payload = record.decode_payload::<Dispatch>().map_err(stringify)?;
        if let DispatchTarget::Audience(audience) = payload.target {
            audiences.insert(audience);
        }
    }
    if let Some(release) = release {
        let record = records.required(release)?;
        let payload = record.decode_payload::<Release>().map_err(stringify)?;
        if payload.status != ReleaseStatus::Active {
            return Err("access through a release requires an active release".to_owned());
        }
        if payload.scope == ReleaseScope::Audience {
            audiences.insert(single_typed_role::<Audience>(
                &record.references,
                "audience",
            )?);
        }
    }
    if audiences.is_empty() {
        if evidence.is_some() {
            return Err("open or non-audience access cannot carry audience evidence".to_owned());
        }
        return Ok(());
    }
    if audiences.len() != 1 {
        return Err("access contexts resolve to different audiences".to_owned());
    }
    let audience = audiences.into_iter().next().expect("length was checked");
    let record = records.required(&audience)?;
    let payload = record.decode_payload::<Audience>().map_err(stringify)?;
    let holder_entity = holder_entity(holder);
    let listed = role_targets(&record.references, "member")
        .into_iter()
        .any(|target| holder_entity == target_entity(target));
    match (payload.membership, evidence) {
        (
            AudienceMembership::ExplicitMembers | AudienceMembership::ResolvedGroupSnapshot,
            Some(AudienceAccessEvidence::ListedMember),
        ) if listed => Ok(()),
        (
            AudienceMembership::ResolvedGroupSnapshot,
            Some(AudienceAccessEvidence::MembershipProof(proof)),
        ) => verify_audience_membership_proof_v1(proof, holder, &payload, limits),
        (AudienceMembership::ExplicitMembers, _) => {
            Err("access holder is not an explicit audience member".to_owned())
        }
        (AudienceMembership::ResolvedGroupSnapshot, _) => Err(
            "resolved audience access requires a stored member or verified membership proof"
                .to_owned(),
        ),
    }
}

fn validate_membership_proof_shape(
    proof: &crate::AudienceMembershipProofV1,
    holder: &KnowledgeHolderRef,
    audience: &AudiencePayload,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    if &proof.holder != holder
        || proof.member_count != audience.member_count
        || proof.leaf_index >= proof.member_count
        || proof.sibling_hashes.len() > limits.max_membership_proof_siblings as usize
        || proof
            .sibling_hashes
            .iter()
            .any(|hash| !is_lower_hex_32_bytes(hash))
    {
        return Err("audience membership proof shape is invalid".to_owned());
    }
    let bytes = serde_json::to_vec(proof).map_err(stringify)?;
    if bytes.len() > limits.max_membership_proof_bytes as usize {
        return Err("audience membership proof exceeds its canonical byte limit".to_owned());
    }
    Ok(())
}

const AUDIENCE_MEMBER_LEAF_DOMAIN: &str = "canwu.audience.member.leaf.v1";
const AUDIENCE_MEMBER_NODE_DOMAIN: &str = "canwu.audience.member.node.v1";

/// Computes the canonical V1 commitment root for a complete bounded member list.
pub fn audience_membership_root_v1(
    members: &[KnowledgeHolderRef],
    limits: InformationLimitsV1,
) -> Result<String, String> {
    let mut canonical_members = members.to_vec();
    canonical_members.sort();
    canonical_members.dedup();
    if canonical_members.is_empty() {
        return Err("audience membership requires at least one holder".to_owned());
    }
    if canonical_members.len() > limits.max_explicit_audience_members as usize {
        return Err("audience membership exceeds the complete-member limit".to_owned());
    }
    let mut level = canonical_members
        .into_iter()
        .map(|holder| audience_member_leaf_hash(&holder))
        .collect::<Result<Vec<_>, _>>()?;
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            let right = pair.get(1).unwrap_or(&pair[0]);
            next.push(audience_member_node_hash(&pair[0], right)?);
        }
        level = next;
    }
    Ok(encode_hash(&level[0]))
}

/// Verifies one proof against an immutable V1 audience commitment.
pub fn verify_audience_membership_proof_v1(
    proof: &crate::AudienceMembershipProofV1,
    holder: &KnowledgeHolderRef,
    audience: &AudiencePayload,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    validate_membership_proof_shape(proof, holder, audience, limits)?;
    if proof.sibling_hashes.len() != membership_proof_depth(proof.member_count) {
        return Err("audience membership proof has the wrong depth".to_owned());
    }
    let mut current = audience_member_leaf_hash(&proof.holder)?;
    let mut index = proof.leaf_index;
    let mut count = proof.member_count;
    for sibling in &proof.sibling_hashes {
        let sibling = decode_hash(sibling)?;
        if index.is_multiple_of(2) {
            if index + 1 >= count && sibling != current {
                return Err(
                    "audience membership proof has an invalid duplicated odd node".to_owned(),
                );
            }
            current = audience_member_node_hash(&current, &sibling)?;
        } else {
            current = audience_member_node_hash(&sibling, &current)?;
        }
        index /= 2;
        count = count.div_ceil(2);
    }
    if count != 1 || index != 0 || encode_hash(&current) != audience.membership_root {
        return Err("audience membership proof does not match the committed root".to_owned());
    }
    Ok(())
}

fn audience_member_leaf_hash(holder: &KnowledgeHolderRef) -> Result<[u8; 32], String> {
    canonical_hash(
        AUDIENCE_MEMBER_LEAF_DOMAIN,
        &crate::AudienceMembershipLeafV1 {
            format_version: 1,
            holder: holder.clone(),
        },
    )
    .map_err(stringify)
    .and_then(|hash| decode_hash(&hash))
}

fn audience_member_node_hash(left: &[u8; 32], right: &[u8; 32]) -> Result<[u8; 32], String> {
    let mut payload = [0_u8; 64];
    payload[..32].copy_from_slice(left);
    payload[32..].copy_from_slice(right);
    decode_hash(&canonical_byte_hash(AUDIENCE_MEMBER_NODE_DOMAIN, &payload))
}

fn membership_proof_depth(mut member_count: u64) -> usize {
    let mut depth = 0;
    while member_count > 1 {
        member_count = member_count.div_ceil(2);
        depth += 1;
    }
    depth
}

fn decode_hash(value: &str) -> Result<[u8; 32], String> {
    if !is_lower_hex_32_bytes(value) {
        return Err("audience membership hash is not canonical lowercase hex".to_owned());
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(decoded)
}

const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

fn encode_hash(value: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in value {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}
pub fn validate_interpretation(
    records: &InformationRecordSet,
    payload: &InterpretationPayload,
    binding: &RecordBinding<Interpretation>,
    authority: &InterpretationAuthority,
) -> Result<(), String> {
    validate_canonical_text(&payload.capability, "interpretation capability")?;
    validate_per_mille(payload.confidence_per_mille, "interpretation confidence")?;
    require_single_role(binding, "performed_by")?;
    require_single_role(binding, "performed_for")?;
    let accesses = role_typed_refs::<Access>(&binding.references, "input_access")?;
    let representations =
        role_typed_refs::<Representation>(&binding.references, "input_representation")?;
    if accesses.is_empty() || representations.is_empty() {
        return Err("interpretation requires input access and representation records".to_owned());
    }
    let available: BTreeSet<_> = accesses
        .iter()
        .map(|access| {
            let record = records.required(access)?;
            single_typed_role::<Representation>(&record.references, "representation")
        })
        .collect::<Result<_, _>>()?;
    if representations
        .iter()
        .any(|representation| !available.contains(representation))
    {
        return Err("every interpreted representation must be named by an input access".to_owned());
    }
    let result_content = optional_typed_role::<Content>(&binding.references, "result_content")?;
    match payload.status {
        InterpretationStatus::Failed if result_content.is_some() => {
            return Err("failed interpretation cannot have result content".to_owned());
        }
        InterpretationStatus::Partial | InterpretationStatus::Succeeded
            if result_content.is_none() =>
        {
            return Err("partial or successful interpretation requires result content".to_owned());
        }
        InterpretationStatus::Failed
        | InterpretationStatus::Partial
        | InterpretationStatus::Succeeded => {}
    }
    match authority {
        InterpretationAuthority::HolderSelf => {
            let performed_by = single_entity_role(&binding.references, "performed_by")?;
            let performed_for = single_holder_role(&binding.references, "performed_for")?;
            if holder_entity(&performed_for) != performed_by {
                return Err(
                    "self interpretation authority requires performer and holder equality"
                        .to_owned(),
                );
            }
        }
        InterpretationAuthority::InstitutionalRole {
            assignment,
            authority_grant,
        } => {
            if assignment.version == 0 {
                return Err("institutional assignment version must be non-zero".to_owned());
            }
            validate_canonical_text(authority_grant, "authority grant")?;
        }
        InterpretationAuthority::Delegated {
            authority_grant, ..
        } => validate_canonical_text(authority_grant, "authority grant")?,
    }
    Ok(())
}

pub fn validate_delegation_grant(grant: &DelegationAuthorityGrant) -> Result<(), String> {
    validate_canonical_text(&grant.code, "delegation grant code")?;
    match &grant.selector {
        DelegationEvidenceSelector::Command {
            producer_plugin,
            command_type,
        } => {
            validate_canonical_text(producer_plugin, "delegation command producer")?;
            validate_canonical_text(command_type, "delegation command type")?;
        }
        DelegationEvidenceSelector::Ingress {
            producer_plugin,
            packet_type,
        } => {
            validate_canonical_text(producer_plugin, "delegation ingress producer")?;
            validate_canonical_text(packet_type, "delegation ingress packet type")?;
        }
        DelegationEvidenceSelector::DomainRecord { owner_plugin, kind } => {
            validate_canonical_text(owner_plugin, "delegation record owner")?;
            validate_canonical_text(&kind.namespace, "delegation record namespace")?;
            validate_canonical_text(&kind.name, "delegation record kind")?;
        }
    }
    if !(1..=8).contains(&grant.claim_path.len()) {
        return Err("delegation claim path must contain one to eight keys".to_owned());
    }
    for key in &grant.claim_path {
        validate_canonical_text(key, "delegation claim-path key")?;
        if key
            .chars()
            .any(|character| matches!(character, '.' | '[' | ']' | '*' | '\\'))
        {
            return Err("delegation claim-path keys cannot contain path syntax".to_owned());
        }
    }
    Ok(())
}

pub fn validate_delegation_claim(
    claim: &DelegationClaimV1,
    performed_by: &EntityRef,
    performed_for: &KnowledgeHolderRef,
    capability: &str,
    interpreted_at: canwu_api::SimTime,
) -> Result<(), String> {
    if claim.format_version != 1
        || &claim.performed_by != performed_by
        || &claim.performed_for != performed_for
        || claim.capabilities.is_empty()
        || !is_sorted_unique(&claim.capabilities)
        || claim
            .capabilities
            .binary_search_by(|candidate| candidate.as_str().cmp(capability))
            .is_err()
    {
        return Err("delegation claim does not bind the interpretation request".to_owned());
    }
    for claimed in &claim.capabilities {
        validate_canonical_text(claimed, "delegation capability")?;
    }
    if claim
        .not_before
        .zip(claim.expires_at)
        .is_some_and(|(start, end)| end <= start)
        || claim.not_before.is_some_and(|start| interpreted_at < start)
        || claim.expires_at.is_some_and(|end| interpreted_at >= end)
    {
        return Err("interpretation time is outside the delegation interval".to_owned());
    }
    Ok(())
}

pub fn validate_release_transition(
    previous: &ReleasePayload,
    proposed: &ReleasePayload,
) -> Result<(), String> {
    if previous.scope != proposed.scope || previous.prepared_at != proposed.prepared_at {
        return Err("release transition changed immutable fields".to_owned());
    }
    if previous.status.is_terminal() {
        return Err("terminal release cannot reopen".to_owned());
    }
    let allowed = matches!(
        (previous.status, proposed.status),
        (
            ReleaseStatus::Prepared,
            ReleaseStatus::Active | ReleaseStatus::Withdrawn
        ) | (
            ReleaseStatus::Active,
            ReleaseStatus::Withdrawn | ReleaseStatus::Expired
        )
    );
    if !allowed {
        return Err("invalid release status transition".to_owned());
    }
    validate_release_times(proposed)
}

fn validate_channel(payload: &ChannelPayload) -> Result<(), String> {
    validate_canonical_text(&payload.profile, "channel profile")?;
    if payload.capabilities.is_empty()
        || payload
            .capabilities
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err("channel capabilities must be non-empty, sorted, and unique".to_owned());
    }
    if !payload.capabilities.iter().any(|capability| {
        matches!(
            capability,
            ChannelCapability::AddressedDelivery
                | ChannelCapability::AudienceDelivery
                | ChannelCapability::OpenReception
        )
    }) {
        return Err("channel requires at least one delivery capability".to_owned());
    }
    Ok(())
}

fn validate_body(body: &InformationBody, limits: InformationLimitsV1) -> Result<(), String> {
    match body {
        InformationBody::InlineJson { value } => {
            let size = serde_json::to_vec(value).map_err(stringify)?.len() as u64;
            if size > limits.max_inline_body_bytes {
                return Err("inline information body exceeds its canonical byte limit".to_owned());
            }
        }
        InformationBody::Resource {
            digest,
            media_type,
            byte_length,
        } => {
            validate_canonical_text(media_type, "resource media type")?;
            if *byte_length > limits.max_external_resource_bytes {
                return Err("external resource exceeds its declared byte limit".to_owned());
            }
            if !is_lower_hex_32_bytes(&digest.value) {
                return Err("resource digest must be 32-byte lowercase hexadecimal".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_dispatch_shape(
    records: &InformationRecordSet,
    payload: &DispatchPayload,
    binding: &RecordBinding<Dispatch>,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    if payload.status != DispatchStatus::Prepared {
        return Err("new dispatch must begin prepared".to_owned());
    }
    validate_dispatch_times(payload)?;
    let channel = single_typed_role::<Channel>(&binding.references, "channel")?;
    require_single_role(binding, "representation")?;
    let channel_payload = records
        .required(&channel)?
        .decode_payload::<Channel>()
        .map_err(stringify)?;
    let source_instance = optional_typed_role::<Instance>(&binding.references, "source_instance")?;
    if source_instance.is_some()
        && !channel_payload
            .capabilities
            .contains(&ChannelCapability::PersistentInstance)
    {
        return Err("channel does not support a persistent source instance".to_owned());
    }
    let recipients = role_targets(&binding.references, "intended_recipient");
    match &payload.target {
        DispatchTarget::Addressed(holders) => {
            if !channel_payload
                .capabilities
                .contains(&ChannelCapability::AddressedDelivery)
            {
                return Err("channel does not support addressed delivery".to_owned());
            }
            if holders.is_empty() || holders.len() > limits.max_addressed_recipients as usize {
                return Err("addressed dispatch recipient count is outside limits".to_owned());
            }
            if !is_sorted_unique(holders) {
                return Err("addressed dispatch recipients must be sorted and unique".to_owned());
            }
            let expected: BTreeSet<_> = holders.iter().map(holder_entity).collect();
            let actual: BTreeSet<_> = recipients.into_iter().map(target_entity).collect();
            if expected != actual {
                return Err(
                    "addressed target must exactly match intended-recipient references".to_owned(),
                );
            }
        }
        DispatchTarget::Audience(audience) => {
            if !channel_payload
                .capabilities
                .contains(&ChannelCapability::AudienceDelivery)
            {
                return Err("channel does not support audience delivery".to_owned());
            }
            if !recipients.is_empty()
                || optional_typed_role::<Audience>(&binding.references, "audience")?.as_ref()
                    != Some(audience)
            {
                return Err(
                    "audience dispatch requires its audience and no intended recipients".to_owned(),
                );
            }
        }
        DispatchTarget::Open => {
            if !channel_payload
                .capabilities
                .contains(&ChannelCapability::OpenReception)
            {
                return Err("channel does not support open reception".to_owned());
            }
            if !recipients.is_empty() {
                return Err("open dispatch cannot carry intended recipients".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_delivery_attempt_shape(
    records: &InformationRecordSet,
    payload: &DeliveryAttemptPayload,
    binding: &RecordBinding<DeliveryAttempt>,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    if payload.status != DeliveryAttemptStatus::Prepared || payload.attempt_number == 0 {
        return Err("new delivery attempt must be prepared and one-based".to_owned());
    }
    validate_delivery_attempt_times(payload)?;
    let dispatch = single_typed_role::<Dispatch>(&binding.references, "dispatch")?;
    let recipient = single_holder_role(&binding.references, "recipient")?;
    let dispatch_record = records.required(&dispatch)?;
    let dispatch_payload = dispatch_record
        .decode_payload::<Dispatch>()
        .map_err(stringify)?;
    if dispatch_payload.status != DispatchStatus::Active {
        return Err("new delivery attempt requires an active dispatch".to_owned());
    }
    if let DispatchTarget::Addressed(recipients) = &dispatch_payload.target
        && recipients.binary_search(&recipient).is_err()
    {
        return Err("delivery-attempt recipient is not addressed by its dispatch".to_owned());
    }
    let previous = optional_typed_role::<DeliveryAttempt>(&binding.references, "previous_attempt")?;
    if payload.attempt_number == 1 {
        if previous.is_some() {
            return Err("first delivery attempt cannot name a previous attempt".to_owned());
        }
    } else {
        let previous = previous.ok_or_else(|| {
            "later delivery attempt must name the immediately previous attempt".to_owned()
        })?;
        let previous_record = records.required(&previous)?;
        let previous_payload = previous_record
            .decode_payload::<DeliveryAttempt>()
            .map_err(stringify)?;
        let previous_dispatch =
            single_typed_role::<Dispatch>(&previous_record.references, "dispatch")?;
        let previous_recipient = single_holder_role(&previous_record.references, "recipient")?;
        if previous_payload.attempt_number + 1 != payload.attempt_number
            || previous_dispatch != dispatch
            || previous_recipient != recipient
        {
            return Err(
                "delivery attempts must form a contiguous per-dispatch/recipient chain".to_owned(),
            );
        }
    }
    if payload.attempt_number > limits.max_delivery_attempts_per_recipient {
        return Err("delivery-attempt number exceeds its per-recipient limit".to_owned());
    }
    Ok(())
}

fn validate_dispatch_completion(
    records: &InformationRecordSet,
    dispatch: &TypedDomainRecordRef<Dispatch>,
    previous: &DispatchPayload,
) -> Result<(), String> {
    let DispatchTarget::Addressed(recipients) = &previous.target else {
        return Ok(());
    };
    for recipient in recipients {
        let attempts = records.delivery_attempts(dispatch, Some(recipient))?;
        let latest = attempts.last().ok_or_else(|| {
            "addressed dispatch cannot complete without a recipient disposition".to_owned()
        })?;
        let payload = latest
            .decode_payload::<DeliveryAttempt>()
            .map_err(stringify)?;
        if !payload.status.is_terminal() {
            return Err("addressed dispatch cannot complete with pending attempts".to_owned());
        }
    }
    Ok(())
}

fn validate_audience(
    payload: &AudiencePayload,
    binding: &RecordBinding<Audience>,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    if payload.resolution_version == 0
        || payload.member_count == 0
        || !is_lower_hex_32_bytes(&payload.membership_root)
    {
        return Err("audience identity, count, and membership root are invalid".to_owned());
    }
    let members = role_targets(&binding.references, "member");
    let member_holders = members
        .iter()
        .cloned()
        .map(holder_from_target)
        .collect::<Vec<_>>();
    let groups = role_targets(&binding.references, "group");
    match payload.membership {
        AudienceMembership::ExplicitMembers => {
            if !groups.is_empty()
                || members.len() as u64 != payload.member_count
                || members.len() > limits.max_explicit_audience_members as usize
                || !is_sorted_unique(&member_holders)
            {
                return Err(
                    "explicit audience requires a bounded complete unique member list".to_owned(),
                );
            }
            if audience_membership_root_v1(&member_holders, limits)? != payload.membership_root {
                return Err("explicit audience members do not match the committed root".to_owned());
            }
        }
        AudienceMembership::ResolvedGroupSnapshot => {
            if groups.is_empty()
                || groups.len() > limits.max_resolved_group_references as usize
                || !is_sorted_unique(&groups)
            {
                return Err("resolved audience requires bounded unique group references".to_owned());
            }
            if !members.is_empty()
                && (members.len() as u64 != payload.member_count
                    || members.len() > limits.max_explicit_audience_members as usize
                    || !is_sorted_unique(&member_holders))
            {
                return Err(
                    "stored resolved-audience members must be a bounded complete unique snapshot"
                        .to_owned(),
                );
            }
            if !members.is_empty()
                && audience_membership_root_v1(&member_holders, limits)? != payload.membership_root
            {
                return Err(
                    "stored resolved-audience members do not match the committed root".to_owned(),
                );
            }
        }
    }
    Ok(())
}

fn validate_release_shape(
    payload: &ReleasePayload,
    binding: &RecordBinding<Release>,
) -> Result<(), String> {
    if payload.status != ReleaseStatus::Prepared {
        return Err("new release must begin prepared".to_owned());
    }
    validate_release_times(payload)?;
    require_single_role(binding, "representation")?;
    let audience = optional_typed_role::<Audience>(&binding.references, "audience")?;
    match payload.scope {
        ReleaseScope::Audience if audience.is_none() => {
            Err("audience release requires one audience".to_owned())
        }
        ReleaseScope::OpenAvailability if audience.is_some() => {
            Err("open release cannot name an audience".to_owned())
        }
        ReleaseScope::Audience | ReleaseScope::OpenAvailability => Ok(()),
    }
}

fn validate_context_representation(
    records: &InformationRecordSet,
    context: &AccessContext,
) -> Result<(), String> {
    if let Some(instance) = &context.instance {
        let record = records.required(instance)?;
        if single_typed_role::<Representation>(&record.references, "representation")?
            != context.representation
        {
            return Err("access instance carries another representation".to_owned());
        }
    }
    if let Some(dispatch) = &context.dispatch {
        let record = records.required(dispatch)?;
        if single_typed_role::<Representation>(&record.references, "representation")?
            != context.representation
        {
            return Err("access dispatch carries another representation".to_owned());
        }
    }
    if let Some(attempt) = &context.delivery_attempt {
        let record = records.required(attempt)?;
        let dispatch = single_typed_role::<Dispatch>(&record.references, "dispatch")?;
        let dispatch_record = records.required(&dispatch)?;
        if single_typed_role::<Representation>(&dispatch_record.references, "representation")?
            != context.representation
        {
            return Err("access delivery attempt carries another representation".to_owned());
        }
    }
    if let Some(release) = &context.release {
        let record = records.required(release)?;
        if single_typed_role::<Representation>(&record.references, "representation")?
            != context.representation
        {
            return Err("access release carries another representation".to_owned());
        }
    }
    Ok(())
}

fn validate_dispatch_times(payload: &DispatchPayload) -> Result<(), String> {
    match payload.status {
        DispatchStatus::Prepared => {
            if payload.dispatched_at.is_some() || payload.completed_at.is_some() {
                return Err("prepared dispatch cannot carry dispatch/completion time".to_owned());
            }
        }
        DispatchStatus::Active => {
            if payload.dispatched_at.is_none() || payload.completed_at.is_some() {
                return Err("active dispatch requires dispatch time and no completion".to_owned());
            }
        }
        DispatchStatus::Completed => {
            let dispatched = payload
                .dispatched_at
                .ok_or_else(|| "terminal dispatch requires a dispatch time".to_owned())?;
            let completed = payload
                .completed_at
                .ok_or_else(|| "terminal dispatch requires a completion time".to_owned())?;
            if completed < dispatched || dispatched < payload.prepared_at {
                return Err("dispatch timestamps are not monotonic".to_owned());
            }
        }
        DispatchStatus::Cancelled => {
            let completed = payload
                .completed_at
                .ok_or_else(|| "cancelled dispatch requires a completion time".to_owned())?;
            if completed < payload.prepared_at
                || payload.dispatched_at.is_some_and(|dispatched| {
                    completed < dispatched || dispatched < payload.prepared_at
                })
            {
                return Err("cancelled dispatch timestamps are not monotonic".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_delivery_attempt_times(payload: &DeliveryAttemptPayload) -> Result<(), String> {
    match payload.status {
        DeliveryAttemptStatus::Prepared => {
            if payload.dispatched_at.is_some() || payload.completed_at.is_some() {
                return Err("prepared attempt cannot carry dispatch/completion time".to_owned());
            }
        }
        DeliveryAttemptStatus::InTransit => {
            let dispatched = payload
                .dispatched_at
                .ok_or_else(|| "in-transit attempt requires dispatch time".to_owned())?;
            if payload.completed_at.is_some()
                || dispatched < payload.prepared_at
                || payload.due_at < dispatched
            {
                return Err("in-transit attempt timestamps are invalid".to_owned());
            }
        }
        DeliveryAttemptStatus::Delivered => {
            let dispatched = payload
                .dispatched_at
                .ok_or_else(|| "delivered attempt requires dispatch time".to_owned())?;
            let completed = payload
                .completed_at
                .ok_or_else(|| "delivered attempt requires completion time".to_owned())?;
            if dispatched < payload.prepared_at
                || payload.due_at < dispatched
                || completed < dispatched
            {
                return Err("terminal attempt timestamps are not monotonic".to_owned());
            }
        }
        DeliveryAttemptStatus::Failed | DeliveryAttemptStatus::Cancelled => {
            let completed = payload
                .completed_at
                .ok_or_else(|| "terminal attempt requires completion time".to_owned())?;
            if completed < payload.prepared_at
                || payload.dispatched_at.is_some_and(|dispatched| {
                    dispatched < payload.prepared_at
                        || payload.due_at < dispatched
                        || completed < dispatched
                })
            {
                return Err("terminal attempt timestamps are not monotonic".to_owned());
            }
        }
    }
    Ok(())
}

fn validate_release_times(payload: &ReleasePayload) -> Result<(), String> {
    match payload.status {
        ReleaseStatus::Prepared => {
            if payload.active_at.is_some() {
                return Err("prepared release cannot carry activation time".to_owned());
            }
        }
        ReleaseStatus::Active | ReleaseStatus::Withdrawn | ReleaseStatus::Expired => {
            if payload
                .active_at
                .is_some_and(|active| active < payload.prepared_at)
            {
                return Err("release activation cannot precede preparation".to_owned());
            }
            if payload.status == ReleaseStatus::Active && payload.active_at.is_none() {
                return Err("active release requires activation time".to_owned());
            }
        }
    }
    Ok(())
}

fn create<T>(
    binding: &RecordBinding<T>,
    payload: &T::Payload,
) -> Result<InformationMutationPlan, String>
where
    T: DomainRecordType,
    T::Payload: Serialize,
{
    let mut draft =
        DomainRecordDraft::from_typed(binding.reference.clone(), payload).map_err(stringify)?;
    draft.references.clone_from(&binding.references);
    Ok(InformationMutationPlan {
        mutations: vec![DomainRecordMutation::Create { record: draft }],
        publications: Vec::new(),
    })
}

fn update<T>(
    reference: &TypedDomainRecordRef<T>,
    payload: &T::Payload,
    references: Vec<DomainReference>,
    expected_version: u64,
) -> Result<InformationMutationPlan, String>
where
    T: DomainRecordType,
    T::Payload: Serialize,
{
    let mut draft = DomainRecordDraft::from_typed(reference.clone(), payload).map_err(stringify)?;
    draft.references = references;
    Ok(InformationMutationPlan {
        mutations: vec![DomainRecordMutation::Update {
            record: draft,
            expected_version,
        }],
        publications: Vec::new(),
    })
}

fn require_single_role<T: DomainRecordType>(
    binding: &RecordBinding<T>,
    role: &str,
) -> Result<(), String> {
    if role_targets(&binding.references, role).len() != 1 {
        return Err(format!(
            "information record requires exactly one {role} reference"
        ));
    }
    Ok(())
}

fn optional_typed_role<T: DomainRecordType>(
    references: &[DomainReference],
    role: &str,
) -> Result<Option<TypedDomainRecordRef<T>>, String> {
    let values = role_typed_refs::<T>(references, role)?;
    if values.len() > 1 {
        return Err(format!(
            "information record allows at most one {role} reference"
        ));
    }
    Ok(values.into_iter().next())
}

fn single_typed_role<T: DomainRecordType>(
    references: &[DomainReference],
    role: &str,
) -> Result<TypedDomainRecordRef<T>, String> {
    optional_typed_role(references, role)?
        .ok_or_else(|| format!("information record requires one {role} reference"))
}

fn role_typed_refs<T: DomainRecordType>(
    references: &[DomainReference],
    role: &str,
) -> Result<Vec<TypedDomainRecordRef<T>>, String> {
    role_domain_refs(references, role)?
        .into_iter()
        .map(|reference| {
            TypedDomainRecordRef::from_untyped(reference)
                .map_err(|_| format!("{role} reference has the wrong record kind"))
        })
        .collect()
}

fn role_domain_refs(
    references: &[DomainReference],
    role: &str,
) -> Result<Vec<canwu_api::DomainRecordRef>, String> {
    role_targets(references, role)
        .into_iter()
        .map(|target| match target {
            DomainReferenceTarget::Domain(reference) => Ok(reference),
            DomainReferenceTarget::Core(_) => {
                Err(format!("{role} requires a domain-record reference"))
            }
        })
        .collect()
}

fn role_targets(references: &[DomainReference], role: &str) -> Vec<DomainReferenceTarget> {
    references
        .iter()
        .filter(|reference| reference.role == role)
        .map(|reference| reference.target.clone())
        .collect()
}

fn single_entity_role(references: &[DomainReference], role: &str) -> Result<EntityRef, String> {
    let values = role_targets(references, role);
    if values.len() != 1 {
        return Err(format!(
            "information record requires exactly one {role} entity reference"
        ));
    }
    Ok(target_entity(
        values.into_iter().next().expect("length was checked"),
    ))
}

fn single_holder_role(
    references: &[DomainReference],
    role: &str,
) -> Result<KnowledgeHolderRef, String> {
    let values = role_targets(references, role);
    if values.len() != 1 {
        return Err(format!(
            "information record requires exactly one {role} holder reference"
        ));
    }
    Ok(holder_from_target(
        values.into_iter().next().expect("length was checked"),
    ))
}

fn holder_from_target(target: DomainReferenceTarget) -> KnowledgeHolderRef {
    match target {
        DomainReferenceTarget::Core(EntityRef::Person(person)) => {
            KnowledgeHolderRef::Person(person)
        }
        DomainReferenceTarget::Core(entity) => KnowledgeHolderRef::Entity(entity),
        DomainReferenceTarget::Domain(reference) => {
            KnowledgeHolderRef::Entity(EntityRef::Domain(reference))
        }
    }
}

fn retain_role(references: &[DomainReference], role: &str) -> Vec<DomainReference> {
    references
        .iter()
        .filter(|reference| reference.role == role)
        .cloned()
        .collect()
}

fn push_entity_role(references: &mut Vec<DomainReference>, role: &str, entity: Option<EntityRef>) {
    if let Some(entity) = entity {
        references.push(DomainReference {
            role: role.to_owned(),
            target: match entity {
                EntityRef::Domain(reference) => DomainReferenceTarget::Domain(reference),
                core => DomainReferenceTarget::Core(core),
            },
        });
    }
    references.sort();
    references.dedup();
}

fn holder_entity(holder: &KnowledgeHolderRef) -> EntityRef {
    match holder {
        KnowledgeHolderRef::Person(person) => EntityRef::Person(*person),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    }
}

fn target_entity(target: DomainReferenceTarget) -> EntityRef {
    match target {
        DomainReferenceTarget::Core(entity) => entity,
        DomainReferenceTarget::Domain(reference) => EntityRef::Domain(reference),
    }
}

fn validate_per_mille(value: u16, label: &str) -> Result<(), String> {
    if value > 1_000 {
        return Err(format!("{label} must not exceed 1000"));
    }
    Ok(())
}

fn validate_canonical_text(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value {
        return Err(format!("{label} must be non-empty trimmed text"));
    }
    Ok(())
}

fn validate_unique_refs<T: Ord + Clone>(values: &[T], label: &str) -> Result<(), String> {
    let unique: BTreeSet<_> = values.iter().cloned().collect();
    if unique.len() != values.len() {
        return Err(format!("{label} values must be unique"));
    }
    Ok(())
}

fn as_set<T: Ord>(values: Vec<T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn is_sorted_unique<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn is_lower_hex_32_bytes(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{PersonId, SimTime};

    #[test]
    fn terminal_states_never_reopen() {
        let destroyed = InstancePayload {
            created_at: SimTime::EPOCH,
            status: InstanceStatus::Destroyed,
        };
        let available = InstancePayload {
            created_at: SimTime::EPOCH,
            status: InstanceStatus::Available,
        };
        assert!(validate_instance_transition(&destroyed, &available).is_err());

        let withdrawn = ReleasePayload {
            status: ReleaseStatus::Withdrawn,
            scope: ReleaseScope::OpenAvailability,
            prepared_at: SimTime::EPOCH,
            active_at: None,
        };
        let active = ReleasePayload {
            status: ReleaseStatus::Active,
            active_at: Some(SimTime::EPOCH),
            ..withdrawn.clone()
        };
        assert!(validate_release_transition(&withdrawn, &active).is_err());
    }

    #[test]
    fn content_source_edges_and_reference_roles_must_match() {
        let source = TypedDomainRecordRef::<Content>::new("source");
        let binding = RecordBinding::new(
            TypedDomainRecordRef::<Content>::new("child"),
            vec![DomainReference::from_typed(
                "source_content",
                source.clone(),
            )],
        );
        let payload = ContentPayload {
            content_type: "structured_payload".to_owned(),
            body: InformationBody::InlineJson {
                value: serde_json::json!({"value": 1}),
            },
            created_at: SimTime::EPOCH,
            derivation: Some(crate::ContentDerivation {
                operation: "selection".to_owned(),
                sources: vec![crate::ContentSourceEdge {
                    source,
                    role: crate::ContentSourceRole::Quotation,
                    completeness_per_mille: 500,
                    fidelity_per_mille: 1_000,
                }],
            }),
        };
        assert!(validate_content_lineage(&payload, &binding).is_ok());
    }

    #[test]
    fn audience_membership_v1_rejects_root_proof_and_odd_node_tampering() {
        let holders = [
            KnowledgeHolderRef::Person(PersonId::new(1)),
            KnowledgeHolderRef::Person(PersonId::new(2)),
            KnowledgeHolderRef::Person(PersonId::new(3)),
        ];
        let limits = InformationLimitsV1::canonical();
        let root = audience_membership_root_v1(&holders, limits).expect("membership root");
        let leaves = holders
            .iter()
            .map(audience_member_leaf_hash)
            .collect::<Result<Vec<_>, _>>()
            .expect("membership leaves");
        let left_parent = audience_member_node_hash(&leaves[0], &leaves[1]).expect("left parent");
        let proof = crate::AudienceMembershipProofV1 {
            holder: holders[2].clone(),
            leaf_index: 2,
            member_count: 3,
            sibling_hashes: vec![encode_hash(&leaves[2]), encode_hash(&left_parent)],
        };
        let audience = AudiencePayload {
            membership: AudienceMembership::ResolvedGroupSnapshot,
            resolved_at: SimTime::EPOCH,
            resolution_version: 1,
            resolved_boundary: None,
            member_count: 3,
            membership_root: root.clone(),
        };
        assert!(
            verify_audience_membership_proof_v1(&proof, &holders[2], &audience, limits).is_ok()
        );

        let mut tampered = proof.clone();
        tampered.sibling_hashes[0] = encode_hash(&leaves[1]);
        assert!(
            verify_audience_membership_proof_v1(&tampered, &holders[2], &audience, limits).is_err()
        );
        let mut tampered = proof.clone();
        tampered.sibling_hashes.pop();
        assert!(
            verify_audience_membership_proof_v1(&tampered, &holders[2], &audience, limits).is_err()
        );
        let mut tampered = proof.clone();
        tampered.leaf_index = 3;
        assert!(
            verify_audience_membership_proof_v1(&tampered, &holders[2], &audience, limits).is_err()
        );
        let mut tampered = proof.clone();
        tampered.member_count = 4;
        assert!(
            verify_audience_membership_proof_v1(&tampered, &holders[2], &audience, limits).is_err()
        );
        let mut tampered_audience = audience.clone();
        tampered_audience.membership_root = "0".repeat(64);
        assert!(
            verify_audience_membership_proof_v1(&proof, &holders[2], &tampered_audience, limits,)
                .is_err()
        );
        let mut tampered = proof.clone();
        tampered.sibling_hashes[0].make_ascii_uppercase();
        assert!(
            verify_audience_membership_proof_v1(&tampered, &holders[2], &audience, limits).is_err()
        );
        let mut small_bytes = limits;
        small_bytes.max_membership_proof_bytes = 1;
        assert!(
            verify_audience_membership_proof_v1(&proof, &holders[2], &audience, small_bytes)
                .is_err()
        );

        let binding = RecordBinding::new(
            TypedDomainRecordRef::<Audience>::new("explicit-members"),
            holders
                .iter()
                .map(|holder| DomainReference {
                    role: "member".to_owned(),
                    target: DomainReferenceTarget::Core(holder_entity(holder)),
                })
                .collect(),
        );
        let explicit = AudiencePayload {
            membership: AudienceMembership::ExplicitMembers,
            ..audience
        };
        assert!(validate_audience(&explicit, &binding, limits).is_ok());
        let mismatched = AudiencePayload {
            membership_root: "f".repeat(64),
            ..explicit
        };
        assert!(validate_audience(&mismatched, &binding, limits).is_err());
    }
}
