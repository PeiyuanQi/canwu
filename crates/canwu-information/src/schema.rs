use crate::model::{
    AccessPayload, AudiencePayload, AuthorityAssignmentPayload, ChannelPayload, ContentPayload,
    DeliveryAttemptPayload, DispatchPayload, InstancePayload, InterpretationPayload,
    ReleasePayload, RepresentationPayload,
};
use crate::operation::InformationOperationPayload;
use crate::{PLUGIN_NAMESPACE, model::InformationLimitsV1};
use canwu_api::{
    DomainRecordKind, DomainRecordMutationPolicy, DomainRecordSchema, DomainRecordType,
    DomainReferenceSchema, DomainReferenceTargetKind, DomainValueKindClass, DomainValueType,
    KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubjectSchema, KnowledgeSubjectTargetKind,
    PayloadProperty, PayloadSchema, PayloadValueType, PluginKnowledgeSchema,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! information_record_type {
    ($name:ident, $kind:literal, $payload:ty) => {
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name;

        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = DomainValueKindClass;

            const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
            const NAME: &'static str = $kind;
        }
    };
}

information_record_type!(
    AuthorityAssignment,
    "authority_assignment",
    AuthorityAssignmentPayload
);
information_record_type!(Channel, "channel", ChannelPayload);
information_record_type!(Content, "content", ContentPayload);
information_record_type!(Representation, "representation", RepresentationPayload);
information_record_type!(Instance, "instance", InstancePayload);
information_record_type!(Dispatch, "dispatch", DispatchPayload);
information_record_type!(DeliveryAttempt, "delivery_attempt", DeliveryAttemptPayload);
information_record_type!(Access, "access", AccessPayload);
information_record_type!(Interpretation, "interpretation", InterpretationPayload);
information_record_type!(Audience, "audience", AudiencePayload);
information_record_type!(Release, "release", ReleasePayload);
information_record_type!(
    InformationOperationRecord,
    "operation",
    InformationOperationPayload
);

/// Human-readable descriptor for one neutral holder-facing schema.
///
/// The authoritative runtime schemas are fixed by
/// [`information_knowledge_schemas`]; this view never accepts caller-selected
/// knowledge kinds or application-semantic assertions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NeutralKnowledgeSchema {
    pub name: String,
    pub version: u32,
    pub subject_roles: Vec<String>,
    pub payload_schema: PayloadSchema,
}

#[must_use]
pub fn neutral_knowledge_schemas() -> Vec<NeutralKnowledgeSchema> {
    [
        ("access_recorded", vec!["access", "representation"]),
        ("interpretation_recorded", vec!["interpretation"]),
        ("release_available", vec!["release", "representation"]),
        (
            "representation_available",
            vec!["representation", "content"],
        ),
    ]
    .into_iter()
    .map(|(name, roles)| NeutralKnowledgeSchema {
        name: name.to_owned(),
        version: 1,
        subject_roles: roles.into_iter().map(str::to_owned).collect(),
        payload_schema: PayloadSchema::Object {
            properties: BTreeMap::from([(
                "record_version".to_owned(),
                required(PayloadValueType::Integer),
            )]),
            allow_additional: false,
        },
    })
    .collect()
}

/// Runtime knowledge schemas owned by the neutral information extension.
///
/// These schemas expose only the existence and exact version of lifecycle
/// records. They deliberately do not encode application-level conclusions.
#[must_use]
pub fn information_knowledge_schemas() -> Vec<PluginKnowledgeSchema> {
    let record_version = PayloadSchema::Object {
        properties: BTreeMap::from([(
            "record_version".to_owned(),
            required(PayloadValueType::Integer),
        )]),
        allow_additional: false,
    };
    let mut schemas = vec![
        knowledge_schema(
            "access_recorded",
            "07ea062f70d78429368aad217f4dc47cdd1953feb951cbc7d6d93bde7fbc538d",
            record_version.clone(),
            vec![
                knowledge_subject::<Access>("access"),
                knowledge_subject::<Representation>("representation"),
            ],
        ),
        knowledge_schema(
            "interpretation_recorded",
            "f12a1bb18475aff0f250f1a769cf7fbe3c47b64080785c5c94d3d8c5d4e1aad4",
            record_version.clone(),
            vec![knowledge_subject::<Interpretation>("interpretation")],
        ),
        knowledge_schema(
            "release_available",
            "96c35794c4bf6a19f5961b007eed2c71d49011734c4ba2b4fba7d4e19f52e2c8",
            record_version.clone(),
            vec![
                knowledge_subject::<Release>("release"),
                knowledge_subject::<Representation>("representation"),
            ],
        ),
        knowledge_schema(
            "representation_available",
            "b6327b8c785be0d62cee8180b817496c31a6b41d4aa0f11cd6da517295a9d333",
            record_version,
            vec![
                knowledge_subject::<Content>("content"),
                knowledge_subject::<Representation>("representation"),
            ],
        ),
    ];
    schemas.sort_by(|left, right| left.id.cmp(&right.id));
    schemas
}

#[must_use]
pub fn information_record_schemas() -> Vec<DomainRecordSchema> {
    let mut schemas = vec![
        record_schema::<AuthorityAssignment>(
            true,
            object_schema(&[("claim", PayloadValueType::Object, true)]),
            vec![],
        ),
        record_schema::<Channel>(
            true,
            object_schema(&[
                ("capabilities", PayloadValueType::Array, true),
                ("profile", PayloadValueType::String, true),
            ]),
            vec![],
        ),
        record_schema::<Content>(
            true,
            object_schema(&[
                ("body", PayloadValueType::Object, true),
                ("content_type", PayloadValueType::String, true),
                ("created_at", PayloadValueType::Integer, true),
                ("derivation", PayloadValueType::Object, false),
            ]),
            vec![
                reference(
                    "creator",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
                reference(
                    "source_content",
                    vec![DomainReferenceTargetKind::for_domain::<Content>()],
                    false,
                    true,
                ),
            ],
        ),
        record_schema::<Representation>(
            true,
            object_schema(&[
                ("claimed_source", PayloadValueType::Object, false),
                ("content_relation", PayloadValueType::String, true),
                ("created_at", PayloadValueType::Integer, true),
                ("format", PayloadValueType::String, true),
                ("interpretation_capability", PayloadValueType::String, false),
                ("operation", PayloadValueType::String, true),
                ("sources", PayloadValueType::Array, true),
            ]),
            vec![
                reference(
                    "content",
                    vec![DomainReferenceTargetKind::for_domain::<Content>()],
                    true,
                    false,
                ),
                reference(
                    "parent_representation",
                    vec![DomainReferenceTargetKind::for_domain::<Representation>()],
                    false,
                    true,
                ),
                reference(
                    "producer",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
            ],
        ),
        record_schema::<Instance>(
            false,
            object_schema(&[
                ("created_at", PayloadValueType::Integer, true),
                ("status", PayloadValueType::String, true),
            ]),
            vec![
                reference(
                    "representation",
                    vec![DomainReferenceTargetKind::for_domain::<Representation>()],
                    true,
                    false,
                ),
                reference(
                    "custodian",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
                reference(
                    "location",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
            ],
        ),
        record_schema::<Dispatch>(
            false,
            object_schema(&[
                ("completed_at", PayloadValueType::Integer, false),
                ("dispatched_at", PayloadValueType::Integer, false),
                ("prepared_at", PayloadValueType::Integer, true),
                ("status", PayloadValueType::String, true),
                ("target", PayloadValueType::Object, true),
            ]),
            vec![
                reference(
                    "audience",
                    vec![DomainReferenceTargetKind::for_domain::<Audience>()],
                    false,
                    false,
                ),
                reference(
                    "channel",
                    vec![DomainReferenceTargetKind::for_domain::<Channel>()],
                    true,
                    false,
                ),
                reference(
                    "intended_recipient",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    true,
                ),
                reference(
                    "representation",
                    vec![DomainReferenceTargetKind::for_domain::<Representation>()],
                    true,
                    false,
                ),
                reference(
                    "sender",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
                reference(
                    "source_instance",
                    vec![DomainReferenceTargetKind::for_domain::<Instance>()],
                    false,
                    false,
                ),
            ],
        ),
        record_schema::<DeliveryAttempt>(
            false,
            object_schema(&[
                ("attempt_number", PayloadValueType::Integer, true),
                ("completed_at", PayloadValueType::Integer, false),
                ("dispatched_at", PayloadValueType::Integer, false),
                ("due_at", PayloadValueType::Integer, true),
                ("prepared_at", PayloadValueType::Integer, true),
                ("status", PayloadValueType::String, true),
            ]),
            vec![
                reference(
                    "dispatch",
                    vec![DomainReferenceTargetKind::for_domain::<Dispatch>()],
                    true,
                    false,
                ),
                reference(
                    "previous_attempt",
                    vec![DomainReferenceTargetKind::for_domain::<DeliveryAttempt>()],
                    false,
                    false,
                ),
                reference(
                    "recipient",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    true,
                    false,
                ),
                reference(
                    "relay",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
            ],
        ),
        record_schema::<Access>(
            true,
            object_schema(&[
                ("accessed_at", PayloadValueType::Integer, true),
                ("extent_per_mille", PayloadValueType::Integer, true),
                ("method", PayloadValueType::String, true),
            ]),
            vec![
                reference(
                    "delivery_attempt",
                    vec![DomainReferenceTargetKind::for_domain::<DeliveryAttempt>()],
                    false,
                    false,
                ),
                reference(
                    "dispatch",
                    vec![DomainReferenceTargetKind::for_domain::<Dispatch>()],
                    false,
                    false,
                ),
                reference(
                    "holder",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    true,
                    false,
                ),
                reference(
                    "instance",
                    vec![DomainReferenceTargetKind::for_domain::<Instance>()],
                    false,
                    false,
                ),
                reference(
                    "release",
                    vec![DomainReferenceTargetKind::for_domain::<Release>()],
                    false,
                    false,
                ),
                reference(
                    "representation",
                    vec![DomainReferenceTargetKind::for_domain::<Representation>()],
                    true,
                    false,
                ),
            ],
        ),
        record_schema::<Interpretation>(
            true,
            object_schema(&[
                ("capability", PayloadValueType::String, true),
                ("confidence_per_mille", PayloadValueType::Integer, true),
                ("interpreted_at", PayloadValueType::Integer, true),
                ("status", PayloadValueType::String, true),
            ]),
            vec![
                reference(
                    "input_access",
                    vec![DomainReferenceTargetKind::for_domain::<Access>()],
                    true,
                    true,
                ),
                reference(
                    "input_representation",
                    vec![DomainReferenceTargetKind::for_domain::<Representation>()],
                    true,
                    true,
                ),
                reference(
                    "performed_by",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    true,
                    false,
                ),
                reference(
                    "performed_for",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    true,
                    false,
                ),
                reference(
                    "result_content",
                    vec![DomainReferenceTargetKind::for_domain::<Content>()],
                    false,
                    false,
                ),
            ],
        ),
        record_schema::<Audience>(
            true,
            object_schema(&[
                ("member_count", PayloadValueType::Integer, true),
                ("membership", PayloadValueType::String, true),
                ("membership_root", PayloadValueType::String, true),
                ("resolution_version", PayloadValueType::Integer, true),
                ("resolved_at", PayloadValueType::Integer, true),
                ("resolved_boundary", PayloadValueType::Integer, false),
            ]),
            vec![
                reference(
                    "group",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    true,
                ),
                reference(
                    "member",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    true,
                ),
            ],
        ),
        record_schema::<Release>(
            false,
            object_schema(&[
                ("active_at", PayloadValueType::Integer, false),
                ("prepared_at", PayloadValueType::Integer, true),
                ("scope", PayloadValueType::String, true),
                ("status", PayloadValueType::String, true),
            ]),
            vec![
                reference(
                    "audience",
                    vec![DomainReferenceTargetKind::for_domain::<Audience>()],
                    false,
                    false,
                ),
                reference(
                    "publisher",
                    vec![DomainReferenceTargetKind::AnyEntity],
                    false,
                    false,
                ),
                reference(
                    "representation",
                    vec![DomainReferenceTargetKind::for_domain::<Representation>()],
                    true,
                    false,
                ),
            ],
        ),
        record_schema::<InformationOperationRecord>(
            false,
            object_schema(&[
                ("accepted_cause", PayloadValueType::Object, true),
                ("authority_claim_hash", PayloadValueType::String, false),
                ("admitted_at", PayloadValueType::Integer, true),
                ("canonical_input_hash", PayloadValueType::String, true),
                ("completed_at", PayloadValueType::Integer, false),
                ("continuation", PayloadValueType::Object, false),
                ("domain_result_evidence", PayloadValueType::Array, false),
                ("domain_result_refs", PayloadValueType::Array, true),
                ("id", PayloadValueType::Object, true),
                ("operation_kind", PayloadValueType::String, true),
                ("operation_version", PayloadValueType::Integer, true),
                ("output_slots", PayloadValueType::Array, true),
                ("publication_result_ids", PayloadValueType::Array, true),
                ("rejection_code", PayloadValueType::String, false),
                ("status", PayloadValueType::String, true),
            ]),
            vec![reference(
                "domain_result",
                information_result_targets(),
                false,
                true,
            )],
        ),
    ];
    schemas.sort_by(|left, right| left.kind.cmp(&right.kind));
    schemas
}

#[must_use]
pub fn information_semantic_identity() -> InformationLimitsV1 {
    InformationLimitsV1::canonical()
}

fn record_schema<T: DomainValueType>(
    create_only: bool,
    payload_schema: PayloadSchema,
    references: Vec<DomainReferenceSchema>,
) -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_record::<T>();
    schema.mutation_policy = if create_only {
        DomainRecordMutationPolicy::CreateOnly
    } else {
        DomainRecordMutationPolicy::Versioned
    };
    schema.payload_schema = payload_schema;
    schema.references = references;
    schema
}

fn reference(
    role: &str,
    targets: Vec<DomainReferenceTargetKind>,
    required: bool,
    multiple: bool,
) -> DomainReferenceSchema {
    DomainReferenceSchema {
        role: role.to_owned(),
        targets,
        required,
        multiple,
        allow_retired: false,
    }
}

fn information_result_targets() -> Vec<DomainReferenceTargetKind> {
    [
        DomainRecordKind::for_type::<Access>(),
        DomainRecordKind::for_type::<Audience>(),
        DomainRecordKind::for_type::<Channel>(),
        DomainRecordKind::for_type::<Content>(),
        DomainRecordKind::for_type::<DeliveryAttempt>(),
        DomainRecordKind::for_type::<Dispatch>(),
        DomainRecordKind::for_type::<Instance>(),
        DomainRecordKind::for_type::<Interpretation>(),
        DomainRecordKind::for_type::<Release>(),
        DomainRecordKind::for_type::<Representation>(),
    ]
    .into_iter()
    .map(DomainReferenceTargetKind::Domain)
    .collect()
}

fn knowledge_schema(
    name: &str,
    schema_hash: &str,
    payload_schema: PayloadSchema,
    subjects: Vec<KnowledgeSubjectSchema>,
) -> PluginKnowledgeSchema {
    PluginKnowledgeSchema {
        id: KnowledgeSchemaId::new(KnowledgeRecordKind::new(PLUGIN_NAMESPACE, name), 1),
        schema_hash: schema_hash.to_owned(),
        writable: true,
        payload_schema,
        subjects,
    }
}

fn knowledge_subject<T: DomainValueType>(role: &str) -> KnowledgeSubjectSchema {
    KnowledgeSubjectSchema {
        role: role.to_owned(),
        targets: vec![KnowledgeSubjectTargetKind::Domain(
            DomainRecordKind::for_type::<T>(),
        )],
        required: true,
        multiple: false,
    }
}

fn object_schema(fields: &[(&str, PayloadValueType, bool)]) -> PayloadSchema {
    PayloadSchema::Object {
        properties: fields
            .iter()
            .map(|(name, value_type, required)| {
                (
                    (*name).to_owned(),
                    PayloadProperty {
                        value_type: value_type.clone(),
                        required: *required,
                    },
                )
            })
            .collect(),
        allow_additional: false,
    }
}

const fn required(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_are_unique_neutral_and_mark_immutable_kinds_create_only() {
        let schemas = information_record_schemas();
        assert_eq!(schemas.len(), 12);
        assert!(schemas.windows(2).all(|pair| pair[0].kind < pair[1].kind));
        assert!(
            schemas
                .iter()
                .all(|schema| schema.kind.namespace == PLUGIN_NAMESPACE)
        );

        for name in [
            Channel::NAME,
            Content::NAME,
            Representation::NAME,
            Access::NAME,
            Interpretation::NAME,
            Audience::NAME,
        ] {
            let schema = schemas
                .iter()
                .find(|schema| schema.kind.name == name)
                .expect("schema should exist");
            assert_eq!(
                schema.mutation_policy,
                DomainRecordMutationPolicy::CreateOnly
            );
        }
    }

    #[test]
    fn runtime_knowledge_schemas_are_fixed_writable_lifecycle_facts() {
        let schemas = information_knowledge_schemas();
        assert_eq!(schemas.len(), 4);
        assert!(schemas.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert!(schemas.iter().all(|schema| {
            schema.id.kind.namespace == PLUGIN_NAMESPACE
                && schema.id.version == 1
                && schema.writable
                && schema.schema_hash.len() == 64
                && !schema.subjects.is_empty()
                && schema.subjects.iter().all(|subject| {
                    subject.required
                        && !subject.multiple
                        && matches!(
                            subject.targets.as_slice(),
                            [KnowledgeSubjectTargetKind::Domain(kind)]
                                if kind.namespace == PLUGIN_NAMESPACE
                        )
                })
        }));
    }
}
