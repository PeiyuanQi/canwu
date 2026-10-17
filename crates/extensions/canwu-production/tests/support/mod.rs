//! Shared fixture of the production archive and admission tests: an
//! in-memory archive store, a household production root, and settled or
//! startable executions built through the public state API.

#![allow(dead_code, clippy::too_many_lines, clippy::trivially_copy_pass_by_ref)]

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, EntityRef, Government, GovernmentId, KnowledgeHolderRef,
    KnowledgeSnapshot, MapPoint, Person, PersonId, Scenario, SimTime, Territory, TerritoryId,
    TypedDomainRecordRef, WorldSnapshot,
};
use canwu_production::*;
use canwu_resource::{
    CompletionCapacityGrantId, CompletionCapacityGrantV1, CompletionCapacityPartitionV1,
    CompletionCapacityRecipeV1, CompletionGrantStateV1, CompletionLeaseAcquisitionId,
    CompletionLeaseActivationCertificateV1, CompletionLockedTargetV1, CompletionPolicyClassV1,
    ConsumptionStatus, EligibilityEnvelopeV1, GrantCompletionCapacityV1,
    MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE, PrepareCompletionCapacityV1, RequestCompletionLeaseV1,
    ResourceAccountId, ResourceAllocationLegId, ResourceAllocationLegVersionV1,
    ResourceArchiveRetentionHandleV1, ResourceArchiveRetentionPhaseV1, ResourceArchiveStore,
    ResourceConsumption, ResourceConsumptionId, ResourceConsumptionVersionV1,
    ResourceDefinitionRevisionId, ResourceDemandId, ResourceError, ResourceOperationKey,
    ResourceOperationKind, ResourceOperationOutcome, ResourceOperationOutcomeId,
    ResourceOperationOutcomeVersionV1, ResourceOperationRequestV1, ResourceOperationStatus,
    ResourceRevision, ResourceRuntimeRecord, ResourceUnitRevisionId, RunBudgetRevisionV1,
    canonical_digest,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::RwLock;

type Objects = BTreeMap<(String, String), Vec<u8>>;

/// An in-memory archive store for both plugins. It records the last phase
/// each retention handle was persisted or finalized in.
#[derive(Clone, Default)]
pub struct TestArchive {
    pub objects: Arc<RwLock<Objects>>,
    pub retention: Arc<RwLock<BTreeMap<String, ProductionArchiveRetentionPhaseV1>>>,
}

impl TestArchive {
    /// Every object id the store holds under `namespace`.
    pub fn ids(&self, namespace: &str) -> BTreeSet<String> {
        self.objects
            .read()
            .unwrap()
            .keys()
            .filter(|(stored, _)| stored == namespace)
            .map(|(_, id)| id.clone())
            .collect()
    }

    pub fn remove(&self, namespace: &str, id: &str) -> Option<Vec<u8>> {
        self.objects
            .write()
            .unwrap()
            .remove(&(namespace.to_owned(), id.to_owned()))
    }

    pub fn put(&self, namespace: &str, id: &str, bytes: Vec<u8>) {
        self.objects
            .write()
            .unwrap()
            .insert((namespace.to_owned(), id.to_owned()), bytes);
    }
}

impl ProductionArchiveStore for TestArchive {
    fn store_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), CanwuError> {
        self.put(namespace, object_id, bytes.to_vec());
        Ok(())
    }

    fn load_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_production_archive_retention(
        &self,
        handle: &ProductionArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        self.retention
            .write()
            .unwrap()
            .insert(handle.handle_id.clone(), handle.phase);
        Ok(())
    }

    fn finalize_production_archive_retention(
        &self,
        handle_id: &str,
        phase: ProductionArchiveRetentionPhaseV1,
    ) -> Result<(), CanwuError> {
        self.retention
            .write()
            .unwrap()
            .insert(handle_id.to_owned(), phase);
        Ok(())
    }
}

impl ResourceArchiveStore for TestArchive {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.put(namespace, object_id, bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        _handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for TestArchive {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.load_production_archive_object(namespace, object_id)
    }
}

pub fn version<T: DomainRecordType>(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: TypedDomainRecordRef::<T>::new(id).into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

pub fn revision(value: u64) -> ResourceRevision {
    ResourceRevision::new(value).expect("resource revision")
}

pub fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

pub fn site_id() -> ProductionSiteId {
    ProductionSiteId::new("production:fixture-site").expect("site")
}

pub fn facility_id() -> FacilityAssetId {
    FacilityAssetId::new("production:fixture-facility").expect("facility")
}

fn requirement(
    id: &str,
    kind: ProductionRequirementKind,
    capability: &str,
) -> ProductionRequirementGroup {
    ProductionRequirementGroup {
        id: format!("production:requirement:{id}"),
        kind,
        any_of: vec![ProductionRequirementAlternative {
            id: format!("production:requirement-alternative:{id}:primary"),
            capability: capability.to_owned(),
            minimum_quantity: 1,
        }],
    }
}

pub fn process() -> ProcessRevision {
    let unit = ResourceUnitRevisionId::new("resource:mass-unit:v1").expect("unit");
    ProcessRevision {
        id: ProcessRevisionId::new("production:household-process:v1").expect("process"),
        label: "household hand mill".to_owned(),
        semantic_digest: "digest-household-process".to_owned(),
        effective_from: SimTime::EPOCH,
        effective_until: None,
        work_units: 10,
        requirements: vec![
            requirement(
                "household-skill",
                ProductionRequirementKind::LaborCapability,
                "customary-hand-milling",
            ),
            requirement(
                "household-access",
                ProductionRequirementKind::Authorization,
                "household-authority",
            ),
        ],
        inputs: vec![ResourceRequirement {
            resource: ResourceDefinitionRevisionId::new("resource:grain:v1").expect("grain"),
            unit: unit.clone(),
            quantity: 10,
        }],
        outputs: vec![ProductionOutputSpec {
            resource: ResourceDefinitionRevisionId::new("resource:flour:v1").expect("flour"),
            unit,
            quantity: 8,
            quality_class: "ordinary".to_owned(),
        }],
        capacity: vec![CapacityRequirement {
            capability: "bench".to_owned(),
            quantity: 1,
        }],
        adoption_required: false,
        max_realized_per_mille: NOMINAL_REALIZED_OUTPUT_PER_MILLE,
        realization_evidence_kinds: BTreeSet::new(),
    }
}

/// One site with one operational facility and the household process.
pub fn base_state() -> ProductionState {
    let mut state = ProductionState {
        revision: 1,
        ..ProductionState::default()
    };
    let process = process();
    state.processes.insert(process.id.clone(), process);
    state.sites.insert(
        site_id(),
        ProductionSite {
            id: site_id(),
            holder: holder(),
            place: EntityRef::Territory(TerritoryId::new(1)),
            form: ProductionSiteForm::DistributedWorkshop,
            active: true,
        },
    );
    state.facilities.insert(
        facility_id(),
        FacilityAsset {
            id: facility_id(),
            site: site_id(),
            generation: 1,
            lifecycle: FacilityLifecycle::Operational,
            condition_per_mille: 1_000,
            capacity: BTreeMap::from([("bench".to_owned(), 1)]),
            maintenance_evidence: Vec::new(),
            operational_stage_capacity_per_mille: 0,
            incident_risk_per_mille: 0,
            incident_max_severity_per_mille: 0,
        },
    );
    state
}

pub fn command(
    state: &ProductionState,
    id: &str,
    operation: ProductionOperation,
) -> ProductionCommandEnvelope {
    ProductionCommandEnvelope {
        operation_id: ProductionOperationOutcomeId::new(id).expect("operation ID"),
        holder: holder(),
        expected_runtime_revision: state.revision,
        operation,
    }
}

/// The consumption and consumption outcome a production input binds.
fn input_payloads(input: &ResourceInputBinding) -> (ResourceConsumption, ResourceOperationOutcome) {
    let mut consumption = ResourceConsumption {
        id: input.consumption.id.clone(),
        revision: input.consumption.revision,
        account: input.consumption.account.clone(),
        allocation_leg: input.consumption.allocation_leg.clone(),
        demand: ResourceDemandId::new("resource:fixture-input-demand").expect("demand"),
        resource_revision: input.allocation_leg.resource_revision.clone(),
        unit_revision: input.allocation_leg.unit_revision.clone(),
        quantity: input.quantity,
        consumer_evidence: input.consumption.consumer_evidence.clone(),
        completion_acquisition: CompletionLeaseAcquisitionId::new(
            "resource:fixture-input-completion",
        )
        .expect("acquisition"),
        status: ConsumptionStatus::Settled,
        operation_key: input.consumption_outcome.operation_key.clone(),
        fulfillment: canwu_resource::ResourceFulfillmentId::new("resource:fulfillment:2")
            .expect("fulfillment"),
        semantic_digest: String::new(),
        terminal_sequence: 1,
    };
    consumption.semantic_digest =
        canonical_digest("canwu.resource.consumption.v1", &consumption).expect("digest");
    let request =
        ResourceOperationRequestV1::Consume(canwu_resource::ResourceConsumptionRequestV1 {
            operation_key: input.consumption_outcome.operation_key.clone(),
            consumption_id: input.consumption.id.clone(),
            allocation: input.allocation_leg.clone(),
            expected_account_revision: input.allocation_leg.account_revision,
            consumer_evidence: input.consumption.consumer_evidence.clone(),
            at: SimTime::EPOCH,
            completion_certificate: CompletionLeaseActivationCertificateV1 {
                acquisition: consumption.completion_acquisition.clone(),
                acquisition_revision: ResourceRevision::INITIAL,
                operation_key: input.consumption_outcome.operation_key.clone(),
                prepared_grants: Vec::new(),
                locked_target_versions: Vec::new(),
                recipe_digest: "0".repeat(64),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope_digest: "0".repeat(64),
                activation_boundary: 0,
                semantic_digest: "0".repeat(64),
            },
        });
    let request_digest =
        canonical_digest("canwu.resource.operation-request.v1", &request).expect("digest");
    let mut outcome = ResourceOperationOutcome {
        id: ResourceOperationOutcomeId::new(format!(
            "resource:outcome:{}:{}",
            input
                .consumption_outcome
                .operation_key
                .as_str()
                .replace(':', "-"),
            &request_digest[..16]
        ))
        .expect("outcome"),
        revision: input.consumption_outcome.revision,
        operation_key: input.consumption_outcome.operation_key.clone(),
        request_digest,
        kind: ResourceOperationKind::Consume,
        status: input.consumption_outcome.status,
        quantity: input.quantity,
        remainder: input.consumption_outcome.remainder,
        result_ref: Some(canwu_resource::ResourceRecordRefV1::Consumption(
            consumption.id.clone(),
        )),
        cited_transfers: Vec::new(),
        rejection_code: None,
        rejection_reason: None,
        exact_evidence: vec![consumption.consumer_evidence.clone()],
        semantic_digest: String::new(),
        sequence: 2,
    };
    outcome.semantic_digest =
        canonical_digest("canwu.resource.operation-outcome.v1", &outcome).expect("digest");
    (consumption, outcome)
}

fn resource_input(suffix: &str) -> ResourceInputBinding {
    let process = process();
    let account = ResourceAccountId::new(format!("resource:input-account:{suffix}")).expect("id");
    let leg_id = ResourceAllocationLegId::new(format!("resource:input-leg:{suffix}")).expect("id");
    let mut binding = ResourceInputBinding {
        allocation_leg: ResourceAllocationLegVersionV1 {
            id: leg_id.clone(),
            revision: revision(1),
            account: account.clone(),
            account_revision: revision(1),
            resource_revision: process.inputs[0].resource.clone(),
            unit_revision: process.inputs[0].unit.clone(),
            quantity: process.inputs[0].quantity,
            semantic_digest: "input-leg-digest".to_owned(),
        },
        consumption: ResourceConsumptionVersionV1 {
            id: ResourceConsumptionId::new(format!("resource:consumption:{suffix}")).expect("id"),
            revision: revision(1),
            account,
            allocation_leg: leg_id,
            quantity: process.inputs[0].quantity,
            consumer_evidence: version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID),
            semantic_digest: String::new(),
        },
        consumption_outcome: ResourceOperationOutcomeVersionV1 {
            id: ResourceOperationOutcomeId::new(format!("resource:consume-outcome:{suffix}"))
                .expect("id"),
            revision: revision(1),
            operation_key: ResourceOperationKey::new(format!("resource:consume-key:{suffix}"))
                .expect("id"),
            status: ResourceOperationStatus::Applied,
            quantity: process.inputs[0].quantity,
            remainder: 0,
            result_ref: None,
            semantic_digest: String::new(),
        },
        quantity: process.inputs[0].quantity,
    };
    let (consumption, outcome) = input_payloads(&binding);
    binding.consumption_outcome.id = outcome.id;
    binding.consumption_outcome.result_ref = outcome.result_ref;
    binding.consumption.semantic_digest = consumption.semantic_digest;
    binding.consumption_outcome.semantic_digest = outcome.semantic_digest;
    binding
}

/// A work order named by `suffix`, created and authorized.
pub fn authorized_order(state: &mut ProductionState, suffix: &str) -> WorkOrderId {
    let order_id = WorkOrderId::new(format!("production:order:{suffix}")).expect("order");
    state
        .apply_operation(
            &command(
                state,
                &format!("production:create:{suffix}"),
                ProductionOperation::CreateWorkOrder {
                    work_order: WorkOrder {
                        id: order_id.clone(),
                        holder: holder(),
                        process: process().id,
                        site: site_id(),
                        quantity: 1,
                        lifecycle: WorkOrderLifecycle::Proposed,
                        execution: None,
                        expected_revision: 1,
                        created_at: SimTime::EPOCH,
                    },
                },
            ),
            SimTime::EPOCH,
        )
        .expect("create work order");
    state
        .apply_operation(
            &command(
                state,
                &format!("production:authorize:{suffix}"),
                ProductionOperation::AuthorizeWorkOrder {
                    work_order: order_id.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize work order");
    order_id
}

/// An activated completion lease for one execution named by `suffix`, with
/// the participant grant the resource plugin would hold for it. It locks one
/// output account per output of the process, and its key is the first
/// output's.
pub struct ActivatedLease {
    pub suffix: String,
    pub at: SimTime,
    pub order: WorkOrderId,
    pub input: ResourceInputBinding,
    pub outputs: Vec<ProductionOutputSettlementRequest>,
    pub certificate: CompletionLeaseActivationCertificateV1,
    pub production_grant: CompletionCapacityGrantId,
    pub resource_grant: CompletionCapacityGrantId,
    pub participant_grant: CompletionCapacityGrantV1,
}

fn completion_recipe() -> CompletionCapacityRecipeV1 {
    CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 1,
        holders: 1,
        bytes: 4_096,
    }
}

fn eligibility_envelope() -> EligibilityEnvelopeV1 {
    EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("eligibility")
}

/// The request of the production completion lease named by `suffix`, under
/// `operation_key`, eligible at `at`.
pub fn lease_request(
    suffix: &str,
    operation_key: ResourceOperationKey,
    at: SimTime,
) -> RequestCompletionLeaseV1 {
    RequestCompletionLeaseV1 {
        id: CompletionLeaseAcquisitionId::new(format!(
            "production:completion-acquisition:{suffix}"
        ))
        .expect("acquisition"),
        operation_key,
        holder: holder(),
        operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
        eligibility_time: at,
        eligibility_envelope: eligibility_envelope(),
        recipe: completion_recipe(),
        expected_participants: BTreeSet::from([
            canwu_production::PLUGIN_NAME.to_owned(),
            canwu_resource::PLUGIN_NAME.to_owned(),
        ]),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    }
}

pub fn activate_lease(state: &mut ProductionState, suffix: &str) -> ActivatedLease {
    let order = authorized_order(state, suffix);
    let at = SimTime::from_minutes(
        i64::try_from(state.completion_acquisitions.len()).expect("completion time"),
    );
    let input = resource_input(suffix);
    let outputs = state.processes[&process().id]
        .outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let name = if index == 0 {
                suffix.to_owned()
            } else {
                format!("{suffix}:{index}")
            };
            ProductionOutputSettlementRequest {
                operation_key: ResourceOperationKey::new(format!("resource:output-key:{name}"))
                    .expect("key"),
                account: ResourceAccountId::new(format!("resource:output-account:{name}"))
                    .expect("account"),
                expected_account_revision: revision(1),
                resource: output.resource.clone(),
                unit: output.unit.clone(),
                quantity: output.quantity,
            }
        })
        .collect::<Vec<_>>();
    let output_key = outputs[0].operation_key.clone();
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("production:completion-acquisition:{suffix}"))
            .expect("acquisition");
    let production_grant =
        CompletionCapacityGrantId::new(format!("production:completion-grant:{suffix}"))
            .expect("grant");
    let resource_grant =
        CompletionCapacityGrantId::new(format!("resource:completion-grant:{suffix}"))
            .expect("grant");
    let recipe = completion_recipe();
    let units = recipe.canonical_units().expect("recipe units");
    let eligibility = eligibility_envelope();
    state.production_run_budget = Some(
        RunBudgetRevisionV1 {
            revision: revision(1),
            total_completion_units: units.saturating_mul(4),
            shared_pending_slots: 0,
            partitions: vec![CompletionCapacityPartitionV1 {
                authority: holder(),
                operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
                guaranteed_units: units.saturating_mul(2),
                reserved_pending_slots: 2,
                maximum_burst_units: units,
                request_token_capacity: 2,
                request_token_refill_minutes: 1,
                reacquire_cooldown_minutes: 1,
                root_acquisition_cap_per_sim_time: 2,
                guaranteed_max_wait_boundaries: 4,
            }],
            semantic_digest: String::new(),
        }
        .seal()
        .expect("run budget"),
    );
    state
        .request_completion_acquisition(lease_request(suffix, output_key.clone(), at))
        .expect("completion acquisition");
    state
        .apply_completion_ingress(&ProductionCompletionIngressV1::GrantLocal(
            GrantCompletionCapacityV1 {
                grant_id: production_grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: revision(1),
                owner_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                // Each lease locks its own production runtime version, as
                // leases activated at different boundaries do.
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: DomainRecordVersionRef {
                        version: u64::try_from(state.completion_acquisitions.len())
                            .expect("lease count"),
                        ..version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID)
                    },
                }],
                current_boundary: 1,
            },
        ))
        .expect("local grant");
    let mut participant_grant = CompletionCapacityGrantV1 {
        id: resource_grant.clone(),
        revision: revision(1),
        acquisition: acquisition.clone(),
        operation_key: output_key,
        owner_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
        run_budget_revision: revision(1),
        target_versions: outputs
            .iter()
            .map(|output| CompletionLockedTargetV1::Account {
                id: output.account.clone(),
                revision: output.expected_account_revision,
            })
            .chain([CompletionLockedTargetV1::AllocationLeg {
                id: input.allocation_leg.id.clone(),
                revision: input.allocation_leg.revision,
            }])
            .collect(),
        recipe_digest: recipe.digest().expect("recipe digest"),
        reserved_units: units,
        expires_after_boundary: 9,
        activation_deadline_boundary: None,
        state: CompletionGrantStateV1::Held,
        rejection: None,
    };
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantGrant {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: revision(2),
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant.clone(),
            },
        )
        .expect("participant grant");
    state
        .apply_completion_ingress(&ProductionCompletionIngressV1::PrepareLocal(
            PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: revision(3),
                grant: production_grant.clone(),
                expected_grant_revision: revision(1),
                current_boundary: 1,
                eligibility_envelope_digest: eligibility.digest.clone(),
            },
        ))
        .expect("local prepare");
    participant_grant.state = CompletionGrantStateV1::Prepared;
    participant_grant.revision = revision(2);
    participant_grant.activation_deadline_boundary = Some(8);
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantPrepared {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: revision(4),
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant.clone(),
            },
        )
        .expect("participant prepare");
    let certificate = state
        .apply_completion_ingress(&ProductionCompletionIngressV1::Activate {
            acquisition,
            expected_acquisition_revision: revision(5),
            current_boundary: 1,
        })
        .expect("activation")
        .expect("activation certificate");
    ActivatedLease {
        suffix: suffix.to_owned(),
        at,
        order,
        input,
        outputs,
        certificate,
        production_grant,
        resource_grant,
        participant_grant,
    }
}

/// The `StartExecution` operation of `lease`, with one output request per
/// output of the process.
pub fn start_operation(lease: &ActivatedLease) -> ProductionOperation {
    let suffix = &lease.suffix;
    let execution_id =
        ProductionExecutionId::new(format!("production:execution:{suffix}")).expect("execution");
    let allocation_id =
        ProductionCapacityAllocationId::new(format!("production:allocation:{suffix}"))
            .expect("allocation");
    let evidence = |kind, capability: &str| ProductionEvidenceBinding {
        kind,
        capability: capability.to_owned(),
        version: version::<canwu_technology::AssetBinding>(&format!(
            "technology:fixture-evidence:{capability}"
        )),
        semantic_digest: format!("digest-{capability}"),
        available_quantity: 1,
    };
    ProductionOperation::StartExecution {
        execution: ProductionExecution {
            id: execution_id.clone(),
            work_order: lease.order.clone(),
            process: process().id,
            site: site_id(),
            facility: facility_id(),
            allocations: vec![allocation_id.clone()],
            lifecycle: WorkOrderLifecycle::Running,
            started_at: lease.at,
            completed_at: None,
            evidence: vec![
                evidence(
                    ProductionRequirementKind::LaborCapability,
                    "customary-hand-milling",
                ),
                evidence(
                    ProductionRequirementKind::Authorization,
                    "household-authority",
                ),
            ],
            technology: TechnologyEvidenceBinding {
                technique_revision: version::<canwu_technology::TechniqueRevision>(
                    "technology:fixture-technique:v1",
                ),
                capability_qualification: Some(
                    version::<canwu_technology::CapabilityQualification>(
                        "technology:fixture-qualification:v1",
                    ),
                ),
                implementation: Some(version::<canwu_technology::ImplementationRecord>(
                    "technology:fixture-implementation:v1",
                )),
                adoption: None,
                semantic_digest: "technology-binding-digest".to_owned(),
            },
            inputs: vec![lease.input.clone()],
            output_requests: lease.outputs.clone(),
            output_outcomes: Vec::new(),
            output_source: None,
            output_ack_digest: None,
            completion_certificate: lease.certificate.clone(),
            production_completion_grant: lease.production_grant.clone(),
            resource_completion_grant: lease.resource_grant.clone(),
            realized_output_per_mille: None,
            realization_evidence: None,
            output_settled_at: None,
        },
        allocations: vec![ProductionCapacityAllocation {
            id: allocation_id,
            facility: facility_id(),
            facility_generation: 1,
            capability: "bench".to_owned(),
            start: lease.at,
            end: lease
                .at
                .checked_add(canwu_api::SimDuration::minutes(60))
                .expect("allocation end"),
            quantity: 1,
            work_order: lease.order.clone(),
            execution: execution_id,
            operation_key: format!("production:capacity:{suffix}"),
            state: CapacityAllocationState::Reserved,
        }],
    }
}

/// Adds one settled execution named by `suffix` to `state`: started,
/// advanced, completed, its output credited and acknowledged, and both
/// completion grants closed. It is then a terminal archive candidate.
pub fn settle_execution(state: &mut ProductionState, suffix: &str) -> ProductionExecutionId {
    let lease = activate_lease(state, suffix);
    let execution_id =
        ProductionExecutionId::new(format!("production:execution:{suffix}")).expect("execution");
    state
        .apply_operation(
            &command(
                state,
                &format!("production:start:{suffix}"),
                start_operation(&lease),
            ),
            lease.at,
        )
        .expect("start execution");
    let minutes = |value: i64| {
        lease
            .at
            .checked_add(canwu_api::SimDuration::minutes(value))
            .expect("fixture time")
    };
    state
        .apply_operation(
            &command(
                state,
                &format!("production:advance:{suffix}"),
                ProductionOperation::AdvanceExecution {
                    execution: execution_id.clone(),
                    completed_units: 10,
                },
            ),
            minutes(10),
        )
        .expect("advance");
    state
        .apply_operation(
            &command(
                state,
                &format!("production:complete:{suffix}"),
                ProductionOperation::CompleteExecution {
                    execution: execution_id.clone(),
                    realized_output_per_mille: None,
                    realization_evidence: None,
                },
            ),
            minutes(20),
        )
        .expect("complete");
    let source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let outcomes = lease
        .outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let request = ResourceOperationRequestV1::Credit(output.resource_credit_request(
                source.clone(),
                lease.certificate.clone(),
                minutes(20),
            ));
            let request_digest =
                canonical_digest("canwu.resource.operation-request.v1", &request).expect("digest");
            let mut outcome = ResourceOperationOutcome {
                id: ResourceOperationOutcomeId::new(format!(
                    "resource:outcome:{}:{}",
                    output.operation_key.as_str().replace(':', "-"),
                    &request_digest[..16]
                ))
                .expect("outcome"),
                revision: revision(1),
                operation_key: output.operation_key.clone(),
                request_digest,
                kind: ResourceOperationKind::Credit,
                status: ResourceOperationStatus::Applied,
                quantity: output.quantity,
                remainder: 0,
                result_ref: None,
                cited_transfers: Vec::new(),
                rejection_code: None,
                rejection_reason: None,
                exact_evidence: vec![source.clone()],
                semantic_digest: String::new(),
                sequence: u64::try_from(index + 1).expect("sequence"),
            };
            outcome.semantic_digest =
                canonical_digest("canwu.resource.operation-outcome.v1", &outcome).expect("digest");
            outcome
        })
        .collect();
    state
        .executions
        .get_mut(&execution_id)
        .expect("completed execution")
        .output_source = Some(source.clone());
    state
        .acknowledge_output(
            &ProductionOutputAcknowledgement {
                execution: execution_id.clone(),
                production_source: source,
                outcomes,
                settled_at: minutes(20),
            },
            minutes(30),
        )
        .expect("output settlement");
    let resource_source = DomainRecordVersionRef {
        record: canwu_resource::resource_runtime_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let acquisition = lease.certificate.acquisition.clone();
    let mut participant_grant = lease.participant_grant.clone();
    participant_grant.state = CompletionGrantStateV1::Consumed;
    participant_grant.revision = revision(3);
    let consumed_revision = state.completion_acquisitions[&acquisition].revision;
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantConsumed {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: consumed_revision,
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: resource_source.clone(),
                grant: participant_grant.clone(),
            },
        )
        .expect("participant consumption");
    participant_grant.state = CompletionGrantStateV1::Completed;
    participant_grant.revision = revision(4);
    state
        .finalize_execution_resource_completion(&acquisition, &resource_source, &participant_grant)
        .expect("participant completion");
    execution_id
}

/// A production root holding `count` settled executions, ready to archive.
pub fn settled_state(count: usize) -> ProductionState {
    let mut state = base_state();
    for index in 0..count {
        settle_execution(&mut state, &format!("settled-{index:04}"));
    }
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    state.rebuild_runtime_indexes().expect("runtime indexes");
    state.validate().expect("settled fixture validates");
    state
}

pub fn scenario(state: ProductionState) -> Scenario {
    let actor = PersonId::new(1);
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    let world = WorldSnapshot {
        people: vec![Person {
            id: actor,
            name: "Workshop operator".to_owned(),
            government,
            current_location: territory,
            roles: Vec::new(),
            transit: None,
        }],
        governments: vec![Government {
            id: government,
            name: "Workshop authority".to_owned(),
            capital: territory,
        }],
        territories: vec![Territory {
            id: territory,
            name: "Workshop district".to_owned(),
            controller: government,
            position: MapPoint::default(),
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    Scenario {
        start_time: SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![state.into_initial_record().expect("production root")],
    }
}

/// A production-only run over `state`, with `archive` as its archive
/// provider.
pub fn runtime(plugin: &ProductionPlugin, state: ProductionState, archive: &TestArchive) -> Canwu {
    let mut canwu = Canwu::new_with_plugins(57, scenario(state), &[plugin]).expect("runtime");
    canwu.set_plugin_archive_object_provider(Arc::new(archive.clone()));
    canwu
}

pub fn production_state(canwu: &Canwu) -> ProductionState {
    canwu
        .typed_domain_record(&production_runtime_reference())
        .expect("production runtime")
        .decode_payload::<ProductionRuntimeRecord>()
        .expect("production state")
}

pub fn settle(canwu: &mut Canwu) {
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("boundary");
}

/// Queues one tracked production command. Its boundary admits it, and the
/// next boundary settles the operation.
pub fn enqueue_operation(
    canwu: &mut Canwu,
    request_id: u64,
    operation_id: &str,
    operation: ProductionOperation,
) {
    let expected_runtime_revision = production_state(canwu).revision;
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            canwu_api::CommandRequest::new(
                canwu_api::CommandRequestId::new(request_id),
                canwu.revision(),
                canwu_api::CommandEnvelope::new(
                    canwu_api::Issuer::Actor(PersonId::new(1)),
                    canwu_api::Command::Plugin {
                        plugin: canwu_production::PLUGIN_NAME.to_owned(),
                        command: PRODUCTION_COMMAND.to_owned(),
                        payload: serde_json::to_value(ProductionCommandEnvelope {
                            operation_id: ProductionOperationOutcomeId::new(operation_id)
                                .expect("operation ID"),
                            holder: holder(),
                            expected_runtime_revision,
                            operation,
                        })
                        .expect("production command payload"),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("tracked production command");
}
