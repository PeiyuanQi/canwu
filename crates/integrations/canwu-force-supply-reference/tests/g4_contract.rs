use canwu_api::{
    DomainRecordVersionRef, DomainRecordVersionSource, KnowledgeHolderRef, PersonId, SimTime,
    canonical_hash,
};
use canwu_economy_reference_content::{compile_content_pack, synthetic_grain_fixture};
use canwu_force_supply_reference::*;
use canwu_resource::{
    ActivateCompletionLeaseV1, CompletionCapacityGrantId, CompletionCapacityRecipeV1,
    CompletionGrantStateV1, CompletionLeaseAcquisitionId, CompletionLockedTargetV1,
    CompletionPolicyClassV1, EligibilityEnvelopeV1, ExternalCompletionParticipantGrantV1,
    FulfillmentStatus, GrantCompletionCapacityV1, PrepareCompletionCapacityV1,
    RequestCompletionLeaseV1, ResourceAccountId, ResourceAllocationLegId,
    ResourceAllocationLegVersionV1, ResourceConsumptionId, ResourceConsumptionVersionV1,
    ResourceDefinitionRevisionId, ResourceDemandId, ResourceFulfillmentId,
    ResourceFulfillmentVersionV1, ResourceOperationKey, ResourceOperationOutcomeId,
    ResourceOperationOutcomeVersionV1, ResourceOperationStatus, ResourceRecordRefV1,
    ResourceRevision, ResourceUnitRevisionId, resource_runtime_reference,
};
use std::collections::{BTreeMap, BTreeSet};

fn revision() -> ResourceRevision {
    ResourceRevision::new(1).expect("resource revision")
}

fn seal_resource_outcome(
    mut outcome: ResourceOperationOutcomeVersionV1,
) -> ResourceOperationOutcomeVersionV1 {
    outcome.semantic_digest.clear();
    outcome.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-outcome.v1", &outcome)
            .expect("resource outcome digest");
    outcome
}

fn settlement_for(
    intent: &ForceConsumptionIntent,
    outcome: &ResourceOperationOutcomeVersionV1,
) -> ForceResourceSettlementEvidenceV1 {
    let mut consumption = ResourceConsumptionVersionV1 {
        id: intent.consumption_id.clone(),
        revision: revision(),
        account: intent.stock_custody.destination_account.clone(),
        allocation_leg: intent.allocation.id.clone(),
        quantity: outcome.quantity,
        consumer_evidence: provider_version(
            force_supply_runtime_reference().into_untyped(),
            intent.expected_force_runtime_revision,
        ),
        semantic_digest: String::new(),
    };
    consumption.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.consumption.v1", &consumption)
            .expect("consumption digest");
    let mut fulfillment = ResourceFulfillmentVersionV1 {
        id: ResourceFulfillmentId::new(format!("canwu.resource:fulfillment:{}", intent.id))
            .expect("ID"),
        revision: revision(),
        demand: ResourceDemandId::new(format!("canwu.resource:demand:{}", intent.id)).expect("ID"),
        consumed_quantity: outcome.quantity,
        remainder: outcome.remainder,
        status: if outcome.remainder == 0 {
            FulfillmentStatus::Complete
        } else {
            FulfillmentStatus::Partial
        },
        operation_key: intent.resource_operation_key.clone(),
        semantic_digest: String::new(),
    };
    fulfillment.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.fulfillment.v1", &fulfillment)
            .expect("fulfillment digest");
    let mut settlement = ForceResourceSettlementEvidenceV1 {
        provider_state: provider_version(resource_runtime_reference().into_untyped(), 1),
        outcome: outcome.clone(),
        consumption,
        fulfillment,
        destination_account_revision: ResourceRevision::new(2).expect("resource revision"),
        destination_custodian: intent.stock_custody.destination_custodian.clone(),
        accepted_transfer: intent.stock_custody.accepted_transfer.clone(),
        semantic_digest: String::new(),
    };
    settlement.semantic_digest =
        canonical_hash("canwu.force-supply.resource-settlement.v1", &settlement)
            .expect("resource settlement digest");
    settlement
}

fn provider_version(record: canwu_api::DomainRecordRef, version: u64) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record,
        version,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn external_participant(
    state: &ForceSupplyStateV1,
    acquisition: &CompletionLeaseAcquisitionId,
    owner: &str,
    grant: CompletionCapacityGrantId,
    targets: Vec<CompletionLockedTargetV1>,
) -> ExternalCompletionParticipantGrantV1 {
    let acquisition = &state.completion_leases.acquisitions[acquisition];
    let units = acquisition
        .recipe
        .canonical_units()
        .expect("completion units");
    ExternalCompletionParticipantGrantV1 {
        coordinator_plugin: PLUGIN_NAME.to_owned(),
        coordinator_source: provider_version(
            force_supply_runtime_reference().into_untyped(),
            state.revision,
        ),
        coordinator_acquisition_revision: acquisition.revision,
        holder: acquisition.holder.clone(),
        operation_namespace: acquisition.operation_namespace.clone(),
        eligibility_time: acquisition.eligibility_time,
        eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
        recipe: acquisition.recipe.clone(),
        policy_class: acquisition.policy_class,
        grant: canwu_resource::CompletionCapacityGrantV1 {
            id: grant,
            revision: ResourceRevision::INITIAL,
            acquisition: acquisition.id.clone(),
            operation_key: acquisition.operation_key.clone(),
            owner_plugin: owner.to_owned(),
            run_budget_revision: ResourceRevision::INITIAL,
            target_versions: targets,
            recipe_digest: acquisition.recipe_digest.clone(),
            reserved_units: units,
            expires_after_boundary: canwu_resource::PREACTIVATION_LEASE_TTL_BOUNDARIES,
            activation_deadline_boundary: None,
            state: CompletionGrantStateV1::Held,
            rejection: None,
        },
        certificate: None,
    }
}

fn complete_external_owner(
    state: &mut ForceSupplyStateV1,
    intent: &ForceConsumptionIntent,
    owner: &str,
) {
    let acquisition = &intent.completion_certificate.acquisition;
    let mut participant = state.completion_participant_grants[acquisition][owner].clone();
    if participant.grant.state == CompletionGrantStateV1::Prepared {
        participant.grant.state = CompletionGrantStateV1::Consumed;
        participant.grant.revision = participant
            .grant
            .revision
            .next()
            .expect("consumed revision");
        participant.certificate = Some(intent.completion_certificate.clone());
        state
            .acknowledge_external_participant(&participant.holder.clone(), participant.clone())
            .expect("acknowledge consumed external owner");
    }
    participant.grant.state = CompletionGrantStateV1::Completed;
    participant.grant.revision = participant
        .grant
        .revision
        .next()
        .expect("completed revision");
    state
        .acknowledge_external_participant(&participant.holder.clone(), participant)
        .expect("acknowledge completed external owner");
}

fn base_state() -> (
    ForceSupplyStateV1,
    KnowledgeHolderRef,
    ReferenceForceId,
    ForceRequirementId,
    ResourceDefinitionRevisionId,
    ResourceUnitRevisionId,
) {
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let content = compile_content_pack(&synthetic_grain_fixture()).expect("compiled fixture");
    let mut state = ForceSupplyStateV1::from_compiled_content(content).expect("force content");
    let profile = state
        .profiles
        .values()
        .find(|profile| {
            profile
                .requirements
                .iter()
                .any(|requirement| requirement.kind == SupplyResourceKind::Food)
        })
        .cloned()
        .expect("preindustrial food profile");
    let requirement = profile
        .requirements
        .iter()
        .find(|requirement| requirement.kind == SupplyResourceKind::Food)
        .cloned()
        .expect("preindustrial food requirement");
    let resource = requirement.resource_revision.clone();
    let unit = requirement.unit_revision.clone();
    let requirement_id = requirement.id.clone();
    let force_id = ReferenceForceId::new("canwu.force-supply-reference:force:test").expect("ID");
    let force = ReferenceForce {
        id: force_id.clone(),
        revision: 1,
        holder: holder.clone(),
        profile: profile.id.clone(),
        active: true,
        readiness_per_mille: 900,
        fatigue_per_mille: 100,
        cohesion_per_mille: 800,
        disease_per_mille: 0,
        desertion_per_mille: 0,
        supply_posture: "awaiting daily issue".to_owned(),
        due: profile
            .requirements
            .iter()
            .map(|requirement| {
                (
                    requirement.id.clone(),
                    DueRequirementStateV1 {
                        requirement: requirement.id.clone(),
                        next_due: SimTime::EPOCH,
                        persisted_remainder_minutes: 0,
                    },
                )
            })
            .collect(),
        blocked_by_active_requisition: None,
    };
    let grant = ForceObserverGrantV1 {
        id: ForceObserverGrantId::new("canwu.force-supply-reference:grant:commander").expect("ID"),
        holder: holder.clone(),
        force: force_id.clone(),
        role: ForceObservationRole::Commander,
        observation_delay_minutes: 0,
        confidence_per_mille: 1_000,
    };
    state.forces.insert(force.id.clone(), force);
    for requirement in &profile.requirements {
        state
            .due_index
            .entry(SimTime::EPOCH)
            .or_default()
            .insert((force_id.clone(), requirement.id.clone()));
    }
    state.observation_grants.insert(grant.id.clone(), grant);
    state.validate().expect("base state");
    (state, holder, force_id, requirement_id, resource, unit)
}

fn requisition_intent(
    state: &mut ForceSupplyStateV1,
    force: &ReferenceForceId,
    requirement: &ForceRequirementId,
    resource: &ResourceDefinitionRevisionId,
    unit: &ResourceUnitRevisionId,
) -> ForceConsumptionIntent {
    consumption_intent_with_suffix(
        state,
        force,
        requirement,
        resource,
        unit,
        "requisition",
        true,
        SimTime::EPOCH,
    )
}

fn requisition_intent_at(
    state: &mut ForceSupplyStateV1,
    force: &ReferenceForceId,
    requirement: &ForceRequirementId,
    resource: &ResourceDefinitionRevisionId,
    unit: &ResourceUnitRevisionId,
    service_at: SimTime,
) -> ForceConsumptionIntent {
    consumption_intent_with_suffix(
        state,
        force,
        requirement,
        resource,
        unit,
        "requisition-delayed",
        true,
        service_at,
    )
}

#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn consumption_intent_with_suffix(
    state: &mut ForceSupplyStateV1,
    force: &ReferenceForceId,
    requirement: &ForceRequirementId,
    resource: &ResourceDefinitionRevisionId,
    unit: &ResourceUnitRevisionId,
    suffix: &str,
    requisition: bool,
    service_at: SimTime,
) -> ForceConsumptionIntent {
    let operation_key =
        ResourceOperationKey::new(format!("canwu.resource:operation:force-{suffix}")).expect("ID");
    let acquisition = CompletionLeaseAcquisitionId::new(format!(
        "canwu.force-supply-reference:completion-acquisition:{suffix}"
    ))
    .expect("ID");
    let holder = state.forces[force].holder.clone();
    state
        .configure_completion_authority(holder.clone())
        .expect("completion authority");
    let force_target = CompletionLockedTargetV1::ExternalRecord {
        version: provider_version(
            force_supply_runtime_reference().into_untyped(),
            state.revision,
        ),
    };
    let economy_target = CompletionLockedTargetV1::ExternalRecord {
        version: provider_version(
            canwu_api::DomainRecordRef::new(
                "canwu.economy-reference",
                "runtime",
                "canwu.economy-reference:runtime:v1",
            ),
            1,
        ),
    };
    let resource_targets = vec![
        CompletionLockedTargetV1::Account {
            id: ResourceAccountId::new("canwu.resource:account:local-granary").expect("ID"),
            revision: revision(),
        },
        CompletionLockedTargetV1::AllocationLeg {
            id: ResourceAllocationLegId::new(format!("canwu.resource:allocation:{suffix}"))
                .expect("ID"),
            revision: revision(),
        },
    ];
    let mut expected_participants = BTreeSet::from([
        PLUGIN_NAME.to_owned(),
        canwu_resource::PLUGIN_NAME.to_owned(),
    ]);
    if requisition {
        expected_participants.insert(ECONOMY_EXTERNALITY_PROVIDER.to_owned());
    }
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(format!(
                    "canwu.force-supply-reference:operation:completion-acquire:{suffix}"
                ))
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::Completion {
                    operation: ForceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
                        id: acquisition.clone(),
                        operation_key: operation_key.clone(),
                        holder: holder.clone(),
                        operation_namespace: "canwu.force-supply-reference:requisition".to_owned(),
                        eligibility_time: service_at,
                        eligibility_envelope: EligibilityEnvelopeV1::new(
                            vec![match &force_target {
                                CompletionLockedTargetV1::ExternalRecord { version } => {
                                    version.clone()
                                }
                                _ => unreachable!(),
                            }],
                            BTreeMap::new(),
                            BTreeSet::new(),
                            Vec::new(),
                            vec![match &economy_target {
                                CompletionLockedTargetV1::ExternalRecord { version } => {
                                    version.clone()
                                }
                                _ => unreachable!(),
                            }],
                        )
                        .expect("eligibility envelope"),
                        recipe: CompletionCapacityRecipeV1 {
                            receipts: 8,
                            mutations: 16,
                            reports_per_holder: 1,
                            holders: 1,
                            bytes: 8_192,
                        },
                        expected_participants,
                        policy_class: CompletionPolicyClassV1::Guaranteed,
                    }),
                },
            },
            service_at,
        )
        .expect("acquire force completion");
    let mut grant_ids = BTreeMap::new();
    let force_grant = CompletionCapacityGrantId::new(format!(
        "canwu.force-supply-reference:completion-grant:force:{suffix}"
    ))
    .expect("ID");
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(format!(
                    "canwu.force-supply-reference:operation:completion-grant:force:{suffix}"
                ))
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::Completion {
                    operation: ForceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                        grant_id: force_grant.clone(),
                        acquisition: acquisition.clone(),
                        expected_acquisition_revision: state.completion_leases.acquisitions
                            [&acquisition]
                            .revision,
                        owner_plugin: PLUGIN_NAME.to_owned(),
                        target_versions: vec![force_target.clone()],
                        current_boundary: 0,
                    }),
                },
            },
            service_at,
        )
        .expect("grant force-owned completion participant");
    grant_ids.insert("force", force_grant.clone());

    let mut external = vec![(
        canwu_resource::PLUGIN_NAME,
        resource_targets.clone(),
        "resource",
    )];
    if requisition {
        external.push((
            ECONOMY_EXTERNALITY_PROVIDER,
            vec![economy_target.clone()],
            "economy",
        ));
    }
    for (owner, targets, label) in external {
        let grant_id = CompletionCapacityGrantId::new(format!(
            "canwu.force-supply-reference:completion-grant:{label}:{suffix}"
        ))
        .expect("ID");
        let participant =
            external_participant(state, &acquisition, owner, grant_id.clone(), targets);
        state
            .acknowledge_external_participant(&holder, participant)
            .expect("acknowledge owner-held completion participant");
        grant_ids.insert(label, grant_id);
    }
    let envelope_digest = state.completion_leases.acquisitions[&acquisition]
        .eligibility_envelope
        .digest
        .clone();
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(format!(
                    "canwu.force-supply-reference:operation:completion-prepare:force:{suffix}"
                ))
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::Completion {
                    operation: ForceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                        acquisition: acquisition.clone(),
                        expected_acquisition_revision: state.completion_leases.acquisitions
                            [&acquisition]
                            .revision,
                        grant: force_grant.clone(),
                        expected_grant_revision: state.completion_leases.grants[&force_grant]
                            .revision,
                        current_boundary: 1,
                        eligibility_envelope_digest: envelope_digest.clone(),
                    }),
                },
            },
            service_at,
        )
        .expect("prepare force-owned completion participant");
    for owner in [canwu_resource::PLUGIN_NAME, ECONOMY_EXTERNALITY_PROVIDER]
        .into_iter()
        .filter(|owner| requisition || *owner == canwu_resource::PLUGIN_NAME)
    {
        let mut participant = state.completion_participant_grants[&acquisition][owner].clone();
        participant.grant.state = CompletionGrantStateV1::Prepared;
        participant.grant.activation_deadline_boundary =
            Some(participant.grant.expires_after_boundary);
        participant.grant.revision = participant
            .grant
            .revision
            .next()
            .expect("prepared revision");
        state
            .acknowledge_external_participant(&holder, participant)
            .expect("acknowledge owner-prepared completion participant");
    }
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(format!(
                    "canwu.force-supply-reference:operation:completion-activate:{suffix}"
                ))
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::Completion {
                    operation: ForceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                        acquisition: acquisition.clone(),
                        expected_acquisition_revision: state.completion_leases.acquisitions
                            [&acquisition]
                            .revision,
                        grant: force_grant.clone(),
                        expected_grant_revision: state.completion_leases.grants[&force_grant]
                            .revision,
                        at: service_at,
                        current_boundary: 1,
                        eligibility_envelope_digest: envelope_digest,
                    }),
                },
            },
            service_at,
        )
        .expect("activate force completion");
    let completion_certificate = state.completion_leases.certificates[&acquisition].clone();
    let mut stock_custody = ForceStockCustodyBindingV1 {
        destination_account: ResourceAccountId::new("canwu.resource:account:local-granary")
            .expect("ID"),
        destination_custodian: holder.clone(),
        accepted_transfer: None,
        semantic_digest: String::new(),
    };
    stock_custody.semantic_digest =
        canonical_hash("canwu.force-supply.stock-custody.v1", &stock_custody)
            .expect("stock custody digest");
    let mut intent = ForceConsumptionIntent {
        id: ForceConsumptionIntentId::new(format!("canwu.force-supply-reference:intent:{suffix}"))
            .expect("ID"),
        revision: 1,
        force: force.clone(),
        expected_force_runtime_revision: state.revision,
        expected_force_revision: state.forces[force].revision,
        requirement: requirement.clone(),
        scheduled_due: SimTime::EPOCH,
        due_at: service_at,
        due_count: 0,
        requested_quantity: 0,
        allocation: ResourceAllocationLegVersionV1 {
            id: ResourceAllocationLegId::new(format!("canwu.resource:allocation:{suffix}"))
                .expect("ID"),
            revision: revision(),
            account: ResourceAccountId::new("canwu.resource:account:local-granary").expect("ID"),
            account_revision: revision(),
            resource_revision: resource.clone(),
            unit_revision: unit.clone(),
            quantity: 30,
            semantic_digest: "c".repeat(64),
        },
        stock_custody,
        resource_operation_key: operation_key,
        consumption_id: ResourceConsumptionId::new(format!(
            "canwu.resource:consumption:force-{suffix}"
        ))
        .expect("ID"),
        requisition_policy: requisition
            .then(|| state.requisition_policies.keys().next().cloned())
            .flatten(),
        completion_certificate,
        status: ForceConsumptionIntentStatus::PendingResourceConsumption,
        resource_outcome: None,
        resource_outcome_source: None,
        consequence: None,
        semantic_digest: String::new(),
    };
    intent.semantic_digest =
        canonical_hash("canwu.force-supply.intent.v1", &intent).expect("canonical force intent");
    intent
}

fn persisted_roundtrip(state: &ForceSupplyStateV1) -> ForceSupplyStateV1 {
    let encoded = serde_json::to_string(state).expect("serialize force-supply state");
    let restored: ForceSupplyStateV1 =
        serde_json::from_str(&encoded).expect("restore force-supply state");
    restored.validate().expect("validate restored state");
    assert_eq!(&restored, state);
    restored
}

#[test]
#[allow(clippy::too_many_lines)]
fn requisition_keeps_resource_force_externality_and_ack_as_distinct_steps() {
    let (mut state, holder, force, requirement, resource, unit) = base_state();
    let intent = requisition_intent(&mut state, &force, &requirement, &resource, &unit);
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:submit-requisition",
                )
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::SubmitConsumptionIntent {
                    intent: intent.clone(),
                },
            },
            SimTime::EPOCH,
        )
        .expect("admit intent");
    state = persisted_roundtrip(&state);
    let saga_id = state.forces[&force]
        .blocked_by_active_requisition
        .clone()
        .expect("active saga");
    assert_eq!(
        state.sagas[&saga_id].stage,
        RequisitionSagaStage::PendingResourceConsumption
    );

    let outcome = seal_resource_outcome(ResourceOperationOutcomeVersionV1 {
        id: ResourceOperationOutcomeId::new("canwu.resource:outcome:force-requisition")
            .expect("ID"),
        revision: revision(),
        operation_key: intent.resource_operation_key.clone(),
        status: ResourceOperationStatus::Applied,
        quantity: 30,
        remainder: 10,
        result_ref: Some(ResourceRecordRefV1::Consumption(
            intent.consumption_id.clone(),
        )),
        semantic_digest: String::new(),
    });
    let packet = ResourceOutcomePacketV1 {
        intent: intent.id.clone(),
        authoritative_resource_state: provider_version(
            resource_runtime_reference().into_untyped(),
            1,
        ),
        outcome_id: outcome.id.clone(),
    };
    let settlement = settlement_for(&intent, &outcome);
    complete_external_owner(&mut state, &intent, canwu_resource::PLUGIN_NAME);
    state
        .acknowledge_resource_outcome(&packet, &settlement, SimTime::EPOCH)
        .expect("resource outcome");
    state
        .acknowledge_resource_outcome(&packet, &settlement, SimTime::EPOCH)
        .expect("duplicate exact outcome");
    state = persisted_roundtrip(&state);
    assert_eq!(state.forces[&force].readiness_per_mille, 800);
    assert_eq!(state.consequences.len(), 1);
    assert_eq!(
        state.sagas[&saga_id].stage,
        RequisitionSagaStage::ExternalityPending
    );
    assert_eq!(state.externality_intents.len(), 1);

    let externality_intent = state.sagas[&saga_id]
        .externality_intent
        .clone()
        .expect("externality intent");
    let mut externality = EconomyExternalityOutcomeVersionV1 {
        id: ExternalityOutcomeId::new("canwu.force-supply-reference:economy-outcome:requisition")
            .expect("ID"),
        revision: 1,
        intent: externality_intent,
        disposition: ExternalityOutcomeDisposition::Applied,
        expected_target: intent
            .completion_certificate
            .locked_target_versions
            .iter()
            .find_map(|target| match target {
                CompletionLockedTargetV1::ExternalRecord { version }
                    if version.record.kind.namespace == "canwu.economy-reference" =>
                {
                    Some(version.clone())
                }
                _ => None,
            })
            .expect("exact economy target"),
        resulting_target_revision: Some(2),
        blocker: None,
        semantic_digest: String::new(),
    };
    externality.semantic_digest = canonical_hash(
        "canwu.force-supply.economy-externality-outcome.v1",
        &externality,
    )
    .expect("externality digest");
    let mut wrong_target = externality.clone();
    wrong_target.expected_target.record.id = "canwu.economy-reference:runtime:different".to_owned();
    wrong_target.semantic_digest.clear();
    wrong_target.semantic_digest = canonical_hash(
        "canwu.force-supply.economy-externality-outcome.v1",
        &wrong_target,
    )
    .expect("wrong-target digest");
    let wrong_target_source = provider_version(
        economy_externality_outcome_reference(&wrong_target.id).into_untyped(),
        wrong_target.revision,
    );
    let error = state
        .acknowledge_externality_outcome(
            &ExternalityOutcomePacketV1 {
                saga: saga_id.clone(),
                authoritative_outcome: wrong_target_source,
                authoritative_participant: provider_version(
                    force_externality_completion_participant_reference(
                        &intent.completion_certificate.acquisition,
                    )
                    .into_untyped(),
                    1,
                ),
            },
            &wrong_target,
            SimTime::EPOCH,
        )
        .expect_err("different exact economy target must fail closed");
    assert!(
        error
            .message
            .contains("different exact economy record version")
    );
    let externality_source = provider_version(
        economy_externality_outcome_reference(&externality.id).into_untyped(),
        externality.revision,
    );
    complete_external_owner(&mut state, &intent, ECONOMY_EXTERNALITY_PROVIDER);
    state
        .acknowledge_externality_outcome(
            &ExternalityOutcomePacketV1 {
                saga: saga_id.clone(),
                authoritative_outcome: externality_source,
                authoritative_participant: provider_version(
                    force_externality_completion_participant_reference(
                        &intent.completion_certificate.acquisition,
                    )
                    .into_untyped(),
                    1,
                ),
            },
            &externality,
            SimTime::EPOCH,
        )
        .expect("externality outcome");
    state = persisted_roundtrip(&state);
    assert_eq!(
        state.sagas[&saga_id].stage,
        RequisitionSagaStage::ExternalityApplied
    );

    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:finalize-requisition",
                )
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::FinalizeRequisition {
                    saga: saga_id.clone(),
                },
            },
            SimTime::EPOCH,
        )
        .expect("final force acknowledgement");
    assert!(!state.sagas.contains_key(&saga_id));
    assert!(
        state
            .terminal_receipts
            .values()
            .any(|receipt| receipt.saga.as_ref() == Some(&saga_id)
                && receipt.final_ack_digest.is_some())
    );
    assert!(state.forces[&force].blocked_by_active_requisition.is_none());
}

#[test]
fn forged_resource_packet_identity_cannot_override_authoritative_outcome() {
    let (mut state, holder, force, requirement, resource, unit) = base_state();
    let intent = requisition_intent(&mut state, &force, &requirement, &resource, &unit);
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:submit-forged-packet-test",
                )
                .expect("ID"),
                holder,
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::SubmitConsumptionIntent {
                    intent: intent.clone(),
                },
            },
            SimTime::EPOCH,
        )
        .expect("submit intent");
    let authoritative = seal_resource_outcome(ResourceOperationOutcomeVersionV1 {
        id: ResourceOperationOutcomeId::new("canwu.resource:outcome:authoritative").expect("ID"),
        revision: revision(),
        operation_key: intent.resource_operation_key.clone(),
        status: ResourceOperationStatus::Applied,
        quantity: 30,
        remainder: 10,
        result_ref: Some(ResourceRecordRefV1::Consumption(
            intent.consumption_id.clone(),
        )),
        semantic_digest: String::new(),
    });
    let forged_packet = ResourceOutcomePacketV1 {
        intent: intent.id.clone(),
        authoritative_resource_state: provider_version(
            resource_runtime_reference().into_untyped(),
            1,
        ),
        outcome_id: ResourceOperationOutcomeId::new("canwu.resource:outcome:forged").expect("ID"),
    };
    let settlement = settlement_for(&intent, &authoritative);
    let error = state
        .acknowledge_resource_outcome(&forged_packet, &settlement, SimTime::EPOCH)
        .expect_err("forged packet must fail");
    assert!(error.message.contains("exactly settle"));
    assert_eq!(state.forces[&force].readiness_per_mille, 900);
}

#[test]
fn holder_relative_report_rejects_an_unauthorized_commander() {
    let (mut state, holder, force, requirement, _, _) = base_state();
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:record-observation",
                )
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::RecordSupplyObservation {
                    force: force.clone(),
                    observation: ForceSupplyObservationV1 {
                        requirement,
                        known_stock_low: 20,
                        known_stock_high: 30,
                        demand_forecast: 40,
                        arrival_state: "arrival_pending".to_owned(),
                        source: ForceSupplyObservationSourceV1::ResourceProvider,
                        observed_at: SimTime::EPOCH,
                        confidence_per_mille: 800,
                        source_versions: vec![provider_version(
                            resource_runtime_reference().into_untyped(),
                            1,
                        )],
                    },
                },
            },
            SimTime::EPOCH,
        )
        .expect("publish holder observation");
    let report =
        report_from_state(&state, &holder, &force, SimTime::EPOCH).expect("authorized report");
    assert_eq!(report.observations.len(), 1);
    assert_eq!(report.observations[0].arrival_state, "arrival_pending");

    let unauthorized = KnowledgeHolderRef::Person(PersonId::new(2));
    assert!(report_from_state(&state, &unauthorized, &force, SimTime::EPOCH).is_err());
}

#[test]
fn delayed_holder_report_does_not_reveal_current_requisition_truth() {
    let (mut state, holder, force, requirement, resource, unit) = base_state();
    let grant = state
        .observation_grants
        .values_mut()
        .find(|grant| grant.force == force)
        .expect("grant");
    grant.role = ForceObservationRole::RemoteCommander;
    grant.observation_delay_minutes = 60;
    let intent = requisition_intent_at(
        &mut state,
        &force,
        &requirement,
        &resource,
        &unit,
        SimTime::from_minutes(120),
    );
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:submit-delayed-report-test",
                )
                .expect("ID"),
                holder: holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::SubmitConsumptionIntent {
                    intent: intent.clone(),
                },
            },
            SimTime::from_minutes(120),
        )
        .expect("submit intent");
    let outcome = seal_resource_outcome(ResourceOperationOutcomeVersionV1 {
        id: ResourceOperationOutcomeId::new("canwu.resource:outcome:delayed-report").expect("ID"),
        revision: revision(),
        operation_key: intent.resource_operation_key.clone(),
        status: ResourceOperationStatus::Applied,
        quantity: 30,
        remainder: 10,
        result_ref: Some(ResourceRecordRefV1::Consumption(
            intent.consumption_id.clone(),
        )),
        semantic_digest: String::new(),
    });
    let packet = ResourceOutcomePacketV1 {
        intent: intent.id.clone(),
        authoritative_resource_state: provider_version(
            resource_runtime_reference().into_untyped(),
            1,
        ),
        outcome_id: outcome.id.clone(),
    };
    let settlement = settlement_for(&intent, &outcome);
    complete_external_owner(&mut state, &intent, canwu_resource::PLUGIN_NAME);
    state
        .acknowledge_resource_outcome(&packet, &settlement, SimTime::from_minutes(120))
        .expect("acknowledge outcome");
    assert!(state.sagas.values().any(|saga| {
        saga.force == force && saga.stage == RequisitionSagaStage::ExternalityPending
    }));

    let before_publication = report_from_state(&state, &holder, &force, SimTime::from_minutes(150))
        .expect("delayed report");
    assert_eq!(before_publication.requisition_stage, None);
    assert!(before_publication.shortage_attribution.is_empty());

    let after_publication = report_from_state(&state, &holder, &force, SimTime::from_minutes(180))
        .expect("published report");
    assert_eq!(
        after_publication.requisition_stage,
        Some(RequisitionSagaStage::ExternalityPending)
    );
    assert_eq!(after_publication.shortage_attribution.len(), 1);
    persisted_roundtrip(&state);
}

#[test]
fn distinct_resources_and_cadences_do_not_collapse_into_a_supply_score() {
    let (state, _, _, _, _, _) = base_state();
    let food = state
        .profiles
        .values()
        .flat_map(|profile| &profile.requirements)
        .find(|requirement| requirement.kind == SupplyResourceKind::Food)
        .expect("compiled food requirement");
    let fuel = state
        .profiles
        .values()
        .flat_map(|profile| &profile.requirements)
        .find(|requirement| requirement.kind == SupplyResourceKind::Fuel)
        .expect("compiled fuel requirement");
    assert_eq!(
        food.cadence,
        ForceSupplyCadenceV1::FixedMinutes {
            interval_minutes: 1_440
        }
    );
    assert_eq!(
        fuel.cadence,
        ForceSupplyCadenceV1::FixedMinutes {
            interval_minutes: 360
        }
    );
    assert_ne!(food.resource_revision, fuel.resource_revision);
    assert_ne!(food.quantity_per_due, fuel.quantity_per_due);
    let due = state
        .due_requirements(SimTime::from_minutes(2_880), 64)
        .expect("bounded due work");
    let (_, _, scheduled, due_count, requested_quantity) = due
        .iter()
        .find(|(_, requirement, ..)| requirement == &food.id)
        .expect("food due work");
    assert_eq!(*scheduled, SimTime::EPOCH);
    assert_eq!(*due_count, 3);
    assert_eq!(
        *requested_quantity,
        food.quantity_per_due * u64::from(*due_count)
    );
}

#[test]
fn resealed_holder_authored_profile_or_policy_cannot_replace_compiled_coverage() {
    let (mut profile_state, _, _, _, _, _) = base_state();
    let profile = profile_state
        .profiles
        .values_mut()
        .next()
        .expect("compiled profile");
    profile.requirements[0].quantity_per_due += 1;
    profile.semantic_digest.clear();
    profile.semantic_digest =
        canonical_hash("canwu.force-supply.profile.v1", profile).expect("reseal forged profile");
    let error = profile_state
        .validate()
        .expect_err("profile must remain exact compiled coverage");
    assert!(error.message.contains("exact compiled economy coverage"));

    let (mut policy_state, _, _, _, _, _) = base_state();
    let policy = policy_state
        .requisition_policies
        .values_mut()
        .next()
        .expect("compiled policy");
    policy.cooperation_delta_per_mille -= 1;
    policy.semantic_digest.clear();
    policy.semantic_digest = canonical_hash("canwu.force-supply.requisition-policy.v1", policy)
        .expect("reseal forged policy");
    let error = policy_state
        .validate()
        .expect_err("policy must remain exact compiled coverage");
    assert!(error.message.contains("exact compiled economy coverage"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn terminal_capacity_rejects_new_acquisition_before_irreversible_work() {
    let (mut state, first_holder, force_one, requirement, resource, unit) = base_state();
    state.limits.max_terminal_receipts = 1;
    let force_two =
        ReferenceForceId::new("canwu.force-supply-reference:force:test-two").expect("ID");
    let mut second = state.forces[&force_one].clone();
    second.id = force_two.clone();
    let second_holder = KnowledgeHolderRef::Person(PersonId::new(2));
    second.holder = second_holder.clone();
    state.forces.insert(force_two.clone(), second);
    for due in state.forces[&force_two].due.values() {
        state
            .due_index
            .entry(due.next_due)
            .or_default()
            .insert((force_two.clone(), due.requirement.clone()));
    }
    state
        .configure_completion_authority(first_holder)
        .expect("first completion authority");
    state
        .configure_completion_authority(second_holder)
        .expect("second completion authority");

    let intent = consumption_intent_with_suffix(
        &mut state,
        &force_one,
        &requirement,
        &resource,
        &unit,
        "terminal-1",
        false,
        SimTime::EPOCH,
    );
    state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:terminal-submit-1",
                )
                .expect("ID"),
                holder: state.forces[&force_one].holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::SubmitConsumptionIntent {
                    intent: intent.clone(),
                },
            },
            SimTime::EPOCH,
        )
        .expect("submit terminal intent");
    let outcome = seal_resource_outcome(ResourceOperationOutcomeVersionV1 {
        id: ResourceOperationOutcomeId::new("canwu.resource:outcome:terminal-1").expect("ID"),
        revision: revision(),
        operation_key: intent.resource_operation_key.clone(),
        status: ResourceOperationStatus::Applied,
        quantity: 30,
        remainder: 10,
        result_ref: Some(ResourceRecordRefV1::Consumption(
            intent.consumption_id.clone(),
        )),
        semantic_digest: String::new(),
    });
    let settlement = settlement_for(&intent, &outcome);
    complete_external_owner(&mut state, &intent, canwu_resource::PLUGIN_NAME);
    state
        .acknowledge_resource_outcome(
            &ResourceOutcomePacketV1 {
                intent: intent.id,
                authoritative_resource_state: provider_version(
                    resource_runtime_reference().into_untyped(),
                    1,
                ),
                outcome_id: outcome.id.clone(),
            },
            &settlement,
            SimTime::EPOCH,
        )
        .expect("complete first terminal intent");
    assert_eq!(state.terminal_receipts.len(), 1);
    let acquisition = CompletionLeaseAcquisitionId::new(
        "canwu.force-supply-reference:completion-acquisition:terminal-2",
    )
    .expect("ID");
    let error = state
        .apply_operation(
            &ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(
                    "canwu.force-supply-reference:operation:completion-acquire:terminal-2",
                )
                .expect("ID"),
                holder: state.forces[&force_two].holder.clone(),
                expected_runtime_revision: state.revision,
                operation: ForceOperationV1::Completion {
                    operation: ForceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
                        id: acquisition.clone(),
                        operation_key: ResourceOperationKey::new(
                            "canwu.resource:operation:force-terminal-2",
                        )
                        .expect("ID"),
                        holder: state.forces[&force_two].holder.clone(),
                        operation_namespace: "canwu.force-supply-reference:requisition".to_owned(),
                        eligibility_time: SimTime::EPOCH,
                        eligibility_envelope: EligibilityEnvelopeV1::new(
                            vec![provider_version(
                                force_supply_runtime_reference().into_untyped(),
                                state.revision,
                            )],
                            BTreeMap::new(),
                            BTreeSet::new(),
                            Vec::new(),
                            Vec::new(),
                        )
                        .expect("eligibility"),
                        recipe: CompletionCapacityRecipeV1 {
                            receipts: 8,
                            mutations: 16,
                            reports_per_holder: 1,
                            holders: 1,
                            bytes: 8_192,
                        },
                        expected_participants: BTreeSet::from([
                            PLUGIN_NAME.to_owned(),
                            canwu_resource::PLUGIN_NAME.to_owned(),
                        ]),
                        policy_class: CompletionPolicyClassV1::Guaranteed,
                    }),
                },
            },
            SimTime::EPOCH,
        )
        .expect_err("archive backpressure must reject before grant/prepare/activation");
    assert!(error.message.contains("archive_backpressure"));
    assert!(
        !state
            .completion_leases
            .acquisitions
            .contains_key(&acquisition)
    );
    persisted_roundtrip(&state);
}
