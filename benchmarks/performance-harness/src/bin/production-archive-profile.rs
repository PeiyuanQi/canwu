//! Production archive profile: what restore validation pays for each
//! compacted resource input as the resource archive grows, what a production
//! archive commit, a lookup, and the restore audit cost over a long chain of
//! commits, and what one commit of several pages costs and writes.
//!
//! Every production record is real: settled executions are built through the
//! public state API, and every commit goes through
//! `prepare_production_archive`, `enqueue_production_archive`, and a boundary
//! of the production plugin, each commit archiving the maintenance receipt of
//! the one before it. The resource archive of the restore cases holds the
//! consumption and outcome of one production input under filler commits of
//! rejected-operation outcomes, prepared and stored through the resource
//! archive API.

use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, EntityRef, Government, GovernmentId, KnowledgeHolderRef,
    KnowledgeSnapshot, MapPoint, Person, PersonId, Scenario, SimTime, Territory, TerritoryId,
    TypedDomainRecordRef, WorldSnapshot,
};
use canwu_production::*;
use canwu_resource::{
    AllocationLegStatus, CompletionCapacityGrantId, CompletionCapacityGrantV1,
    CompletionCapacityPartitionV1, CompletionCapacityRecipeV1, CompletionGrantStateV1,
    CompletionLeaseAcquisitionId, CompletionLockedTargetV1, CompletionPolicyClassV1,
    ConsumptionStatus, EligibilityEnvelopeV1, GrantCompletionCapacityV1,
    MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE, PrepareCompletionCapacityV1, RequestCompletionLeaseV1,
    ResourceAccountId, ResourceAllocationLeg, ResourceAllocationLegId,
    ResourceAllocationLegVersionV1, ResourceArchiveRetentionHandleV1,
    ResourceArchiveRetentionPhaseV1, ResourceArchiveStore, ResourceCancelDemandRequestV1,
    ResourceConsumption, ResourceConsumptionId, ResourceConsumptionVersionV1,
    ResourceDefinitionRevisionId, ResourceDemandId, ResourceError, ResourceLimitsV1,
    ResourceOperationKey, ResourceOperationKind, ResourceOperationOutcome,
    ResourceOperationOutcomeId, ResourceOperationOutcomeVersionV1, ResourceOperationRequestV1,
    ResourceOperationStatus, ResourceReservationId, ResourceRevision, ResourceRuntimeRecord,
    ResourceState, ResourceTerminalRecordKeyV1, ResourceTieBreakKey, ResourceUnitRevisionId,
    RunBudgetRevisionV1, canonical_digest,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

type Objects = BTreeMap<(String, String), Vec<u8>>;

/// An in-memory archive store for both plugins that copies the bytes of
/// every object it serves and counts loads and load bytes. Clones share it,
/// and it is `Send + Sync`, as a store a run holds must be.
#[derive(Clone, Default)]
struct ProfileStore {
    objects: Arc<RwLock<Objects>>,
    loads: Arc<AtomicU64>,
    loaded_bytes: Arc<AtomicU64>,
}

impl ProfileStore {
    fn put(&self, namespace: &str, object_id: &str, bytes: &[u8]) {
        self.objects
            .write()
            .unwrap()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
    }

    fn get(&self, namespace: &str, object_id: &str) -> Option<Vec<u8>> {
        self.loads.fetch_add(1, Ordering::Relaxed);
        let value = self
            .objects
            .read()
            .unwrap()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned();
        if let Some(value) = &value {
            self.loaded_bytes
                .fetch_add(value.len() as u64, Ordering::Relaxed);
        }
        value
    }

    fn reset_counts(&self) {
        self.loads.store(0, Ordering::Relaxed);
        self.loaded_bytes.store(0, Ordering::Relaxed);
    }
}

impl ProductionArchiveStore for ProfileStore {
    fn store_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), CanwuError> {
        self.put(namespace, object_id, bytes);
        Ok(())
    }

    fn load_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self.get(namespace, object_id))
    }

    fn persist_production_archive_retention(
        &self,
        _handle: &ProductionArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        Ok(())
    }

    fn finalize_production_archive_retention(
        &self,
        _handle_id: &str,
        _phase: ProductionArchiveRetentionPhaseV1,
    ) -> Result<(), CanwuError> {
        Ok(())
    }
}

impl ResourceArchiveStore for ProfileStore {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.put(namespace, object_id, bytes);
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        Ok(self.get(namespace, object_id))
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

impl canwu_api::PluginArchiveObjectProvider for ProfileStore {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        Ok(self.get(namespace, object_id))
    }
}

fn milliseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

// ---------------------------------------------------------------------------
// One settled production execution, built through the public state API.

fn version<T: DomainRecordType>(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: TypedDomainRecordRef::<T>::new(id).into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn revision(value: u64) -> ResourceRevision {
    ResourceRevision::new(value).expect("resource revision")
}

fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

fn evidence(kind: ProductionRequirementKind, capability: &str) -> ProductionEvidenceBinding {
    ProductionEvidenceBinding {
        kind,
        capability: capability.to_owned(),
        version: version::<canwu_technology::AssetBinding>(&format!(
            "technology:profile-evidence:{capability}"
        )),
        semantic_digest: format!("digest-{capability}"),
        available_quantity: 1,
    }
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

fn process() -> ProcessRevision {
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

fn site_id() -> ProductionSiteId {
    ProductionSiteId::new("production:profile-site").expect("site")
}

fn facility_id() -> FacilityAssetId {
    FacilityAssetId::new("production:profile-facility").expect("facility")
}

fn base_state() -> ProductionState {
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

fn command(
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

/// The consumption and consumption outcome an archived production input
/// binds, as the resource archive holds them.
fn archived_input_payloads(
    input: &ResourceInputBinding,
) -> (ResourceConsumption, ResourceOperationOutcome) {
    let mut consumption = ResourceConsumption {
        id: input.consumption.id.clone(),
        revision: input.consumption.revision,
        account: input.consumption.account.clone(),
        allocation_leg: input.consumption.allocation_leg.clone(),
        demand: ResourceDemandId::new("resource:archived-production-input-demand").expect("demand"),
        resource_revision: input.allocation_leg.resource_revision.clone(),
        unit_revision: input.allocation_leg.unit_revision.clone(),
        quantity: input.quantity,
        consumer_evidence: input.consumption.consumer_evidence.clone(),
        completion_acquisition: CompletionLeaseAcquisitionId::new(
            "resource:archived-production-input-completion",
        )
        .expect("acquisition"),
        status: ConsumptionStatus::Settled,
        operation_key: input.consumption_outcome.operation_key.clone(),
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
            completion_certificate: canwu_resource::CompletionLeaseActivationCertificateV1 {
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

fn resource_input(process: &ProcessRevision, suffix: &str) -> ResourceInputBinding {
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
    let (consumption, outcome) = archived_input_payloads(&binding);
    binding.consumption_outcome.id = outcome.id;
    binding.consumption_outcome.result_ref = outcome.result_ref;
    binding.consumption.semantic_digest = consumption.semantic_digest;
    binding.consumption_outcome.semantic_digest = outcome.semantic_digest;
    binding
}

/// Adds one settled execution named by `suffix` to `state`.
fn settle_execution(state: &mut ProductionState, suffix: &str) -> ProductionExecutionId {
    let process = process();
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
                        process: process.id.clone(),
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

    let at = SimTime::from_minutes(
        i64::try_from(state.completion_acquisitions.len()).expect("completion time"),
    );
    let execution_id =
        ProductionExecutionId::new(format!("production:execution:{suffix}")).expect("execution");
    let allocation_id =
        ProductionCapacityAllocationId::new(format!("production:allocation:{suffix}"))
            .expect("allocation");
    let output_key =
        ResourceOperationKey::new(format!("resource:output-key:{suffix}")).expect("key");
    let input = resource_input(&process, suffix);
    let output = ProductionOutputSettlementRequest {
        operation_key: output_key.clone(),
        account: ResourceAccountId::new(format!("resource:output-account:{suffix}"))
            .expect("account"),
        expected_account_revision: revision(1),
        resource: process.outputs[0].resource.clone(),
        unit: process.outputs[0].unit.clone(),
        quantity: process.outputs[0].quantity,
    };
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("production:completion-acquisition:{suffix}"))
            .expect("acquisition");
    let production_grant =
        CompletionCapacityGrantId::new(format!("production:completion-grant:{suffix}"))
            .expect("grant");
    let resource_grant =
        CompletionCapacityGrantId::new(format!("resource:completion-grant:{suffix}"))
            .expect("grant");
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 1,
        holders: 1,
        bytes: 4_096,
    };
    let units = recipe.canonical_units().expect("recipe units");
    let eligibility = EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("eligibility");
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
        .request_completion_acquisition(RequestCompletionLeaseV1 {
            id: acquisition.clone(),
            operation_key: output_key.clone(),
            holder: holder(),
            operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
            eligibility_time: at,
            eligibility_envelope: eligibility.clone(),
            recipe: recipe.clone(),
            expected_participants: BTreeSet::from([
                canwu_production::PLUGIN_NAME.to_owned(),
                canwu_resource::PLUGIN_NAME.to_owned(),
            ]),
            policy_class: CompletionPolicyClassV1::Guaranteed,
        })
        .expect("completion acquisition");
    state
        .apply_completion_ingress(&ProductionCompletionIngressV1::GrantLocal(
            GrantCompletionCapacityV1 {
                grant_id: production_grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: revision(1),
                owner_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID),
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
        target_versions: vec![
            CompletionLockedTargetV1::Account {
                id: output.account.clone(),
                revision: output.expected_account_revision,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: input.allocation_leg.id.clone(),
                revision: input.allocation_leg.revision,
            },
        ],
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
            acquisition: acquisition.clone(),
            expected_acquisition_revision: revision(5),
            current_boundary: 1,
        })
        .expect("activation")
        .expect("activation certificate");
    let execution = ProductionExecution {
        id: execution_id.clone(),
        work_order: order_id.clone(),
        process: process.id.clone(),
        site: site_id(),
        facility: facility_id(),
        allocations: vec![allocation_id.clone()],
        lifecycle: WorkOrderLifecycle::Running,
        started_at: at,
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
                "technology:profile-technique:v1",
            ),
            capability_qualification: Some(version::<canwu_technology::CapabilityQualification>(
                "technology:profile-qualification:v1",
            )),
            implementation: Some(version::<canwu_technology::ImplementationRecord>(
                "technology:profile-implementation:v1",
            )),
            adoption: None,
            semantic_digest: "technology-binding-digest".to_owned(),
        },
        inputs: vec![input],
        output_requests: vec![output.clone()],
        output_outcomes: Vec::new(),
        output_source: None,
        output_ack_digest: None,
        completion_certificate: certificate.clone(),
        production_completion_grant: production_grant,
        resource_completion_grant: resource_grant,
        realized_output_per_mille: None,
        realization_evidence: None,
        output_settled_at: None,
    };
    let allocation = ProductionCapacityAllocation {
        id: allocation_id,
        facility: facility_id(),
        facility_generation: 1,
        capability: "bench".to_owned(),
        start: at,
        end: at
            .checked_add(canwu_api::SimDuration::minutes(60))
            .expect("allocation end"),
        quantity: 1,
        work_order: order_id,
        execution: execution_id.clone(),
        operation_key: format!("production:capacity:{suffix}"),
        state: CapacityAllocationState::Reserved,
    };
    state
        .apply_operation(
            &command(
                state,
                &format!("production:start:{suffix}"),
                ProductionOperation::StartExecution {
                    execution,
                    allocations: vec![allocation],
                },
            ),
            at,
        )
        .expect("start execution");
    let advance_at = at
        .checked_add(canwu_api::SimDuration::minutes(10))
        .expect("advance time");
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
            advance_at,
        )
        .expect("advance");
    let complete_at = at
        .checked_add(canwu_api::SimDuration::minutes(20))
        .expect("complete time");
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
            complete_at,
        )
        .expect("complete");
    let source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let request = ResourceOperationRequestV1::Credit(output.resource_credit_request(
        source.clone(),
        certificate.clone(),
        complete_at,
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
        sequence: 1,
    };
    outcome.semantic_digest =
        canonical_digest("canwu.resource.operation-outcome.v1", &outcome).expect("digest");
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
                outcomes: vec![outcome],
                settled_at: complete_at,
            },
            complete_at,
        )
        .expect("output settlement");
    let resource_source = DomainRecordVersionRef {
        record: canwu_resource::resource_runtime_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    participant_grant.state = CompletionGrantStateV1::Consumed;
    participant_grant.revision = revision(3);
    participant_grant.activation_deadline_boundary = Some(8);
    let consumed_revision = state.completion_acquisitions[&certificate.acquisition].revision;
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantConsumed {
                acquisition: certificate.acquisition.clone(),
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
        .finalize_execution_resource_completion(
            &certificate.acquisition,
            &resource_source,
            &participant_grant,
        )
        .expect("participant completion");
    execution_id
}

fn scenario(state: ProductionState) -> Scenario {
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

fn production_state(canwu: &Canwu) -> ProductionState {
    canwu
        .typed_domain_record(&production_runtime_reference())
        .expect("production runtime")
        .decode_payload::<ProductionRuntimeRecord>()
        .expect("production state")
}

// ---------------------------------------------------------------------------
// Restore validation of one archived production input.

fn filler_template() -> Result<ResourceOperationOutcome, Box<dyn Error>> {
    let mut state = ResourceState::empty(ResourceLimitsV1::canonical())?;
    Ok(
        state.apply_operation(&ResourceOperationRequestV1::CancelDemand(
            ResourceCancelDemandRequestV1 {
                operation_key: ResourceOperationKey::new("profile:filler:template")?,
                demand: ResourceDemandId::new("profile:missing:template")?,
                expected_demand_revision: ResourceRevision::INITIAL,
            },
        ))?,
    )
}

fn add_hot_filler(
    state: &mut ResourceState,
    template: &ResourceOperationOutcome,
    index: usize,
) -> Result<(), ResourceError> {
    let mut outcome = template.clone();
    outcome.operation_key = ResourceOperationKey::new(format!("profile:filler:{index}"))?;
    outcome.id = ResourceOperationOutcomeId::new(format!(
        "resource:outcome:{}:{}",
        outcome.operation_key.as_str().replace(':', "-"),
        &outcome.request_digest[..16]
    ))?;
    outcome.sequence = state.next_admitted_sequence;
    state.next_admitted_sequence += 1;
    outcome.semantic_digest.clear();
    outcome.semantic_digest = canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
    state.terminal_archive_candidates.insert(
        outcome.sequence,
        ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
    );
    state
        .outcomes
        .insert(outcome.operation_key.clone(), outcome);
    Ok(())
}

fn move_resource_head(
    state: &mut ResourceState,
    directory: &str,
    records: u64,
) -> Result<(), ResourceError> {
    state.outcomes.clear();
    state.consumptions.clear();
    state.terminal_archive_candidates.clear();
    state.refresh_hot_terminal_ledger()?;
    state.archive_head.revision += 1;
    state.archive_head.directory_root = Some(directory.to_owned());
    state.archive_head.archived_record_count += records;
    state.archive_head.semantic_digest.clear();
    state.archive_head.semantic_digest =
        canonical_digest("canwu.resource.archive-head.v1", &state.archive_head)?;
    Ok(())
}

/// A resource root whose archive holds the consumption and the consumption
/// outcome of one production input in its first commit, under `commits`
/// further commits of `per_commit` filler records each.
fn archived_input_case(
    template: &ResourceOperationOutcome,
    commits: usize,
    per_commit: usize,
) -> Result<Value, Box<dyn Error>> {
    let process = process();
    let input = resource_input(&process, "restored");
    let mut limits = ResourceLimitsV1::canonical();
    limits.max_archive_candidates = per_commit.max(limits.max_archive_candidates);
    limits.max_archive_maintenance_receipts = limits
        .max_archive_candidates
        .max(limits.max_archive_maintenance_receipts);
    let mut resource = ResourceState::empty(limits)?;
    let store = ProfileStore::default();
    let (consumption, outcome) = archived_input_payloads(&input);
    // The first commit: the input's consumption and outcome, as hot terminal
    // records under their own sequences.
    let mut consumption = consumption;
    consumption.terminal_sequence = resource.next_admitted_sequence;
    consumption.semantic_digest.clear();
    consumption.semantic_digest = canonical_digest("canwu.resource.consumption.v1", &consumption)?;
    resource.next_admitted_sequence += 1;
    resource.terminal_archive_candidates.insert(
        consumption.terminal_sequence,
        ResourceTerminalRecordKeyV1::Consumption(consumption.id.clone()),
    );
    let mut input = input;
    input.consumption.semantic_digest = consumption.semantic_digest.clone();
    resource
        .consumptions
        .insert(consumption.id.clone(), consumption);
    let mut outcome = outcome;
    outcome.sequence = resource.next_admitted_sequence;
    outcome.semantic_digest.clear();
    outcome.semantic_digest = canonical_digest("canwu.resource.operation-outcome.v1", &outcome)?;
    resource.next_admitted_sequence += 1;
    resource.terminal_archive_candidates.insert(
        outcome.sequence,
        ResourceTerminalRecordKeyV1::Outcome(outcome.operation_key.clone()),
    );
    input.consumption_outcome = ResourceOperationOutcomeVersionV1::from(&outcome);
    resource
        .outcomes
        .insert(outcome.operation_key.clone(), outcome);
    resource.refresh_hot_terminal_ledger()?;
    let prepared = resource.prepare_resource_archive(2, &store)?;
    prepared.store_and_verify(&store)?;
    move_resource_head(&mut resource, &prepared.directory.id, 2)?;
    for commit in 0..commits {
        for record in 0..per_commit {
            add_hot_filler(&mut resource, template, commit * per_commit + record)?;
        }
        resource.refresh_hot_terminal_ledger()?;
        let prepared = resource.prepare_resource_archive(per_commit, &store)?;
        prepared.store_and_verify(&store)?;
        move_resource_head(&mut resource, &prepared.directory.id, per_commit as u64)?;
    }
    resource.allocation_legs.insert(
        input.allocation_leg.id.clone(),
        ResourceAllocationLeg {
            id: input.allocation_leg.id.clone(),
            revision: input.allocation_leg.revision,
            demand: ResourceDemandId::new("resource:restored-demand")?,
            demand_revision: revision(1),
            reservation: ResourceReservationId::new("resource:restored-reservation")?,
            account: input.allocation_leg.account.clone(),
            account_revision: input.allocation_leg.account_revision,
            resource_revision: input.allocation_leg.resource_revision.clone(),
            unit_revision: input.allocation_leg.unit_revision.clone(),
            quantity: input.allocation_leg.quantity,
            status: AllocationLegStatus::Consumed,
            priority: 0,
            due_at: SimTime::EPOCH,
            tie_break: ResourceTieBreakKey::new("resource:restored-tie")?,
            admitted_sequence: 1,
            operation_key: ResourceOperationKey::new("resource:restored-allocation")?,
            semantic_digest: input.allocation_leg.semantic_digest.clone(),
        },
    );
    let mut samples = Vec::new();
    let mut loads = 0;
    let mut bytes = 0;
    let started = Instant::now();
    while samples.len() < 3 || (samples.len() < 2_000 && started.elapsed().as_millis() < 300) {
        store.reset_counts();
        let sample = Instant::now();
        validate_production_resource_continuation(&resource, &store, &input)?;
        samples.push(sample.elapsed().as_secs_f64() * 1_000_000.0);
        loads = store.loads.load(Ordering::Relaxed);
        bytes = store.loaded_bytes.load(Ordering::Relaxed);
    }
    samples.sort_by(f64::total_cmp);
    Ok(json!({
        "resource_commits": commits + 1,
        "archived_resource_records": commits * per_commit + 2,
        "records_per_commit": per_commit,
        "validation_microseconds": samples[samples.len() / 2],
        "objects_read": loads,
        "bytes_read": bytes,
        "samples": samples.len(),
    }))
}

// ---------------------------------------------------------------------------
// Archive lifetime: a chain of commits, each archiving the receipt before it.

/// Pages and bytes of the lookup index the archive head reaches.
fn live_lookup_index(
    state: &ProductionState,
    store: &ProfileStore,
) -> Result<(u64, u64), Box<dyn Error>> {
    let mut pages = 0;
    let mut bytes = 0;
    for page in live_production_archive_lookup_pages(state, store, [])? {
        pages += 1;
        bytes += store
            .objects
            .read()
            .unwrap()
            .get(&(PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE.to_owned(), page))
            .map_or(0, Vec::len) as u64;
    }
    Ok((pages, bytes))
}

fn stored_bytes(store: &ProfileStore, lookup: bool) -> u64 {
    store
        .objects
        .read()
        .unwrap()
        .iter()
        .filter(|((namespace, _), _)| {
            namespace.starts_with("canwu.production.")
                && (namespace == PRODUCTION_ARCHIVE_LOOKUP_PAGE_NAMESPACE) == lookup
        })
        .map(|(_, bytes)| bytes.len() as u64)
        .sum()
}

/// Times a lookup of each key in turn, at least 300 ms in all, and returns
/// microseconds, objects read, and bytes read per lookup.
fn time_lookups(
    state: &ProductionState,
    store: &ProfileStore,
    keys: &[ProductionTerminalArchiveKeyV1],
    expect_hit: bool,
) -> Result<(f64, f64, f64), Box<dyn Error>> {
    store.reset_counts();
    let started = Instant::now();
    let mut lookups = 0_usize;
    while lookups < keys.len() || started.elapsed().as_millis() < 300 {
        let found = archived_production_terminal_record(state, store, &keys[lookups % keys.len()])?;
        if found.is_some() != expect_hit {
            return Err("a lookup answered the wrong way".into());
        }
        lookups += 1;
    }
    let count = lookups as f64;
    Ok((
        started.elapsed().as_secs_f64() * 1_000_000.0 / count,
        store.loads.load(Ordering::Relaxed) as f64 / count,
        store.loaded_bytes.load(Ordering::Relaxed) as f64 / count,
    ))
}

/// Runs `commits` archive commits through the production plugin: the first
/// archives `executions` settled executions, and every commit archives the
/// receipt of the commit before it. Each commit is acknowledged in the
/// boundary of the next, as a host application would.
fn lifetime_case(commits: usize, executions: usize) -> Result<Value, Box<dyn Error>> {
    let mut state = base_state();
    for index in 0..executions {
        settle_execution(&mut state, &format!("lifetime-{index:04}"));
    }
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    state.rebuild_runtime_indexes()?;
    state.validate()?;
    let first = state
        .archive_due_index
        .first()
        .cloned()
        .ok_or("the fixture has no execution")?;
    let plugin = ProductionPlugin;
    let store = ProfileStore::default();
    let mut canwu = Canwu::new_with_plugins(0x00CA_4E55, scenario(state), &[&plugin])?;
    canwu.set_plugin_archive_object_provider(Arc::new(store.clone()));
    let mut samples = Vec::new();
    let mut unacknowledged = None;
    let (mut lookup_pages_written, mut lookup_pages_superseded) = (0_usize, 0_usize);
    for commit in 1..=commits {
        if let Some(receipt) = unacknowledged.take() {
            finalize_production_archive_retention(&mut canwu, &store, &receipt)?;
        }
        let hot = production_state(&canwu);
        let started = Instant::now();
        let prepared = hot.prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &store)?;
        let prepare_ms = milliseconds(started);
        lookup_pages_written += prepared.lookup_pages.len();
        lookup_pages_superseded += prepared.superseded_lookup_pages.len();
        let started = Instant::now();
        unacknowledged = Some(enqueue_production_archive(&mut canwu, &prepared, &store)?);
        let enqueue_ms = milliseconds(started);
        let cursor = canwu.evidence_cursor()?;
        let started = Instant::now();
        canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
        let boundary_ms = milliseconds(started);
        if [1, 1_000, 2_000, 4_000, 8_192, 8_193, 30_000].contains(&commit) || commit == commits {
            let state = production_state(&canwu);
            let journal = serde_json::to_vec(&canwu.journal_segment_since(cursor)?)?.len();
            samples.push(json!({
                "commits": commit,
                "prepare_ms": prepare_ms,
                "enqueue_ms": enqueue_ms,
                "boundary_ms": boundary_ms,
                "journal_bytes_per_boundary": journal,
                "hot_receipts": state.archive.maintenance_receipts.len(),
                "pending_handles": state.archive.pending_handles.len(),
                "production_root_bytes": serde_json::to_vec(&state)?.len(),
            }));
        }
    }
    let state = production_state(&canwu);
    let archived = state.archive.archived_execution_count + state.archive.archived_receipt_count;
    let absent = (0..2_000)
        .map(|index| {
            ProductionExecutionId::new(format!("production:execution:absent-{index}"))
                .map(ProductionTerminalArchiveKeyV1::Execution)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (miss_microseconds, miss_objects, miss_bytes) =
        time_lookups(&state, &store, &absent, false)?;
    let (first_hit_microseconds, first_hit_objects, first_hit_bytes) = time_lookups(
        &state,
        &store,
        &[ProductionTerminalArchiveKeyV1::Execution(first)],
        true,
    )?;
    let (live_pages, live_bytes) = live_lookup_index(&state, &store)?;
    let started = Instant::now();
    validate_production_archive(&store, &state)?;
    let restore_audit_ms = milliseconds(started);
    Ok(json!({
        "commits": commits,
        "first_commit_executions": executions,
        "archived_records": archived,
        "samples": samples,
        "miss_microseconds": miss_microseconds,
        "miss_objects_read": miss_objects,
        "miss_bytes_read": miss_bytes,
        "first_execution_hit_microseconds": first_hit_microseconds,
        "first_execution_hit_objects_read": first_hit_objects,
        "first_execution_hit_bytes_read": first_hit_bytes,
        "lookup_pages_written": lookup_pages_written,
        "lookup_pages_superseded": lookup_pages_superseded,
        "lookup_bytes_written_per_commit": stored_bytes(&store, true) as f64 / commits as f64,
        "other_bytes_written_per_commit": stored_bytes(&store, false) as f64 / commits as f64,
        "live_lookup_pages": live_pages,
        "live_lookup_bytes": live_bytes,
        "restore_audit_ms": restore_audit_ms,
    }))
}

/// One commit of `executions` settled executions on top of an archive of
/// `prior_commits` receipt commits: what preparing, storing, and settling a
/// multi-page commit costs and writes.
fn multi_page_case(executions: usize, prior_commits: usize) -> Result<Value, Box<dyn Error>> {
    let mut state = base_state();
    for index in 0..executions {
        settle_execution(&mut state, &format!("page-{index:04}"));
    }
    state.observation_dirty_index.clear();
    state.observation_due_index.clear();
    state.rebuild_runtime_indexes()?;
    state.validate()?;
    let keys = state
        .archive_due_index
        .iter()
        .cloned()
        .map(ProductionTerminalArchiveKeyV1::Execution)
        .collect::<Vec<_>>();
    let hot_root_bytes = serde_json::to_vec(&state)?.len();
    let plugin = ProductionPlugin;
    let store = ProfileStore::default();
    let mut canwu = Canwu::new_with_plugins(0x00CA_4E55, scenario(state), &[&plugin])?;
    canwu.set_plugin_archive_object_provider(Arc::new(store.clone()));
    // Prior commits archive one execution, then one receipt each.
    let mut unacknowledged = None;
    for _ in 0..prior_commits {
        if let Some(receipt) = unacknowledged.take() {
            finalize_production_archive_retention(&mut canwu, &store, &receipt)?;
        }
        let prepared = production_state(&canwu).prepare_production_archive(1, &store)?;
        unacknowledged = Some(enqueue_production_archive(&mut canwu, &prepared, &store)?);
        canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    }
    if let Some(receipt) = unacknowledged.take() {
        finalize_production_archive_retention(&mut canwu, &store, &receipt)?;
        canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    }
    let before_lookup = stored_bytes(&store, true);
    let before_other = stored_bytes(&store, false);
    let hot = production_state(&canwu);
    let started = Instant::now();
    let prepared = hot.prepare_production_archive(MAX_ARCHIVE_PREPARE_CANDIDATES, &store)?;
    let prepare_ms = milliseconds(started);
    let records = prepared.selected_count();
    let pages = 1 + prepared.additional_pages.len();
    let started = Instant::now();
    prepared.store_and_verify(&store)?;
    let store_and_verify_ms = milliseconds(started);
    let started = Instant::now();
    enqueue_production_archive(&mut canwu, &prepared, &store)?;
    let enqueue_ms = milliseconds(started);
    let started = Instant::now();
    canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    let boundary_ms = milliseconds(started);
    let state = production_state(&canwu);
    // The first execution in identity order went with the first prior
    // commit, into a blob of its own.
    let (hit_microseconds, hit_objects, hit_bytes) =
        time_lookups(&state, &store, &keys[..1], true)?;
    let (all_hit_microseconds, all_hit_objects, all_hit_bytes) =
        time_lookups(&state, &store, &keys, true)?;
    Ok(json!({
        "executions": executions,
        "prior_commits": prior_commits,
        "hot_root_bytes": hot_root_bytes,
        "commit_records": records,
        "commit_pages": pages,
        "prepare_ms": prepare_ms,
        "store_and_verify_ms": store_and_verify_ms,
        "enqueue_ms": enqueue_ms,
        "boundary_ms": boundary_ms,
        "lookup_bytes_written": stored_bytes(&store, true) - before_lookup,
        "other_bytes_written": stored_bytes(&store, false) - before_other,
        "lookup_pages_written": prepared.lookup_pages.len(),
        "hit_microseconds": all_hit_microseconds,
        "hit_objects_read": all_hit_objects,
        "hit_bytes_read": all_hit_bytes,
        "early_hit_microseconds": hit_microseconds,
        "early_hit_objects_read": hit_objects,
        "early_hit_bytes_read": hit_bytes,
    }))
}

struct Options {
    preset: String,
    machine: String,
    recorded_on: String,
    output: Option<PathBuf>,
}

fn options() -> Result<Options, Box<dyn Error>> {
    let mut options = Options {
        preset: "smoke".to_owned(),
        machine: "local-machine".to_owned(),
        recorded_on: "unspecified".to_owned(),
        output: None,
    };
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| arguments.next().ok_or(format!("{name} requires a value"));
        match argument.as_str() {
            "--preset" => options.preset = value("--preset")?,
            "--machine" => options.machine = value("--machine")?,
            "--recorded-on" => options.recorded_on = value("--recorded-on")?,
            "--output" => options.output = Some(PathBuf::from(value("--output")?)),
            "--help" | "-h" => {
                println!(
                    "Canwu production archive profile\n\n\
                     --preset smoke|full\n\
                     --machine LABEL --recorded-on DATE --output PATH"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unsupported argument: {other}").into()),
        }
    }
    Ok(options)
}

fn command_output(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// BLAKE3 over the measured sources: the production and resource extensions
/// and this file.
fn source_content_hash() -> String {
    let root = PathBuf::from(command_output("git", &["rev-parse", "--show-toplevel"]));
    let mut files =
        vec![root.join("benchmarks/performance-harness/src/bin/production-archive-profile.rs")];
    for crate_dir in [
        "crates/extensions/canwu-production/src",
        "crates/extensions/canwu-resource/src",
    ] {
        if let Ok(entries) = fs::read_dir(root.join(crate_dir)) {
            files.extend(entries.flatten().map(|entry| entry.path()));
        }
    }
    files.sort();
    let mut hasher = blake3::Hasher::new();
    for path in files {
        if let Ok(relative) = path.strip_prefix(&root)
            && let Ok(bytes) = fs::read(&path)
        {
            hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
            hasher.update(&[0]);
            hasher.update(&bytes);
            hasher.update(&[0]);
        }
    }
    hasher.finalize().to_hex().to_string()
}

/// Pairs of case sizes; see `main`.
type Cases = &'static [(usize, usize)];

fn main() -> Result<(), Box<dyn Error>> {
    let options = options()?;
    // (resource commits, records per commit), (commits, first-commit
    // executions), (executions, prior commits)
    let (restores, lifetimes, pages): (Cases, Cases, Cases) = match options.preset.as_str() {
        "smoke" => (&[(37, 27), (100, 1)], &[(100, 1)], &[(70, 10)]),
        "full" => (
            &[
                (37, 27),
                (370, 27),
                (3_704, 27),
                (39, 256),
                (391, 256),
                (100, 1),
                (1_000, 1),
                (10_000, 1),
            ],
            &[(1_000, 1), (8_193, 1), (30_000, 1)],
            &[(70, 10), (1_023, 1)],
        ),
        other => return Err(format!("unsupported preset: {other}").into()),
    };
    let template = filler_template()?;
    let mut restore_cases = Vec::new();
    for (commits, per_commit) in restores {
        eprintln!("restore case: {commits} resource commits of {per_commit}");
        restore_cases.push(archived_input_case(&template, *commits, *per_commit)?);
    }
    let mut lifetime_cases = Vec::new();
    for (commits, executions) in lifetimes {
        eprintln!("lifetime case: {commits} commits");
        lifetime_cases.push(lifetime_case(*commits, *executions)?);
    }
    let mut multi_page_cases = Vec::new();
    for (executions, prior) in pages {
        eprintln!("multi-page case: {executions} executions after {prior} commits");
        multi_page_cases.push(multi_page_case(*executions, *prior)?);
    }
    let report = json!({
        "benchmark": "production-archive-profile",
        "preset": options.preset,
        "machine": options.machine,
        "recorded_on": options.recorded_on,
        "source": {
            "git_commit": command_output("git", &["rev-parse", "HEAD"]),
            "git_dirty": !command_output("git", &["status", "--porcelain"]).is_empty(),
            "content_hash": source_content_hash(),
            "rustc": command_output("rustc", &["-vV"]),
        },
        "restore_cases": restore_cases,
        "lifetime_cases": lifetime_cases,
        "multi_page_cases": multi_page_cases,
    });
    let text = serde_json::to_string_pretty(&report)?;
    match options.output {
        Some(path) => fs::write(path, text)?,
        None => println!("{text}"),
    }
    Ok(())
}
