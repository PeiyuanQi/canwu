use canwu_api::{
    DomainRecord, DomainRecordClass, DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation,
    DomainRecordRef, DomainReference, DomainReferenceTarget, EntityRef, KnowledgeHolderRef,
    PersonId, SimTime, TypedDomainRecordRef,
};
use canwu_information::{
    Access, AccessPayload, Audience, AudienceMembership, AudiencePayload, Channel,
    ChannelCapability, ChannelPayload, Content, ContentPayload, ContentRelation, Dispatch,
    DispatchPayload, DispatchStatus, DispatchTarget, InformationBody, InformationLifecycle,
    InformationLimitsV1, InformationMutationPlan, InformationOutputKind, InformationOutputSlot,
    InformationOutputSlotRef, InformationQuery, InformationRecordSet, LifecycleRequest,
    LineageParent, OperationLineageNode, PLUGIN_NAME, RecordBinding, Representation,
    RepresentationPayload, audience_membership_root_v1, validate_operation_lineage,
};
use serde_json::json;

pub const ADDRESSED_RECIPIENT_COUNT: usize = 10_000;
pub const EXPLICIT_AUDIENCE_MEMBER_COUNT: usize = 10_000;
pub const MIXED_LINEAGE_NODE_COUNT: usize = 1_000;
pub const ACCESS_RECORD_COUNT: usize = 100_000;
pub const ACCESS_HOLDER_COUNT: usize = 1_000;

#[derive(Clone)]
pub struct LifecyclePlanFixture {
    pub records: InformationRecordSet,
    pub request: LifecycleRequest,
}

#[derive(Clone)]
pub struct LineageBatch {
    pub slots: Vec<InformationOutputSlot>,
    pub nodes: Vec<OperationLineageNode>,
}

#[derive(Clone)]
pub struct MixedLineageFixture {
    pub batches: Vec<LineageBatch>,
}

impl MixedLineageFixture {
    pub fn validate(&self) -> Result<usize, String> {
        let mut validated = 0_usize;
        for batch in &self.batches {
            validate_operation_lineage(
                &batch.slots,
                &batch.nodes,
                InformationLimitsV1::canonical(),
            )?;
            validated = validated
                .checked_add(batch.nodes.len())
                .ok_or_else(|| "validated lineage count overflow".to_owned())?;
        }
        Ok(validated)
    }
}

pub fn addressed_dispatch_fixture() -> Result<LifecyclePlanFixture, String> {
    let mut records = Vec::new();
    let content = TypedDomainRecordRef::<Content>::new("stress-content");
    apply_request(
        &mut records,
        LifecycleRequest::CreateContent {
            binding: RecordBinding::new(content.clone(), Vec::new()),
            payload: ContentPayload {
                content_type: "benchmark_payload".to_owned(),
                body: InformationBody::InlineJson {
                    value: json!({"fixture": "addressed-dispatch"}),
                },
                created_at: minute(0),
                derivation: None,
            },
        },
    )?;
    let representation = TypedDomainRecordRef::<Representation>::new("stress-representation");
    apply_request(
        &mut records,
        LifecycleRequest::CreateRepresentation {
            binding: RecordBinding::new(
                representation.clone(),
                vec![DomainReference::from_typed("content", content)],
            ),
            payload: RepresentationPayload {
                format: "benchmark_wire_v1".to_owned(),
                created_at: minute(1),
                operation: "encode".to_owned(),
                content_relation: ContentRelation::SameContent,
                sources: Vec::new(),
                claimed_source: None,
                interpretation_capability: None,
            },
        },
    )?;
    let channel = TypedDomainRecordRef::<Channel>::new("stress-channel");
    apply_request(
        &mut records,
        LifecycleRequest::CreateChannel {
            binding: RecordBinding::new(channel.clone(), Vec::new()),
            payload: ChannelPayload {
                profile: "benchmark_addressed_channel".to_owned(),
                capabilities: vec![ChannelCapability::AddressedDelivery],
            },
        },
    )?;
    let recipients = (1..=ADDRESSED_RECIPIENT_COUNT)
        .map(|index| person(index as u64))
        .collect::<Vec<_>>();
    let mut references = vec![
        DomainReference::from_typed("channel", channel),
        DomainReference::from_typed("representation", representation),
    ];
    references.extend(
        recipients
            .iter()
            .map(|recipient| holder_reference("intended_recipient", recipient)),
    );
    Ok(LifecyclePlanFixture {
        records: InformationRecordSet::from_records(records)?,
        request: LifecycleRequest::BeginDispatch {
            binding: RecordBinding::new(
                TypedDomainRecordRef::<Dispatch>::new("stress-addressed-dispatch"),
                references,
            ),
            payload: DispatchPayload {
                status: DispatchStatus::Prepared,
                target: DispatchTarget::Addressed(recipients),
                prepared_at: minute(2),
                dispatched_at: None,
                completed_at: None,
            },
        },
    })
}

pub fn explicit_audience_fixture() -> Result<LifecyclePlanFixture, String> {
    let member_holders = (1..=EXPLICIT_AUDIENCE_MEMBER_COUNT)
        .map(|index| person(index as u64))
        .collect::<Vec<_>>();
    let members = member_holders
        .iter()
        .map(|holder| holder_reference("member", holder))
        .collect();
    let membership_root =
        audience_membership_root_v1(&member_holders, InformationLimitsV1::canonical())?;
    Ok(LifecyclePlanFixture {
        records: InformationRecordSet::default(),
        request: LifecycleRequest::CreateAudience {
            binding: RecordBinding::new(
                TypedDomainRecordRef::<Audience>::new("stress-explicit-audience"),
                members,
            ),
            payload: AudiencePayload {
                membership: AudienceMembership::ExplicitMembers,
                resolved_at: minute(0),
                resolution_version: 1,
                resolved_boundary: None,
                member_count: EXPLICIT_AUDIENCE_MEMBER_COUNT as u64,
                membership_root,
            },
        },
    })
}

pub fn plan_lifecycle(fixture: &LifecyclePlanFixture) -> Result<InformationMutationPlan, String> {
    InformationLifecycle::plan(
        &fixture.records,
        &fixture.request,
        InformationLimitsV1::canonical(),
    )
}

pub fn mixed_lineage_fixture() -> MixedLineageFixture {
    let mut batches = Vec::with_capacity(4);
    for batch_index in 0..4_u16 {
        let kind = if batch_index % 2 == 0 {
            InformationOutputKind::Content
        } else {
            InformationOutputKind::Representation
        };
        let slots = (0..=250_u16)
            .map(|index| InformationOutputSlot {
                index,
                name: format!("lineage-{batch_index}-{index}"),
                kind,
            })
            .collect::<Vec<_>>();
        let nodes = (1..=250_u16)
            .map(|index| {
                let parent = if index % 2 == 0 {
                    LineageParent::Output(InformationOutputSlotRef {
                        index: index - 1,
                        kind,
                    })
                } else {
                    LineageParent::Persisted(DomainRecordRef {
                        kind: lineage_record_kind(kind),
                        id: format!("persisted-{batch_index}-{index}"),
                    })
                };
                OperationLineageNode {
                    child: InformationOutputSlotRef { index, kind },
                    parents: vec![parent],
                }
            })
            .collect();
        batches.push(LineageBatch { slots, nodes });
    }
    MixedLineageFixture { batches }
}

pub fn access_records() -> Result<Vec<DomainRecord>, String> {
    let representation = TypedDomainRecordRef::<Representation>::new("stress-access-source");
    let mut records = Vec::with_capacity(ACCESS_RECORD_COUNT);
    for index in 0..ACCESS_RECORD_COUNT {
        let holder = person((index % ACCESS_HOLDER_COUNT + 1) as u64);
        records.push(DomainRecord {
            reference: TypedDomainRecordRef::<Access>::new(format!("stress-access-{index:06}"))
                .into_untyped(),
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::to_value(AccessPayload {
                accessed_at: minute(i64::try_from(index).map_err(|error| error.to_string())?),
                method: "benchmark_observation".to_owned(),
                extent_per_mille: 1_000,
            })
            .map_err(|error| error.to_string())?,
            references: vec![
                holder_reference("holder", &holder),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        });
    }
    Ok(records)
}

pub fn access_query() -> InformationQuery {
    InformationQuery {
        kinds: vec![DomainRecordKind::for_type::<Access>()],
        active_only: true,
        reference_role: None,
        reference_target: None,
    }
}

pub fn access_holder_queries() -> Vec<InformationQuery> {
    (1..=ACCESS_HOLDER_COUNT)
        .map(|index| InformationQuery {
            kinds: vec![DomainRecordKind::for_type::<Access>()],
            active_only: true,
            reference_role: Some("holder".to_owned()),
            reference_target: Some(holder_reference_target(&person(index as u64))),
        })
        .collect()
}

fn apply_request(records: &mut Vec<DomainRecord>, request: LifecycleRequest) -> Result<(), String> {
    let record_set = InformationRecordSet::from_records(records.iter().cloned())?;
    let plan = InformationLifecycle::plan(&record_set, &request, InformationLimitsV1::canonical())?;
    for mutation in plan.mutations {
        let DomainRecordMutation::Create { record } = mutation else {
            return Err("stress fixture seeding expected create-only mutations".to_owned());
        };
        records.push(DomainRecord {
            reference: record.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: record.payload,
            references: record.references,
        });
    }
    Ok(())
}

fn lineage_record_kind(kind: InformationOutputKind) -> DomainRecordKind {
    match kind {
        InformationOutputKind::Content => DomainRecordKind::for_type::<Content>(),
        InformationOutputKind::Representation => DomainRecordKind::for_type::<Representation>(),
        _ => unreachable!("stress lineage uses only lineage-capable kinds"),
    }
}

fn holder_reference(role: &str, holder: &KnowledgeHolderRef) -> DomainReference {
    DomainReference {
        role: role.to_owned(),
        target: holder_reference_target(holder),
    }
}

fn holder_reference_target(holder: &KnowledgeHolderRef) -> DomainReferenceTarget {
    match holder {
        KnowledgeHolderRef::Person(person) => {
            DomainReferenceTarget::Core(EntityRef::Person(*person))
        }
        KnowledgeHolderRef::Entity(EntityRef::Domain(reference)) => {
            DomainReferenceTarget::Domain(reference.clone())
        }
        KnowledgeHolderRef::Entity(entity) => DomainReferenceTarget::Core(entity.clone()),
    }
}

fn person(id: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(id))
}

fn minute(value: i64) -> SimTime {
    SimTime::from_minutes(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_fixtures_preserve_the_declared_contract_scales() {
        let addressed = addressed_dispatch_fixture().expect("addressed fixture should build");
        let addressed_plan =
            plan_lifecycle(&addressed).expect("10,000-recipient dispatch should validate");
        assert_eq!(addressed_plan.mutations.len(), 1);

        let audience = explicit_audience_fixture().expect("audience fixture should build");
        let audience_plan =
            plan_lifecycle(&audience).expect("10,000-member audience should validate");
        assert_eq!(audience_plan.mutations.len(), 1);

        let lineage = mixed_lineage_fixture();
        assert_eq!(
            lineage.validate().expect("mixed lineage should validate"),
            MIXED_LINEAGE_NODE_COUNT
        );

        let access = access_records().expect("access fixture should build");
        assert_eq!(access.len(), ACCESS_RECORD_COUNT);
        assert_eq!(access_holder_queries().len(), ACCESS_HOLDER_COUNT);
        let record_set =
            InformationRecordSet::from_records(access).expect("access index should build");
        assert_eq!(
            record_set
                .query(&access_query())
                .expect("access query should run")
                .len(),
            ACCESS_RECORD_COUNT
        );
        let holder_queries = access_holder_queries();
        for query in [
            holder_queries
                .first()
                .expect("first holder query should exist"),
            holder_queries
                .last()
                .expect("last holder query should exist"),
        ] {
            assert_eq!(
                record_set
                    .query(query)
                    .expect("holder query should run")
                    .len(),
                ACCESS_RECORD_COUNT / ACCESS_HOLDER_COUNT
            );
        }
    }
}
