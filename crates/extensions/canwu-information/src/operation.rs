use crate::lifecycle::LifecycleRequest;
use crate::model::InformationLimitsV1;
use crate::schema::{
    Access, Audience, Channel, Content, DeliveryAttempt, Dispatch, InformationOperationRecord,
    Instance, Interpretation, Release, Representation,
};
use canwu_api::{
    CommandId, DomainRecordKind, DomainRecordRef, DomainRecordVersionRef, IngressId,
    KnowledgeRecordId, SimTime, TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InformationOperationId {
    pub namespace: String,
    pub value: String,
}

impl InformationOperationId {
    #[must_use]
    pub fn new(namespace: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InformationOutputKind {
    Channel,
    Content,
    Representation,
    Instance,
    Dispatch,
    DeliveryAttempt,
    Access,
    Interpretation,
    Audience,
    Release,
}

impl InformationOutputKind {
    #[must_use]
    pub fn record_kind(self) -> DomainRecordKind {
        match self {
            Self::Channel => DomainRecordKind::for_type::<Channel>(),
            Self::Content => DomainRecordKind::for_type::<Content>(),
            Self::Representation => DomainRecordKind::for_type::<Representation>(),
            Self::Instance => DomainRecordKind::for_type::<Instance>(),
            Self::Dispatch => DomainRecordKind::for_type::<Dispatch>(),
            Self::DeliveryAttempt => DomainRecordKind::for_type::<DeliveryAttempt>(),
            Self::Access => DomainRecordKind::for_type::<Access>(),
            Self::Interpretation => DomainRecordKind::for_type::<Interpretation>(),
            Self::Audience => DomainRecordKind::for_type::<Audience>(),
            Self::Release => DomainRecordKind::for_type::<Release>(),
        }
    }

    #[must_use]
    pub const fn supports_lineage(self) -> bool {
        matches!(self, Self::Content | Self::Representation)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InformationOutputSlot {
    pub index: u16,
    pub name: String,
    pub kind: InformationOutputKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct InformationOutputSlotRef {
    pub index: u16,
    pub kind: InformationOutputKind,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum LineageParent {
    Persisted(DomainRecordRef),
    Output(InformationOutputSlotRef),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationLineageNode {
    pub child: InformationOutputSlotRef,
    pub parents: Vec<LineageParent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InformationOperation {
    pub request: LifecycleRequest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InformationOperationEnvelope {
    pub id: InformationOperationId,
    pub operation_version: u32,
    pub operation_kind: String,
    pub output_slots: Vec<InformationOutputSlot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineage: Vec<OperationLineageNode>,
    pub operation: InformationOperation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InformationOperationStatus {
    Accepted,
    ApplyingDomainChanges,
    AwaitingPublication,
    AwaitingFinalization,
    Completed,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InformationRetryDisposition {
    New,
    ExactRetry,
}

impl InformationOperationStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Rejected)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum InformationAdmissionRef {
    Command(CommandId),
    Ingress(IngressId),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InformationContinuation {
    pub cursor: u64,
    pub remaining: u64,
    pub chunk_size: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InformationOperationPayload {
    pub id: InformationOperationId,
    pub operation_version: u32,
    pub operation_kind: String,
    pub canonical_input_hash: String,
    pub output_slots: Vec<InformationOutputSlot>,
    pub status: InformationOperationStatus,
    pub admitted_at: SimTime,
    pub accepted_cause: InformationAdmissionRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_claim_hash: Option<String>,
    pub domain_result_refs: Vec<DomainRecordRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domain_result_evidence: Vec<DomainRecordVersionRef>,
    pub publication_result_ids: Vec<KnowledgeRecordId>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<InformationContinuation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_code: Option<String>,
}

/// Returns the canonical JSON bytes used as the operation input-hash material.
///
/// Hashing is deliberately left to the engine facade so this crate does not
/// introduce a second canonical-hash implementation.
pub fn canonical_input_bytes(
    envelope: &InformationOperationEnvelope,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(envelope)
}

#[must_use]
pub fn derive_operation_record_ref(
    id: &InformationOperationId,
) -> TypedDomainRecordRef<InformationOperationRecord> {
    TypedDomainRecordRef::new(encode_operation_identity(id))
}

#[must_use]
pub fn derive_output_record_ref(
    operation: &InformationOperationId,
    slot: &InformationOutputSlot,
) -> DomainRecordRef {
    DomainRecordRef {
        kind: slot.kind.record_kind(),
        id: format!(
            "{}::slot:{}:{}",
            encode_operation_identity(operation),
            slot.index,
            slot.name
        ),
    }
}

pub fn validate_operation_envelope(
    envelope: &InformationOperationEnvelope,
    limits: InformationLimitsV1,
) -> Result<(), String> {
    validate_canonical_text(&envelope.id.namespace, "operation namespace")?;
    validate_canonical_text(&envelope.id.value, "operation value")?;
    validate_canonical_text(&envelope.operation_kind, "operation kind")?;
    if envelope.operation_version != 1 {
        return Err("information operation version must equal one".to_owned());
    }
    if derive_operation_record_ref(&envelope.id).id().len() > 256 {
        return Err("information operation identity exceeds the correlation limit".to_owned());
    }
    if envelope.output_slots.len() > limits.max_operation_output_slots as usize {
        return Err(format!(
            "information operation must declare no more than {} output slots",
            limits.max_operation_output_slots
        ));
    }

    let mut expected_index = 0_u16;
    let mut names = BTreeSet::new();
    for slot in &envelope.output_slots {
        if slot.index != expected_index {
            return Err("information output slots must be contiguous and zero-based".to_owned());
        }
        expected_index = expected_index
            .checked_add(1)
            .ok_or_else(|| "information output-slot index exhausted".to_owned())?;
        validate_canonical_text(&slot.name, "output-slot name")?;
        if !names.insert(slot.name.as_str()) {
            return Err("information output-slot names must be unique".to_owned());
        }
    }
    validate_request_contract(envelope)?;
    validate_operation_lineage(&envelope.output_slots, &envelope.lineage, limits)
}

pub fn classify_operation_retry(
    existing: Option<&InformationOperationPayload>,
    operation_id: &InformationOperationId,
    canonical_input_hash: &str,
) -> Result<InformationRetryDisposition, String> {
    if !is_lower_hex_32_bytes(canonical_input_hash) {
        return Err("operation canonical input hash must be lowercase 32-byte hex".to_owned());
    }
    let Some(existing) = existing else {
        return Ok(InformationRetryDisposition::New);
    };
    if &existing.id != operation_id {
        return Err("operation retry lookup returned another operation identity".to_owned());
    }
    if existing.canonical_input_hash == canonical_input_hash {
        Ok(InformationRetryDisposition::ExactRetry)
    } else {
        Err("information operation ID was reused with different canonical input".to_owned())
    }
}

pub fn validate_operation_lineage(
    slots: &[InformationOutputSlot],
    nodes: &[OperationLineageNode],
    limits: InformationLimitsV1,
) -> Result<(), String> {
    let slots_by_index: BTreeMap<_, _> = slots.iter().map(|slot| (slot.index, slot)).collect();
    let mut seen_children = BTreeSet::new();
    for node in nodes {
        let child = slots_by_index
            .get(&node.child.index)
            .ok_or_else(|| "lineage child names an undeclared output slot".to_owned())?;
        if child.kind != node.child.kind || !child.kind.supports_lineage() {
            return Err(
                "lineage child kind must match a content or representation slot".to_owned(),
            );
        }
        if !seen_children.insert(node.child) {
            return Err("an output slot may have only one lineage declaration".to_owned());
        }
        if node.parents.len() > limits.max_parent_edges as usize {
            return Err(format!(
                "lineage node exceeds the {} parent-edge limit",
                limits.max_parent_edges
            ));
        }
        let mut parents = BTreeSet::new();
        for parent in &node.parents {
            if !parents.insert(parent) {
                return Err("lineage parent edges must be unique".to_owned());
            }
            match parent {
                LineageParent::Persisted(reference) => {
                    if reference.kind != child.kind.record_kind() {
                        return Err("persisted lineage parent has the wrong record kind".to_owned());
                    }
                }
                LineageParent::Output(parent) => {
                    let declared = slots_by_index.get(&parent.index).ok_or_else(|| {
                        "lineage parent names an undeclared output slot".to_owned()
                    })?;
                    if declared.kind != parent.kind || parent.kind != child.kind {
                        return Err("output lineage parent has the wrong record kind".to_owned());
                    }
                    if parent.index >= node.child.index {
                        return Err(
                            "output lineage parents must be earlier than their child slot"
                                .to_owned(),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn validate_operation_transition(
    previous: &InformationOperationPayload,
    proposed: &InformationOperationPayload,
) -> Result<(), String> {
    if previous.id != proposed.id
        || previous.operation_version != proposed.operation_version
        || previous.operation_kind != proposed.operation_kind
        || previous.canonical_input_hash != proposed.canonical_input_hash
        || previous.output_slots != proposed.output_slots
        || previous.admitted_at != proposed.admitted_at
        || previous.accepted_cause != proposed.accepted_cause
        || (previous.authority_claim_hash != proposed.authority_claim_hash
            && !(previous.status == InformationOperationStatus::ApplyingDomainChanges
                && previous.authority_claim_hash.is_none()))
        || (previous.domain_result_refs != proposed.domain_result_refs
            && !(previous.status == InformationOperationStatus::ApplyingDomainChanges
                && previous.domain_result_refs.is_empty()))
        || (!previous.domain_result_evidence.is_empty()
            && previous.domain_result_evidence != proposed.domain_result_evidence)
    {
        return Err("an operation transition changed immutable admission fields".to_owned());
    }
    if previous.status.is_terminal() {
        return Err("a terminal information operation cannot transition".to_owned());
    }
    if !is_lower_hex_32_bytes(&previous.canonical_input_hash) {
        return Err("operation canonical input hash must be lowercase 32-byte hex".to_owned());
    }
    validate_continuation(proposed.continuation.as_ref())?;
    let allowed = match previous.status {
        InformationOperationStatus::Accepted => matches!(
            proposed.status,
            InformationOperationStatus::ApplyingDomainChanges
                | InformationOperationStatus::Rejected
        ),
        InformationOperationStatus::ApplyingDomainChanges => matches!(
            proposed.status,
            InformationOperationStatus::AwaitingPublication
                | InformationOperationStatus::Completed
                | InformationOperationStatus::Rejected
        ),
        InformationOperationStatus::AwaitingPublication => matches!(
            proposed.status,
            InformationOperationStatus::AwaitingFinalization
                | InformationOperationStatus::Completed
                | InformationOperationStatus::Rejected
        ),
        InformationOperationStatus::AwaitingFinalization => matches!(
            proposed.status,
            InformationOperationStatus::AwaitingPublication
                | InformationOperationStatus::Completed
                | InformationOperationStatus::Rejected
        ),
        InformationOperationStatus::Completed | InformationOperationStatus::Rejected => false,
    };
    if !allowed {
        return Err("invalid information operation state transition".to_owned());
    }
    if proposed.status == InformationOperationStatus::Completed {
        if proposed.completed_at.is_none()
            || proposed.continuation.is_some()
            || proposed.rejection_code.is_some()
        {
            return Err(
                "completed operation requires completion time and no continuation or rejection"
                    .to_owned(),
            );
        }
    } else if proposed.status == InformationOperationStatus::Rejected {
        if proposed.rejection_code.as_deref().is_none_or(str::is_empty)
            || proposed.continuation.is_some()
        {
            return Err(
                "rejected operation requires a rejection code and no continuation".to_owned(),
            );
        }
    } else if proposed.completed_at.is_some() || proposed.rejection_code.is_some() {
        return Err(
            "non-terminal operation cannot carry completion or rejection fields".to_owned(),
        );
    }
    Ok(())
}

fn validate_request_contract(envelope: &InformationOperationEnvelope) -> Result<(), String> {
    let (kind, created) = request_contract(&envelope.operation.request);
    if envelope.operation_kind != kind {
        return Err("operation kind does not match its closed request variant".to_owned());
    }
    match created {
        Some((expected_kind, expected_reference)) => {
            let [slot] = envelope.output_slots.as_slice() else {
                return Err(
                    "record-creating V1 operation requires exactly one output slot".to_owned(),
                );
            };
            if slot.index != 0 || slot.name != "result" || slot.kind != expected_kind {
                return Err(
                    "record-creating V1 operation requires the canonical result output slot"
                        .to_owned(),
                );
            }
            if derive_output_record_ref(&envelope.id, slot) != expected_reference {
                return Err(
                    "operation create binding does not use its derived output ID".to_owned(),
                );
            }
        }
        None if !envelope.output_slots.is_empty() => {
            return Err(
                "record-transition V1 operation cannot declare new output slots".to_owned(),
            );
        }
        None => {}
    }
    if envelope.output_slots.is_empty() && !envelope.lineage.is_empty() {
        return Err("operation without created outputs cannot declare lineage".to_owned());
    }
    Ok(())
}

fn request_contract(
    request: &LifecycleRequest,
) -> (
    &'static str,
    Option<(InformationOutputKind, DomainRecordRef)>,
) {
    match request {
        LifecycleRequest::CreateChannel { binding, .. } => (
            "create_channel",
            Some((
                InformationOutputKind::Channel,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::CreateContent { binding, .. } => (
            "create_content",
            Some((
                InformationOutputKind::Content,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::CreateRepresentation { binding, .. } => (
            "create_representation",
            Some((
                InformationOutputKind::Representation,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::CreateInstance { binding, .. } => (
            "create_instance",
            Some((
                InformationOutputKind::Instance,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::TransitionInstance { .. } => ("transition_instance", None),
        LifecycleRequest::BeginDispatch { binding, .. } => (
            "begin_dispatch",
            Some((
                InformationOutputKind::Dispatch,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::TransitionDispatch { .. } => ("transition_dispatch", None),
        LifecycleRequest::BeginDeliveryAttempt { binding, .. } => (
            "begin_delivery_attempt",
            Some((
                InformationOutputKind::DeliveryAttempt,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::TransitionDeliveryAttempt { .. } => ("transition_delivery_attempt", None),
        LifecycleRequest::RecordAccess { binding, .. } => (
            "record_access",
            Some((
                InformationOutputKind::Access,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::RecordInterpretation { binding, .. } => (
            "record_interpretation",
            Some((
                InformationOutputKind::Interpretation,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::CreateAudience { binding, .. } => (
            "create_audience",
            Some((
                InformationOutputKind::Audience,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::CreateRelease { binding, .. } => (
            "create_release",
            Some((
                InformationOutputKind::Release,
                binding.reference.as_untyped().clone(),
            )),
        ),
        LifecycleRequest::TransitionRelease { .. } => ("transition_release", None),
    }
}

fn validate_continuation(continuation: Option<&InformationContinuation>) -> Result<(), String> {
    if continuation
        .is_some_and(|continuation| continuation.remaining == 0 || continuation.chunk_size == 0)
    {
        return Err(
            "information continuation requires positive remaining work and chunk size".to_owned(),
        );
    }
    Ok(())
}

fn encode_operation_identity(id: &InformationOperationId) -> String {
    format!(
        "operation:{}:{}:{}:{}",
        id.namespace.len(),
        id.namespace,
        id.value.len(),
        id.value
    )
}

fn validate_canonical_text(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value {
        return Err(format!("{label} must be non-empty trimmed text"));
    }
    Ok(())
}

fn is_lower_hex_32_bytes(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(index: u16, kind: InformationOutputKind) -> InformationOutputSlot {
        InformationOutputSlot {
            index,
            name: format!("output-{index}"),
            kind,
        }
    }

    #[test]
    fn lineage_accepts_only_backward_same_kind_output_edges() {
        let slots = vec![
            slot(0, InformationOutputKind::Content),
            slot(1, InformationOutputKind::Content),
        ];
        let valid = vec![OperationLineageNode {
            child: InformationOutputSlotRef {
                index: 1,
                kind: InformationOutputKind::Content,
            },
            parents: vec![LineageParent::Output(InformationOutputSlotRef {
                index: 0,
                kind: InformationOutputKind::Content,
            })],
        }];
        assert!(validate_operation_lineage(&slots, &valid, InformationLimitsV1::default()).is_ok());

        let forward = vec![OperationLineageNode {
            child: InformationOutputSlotRef {
                index: 0,
                kind: InformationOutputKind::Content,
            },
            parents: vec![LineageParent::Output(InformationOutputSlotRef {
                index: 1,
                kind: InformationOutputKind::Content,
            })],
        }];
        assert!(
            validate_operation_lineage(&slots, &forward, InformationLimitsV1::default()).is_err()
        );
    }

    #[test]
    fn derived_record_ids_are_stable_and_kind_scoped() {
        let id = InformationOperationId::new("fixture.app", "request-7");
        let content = derive_output_record_ref(&id, &slot(0, InformationOutputKind::Content));
        let access = derive_output_record_ref(&id, &slot(0, InformationOutputKind::Access));
        assert_ne!(content.kind, access.kind);
        assert_eq!(content.id, access.id);
        assert_eq!(
            derive_operation_record_ref(&id).id(),
            "operation:11:fixture.app:9:request-7"
        );
    }
}
