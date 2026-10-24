//! Public-API fixture for downstream gap G-72: domain-entity knowledge
//! holders are correspondence parties. One court writes to another, a third
//! court carries the letter under its own delegation and holds the route
//! seed, and the delivery settles. Every correspondence record names each
//! court by its domain record, the carrier court can be retired after the
//! delivery without failing its owner's boundary, and once retired it takes
//! no new carrying.

#![allow(clippy::too_many_lines)]

#[path = "../examples/support/mod.rs"]
mod support;

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CommandAttemptOutcome, CommandAuthority, CommandEnvelope, CommandRequest,
    CommandRequestId, DecisionOrigin, DomainEntityKindClass, DomainRecord, DomainRecordClass,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    DomainRecordType, DomainReference, DomainReferenceTarget, EntityRef, ErrorCode, IngressClass,
    IngressPayload, Issuer, KnowledgeHolderPolicy, KnowledgeHolderRef, PayloadSchema,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, RoutingNodeRef, RoutingPolicy,
    SimDuration, SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    TransportExecutionId, TypedDomainRecordRef,
};
use canwu_correspondence::{
    CARRY_CORRESPONDENCE_CAPABILITY, CarrierDelegationRequest, CommunicationOpportunityRecord,
    CommunicationOpportunityRequest, CommunicationOpportunityStatus,
    CorrespondenceCapacityAdmission, CorrespondenceOperationRecord, CorrespondencePlugin,
    CorrespondenceStatus, InitiateCorrespondenceRequest, KNOWLEDGE_INGRESS, KnowledgeSeedRecord,
    OPPORTUNITY_INGRESS, PLUGIN_NAME, carrier_delegation_command, carrier_delegation_ref,
    correspondence_command, correspondence_operation_ref, knowledge_seed_ref, opportunity_ref,
};
use canwu_information::{
    DelegationClaimV1, Dispatch, DispatchPayload, DispatchTarget, InformationOperationId,
    InformationPlugin,
};
use serde_json::json;
use support::{network_seed, scenario_with_prepared_dispatch};

const KEY: &str = "court-letter";
const COURT_PLUGIN: &str = "fixture-courts";
const RETIRE_COURT: &str = "retire_court";

struct CourtRecord;

impl DomainRecordType for CourtRecord {
    type Payload = serde_json::Value;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "fixture.court";
    const NAME: &'static str = "court";
}

fn court(id: &str) -> DomainRecordRef {
    TypedDomainRecordRef::<CourtRecord>::new(id).into_untyped()
}

fn court_holder(id: &str) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Domain(court(id)))
}

fn court_schema() -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_entity::<CourtRecord>();
    schema.holder_policy = KnowledgeHolderPolicy::Allowed;
    schema
}

/// Owns the court entities, which may hold knowledge, and retires the court
/// a `retire_court` packet names.
struct CourtPlugin;

impl SimulationPlugin for CourtPlugin {
    fn name(&self) -> &'static str {
        COURT_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000c72"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(court_schema())?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: RETIRE_COURT.to_owned(),
            description: "Retire one court".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut system = BoundarySystemContract::new(
            "retire-courts",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        system.reads = vec![StateKey::core_ingress(), court_schema().state_key()];
        system.writes = vec![court_schema().state_key()];
        system.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(system, retire_courts)
    }
}

fn retire_courts(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != COURT_PLUGIN || packet_type != RETIRE_COURT {
            continue;
        }
        let reference =
            TypedDomainRecordRef::<CourtRecord>::new(payload["court"].as_str().expect("court"));
        let record = view.typed_domain_record(&reference)?.expect("the court");
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Retire {
                record: record.reference.clone(),
                expected_version: record.version,
                successor: None,
            },
            summary: "Retire the court".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn court_record(id: &str) -> DomainRecord {
    DomainRecord {
        reference: court(id),
        owner: COURT_PLUGIN.to_owned(),
        class: DomainRecordClass::Entity,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: json!({}),
        references: Vec::new(),
    }
}

fn target(references: &[DomainReference], role: &str) -> Vec<DomainReferenceTarget> {
    references
        .iter()
        .filter(|reference| reference.role == role)
        .map(|reference| reference.target.clone())
        .collect()
}

#[test]
fn gap_g72_correspondence_domain_entity_parties() {
    let plugins: &[&dyn SimulationPlugin] =
        &[&InformationPlugin, &CorrespondencePlugin, &CourtPlugin];
    let (mut scenario, _, _, prepared_dispatch) = scenario_with_prepared_dispatch();
    let (sender, recipient, carrier) = (court("east"), court_holder("west"), court("envoy"));

    // The prepared dispatch is the east court's, addressed to the west court;
    // it names both by their domain records.
    let dispatch = scenario
        .domain_records
        .iter_mut()
        .find(|record| record.reference == prepared_dispatch.record)
        .expect("the prepared dispatch");
    let mut payload: DispatchPayload = serde_json::from_value(dispatch.payload.clone()).unwrap();
    payload.target = DispatchTarget::Addressed(vec![recipient.clone()]);
    dispatch.payload = serde_json::to_value(&payload).unwrap();
    for reference in &mut dispatch.references {
        match reference.role.as_str() {
            "sender" => reference.target = DomainReferenceTarget::Domain(sender.clone()),
            "intended_recipient" => {
                reference.target = DomainReferenceTarget::Domain(court("west"));
            }
            _ => {}
        }
    }
    dispatch.references.sort();
    assert!(dispatch.reference.kind.matches_type::<Dispatch>());
    scenario
        .domain_records
        .extend(["east", "envoy", "west"].map(court_record));
    scenario
        .domain_records
        .sort_by(|left, right| left.reference.cmp(&right.reference));
    let mut canwu = Canwu::new_with_plugins(72, scenario, plugins).unwrap();
    let now = canwu.time();

    // The envoy court holds the route seed and accepts carrying for the east
    // court under its own institution authority; an automatic opportunity of
    // the east court selects the west court.
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            KNOWLEDGE_INGRESS,
            now,
            serde_json::to_value(network_seed(
                court_holder("envoy"),
                recipient.clone(),
                RoutingNodeRef::new("wuxi/delivery/recipient"),
                false,
            ))
            .unwrap(),
        ))
        .unwrap();
    canwu
        .enqueue_command(
            now,
            0,
            CommandRequest::new(
                CommandRequestId::new(100),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Institution("envoy-court".to_owned()),
                    carrier_delegation_command(&CarrierDelegationRequest {
                        claim: DelegationClaimV1 {
                            format_version: 1,
                            performed_by: EntityRef::Domain(carrier.clone()),
                            performed_for: court_holder("east"),
                            capabilities: vec![CARRY_CORRESPONDENCE_CAPABILITY.to_owned()],
                            not_before: None,
                            expires_at: None,
                        },
                    })
                    .unwrap(),
                )
                .at_time(now)
                .with_authority(CommandAuthority {
                    decision_origin: DecisionOrigin::Institution {
                        institution: EntityRef::Domain(carrier.clone()),
                        responsible_actor: None,
                    },
                    seat_id: None,
                    permission_profile_id: None,
                    command_subject: None,
                }),
            ),
        )
        .unwrap();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            OPPORTUNITY_INGRESS,
            now,
            serde_json::to_value(CommunicationOpportunityRequest {
                operation_key: KEY.to_owned(),
                sender: EntityRef::Domain(sender.clone()),
                candidates: vec![recipient.clone()],
                reason: "court correspondence".to_owned(),
                probability_per_mille: 1_000,
                automatic: true,
            })
            .unwrap(),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    let CommandAttemptOutcome::Accepted {
        command_id: delegation,
    } = canwu.command_attempts().last().unwrap().outcome
    else {
        panic!("the envoy court's delegation must be accepted");
    };
    let delegation_record =
        carrier_delegation_ref(&EntityRef::Domain(carrier.clone()), &court_holder("east")).unwrap();
    for _ in 0..4 {
        if canwu.typed_domain_record(&delegation_record).is_some() {
            break;
        }
        canwu.step_canonical().unwrap().unwrap();
    }
    assert!(canwu.typed_domain_record(&delegation_record).is_some());

    let seed = canwu
        .typed_domain_record(&knowledge_seed_ref("carrier-network-local"))
        .unwrap();
    assert_eq!(
        target(&seed.references, "holder"),
        [DomainReferenceTarget::Domain(carrier.clone())]
    );
    assert_eq!(
        seed.decode_payload::<KnowledgeSeedRecord>()
            .unwrap()
            .refusal,
        None
    );
    let opportunity = canwu.typed_domain_record(&opportunity_ref(KEY)).unwrap();
    assert_eq!(
        target(&opportunity.references, "sender"),
        [DomainReferenceTarget::Domain(sender.clone())]
    );
    assert_eq!(
        target(&opportunity.references, "candidate"),
        [DomainReferenceTarget::Domain(court("west"))]
    );
    assert_eq!(
        opportunity
            .decode_payload::<CommunicationOpportunityRecord>()
            .unwrap()
            .status,
        CommunicationOpportunityStatus::SelectedAutomatic
    );

    // The automatic correspondence of the east court, carried by the envoy
    // court, settles.
    let request = InitiateCorrespondenceRequest {
        operation_key: KEY.to_owned(),
        sender: EntityRef::Domain(sender.clone()),
        recipient: recipient.clone(),
        carrier: court_holder("envoy"),
        channel_profile: "sealed-letter".to_owned(),
        origin: RoutingNodeRef::new("wuxi/hub"),
        due_at: canwu.time() + SimDuration::days(1),
        prepared_dispatch,
        delivery_attempt_operation: InformationOperationId::new(
            "fixture.correspondence",
            "court-attempt",
        ),
        routing_policy: RoutingPolicy::default(),
        capacity_admission: CorrespondenceCapacityAdmission::Unconstrained,
        execution_id: TransportExecutionId(72),
        automatic_opportunity: Some(opportunity_ref(KEY)),
        carrier_delegation: Some(delegation),
    };
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("court-scheduler".to_owned()),
                    correspondence_command(&request).unwrap(),
                )
                .at_time(canwu.time())
                .with_authority(CommandAuthority::no_responsible_actor(
                    "automatic court correspondence",
                )),
            ),
        )
        .unwrap();
    let mut settled = false;
    for _ in 0..40 {
        canwu.step_canonical().unwrap().unwrap();
        if canwu
            .typed_domain_record(&correspondence_operation_ref(KEY))
            .is_some_and(|record| {
                record
                    .decode_payload::<CorrespondenceOperationRecord>()
                    .unwrap()
                    .status
                    .is_terminal()
            })
        {
            settled = true;
            break;
        }
    }
    assert!(settled, "the court correspondence did not end");
    let operation = canwu
        .typed_domain_record(&correspondence_operation_ref(KEY))
        .unwrap();
    assert_eq!(
        operation
            .decode_payload::<CorrespondenceOperationRecord>()
            .unwrap()
            .status,
        CorrespondenceStatus::Settled
    );
    for (role, party) in [
        ("sender", sender.clone()),
        ("recipient", court("west")),
        ("carrier", carrier.clone()),
    ] {
        assert_eq!(
            target(&operation.references, role),
            [DomainReferenceTarget::Domain(party)],
            "{role}"
        );
    }

    // The carrier court's owner retires it: correspondence history does not
    // stop the retirement.
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            COURT_PLUGIN,
            RETIRE_COURT,
            canwu.time(),
            json!({"court": "envoy"}),
        ))
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    assert!(
        !canwu
            .typed_domain_record(&TypedDomainRecordRef::<CourtRecord>::new("envoy"))
            .unwrap()
            .is_active()
    );

    // The retired court stays referenced but takes no new carrying: a
    // correspondence naming it as carrier is a recorded command rejection.
    let mut later = request.clone();
    later.operation_key = "court-letter-after-retirement".to_owned();
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(2),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("court-scheduler".to_owned()),
                    correspondence_command(&later).unwrap(),
                )
                .at_time(canwu.time())
                .with_authority(CommandAuthority::no_responsible_actor(
                    "automatic court correspondence",
                )),
            ),
        )
        .unwrap();
    canwu.step_canonical().unwrap().unwrap();
    assert!(matches!(
        &canwu.command_attempts().last().unwrap().outcome,
        CommandAttemptOutcome::Rejected { error }
            if error.code == ErrorCode::InvalidAuthority
                && error.message.contains("carrier accepts no knowledge")
    ));

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}
