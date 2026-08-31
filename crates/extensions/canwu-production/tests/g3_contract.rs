#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryPhase, BoundaryRequest, Canwu, Command, CommandEnvelope, CommandRequest,
    CommandRequestId, DecisionAuthority, DecisionControllerBinding, DecisionEvaluation,
    DecisionIngressRequest, DecisionMutation, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRequestId, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    EntityRef, EvidenceRef, Government, GovernmentId, Issuer, KnowledgeHolderRef,
    KnowledgeSnapshot, MapPoint, Person, PersonId, PluginIngressRequest, RandomDrawAddress,
    Scenario, SimTime, Territory, TerritoryId, TypedDomainRecordRef, UtilityProfile,
    WeightedUtilityPolicy, WorldSnapshot,
};
use canwu_production::*;
use canwu_resource::{
    AllocationLegStatus, CompletionCapacityGrantId, CompletionCapacityGrantV1,
    CompletionCapacityPartitionV1, CompletionCapacityRecipeV1, CompletionGrantStateV1,
    CompletionLeaseAcquisitionId, CompletionLockedTargetV1, CompletionPolicyClassV1,
    ConsumeExternalCompletionParticipantGrantV1, ConsumptionStatus, EligibilityEnvelopeV1,
    GrantCompletionCapacityV1, MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE, PrepareCompletionCapacityV1,
    PrepareExternalCompletionParticipantGrantV1, RequestCompletionLeaseV1,
    RequestExternalCompletionParticipantGrantV1, ResourceAccount, ResourceAccountId,
    ResourceAllocationLeg, ResourceAllocationLegId, ResourceAllocationLegVersionV1,
    ResourceArchiveRetentionHandleV1, ResourceArchiveRetentionPhaseV1, ResourceArchiveStore,
    ResourceCompletionOperationV1, ResourceConsumption, ResourceConsumptionId,
    ResourceConsumptionVersionV1, ResourceDefinitionId, ResourceDefinitionRevision,
    ResourceDefinitionRevisionId, ResourceDemandId, ResourceLimitsV1, ResourceOperationKey,
    ResourceOperationKind, ResourceOperationOutcome, ResourceOperationOutcomeId,
    ResourceOperationOutcomeVersionV1, ResourceOperationRequestV1, ResourceOperationStatus,
    ResourcePlugin, ResourceQualityId, ResourceReportGrantId, ResourceReportGrantV1,
    ResourceReservationId, ResourceRevision, ResourceRuntimeRecord, ResourceScopeId, ResourceState,
    ResourceTieBreakKey, ResourceUnitRevision, ResourceUnitRevisionId, RunBudgetRevisionV1,
    enqueue_resource_completion_operation,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

type ArchiveObjectMap = BTreeMap<(String, String), Vec<u8>>;

#[derive(Clone, Default)]
struct TestProductionArchiveStore {
    objects: Rc<RefCell<ArchiveObjectMap>>,
    retentions: Rc<RefCell<BTreeMap<String, ProductionArchiveRetentionHandleV1>>>,
    resource_retentions: Rc<RefCell<BTreeMap<String, ResourceArchiveRetentionHandleV1>>>,
}

impl TestProductionArchiveStore {
    fn tamper_production_blob_for_directory(&self, directory_root: &str) {
        let directory_key = (
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            directory_root.to_owned(),
        );
        let blob_id = {
            let objects = self.objects.borrow();
            let directory: ProductionArchiveIndexDirectoryV1 = serde_json::from_slice(
                objects
                    .get(&directory_key)
                    .expect("production archive directory bytes"),
            )
            .expect("production archive directory JSON");
            directory
                .blob_ids
                .first()
                .cloned()
                .expect("production blob")
        };
        let blob_key = (PRODUCTION_ARCHIVE_BLOB_NAMESPACE.to_owned(), blob_id);
        let mut objects = self.objects.borrow_mut();
        let bytes = objects
            .get_mut(&blob_key)
            .expect("production archive blob bytes");
        let mut blob: serde_json::Value =
            serde_json::from_slice(bytes).expect("production archive blob JSON");
        blob["records"][0]["non_recoverable_waste_quantity"] = serde_json::json!(999);
        *bytes = serde_json::to_vec(&blob).expect("tampered production archive blob");
    }

    fn tamper_first_resource_blob(&self) {
        let key = self
            .objects
            .borrow()
            .keys()
            .find(|(namespace, _)| namespace == canwu_resource::RESOURCE_ARCHIVE_BLOB_NAMESPACE)
            .cloned()
            .expect("resource archive blob");
        let mut objects = self.objects.borrow_mut();
        let bytes = objects.get_mut(&key).expect("resource archive blob bytes");
        let mut blob: serde_json::Value =
            serde_json::from_slice(bytes).expect("resource blob JSON");
        blob["records"][0]["quantity"] = serde_json::json!(999);
        *bytes = serde_json::to_vec(&blob).expect("tampered resource archive blob");
    }

    fn tamper_resource_blob_for_directory(&self, directory_root: &str) {
        let directory_key = (
            canwu_resource::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            directory_root.to_owned(),
        );
        let blob_id = {
            let objects = self.objects.borrow();
            let directory: canwu_resource::ResourceArchiveIndexDirectoryV1 =
                serde_json::from_slice(
                    objects
                        .get(&directory_key)
                        .expect("resource archive directory bytes"),
                )
                .expect("resource archive directory JSON");
            directory
                .blob_ids
                .first()
                .cloned()
                .expect("resource archive directory blob")
        };
        let blob_key = (
            canwu_resource::RESOURCE_ARCHIVE_BLOB_NAMESPACE.to_owned(),
            blob_id,
        );
        let mut objects = self.objects.borrow_mut();
        let bytes = objects
            .get_mut(&blob_key)
            .expect("resource archive blob bytes");
        let mut blob: serde_json::Value =
            serde_json::from_slice(bytes).expect("resource blob JSON");
        blob["records"][0]["quantity"] = serde_json::json!(999);
        *bytes = serde_json::to_vec(&blob).expect("tampered resource archive blob");
    }

    fn sever_production_archive_prior_chain(&self, directory_root: &str) {
        let key = (
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE.to_owned(),
            directory_root.to_owned(),
        );
        let mut objects = self.objects.borrow_mut();
        let bytes = objects.get_mut(&key).expect("production directory bytes");
        let mut directory: serde_json::Value =
            serde_json::from_slice(bytes).expect("production directory JSON");
        directory["previous_root"] = serde_json::Value::Null;
        *bytes = serde_json::to_vec(&directory).expect("tampered production directory");
    }
}

impl ProductionArchiveStore for TestProductionArchiveStore {
    fn store_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), canwu_api::CanwuError> {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_production_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_production_archive_retention(
        &self,
        handle: &ProductionArchiveRetentionHandleV1,
    ) -> Result<(), canwu_api::CanwuError> {
        self.retentions
            .borrow_mut()
            .insert(handle.handle_id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_production_archive_retention(
        &self,
        handle_id: &str,
        phase: ProductionArchiveRetentionPhaseV1,
    ) -> Result<(), canwu_api::CanwuError> {
        let mut retentions = self.retentions.borrow_mut();
        let handle = retentions
            .get_mut(handle_id)
            .expect("retention handle should exist");
        handle.phase = phase;
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for TestProductionArchiveStore {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_api::CanwuError> {
        self.load_production_archive_object(namespace, object_id)
    }
}

impl ResourceArchiveStore for TestProductionArchiveStore {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), canwu_resource::ResourceError> {
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, canwu_resource::ResourceError> {
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), canwu_resource::ResourceError> {
        self.resource_retentions
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), canwu_resource::ResourceError> {
        let mut retentions = self.resource_retentions.borrow_mut();
        let handle = retentions.get_mut(handle_id).ok_or_else(|| {
            canwu_resource::ResourceError::NotFound(
                "resource retention handle is unavailable".to_owned(),
            )
        })?;
        handle.phase = phase;
        Ok(())
    }
}

fn version<T: DomainRecordType>(id: &str) -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: TypedDomainRecordRef::<T>::new(id).into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn resource_revision() -> ResourceRevision {
    ResourceRevision::new(1).expect("revision")
}

fn evidence(
    kind: ProductionRequirementKind,
    capability: &str,
    available_quantity: u64,
) -> ProductionEvidenceBinding {
    ProductionEvidenceBinding {
        kind,
        capability: capability.to_owned(),
        version: version::<canwu_technology::AssetBinding>(&format!(
            "technology:test-evidence:{capability}"
        )),
        semantic_digest: format!("digest-{capability}"),
        available_quantity,
    }
}

fn requirement(
    id: &str,
    kind: ProductionRequirementKind,
    capability: &str,
    quantity: u64,
) -> ProductionRequirementGroup {
    ProductionRequirementGroup {
        id: format!("production:requirement:{id}"),
        kind,
        any_of: vec![ProductionRequirementAlternative {
            id: format!("production:requirement-alternative:{id}:primary"),
            capability: capability.to_owned(),
            minimum_quantity: quantity,
        }],
    }
}

fn process(id: &str, industrial: bool) -> ProcessRevision {
    let grain = ResourceDefinitionRevisionId::new("resource:grain:v1").expect("resource ID");
    let flour = ResourceDefinitionRevisionId::new("resource:flour:v1").expect("resource ID");
    let unit = ResourceUnitRevisionId::new("resource:mass-unit:v1").expect("resource ID");
    let requirements = if industrial {
        vec![
            requirement(
                "machine",
                ProductionRequirementKind::ToolsMachines,
                "steam-mill",
                1,
            ),
            requirement(
                "fuel",
                ProductionRequirementKind::Energy,
                "coal-grade-a",
                10,
            ),
            requirement(
                "implementation",
                ProductionRequirementKind::TechnologyImplementation,
                "steam-mill-installed",
                1,
            ),
            requirement(
                "maintenance",
                ProductionRequirementKind::Maintenance,
                "mill-maintained",
                1,
            ),
            requirement(
                "access",
                ProductionRequirementKind::Access,
                "coal-route-open",
                1,
            ),
            requirement(
                "organization",
                ProductionRequirementKind::FinanceOrganization,
                "organized-shift",
                1,
            ),
        ]
    } else {
        vec![
            requirement(
                "household-skill",
                ProductionRequirementKind::LaborCapability,
                "customary-hand-milling",
                1,
            ),
            requirement(
                "household-access",
                ProductionRequirementKind::Authorization,
                "household-authority",
                1,
            ),
        ]
    };
    ProcessRevision {
        id: ProcessRevisionId::new(id).expect("process ID"),
        label: if industrial {
            "machine and fuel mill"
        } else {
            "household hand mill"
        }
        .to_owned(),
        semantic_digest: format!("digest-{id}"),
        effective_from: SimTime::EPOCH,
        effective_until: None,
        work_units: if industrial { 20 } else { 10 },
        requirements,
        inputs: vec![ResourceRequirement {
            resource: grain,
            unit: unit.clone(),
            quantity: 10,
        }],
        outputs: vec![ProductionOutputSpec {
            resource: flour,
            unit,
            quantity: 8,
            quality_class: "ordinary".to_owned(),
        }],
        capacity: vec![CapacityRequirement {
            capability: if industrial { "machine" } else { "bench" }.to_owned(),
            quantity: 1,
        }],
        adoption_required: industrial,
    }
}

fn base_state() -> (
    ProductionState,
    KnowledgeHolderRef,
    ProductionSiteId,
    FacilityAssetId,
) {
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let site_id = ProductionSiteId::new("production:test-site").expect("site ID");
    let facility_id = FacilityAssetId::new("production:test-facility").expect("facility ID");
    let mut state = ProductionState {
        revision: 1,
        ..ProductionState::default()
    };
    for process in [
        process("production:household-process:v1", false),
        process("production:industrial-process:v1", true),
    ] {
        state.processes.insert(process.id.clone(), process);
    }
    state.sites.insert(
        site_id.clone(),
        ProductionSite {
            id: site_id.clone(),
            holder: holder.clone(),
            place: EntityRef::Territory(TerritoryId::new(1)),
            form: ProductionSiteForm::DistributedWorkshop,
            active: true,
        },
    );
    state.facilities.insert(
        facility_id.clone(),
        FacilityAsset {
            id: facility_id.clone(),
            site: site_id.clone(),
            generation: 1,
            lifecycle: FacilityLifecycle::Operational,
            condition_per_mille: 1_000,
            capacity: BTreeMap::from([("bench".to_owned(), 1), ("machine".to_owned(), 1)]),
            maintenance_evidence: Vec::new(),
            operational_stage_capacity_per_mille: 0,
            incident_risk_per_mille: 0,
            incident_max_severity_per_mille: 0,
        },
    );
    let operator_grant_id =
        ProductionObserverGrantId::new("production:observer-grant:operator").expect("grant ID");
    state.observer_grants.insert(
        operator_grant_id.clone(),
        ProductionObserverGrant {
            id: operator_grant_id,
            holder: holder.clone(),
            sites: BTreeSet::from([site_id.clone()]),
            role: ProductionObservationRole::Operator,
            delay_minutes: 0,
        },
    );
    (state, holder, site_id, facility_id)
}

fn scenario_with_production(state: ProductionState) -> Scenario {
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

fn current_record_version<T: DomainRecordType>(
    canwu: &Canwu,
    reference: TypedDomainRecordRef<T>,
) -> DomainRecordVersionRef {
    let current = canwu
        .typed_domain_record(&reference)
        .expect("current typed record");
    if current.version == 1 {
        return DomainRecordVersionRef {
            record: reference.into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
    }
    for boundary in canwu.boundaries().iter().rev() {
        for (change_index, change) in boundary.record_changes.iter().enumerate().rev() {
            if change.current.reference == current.reference
                && change.current.version == current.version
            {
                return DomainRecordVersionRef {
                    record: current.reference.clone(),
                    version: current.version,
                    established_by: DomainRecordVersionSource::BoundaryChange {
                        boundary: boundary.id,
                        change_index: u64::try_from(change_index).expect("change index"),
                    },
                };
            }
        }
    }
    panic!("current record version provenance is unavailable")
}

fn next_boundary(canwu: &Canwu) -> u64 {
    canwu
        .boundaries()
        .last()
        .map_or(1, |boundary| boundary.id.get().saturating_add(1))
}

fn settle_at_epoch(canwu: &mut Canwu, label: &str) {
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .unwrap_or_else(|error| panic!("{label}: {error}"));
}

#[test]
fn canonical_cross_plugin_completion_coordinator_uses_real_ingress_and_exact_grants() {
    let (mut production, holder, _, _) = base_state();
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 1,
        holders: 1,
        bytes: 4_096,
    };
    let units = recipe.canonical_units().expect("recipe units");
    let budget = RunBudgetRevisionV1 {
        revision: ResourceRevision::INITIAL,
        total_completion_units: units.saturating_mul(4),
        shared_pending_slots: 0,
        partitions: vec![CompletionCapacityPartitionV1 {
            authority: holder.clone(),
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
    .expect("run budget");
    production.production_run_budget = Some(budget.clone());
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource");
    resource
        .install_run_budget(budget)
        .expect("resource run budget");
    resource
        .install_report_grant(ResourceReportGrantV1 {
            id: ResourceReportGrantId::new("resource:canonical-completion-report")
                .expect("report grant ID"),
            holder: holder.clone(),
            scope: ResourceScopeId::new("resource:canonical-completion-scope")
                .expect("report scope ID"),
            accounts: BTreeSet::new(),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 60,
            delay_minutes: 0,
        })
        .expect("completion report grant");
    resource.report_dirty_grants.clear();
    let mut scenario = scenario_with_production(production);
    scenario
        .domain_records
        .push(resource.into_record().expect("resource root"));
    let production_plugin = ProductionPlugin;
    let resource_plugin = ResourcePlugin::default();
    let mut canwu = Canwu::new_with_plugins(211, scenario, &[&production_plugin, &resource_plugin])
        .expect("cross-plugin runtime");
    let acquisition =
        CompletionLeaseAcquisitionId::new("production:completion-acquisition:canonical-ingress")
            .expect("acquisition");
    let operation_key = ResourceOperationKey::new("resource:production-output:canonical-ingress")
        .expect("operation key");
    let eligibility = EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("eligibility");
    let request = RequestCompletionLeaseV1 {
        id: acquisition.clone(),
        operation_key: operation_key.clone(),
        holder: holder.clone(),
        operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
        eligibility_time: SimTime::EPOCH,
        eligibility_envelope: eligibility.clone(),
        recipe: recipe.clone(),
        expected_participants: BTreeSet::from([
            canwu_production::PLUGIN_NAME.to_owned(),
            canwu_resource::PLUGIN_NAME.to_owned(),
        ]),
        policy_class: CompletionPolicyClassV1::Guaranteed,
    };
    let expected_runtime_revision = production_state(&canwu).revision;
    canwu
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    Command::Plugin {
                        plugin: canwu_production::PLUGIN_NAME.to_owned(),
                        command: PRODUCTION_COMMAND.to_owned(),
                        payload: serde_json::to_value(ProductionCommandEnvelope {
                            operation_id: ProductionOperationOutcomeId::new(
                                "production:operation:request-canonical-completion",
                            )
                            .expect("outcome ID"),
                            holder: holder.clone(),
                            expected_runtime_revision,
                            operation: ProductionOperation::RequestCompletionLease { request },
                        })
                        .expect("production command payload"),
                    },
                )
                .at_time(SimTime::EPOCH),
            ),
        )
        .expect("tracked completion request ingress");
    settle_at_epoch(&mut canwu, "completion command boundary");
    settle_at_epoch(&mut canwu, "completion request boundary");
    let requested_state = production_state(&canwu);
    let requested = requested_state
        .completion_acquisitions
        .get(&acquisition)
        .unwrap_or_else(|| {
            panic!(
                "completion acquisition was rejected: {:#?}",
                requested_state.operation_outcomes
            )
        });
    assert_eq!(
        requested.state,
        canwu_resource::CompletionLeaseAcquisitionStateV1::Requested
    );

    let production_grant =
        CompletionCapacityGrantId::new("production:completion-grant:canonical-ingress")
            .expect("production grant");
    let production_source = current_record_version(&canwu, production_runtime_reference());
    let boundary = next_boundary(&canwu);
    enqueue_production_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ProductionCompletionIngressV1::GrantLocal(GrantCompletionCapacityV1 {
            grant_id: production_grant.clone(),
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::INITIAL,
            owner_plugin: canwu_production::PLUGIN_NAME.to_owned(),
            target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                version: production_source,
            }],
            current_boundary: boundary,
        }),
    )
    .expect("production grant ingress");
    settle_at_epoch(&mut canwu, "production grant boundary");

    let resource_grant =
        CompletionCapacityGrantId::new("resource:completion-grant:canonical-ingress")
            .expect("resource grant");
    let coordinator_source = current_record_version(&canwu, production_runtime_reference());
    let boundary = next_boundary(&canwu);
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::GrantExternalParticipant(
            RequestExternalCompletionParticipantGrantV1 {
                coordinator_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                coordinator_source: coordinator_source.clone(),
                coordinator_acquisition_revision: ResourceRevision::new(2).expect("revision"),
                acquisition: acquisition.clone(),
                operation_key: operation_key.clone(),
                holder: holder.clone(),
                operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
                eligibility_time: SimTime::EPOCH,
                eligibility_envelope_digest: eligibility.digest.clone(),
                recipe: recipe.clone(),
                policy_class: CompletionPolicyClassV1::Guaranteed,
                grant_id: resource_grant.clone(),
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: coordinator_source,
                }],
                current_boundary: boundary,
            },
        ),
    )
    .expect("resource participant grant ingress");
    settle_at_epoch(&mut canwu, "resource participant grant boundary");
    let (_, resource_state) = canwu_resource::resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    let held = resource_state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let resource_source =
        current_record_version(&canwu, canwu_resource::resource_runtime_reference());
    enqueue_production_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ProductionCompletionIngressV1::AcknowledgeParticipantGrant {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::new(2).expect("revision"),
            participant: canwu_resource::PLUGIN_NAME.to_owned(),
            provider_source: resource_source,
            grant: held,
        },
    )
    .expect("resource grant acknowledgement ingress");
    settle_at_epoch(&mut canwu, "resource grant acknowledgement boundary");

    let boundary = next_boundary(&canwu);
    enqueue_production_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ProductionCompletionIngressV1::PrepareLocal(PrepareCompletionCapacityV1 {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::new(3).expect("revision"),
            grant: production_grant,
            expected_grant_revision: ResourceRevision::INITIAL,
            current_boundary: boundary,
            eligibility_envelope_digest: eligibility.digest.clone(),
        }),
    )
    .expect("production prepare ingress");
    settle_at_epoch(&mut canwu, "production prepare boundary");

    let coordinator_source = current_record_version(&canwu, production_runtime_reference());
    let boundary = next_boundary(&canwu);
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::PrepareExternalParticipant(
            PrepareExternalCompletionParticipantGrantV1 {
                coordinator_source,
                acquisition: acquisition.clone(),
                expected_grant_revision: ResourceRevision::INITIAL,
                current_boundary: boundary,
                eligibility_envelope_digest: eligibility.digest,
            },
        ),
    )
    .expect("resource prepare ingress");
    settle_at_epoch(&mut canwu, "resource prepare boundary");
    let (_, resource_state) = canwu_resource::resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    let prepared = resource_state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    let resource_source =
        current_record_version(&canwu, canwu_resource::resource_runtime_reference());
    enqueue_production_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ProductionCompletionIngressV1::AcknowledgeParticipantPrepared {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::new(4).expect("revision"),
            participant: canwu_resource::PLUGIN_NAME.to_owned(),
            provider_source: resource_source,
            grant: prepared,
        },
    )
    .expect("resource prepare acknowledgement ingress");
    settle_at_epoch(&mut canwu, "resource prepare acknowledgement boundary");

    let boundary = next_boundary(&canwu);
    enqueue_production_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ProductionCompletionIngressV1::Activate {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::new(5).expect("revision"),
            current_boundary: boundary,
        },
    )
    .expect("activation ingress");
    settle_at_epoch(&mut canwu, "activation boundary");
    let certificate =
        production_state(&canwu).production_completion_certificates[&acquisition].clone();

    let coordinator_source = current_record_version(&canwu, production_runtime_reference());
    enqueue_resource_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ResourceCompletionOperationV1::ConsumeExternalParticipant(
            ConsumeExternalCompletionParticipantGrantV1 {
                coordinator_source,
                certificate,
                at: SimTime::EPOCH,
            },
        ),
    )
    .expect("resource consume ingress");
    settle_at_epoch(&mut canwu, "resource consume boundary");
    let (_, resource_state) = canwu_resource::resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    let consumed = resource_state.external_completion_participants.grants[&acquisition]
        .grant
        .clone();
    assert_eq!(consumed.state, CompletionGrantStateV1::Consumed);
    let resource_source =
        current_record_version(&canwu, canwu_resource::resource_runtime_reference());
    enqueue_production_completion_operation(
        &mut canwu,
        SimTime::EPOCH,
        &ProductionCompletionIngressV1::AcknowledgeParticipantConsumed {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::new(6).expect("revision"),
            participant: canwu_resource::PLUGIN_NAME.to_owned(),
            provider_source: resource_source,
            grant: consumed,
        },
    )
    .expect("resource consumption acknowledgement ingress");
    settle_at_epoch(&mut canwu, "resource consumption acknowledgement boundary");
    let state = production_state(&canwu);
    let status = state
        .completion_status_for(&holder, &acquisition)
        .expect("holder completion status");
    assert_eq!(
        status.state,
        canwu_resource::CompletionLeaseAcquisitionStateV1::Activated
    );
    assert_eq!(
        status.grant_states[canwu_resource::PLUGIN_NAME],
        CompletionGrantStateV1::Consumed
    );
    assert!(
        state
            .completion_target_locks
            .values()
            .any(|(_, grant)| grant.as_str().contains("production:completion-grant"))
    );
}

fn command(
    state: &ProductionState,
    holder: &KnowledgeHolderRef,
    id: &str,
    operation: ProductionOperation,
) -> ProductionCommandEnvelope {
    ProductionCommandEnvelope {
        operation_id: ProductionOperationOutcomeId::new(id).expect("operation ID"),
        holder: holder.clone(),
        expected_runtime_revision: state.revision,
        operation,
    }
}

fn unrelated_outcome(
    id: ProductionOperationOutcomeId,
    holder: &KnowledgeHolderRef,
) -> ProductionOperationOutcome {
    let command = ProductionCommandEnvelope {
        operation_id: id.clone(),
        holder: holder.clone(),
        expected_runtime_revision: 0,
        operation: ProductionOperation::RetireFacility {
            facility: FacilityAssetId::new(format!("production:filler:{}", id.as_str()))
                .expect("filler facility ID"),
            expected_generation: 1,
        },
    };
    ProductionOperationOutcome {
        id,
        canonical_input_hash: canwu_api::canonical_hash(
            "canwu.production.operation-input.v1",
            &command,
        )
        .expect("filler operation hash"),
        command,
        disposition: ProductionOperationDisposition::Applied,
        work_order: None,
        execution: None,
        project: None,
        rejection_code: None,
        rejection_message: None,
        settled_at: SimTime::EPOCH,
    }
}

fn work_order(
    id: &str,
    holder: &KnowledgeHolderRef,
    process: &ProcessRevisionId,
    site: &ProductionSiteId,
) -> WorkOrder {
    WorkOrder {
        id: WorkOrderId::new(id).expect("work order ID"),
        holder: holder.clone(),
        process: process.clone(),
        site: site.clone(),
        quantity: 1,
        lifecycle: WorkOrderLifecycle::Proposed,
        execution: None,
        expected_revision: 1,
        created_at: SimTime::EPOCH,
    }
}

fn technology_binding(adoption_required: bool) -> TechnologyEvidenceBinding {
    TechnologyEvidenceBinding {
        technique_revision: version::<canwu_technology::TechniqueRevision>(
            "technology:test-technique:v1",
        ),
        capability_qualification: Some(version::<canwu_technology::CapabilityQualification>(
            "technology:test-qualification:v1",
        )),
        implementation: Some(version::<canwu_technology::ImplementationRecord>(
            "technology:test-implementation:v1",
        )),
        adoption: adoption_required
            .then(|| version::<canwu_technology::AdoptionRecord>("technology:test-adoption:v1")),
        semantic_digest: "technology-binding-digest".to_owned(),
    }
}

fn resource_input(process: &ProcessRevision, suffix: &str) -> ResourceInputBinding {
    let account =
        ResourceAccountId::new(format!("resource:input-account:{suffix}")).expect("resource ID");
    let leg_id =
        ResourceAllocationLegId::new(format!("resource:input-leg:{suffix}")).expect("resource ID");
    let mut binding = ResourceInputBinding {
        allocation_leg: ResourceAllocationLegVersionV1 {
            id: leg_id.clone(),
            revision: resource_revision(),
            account: account.clone(),
            account_revision: resource_revision(),
            resource_revision: process.inputs[0].resource.clone(),
            unit_revision: process.inputs[0].unit.clone(),
            quantity: process.inputs[0].quantity,
            semantic_digest: "input-leg-digest".to_owned(),
        },
        consumption: ResourceConsumptionVersionV1 {
            id: ResourceConsumptionId::new(format!("resource:consumption:{suffix}"))
                .expect("resource ID"),
            revision: resource_revision(),
            account: account.clone(),
            allocation_leg: leg_id.clone(),
            quantity: process.inputs[0].quantity,
            consumer_evidence: version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID),
            semantic_digest: "consumption-record-digest".to_owned(),
        },
        consumption_outcome: ResourceOperationOutcomeVersionV1 {
            id: ResourceOperationOutcomeId::new(format!("resource:consume-outcome:{suffix}"))
                .expect("resource ID"),
            revision: resource_revision(),
            operation_key: ResourceOperationKey::new(format!("resource:consume-key:{suffix}"))
                .expect("resource ID"),
            status: ResourceOperationStatus::Applied,
            quantity: process.inputs[0].quantity,
            remainder: 0,
            result_ref: None,
            semantic_digest: "consume-outcome-digest".to_owned(),
        },
        quantity: process.inputs[0].quantity,
    };
    canonicalize_archived_resource_input(&mut binding);
    binding
}

#[allow(clippy::too_many_arguments)]
fn start_execution(
    state: &mut ProductionState,
    holder: &KnowledgeHolderRef,
    facility_id: &FacilityAssetId,
    order_id: &WorkOrderId,
    execution_suffix: &str,
    local_target_id: &str,
    evidence: Vec<ProductionEvidenceBinding>,
    allocation_state: CapacityAllocationState,
) -> Result<ProductionExecutionId, canwu_api::CanwuError> {
    let at = SimTime::from_minutes(
        i64::try_from(state.completion_acquisitions.len()).expect("test completion time"),
    );
    let order = state.work_orders[order_id].clone();
    let process = state.processes[&order.process].clone();
    let execution_id =
        ProductionExecutionId::new(format!("production:execution:{execution_suffix}"))
            .expect("execution ID");
    let allocation_id =
        ProductionCapacityAllocationId::new(format!("production:allocation:{execution_suffix}"))
            .expect("allocation ID");
    let output_key = ResourceOperationKey::new(format!("resource:output-key:{execution_suffix}"))
        .expect("resource ID");
    let input = resource_input(&process, execution_suffix);
    let output_requests = process
        .outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let operation_key = if index == 0 {
                output_key.clone()
            } else {
                ResourceOperationKey::new(format!("resource:output-key:{execution_suffix}:{index}"))
                    .expect("resource output operation key")
            };
            let account = if index == 0 {
                ResourceAccountId::new(format!("resource:output-account:{execution_suffix}"))
                    .expect("resource ID")
            } else {
                ResourceAccountId::new(format!(
                    "resource:output-account:{execution_suffix}:{index}"
                ))
                .expect("resource output account")
            };
            ProductionOutputSettlementRequest {
                operation_key,
                account,
                expected_account_revision: resource_revision(),
                resource: output.resource.clone(),
                unit: output.unit.clone(),
                quantity: output.quantity,
            }
        })
        .collect::<Vec<_>>();
    let acquisition = CompletionLeaseAcquisitionId::new(format!(
        "production:completion-acquisition:{execution_suffix}"
    ))
    .expect("acquisition ID");
    let production_grant =
        CompletionCapacityGrantId::new(format!("production:completion-grant:{execution_suffix}"))
            .expect("grant ID");
    let resource_grant =
        CompletionCapacityGrantId::new(format!("resource:completion-grant:{execution_suffix}"))
            .expect("grant ID");
    let mut resource_targets = output_requests
        .iter()
        .map(|output| CompletionLockedTargetV1::Account {
            id: output.account.clone(),
            revision: output.expected_account_revision,
        })
        .collect::<Vec<_>>();
    resource_targets.extend([CompletionLockedTargetV1::AllocationLeg {
        id: input.allocation_leg.id.clone(),
        revision: input.allocation_leg.revision,
    }]);
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 1,
        holders: 1,
        bytes: 4_096,
    };
    let recipe_units = recipe.canonical_units().expect("recipe units");
    let eligibility = EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("eligibility envelope");
    state.production_run_budget = Some(
        RunBudgetRevisionV1 {
            revision: resource_revision(),
            total_completion_units: recipe_units.saturating_mul(4),
            shared_pending_slots: 0,
            partitions: vec![CompletionCapacityPartitionV1 {
                authority: holder.clone(),
                operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
                guaranteed_units: recipe_units.saturating_mul(2),
                reserved_pending_slots: 2,
                maximum_burst_units: recipe_units,
                request_token_capacity: 2,
                request_token_refill_minutes: 1,
                reacquire_cooldown_minutes: 1,
                root_acquisition_cap_per_sim_time: 2,
                guaranteed_max_wait_boundaries: 4,
            }],
            semantic_digest: String::new(),
        }
        .seal()
        .expect("production run budget"),
    );
    state
        .request_completion_acquisition(RequestCompletionLeaseV1 {
            id: acquisition.clone(),
            operation_key: output_key.clone(),
            holder: holder.clone(),
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
                expected_acquisition_revision: resource_revision(),
                owner_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: version::<ProductionRuntimeRecord>(local_target_id),
                }],
                current_boundary: 1,
            },
        ))
        .expect("local completion grant");
    let mut participant_grant = CompletionCapacityGrantV1 {
        id: resource_grant.clone(),
        revision: resource_revision(),
        acquisition: acquisition.clone(),
        operation_key: output_key.clone(),
        owner_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
        run_budget_revision: resource_revision(),
        target_versions: resource_targets,
        recipe_digest: recipe.digest().expect("recipe digest"),
        reserved_units: recipe_units,
        expires_after_boundary: 9,
        activation_deadline_boundary: None,
        state: CompletionGrantStateV1::Held,
        rejection: None,
    };
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantGrant {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: ResourceRevision::new(2).expect("revision"),
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant.clone(),
            },
        )
        .expect("resource participant grant acknowledgement");
    state
        .apply_completion_ingress(&ProductionCompletionIngressV1::PrepareLocal(
            PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: ResourceRevision::new(3).expect("revision"),
                grant: production_grant.clone(),
                expected_grant_revision: resource_revision(),
                current_boundary: 1,
                eligibility_envelope_digest: eligibility.digest.clone(),
            },
        ))
        .expect("local completion prepare");
    participant_grant.state = CompletionGrantStateV1::Prepared;
    participant_grant.revision = ResourceRevision::new(2).expect("revision");
    participant_grant.activation_deadline_boundary = Some(8);
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantPrepared {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: ResourceRevision::new(4).expect("revision"),
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant,
            },
        )
        .expect("resource participant prepare acknowledgement");
    let certificate = state
        .apply_completion_ingress(&ProductionCompletionIngressV1::Activate {
            acquisition: acquisition.clone(),
            expected_acquisition_revision: ResourceRevision::new(5).expect("revision"),
            current_boundary: 1,
        })
        .expect("completion activation")
        .expect("activation certificate");
    let execution = ProductionExecution {
        id: execution_id.clone(),
        work_order: order.id.clone(),
        process: process.id.clone(),
        site: order.site.clone(),
        facility: facility_id.clone(),
        allocations: vec![allocation_id.clone()],
        lifecycle: WorkOrderLifecycle::Running,
        started_at: at,
        completed_at: None,
        evidence,
        technology: technology_binding(process.adoption_required),
        inputs: vec![input],
        output_requests,
        output_outcomes: Vec::new(),
        output_source: None,
        output_ack_digest: None,
        completion_certificate: certificate,
        production_completion_grant: production_grant,
        resource_completion_grant: resource_grant,
    };
    let allocation = ProductionCapacityAllocation {
        id: allocation_id,
        facility: facility_id.clone(),
        facility_generation: state.facilities[facility_id].generation,
        capability: process.capacity[0].capability.clone(),
        start: at,
        end: at
            .checked_add(canwu_api::SimDuration::minutes(60))
            .expect("allocation end"),
        quantity: 1,
        work_order: order.id,
        execution: execution_id.clone(),
        operation_key: format!("production:capacity:{execution_suffix}"),
        state: allocation_state,
    };
    state.apply_operation(
        &command(
            state,
            holder,
            &format!("production:start:{execution_suffix}"),
            ProductionOperation::StartExecution {
                execution,
                allocations: vec![allocation],
            },
        ),
        at,
    )?;
    Ok(execution_id)
}

#[allow(clippy::too_many_arguments)]
fn facility_project(
    state: &mut ProductionState,
    holder: &KnowledgeHolderRef,
    site: &ProductionSiteId,
    facility: &FacilityAssetId,
    process_id: &ProcessRevisionId,
    suffix: &str,
    kind: FacilityProjectKind,
    at: SimTime,
) -> FacilityProject {
    let process = state.processes[process_id].clone();
    let operation_key = ResourceOperationKey::new(format!("resource:project:{suffix}"))
        .expect("project operation key");
    let input = resource_input(&process, suffix);
    let acquisition =
        CompletionLeaseAcquisitionId::new(format!("production:project-acquisition:{suffix}"))
            .expect("project acquisition");
    let production_grant =
        CompletionCapacityGrantId::new(format!("production:project-grant:{suffix}"))
            .expect("project grant");
    let resource_grant = CompletionCapacityGrantId::new(format!("resource:project-grant:{suffix}"))
        .expect("resource project grant");
    let recipe = CompletionCapacityRecipeV1 {
        receipts: MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
        mutations: 4,
        reports_per_holder: 1,
        holders: 1,
        bytes: 4_096,
    };
    let recipe_units = recipe.canonical_units().expect("project recipe units");
    let eligibility = EligibilityEnvelopeV1::new(
        Vec::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("project eligibility");
    state.production_run_budget = Some(
        RunBudgetRevisionV1 {
            revision: resource_revision(),
            total_completion_units: recipe_units.saturating_mul(2),
            shared_pending_slots: 0,
            partitions: vec![CompletionCapacityPartitionV1 {
                authority: holder.clone(),
                operation_namespace: PRODUCTION_COMPLETION_OPERATION_NAMESPACE.to_owned(),
                guaranteed_units: recipe_units.saturating_mul(2),
                reserved_pending_slots: 2,
                maximum_burst_units: recipe_units,
                request_token_capacity: 2,
                request_token_refill_minutes: 1,
                reacquire_cooldown_minutes: 1,
                root_acquisition_cap_per_sim_time: 2,
                guaranteed_max_wait_boundaries: 4,
            }],
            semantic_digest: String::new(),
        }
        .seal()
        .expect("project run budget"),
    );
    state
        .request_completion_acquisition(RequestCompletionLeaseV1 {
            id: acquisition.clone(),
            operation_key: operation_key.clone(),
            holder: holder.clone(),
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
        .expect("project completion acquisition");
    let acquisition_revision = state.completion_acquisitions[&acquisition].revision;
    state
        .apply_completion_ingress(&ProductionCompletionIngressV1::GrantLocal(
            GrantCompletionCapacityV1 {
                grant_id: production_grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                owner_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                    version: version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID),
                }],
                current_boundary: 1,
            },
        ))
        .expect("project local grant");
    let mut participant_grant = CompletionCapacityGrantV1 {
        id: resource_grant.clone(),
        revision: resource_revision(),
        acquisition: acquisition.clone(),
        operation_key: operation_key.clone(),
        owner_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
        run_budget_revision: resource_revision(),
        target_versions: vec![
            CompletionLockedTargetV1::AllocationLeg {
                id: input.allocation_leg.id.clone(),
                revision: input.allocation_leg.revision,
            },
            CompletionLockedTargetV1::ExternalRecord {
                version: version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID),
            },
        ],
        recipe_digest: recipe.digest().expect("project recipe digest"),
        reserved_units: recipe_units,
        expires_after_boundary: 9,
        activation_deadline_boundary: None,
        state: CompletionGrantStateV1::Held,
        rejection: None,
    };
    let acquisition_revision = state.completion_acquisitions[&acquisition].revision;
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantGrant {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant.clone(),
            },
        )
        .expect("project resource grant acknowledgement");
    let acquisition_revision = state.completion_acquisitions[&acquisition].revision;
    let grant_revision = state.production_completion_grants[&production_grant].revision;
    state
        .apply_completion_ingress(&ProductionCompletionIngressV1::PrepareLocal(
            PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                grant: production_grant.clone(),
                expected_grant_revision: grant_revision,
                current_boundary: 1,
                eligibility_envelope_digest: eligibility.digest,
            },
        ))
        .expect("project local prepare");
    participant_grant.state = CompletionGrantStateV1::Prepared;
    participant_grant.revision = ResourceRevision::new(2).expect("revision");
    participant_grant.activation_deadline_boundary = Some(8);
    let acquisition_revision = state.completion_acquisitions[&acquisition].revision;
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantPrepared {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant.clone(),
            },
        )
        .expect("project resource prepare acknowledgement");
    let acquisition_revision = state.completion_acquisitions[&acquisition].revision;
    let certificate = state
        .apply_completion_ingress(&ProductionCompletionIngressV1::Activate {
            acquisition,
            expected_acquisition_revision: acquisition_revision,
            current_boundary: 1,
        })
        .expect("project activation")
        .expect("project certificate");
    participant_grant.state = CompletionGrantStateV1::Consumed;
    participant_grant.revision = ResourceRevision::new(3).expect("revision");
    let acquisition_revision = state.completion_acquisitions[&certificate.acquisition].revision;
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantConsumed {
                acquisition: certificate.acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: version::<ResourceRuntimeRecord>("resource:runtime:v1"),
                grant: participant_grant,
            },
        )
        .expect("project resource consumption acknowledgement");
    let base_generation = state.facilities[facility].generation;
    FacilityProject {
        id: FacilityProjectId::new(format!("production:project:{suffix}")).expect("project ID"),
        holder: holder.clone(),
        site: site.clone(),
        facility: facility.clone(),
        kind,
        process: process.id.clone(),
        lifecycle: FacilityProjectLifecycle::Planned,
        completed_units: 0,
        total_units: process.work_units,
        base_generation,
        resulting_generation: base_generation + 1,
        evidence: process
            .requirements
            .iter()
            .map(|group| {
                let alternative = &group.any_of[0];
                evidence(
                    group.kind,
                    &alternative.capability,
                    alternative.minimum_quantity,
                )
            })
            .collect(),
        technology: technology_binding(process.adoption_required),
        inputs: vec![input],
        operation_key,
        completion_certificate: certificate,
        production_completion_grant: production_grant,
        resource_completion_grant: resource_grant,
        resulting_asset: None,
        created_at: at,
        started_at: None,
        completed_at: None,
        result_evidence_digest: None,
    }
}

fn resource_state_for_project(
    production: &ProductionState,
    project: &FacilityProject,
) -> ResourceState {
    let acquisition =
        &production.completion_acquisitions[&project.completion_certificate.acquisition];
    let input = &project.inputs[0];
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource");
    resource
        .install_run_budget(
            production
                .production_run_budget
                .clone()
                .expect("project run budget"),
        )
        .expect("resource project run budget");
    let report_grant_id =
        ResourceReportGrantId::new("resource:project-report-grant").expect("report grant ID");
    resource
        .install_report_grant(ResourceReportGrantV1 {
            id: report_grant_id.clone(),
            holder: project.holder.clone(),
            scope: ResourceScopeId::new("resource:project-report-scope").expect("report scope ID"),
            accounts: BTreeSet::new(),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 60,
            delay_minutes: 0,
        })
        .expect("resource project report grant");
    let report_source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    resource
        .record_observation_head(
            canwu_resource::ResourceObservationHeadV1 {
                id: canwu_resource::ResourceObservationHeadId::new("resource:project-observation")
                    .expect("observation head ID"),
                revision: resource_revision(),
                provider_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                provider_version: "test".to_owned(),
                provider_semantic_hash: "f".repeat(64),
                provider_source: report_source.clone(),
                holder: project.holder.clone(),
                grant: report_grant_id,
                provider_state_revision: ResourceRevision::new(1_000_000)
                    .expect("future provider revision"),
                observed_at: project.created_at,
                confidence_per_mille: 1_000,
                stock: Vec::new(),
                demands: Vec::new(),
                allocations: Vec::new(),
                fulfillments: Vec::new(),
                transfers: Vec::new(),
                consumptions: Vec::new(),
                source_versions: vec![report_source],
                semantic_digest: String::new(),
            }
            .seal()
            .expect("seal resource project observation"),
        )
        .expect("record resource project observation");
    resource.allocation_legs.insert(
        input.allocation_leg.id.clone(),
        ResourceAllocationLeg {
            id: input.allocation_leg.id.clone(),
            revision: input.allocation_leg.revision,
            demand: ResourceDemandId::new("resource:project-demand").expect("demand ID"),
            demand_revision: resource_revision(),
            reservation: ResourceReservationId::new("resource:project-reservation")
                .expect("reservation ID"),
            account: input.allocation_leg.account.clone(),
            account_revision: input.allocation_leg.account_revision,
            resource_revision: input.allocation_leg.resource_revision.clone(),
            unit_revision: input.allocation_leg.unit_revision.clone(),
            quantity: input.allocation_leg.quantity,
            status: AllocationLegStatus::Consumed,
            priority: 0,
            due_at: project.created_at,
            tie_break: ResourceTieBreakKey::new("resource:project-tie").expect("tie break"),
            admitted_sequence: 1,
            operation_key: ResourceOperationKey::new("resource:project-allocation")
                .expect("allocation operation"),
            semantic_digest: input.allocation_leg.semantic_digest.clone(),
        },
    );
    let coordinator_source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let granted = resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: acquisition.revision,
                    acquisition: acquisition.id.clone(),
                    operation_key: project.operation_key.clone(),
                    holder: project.holder.clone(),
                    operation_namespace: acquisition.operation_namespace.clone(),
                    eligibility_time: acquisition.eligibility_time,
                    eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
                    recipe: acquisition.recipe.clone(),
                    policy_class: acquisition.policy_class,
                    grant_id: project.resource_completion_grant.clone(),
                    target_versions: vec![
                        CompletionLockedTargetV1::AllocationLeg {
                            id: input.allocation_leg.id.clone(),
                            revision: input.allocation_leg.revision,
                        },
                        CompletionLockedTargetV1::ExternalRecord {
                            version: coordinator_source.clone(),
                        },
                    ],
                    current_boundary: 1,
                },
            ),
        ))
        .expect("resource project grant");
    assert_eq!(
        granted.status,
        ResourceOperationStatus::Applied,
        "resource project grant rejected: {:?}",
        granted.rejection_reason
    );
    let prepared = resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: coordinator_source.clone(),
                    acquisition: acquisition.id.clone(),
                    expected_grant_revision: resource_revision(),
                    current_boundary: 1,
                    eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
                },
            ),
        ))
        .expect("resource project prepare");
    assert_eq!(
        prepared.status,
        ResourceOperationStatus::Applied,
        "resource project prepare rejected: {:?}",
        prepared.rejection_reason
    );
    let consumed = resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::ConsumeExternalParticipant(
                ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    certificate: project.completion_certificate.clone(),
                    at: project.created_at,
                },
            ),
        ))
        .expect("resource project consume");
    assert_eq!(
        consumed.status,
        ResourceOperationStatus::Applied,
        "resource project consume rejected: {:?}",
        consumed.rejection_reason
    );
    resource.report_dirty_grants.clear();
    resource
}

fn resource_state_for_execution(
    production: &ProductionState,
    execution: &ProductionExecution,
) -> ResourceState {
    let acquisition =
        &production.completion_acquisitions[&execution.completion_certificate.acquisition];
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource");
    resource
        .install_run_budget(
            production
                .production_run_budget
                .clone()
                .expect("execution run budget"),
        )
        .expect("resource execution run budget");
    resource
        .install_report_grant(ResourceReportGrantV1 {
            id: ResourceReportGrantId::new("resource:production-output-report-grant")
                .expect("output report grant ID"),
            holder: production.work_orders[&execution.work_order].holder.clone(),
            scope: ResourceScopeId::new("resource:production-output-report-scope")
                .expect("output report scope ID"),
            accounts: BTreeSet::new(),
            demands: BTreeSet::new(),
            include_transfer_details: false,
            confidence_per_mille: 1_000,
            cadence_minutes: 60,
            delay_minutes: 0,
        })
        .expect("resource output report grant");
    let mut installed_units = BTreeSet::new();
    let mut installed_resources = BTreeSet::new();
    for (index, output) in execution.output_requests.iter().enumerate() {
        if installed_units.insert(output.unit.clone()) {
            resource
                .install_unit(ResourceUnitRevision {
                    id: output.unit.clone(),
                    revision: ResourceRevision::INITIAL,
                    symbol: format!("u{index}"),
                    scale_numerator: 1,
                    scale_denominator: 1,
                    semantic_digest: format!("{:064x}", index + 1),
                })
                .expect("output unit");
        }
        if installed_resources.insert(output.resource.clone()) {
            resource
                .install_definition(ResourceDefinitionRevision {
                    id: output.resource.clone(),
                    resource: ResourceDefinitionId::new(format!(
                        "resource:production-output-definition:{index}"
                    ))
                    .expect("output definition ID"),
                    revision: ResourceRevision::INITIAL,
                    canonical_unit: output.unit.clone(),
                    quality: ResourceQualityId::new(format!(
                        "resource:production-output-quality:{index}"
                    ))
                    .expect("output quality ID"),
                    scope: ResourceScopeId::new(format!(
                        "resource:production-output-scope:{index}"
                    ))
                    .expect("output scope ID"),
                    effective_from: SimTime::EPOCH,
                    effective_until: None,
                    process_suitability: BTreeSet::new(),
                    semantic_digest: format!("{:064x}", index + 100),
                })
                .expect("output definition");
        }
        resource.accounts.insert(
            output.account.clone(),
            ResourceAccount {
                id: output.account.clone(),
                revision: output.expected_account_revision,
                custodian: production.work_orders[&execution.work_order].holder.clone(),
                resource_revision: output.resource.clone(),
                unit_revision: output.unit.clone(),
                balance: 0,
                capacity: None,
                protected_floor_policy: None,
                closed: false,
            },
        );
    }
    let input = &execution.inputs[0];
    resource.allocation_legs.insert(
        input.allocation_leg.id.clone(),
        ResourceAllocationLeg {
            id: input.allocation_leg.id.clone(),
            revision: input.allocation_leg.revision,
            demand: ResourceDemandId::new("resource:production-output-demand")
                .expect("output demand ID"),
            demand_revision: ResourceRevision::INITIAL,
            reservation: ResourceReservationId::new("resource:production-output-reservation")
                .expect("output reservation ID"),
            account: input.allocation_leg.account.clone(),
            account_revision: input.allocation_leg.account_revision,
            resource_revision: input.allocation_leg.resource_revision.clone(),
            unit_revision: input.allocation_leg.unit_revision.clone(),
            quantity: input.allocation_leg.quantity,
            status: AllocationLegStatus::Consumed,
            priority: 0,
            due_at: execution.started_at,
            tie_break: ResourceTieBreakKey::new("resource:production-output-tie")
                .expect("output tie break"),
            admitted_sequence: 1,
            operation_key: ResourceOperationKey::new("resource:production-output-allocation")
                .expect("output allocation operation"),
            semantic_digest: input.allocation_leg.semantic_digest.clone(),
        },
    );
    let coordinator_source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let mirrored = &production.completion_participant_grants[&acquisition.id]
        [canwu_resource::PLUGIN_NAME]
        .grant;
    let mut resource_targets = mirrored.target_versions.clone();
    let production_target = CompletionLockedTargetV1::ExternalRecord {
        version: coordinator_source.clone(),
    };
    if !resource_targets.contains(&production_target) {
        resource_targets.push(production_target);
        resource_targets.sort();
    }
    let granted = resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::GrantExternalParticipant(
                RequestExternalCompletionParticipantGrantV1 {
                    coordinator_plugin: canwu_production::PLUGIN_NAME.to_owned(),
                    coordinator_source: coordinator_source.clone(),
                    coordinator_acquisition_revision: acquisition.revision,
                    acquisition: acquisition.id.clone(),
                    operation_key: acquisition.operation_key.clone(),
                    holder: acquisition.holder.clone(),
                    operation_namespace: acquisition.operation_namespace.clone(),
                    eligibility_time: acquisition.eligibility_time,
                    eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
                    recipe: acquisition.recipe.clone(),
                    policy_class: acquisition.policy_class,
                    grant_id: execution.resource_completion_grant.clone(),
                    target_versions: resource_targets,
                    current_boundary: 1,
                },
            ),
        ))
        .expect("resource output participant grant");
    assert_eq!(
        granted.status,
        ResourceOperationStatus::Applied,
        "resource output participant grant rejected: {:?}",
        granted.rejection_reason
    );
    let prepared = resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source: coordinator_source.clone(),
                    acquisition: acquisition.id.clone(),
                    expected_grant_revision: ResourceRevision::INITIAL,
                    current_boundary: 1,
                    eligibility_envelope_digest: acquisition.eligibility_envelope.digest.clone(),
                },
            ),
        ))
        .expect("resource output participant prepare");
    assert_eq!(
        prepared.status,
        ResourceOperationStatus::Applied,
        "resource output participant prepare rejected: {:?}",
        prepared.rejection_reason
    );
    let consumed = resource
        .apply_operation(&ResourceOperationRequestV1::Completion(
            ResourceCompletionOperationV1::ConsumeExternalParticipant(
                ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    certificate: execution.completion_certificate.clone(),
                    at: execution.started_at,
                },
            ),
        ))
        .expect("resource output participant consume");
    assert_eq!(
        consumed.status,
        ResourceOperationStatus::Applied,
        "resource output participant consume rejected: {:?}",
        consumed.rejection_reason
    );
    resource.report_dirty_grants.clear();
    resource
}

fn enqueue_tracked_production_operation(
    canwu: &mut Canwu,
    holder: &KnowledgeHolderRef,
    request_id: u64,
    operation_id: &str,
    operation: ProductionOperation,
) {
    let expected_runtime_revision = production_state(canwu).revision;
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    Command::Plugin {
                        plugin: canwu_production::PLUGIN_NAME.to_owned(),
                        command: PRODUCTION_COMMAND.to_owned(),
                        payload: serde_json::to_value(ProductionCommandEnvelope {
                            operation_id: ProductionOperationOutcomeId::new(operation_id)
                                .expect("operation ID"),
                            holder: holder.clone(),
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

fn archive_resource_input(
    resource: &mut ResourceState,
    store: &TestProductionArchiveStore,
    input: &ResourceInputBinding,
) {
    let (consumption, outcome) = archived_resource_input_payloads(input);
    let records = vec![
        canwu_resource::ResourceTerminalArchiveRecordV1 {
            key: canwu_resource::ResourceTerminalRecordKeyV1::Consumption(
                input.consumption.id.clone(),
            ),
            operation_key: input.consumption_outcome.operation_key.clone(),
            quantity: input.quantity,
            remainder: 0,
            exact_evidence: vec![input.consumption.consumer_evidence.clone()],
            semantic_digest: input.consumption.semantic_digest.clone(),
            terminal_sequence: 1,
            payload: canwu_resource::ResourceTerminalArchivePayloadV1::Consumption(consumption),
        },
        canwu_resource::ResourceTerminalArchiveRecordV1 {
            key: canwu_resource::ResourceTerminalRecordKeyV1::Outcome(
                input.consumption_outcome.operation_key.clone(),
            ),
            operation_key: input.consumption_outcome.operation_key.clone(),
            quantity: input.quantity,
            remainder: 0,
            exact_evidence: outcome.exact_evidence.clone(),
            semantic_digest: input.consumption_outcome.semantic_digest.clone(),
            terminal_sequence: 2,
            payload: canwu_resource::ResourceTerminalArchivePayloadV1::Outcome(outcome),
        },
    ];
    let mut blob = canwu_resource::ResourceArchiveBlobV1 {
        format_version: 1,
        expected_source_root: "c".repeat(64),
        records: records.clone(),
        content_id: String::new(),
    };
    blob.content_id = canwu_resource::canonical_digest("canwu.resource.archive-blob.v1", &blob)
        .expect("resource blob digest");
    let mut membership = canwu_resource::ResourceArchiveMembershipPageV1 {
        id: String::new(),
        memberships: records
            .iter()
            .enumerate()
            .map(
                |(ordinal, record)| canwu_resource::ResourceArchiveMembershipV1 {
                    key: record.key.clone(),
                    blob_id: blob.content_id.clone(),
                    ordinal: u16::try_from(ordinal).expect("ordinal"),
                    terminal_sequence: record.terminal_sequence,
                    semantic_digest: record.semantic_digest.clone(),
                },
            )
            .collect(),
        semantic_digest: String::new(),
    };
    membership.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-membership-page.v1", &membership)
            .expect("membership digest");
    membership.id = membership.semantic_digest.clone();
    let mut temporal = canwu_resource::ResourceArchiveTemporalPageV1 {
        id: String::new(),
        entries: records
            .iter()
            .map(|record| canwu_resource::ResourceArchiveTemporalEntryV1 {
                terminal_sequence: record.terminal_sequence,
                key: record.key.clone(),
            })
            .collect(),
        semantic_digest: String::new(),
    };
    temporal.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-temporal-page.v1", &temporal)
            .expect("temporal digest");
    temporal.id = temporal.semantic_digest.clone();
    let mut directory = canwu_resource::ResourceArchiveIndexDirectoryV1 {
        id: String::new(),
        previous_root: None,
        membership_pages: vec![membership.id.clone()],
        temporal_pages: vec![temporal.id.clone()],
        blob_ids: vec![blob.content_id.clone()],
        archived_record_count: 2,
        semantic_digest: String::new(),
    };
    directory.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-directory.v1", &directory)
            .expect("directory digest");
    directory.id.clone_from(&directory.semantic_digest);
    for (namespace, object_id, bytes) in [
        (
            canwu_resource::RESOURCE_ARCHIVE_BLOB_NAMESPACE,
            blob.content_id.as_str(),
            serde_json::to_vec(&blob).expect("blob bytes"),
        ),
        (
            canwu_resource::RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            membership.id.as_str(),
            serde_json::to_vec(&membership).expect("membership bytes"),
        ),
        (
            canwu_resource::RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
            temporal.id.as_str(),
            serde_json::to_vec(&temporal).expect("temporal bytes"),
        ),
        (
            canwu_resource::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            directory.id.as_str(),
            serde_json::to_vec(&directory).expect("directory bytes"),
        ),
    ] {
        store
            .store_resource_archive_object(namespace, object_id, &bytes)
            .expect("store resource archive object");
    }
    resource.archive_head.revision = 1;
    resource.archive_head.directory_root = Some(directory.id);
    resource.archive_head.archived_record_count = 2;
    resource.archive_head.semantic_digest.clear();
    resource.archive_head.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-head.v1", &resource.archive_head)
            .expect("resource archive head digest");
}

fn canonicalize_archived_resource_input(input: &mut ResourceInputBinding) {
    let (consumption, outcome) = archived_resource_input_payloads(input);
    input.consumption_outcome.id.clone_from(&outcome.id);
    input
        .consumption_outcome
        .operation_key
        .clone_from(&outcome.operation_key);
    input
        .consumption_outcome
        .result_ref
        .clone_from(&outcome.result_ref);
    input
        .consumption
        .semantic_digest
        .clone_from(&consumption.semantic_digest);
    input
        .consumption_outcome
        .semantic_digest
        .clone_from(&outcome.semantic_digest);
}

fn archived_resource_input_payloads(
    input: &ResourceInputBinding,
) -> (ResourceConsumption, ResourceOperationOutcome) {
    let mut consumption = ResourceConsumption {
        id: input.consumption.id.clone(),
        revision: input.consumption.revision,
        account: input.consumption.account.clone(),
        allocation_leg: input.consumption.allocation_leg.clone(),
        demand: ResourceDemandId::new("resource:archived-production-input-demand")
            .expect("archived demand ID"),
        resource_revision: input.allocation_leg.resource_revision.clone(),
        unit_revision: input.allocation_leg.unit_revision.clone(),
        quantity: input.quantity,
        consumer_evidence: input.consumption.consumer_evidence.clone(),
        completion_acquisition: CompletionLeaseAcquisitionId::new(
            "resource:archived-production-input-completion",
        )
        .expect("archived completion acquisition ID"),
        status: ConsumptionStatus::Settled,
        operation_key: input.consumption_outcome.operation_key.clone(),
        semantic_digest: String::new(),
        terminal_sequence: 1,
    };
    consumption.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.consumption.v1", &consumption)
            .expect("archived consumption digest");

    let request = canwu_resource::ResourceOperationRequestV1::Consume(
        canwu_resource::ResourceConsumptionRequestV1 {
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
        },
    );
    let request_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-request.v1", &request)
            .expect("archived request digest");
    let outcome_id = ResourceOperationOutcomeId::new(format!(
        "resource:outcome:{}:{}",
        input
            .consumption_outcome
            .operation_key
            .as_str()
            .replace(':', "-"),
        &request_digest[..16]
    ))
    .expect("archived outcome ID");
    let mut outcome = ResourceOperationOutcome {
        id: outcome_id,
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
        rejection_code: None,
        rejection_reason: None,
        exact_evidence: vec![consumption.consumer_evidence.clone()],
        semantic_digest: String::new(),
        sequence: 2,
    };
    outcome.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-outcome.v1", &outcome)
            .expect("archived outcome digest");
    (consumption, outcome)
}

fn store_rehashed_forged_project_archive(
    prepared: &PreparedProductionArchiveBatchV1,
    store: &TestProductionArchiveStore,
    mutate: impl FnOnce(&mut ProductionFacilityProjectArchiveRecordV1),
) -> ProductionArchiveIndexDirectoryV1 {
    let mut blob = prepared.blob.clone();
    let record = blob
        .project_records
        .first_mut()
        .expect("project archive record");
    mutate(record);
    record.canonical_digest.clear();
    record.canonical_digest = canwu_api::canonical_hash(
        "canwu.production.facility-project-archive-record.v1",
        record,
    )
    .expect("forged project record digest");
    let record_key = record.key.clone();
    let record_digest = record.canonical_digest.clone();
    blob.content_id.clear();
    blob.content_id = canwu_api::canonical_hash("canwu.production.archive-blob.v1", &blob)
        .expect("forged project blob digest");

    let mut membership = prepared.membership_page.clone();
    for entry in &mut membership.memberships {
        entry.blob_id.clone_from(&blob.content_id);
        if entry.key == record_key {
            entry.semantic_digest.clone_from(&record_digest);
        }
    }
    membership.id.clear();
    membership.semantic_digest.clear();
    membership.semantic_digest =
        canwu_api::canonical_hash("canwu.production.archive-membership-page.v1", &membership)
            .expect("forged membership digest");
    membership.id = membership.semantic_digest.clone();

    let temporal = prepared.temporal_page.clone();
    let mut directory = prepared.directory.clone();
    directory.id.clear();
    directory.semantic_digest.clear();
    directory.blob_ids = vec![blob.content_id.clone()];
    directory.membership_pages = vec![membership.id.clone()];
    directory.semantic_digest =
        canwu_api::canonical_hash("canwu.production.archive-directory.v1", &directory)
            .expect("forged directory digest");
    directory.id = directory.semantic_digest.clone();
    for (namespace, object_id, bytes) in [
        (
            PRODUCTION_ARCHIVE_BLOB_NAMESPACE,
            blob.content_id.as_str(),
            serde_json::to_vec(&blob).expect("forged blob bytes"),
        ),
        (
            PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            membership.id.as_str(),
            serde_json::to_vec(&membership).expect("forged membership bytes"),
        ),
        (
            PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
            temporal.id.as_str(),
            serde_json::to_vec(&temporal).expect("forged temporal bytes"),
        ),
        (
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            directory.id.as_str(),
            serde_json::to_vec(&directory).expect("forged directory bytes"),
        ),
    ] {
        store
            .store_production_archive_object(namespace, object_id, &bytes)
            .expect("store rehashed forged production archive object");
    }
    directory
}

fn store_rehashed_forged_execution_archive(
    prepared: &PreparedProductionArchiveBatchV1,
    store: &TestProductionArchiveStore,
    mutate: impl FnOnce(&mut ProductionTerminalArchiveRecordV1),
) -> ProductionArchiveIndexDirectoryV1 {
    let mut blob = prepared.blob.clone();
    let record = blob.records.first_mut().expect("execution archive record");
    mutate(record);
    record.canonical_digest.clear();
    record.canonical_digest =
        canwu_api::canonical_hash("canwu.production.terminal-archive-record.v1", record)
            .expect("forged execution record digest");
    let record_key = record.key.clone();
    let record_digest = record.canonical_digest.clone();
    blob.content_id.clear();
    blob.content_id = canwu_api::canonical_hash("canwu.production.archive-blob.v1", &blob)
        .expect("forged execution blob digest");

    let mut membership = prepared.membership_page.clone();
    for entry in &mut membership.memberships {
        entry.blob_id.clone_from(&blob.content_id);
        if entry.key == record_key {
            entry.semantic_digest.clone_from(&record_digest);
        }
    }
    membership.id.clear();
    membership.semantic_digest.clear();
    membership.semantic_digest =
        canwu_api::canonical_hash("canwu.production.archive-membership-page.v1", &membership)
            .expect("forged execution membership digest");
    membership.id = membership.semantic_digest.clone();

    let temporal = prepared.temporal_page.clone();
    let mut directory = prepared.directory.clone();
    directory.id.clear();
    directory.semantic_digest.clear();
    directory.blob_ids = vec![blob.content_id.clone()];
    directory.membership_pages = vec![membership.id.clone()];
    directory.semantic_digest =
        canwu_api::canonical_hash("canwu.production.archive-directory.v1", &directory)
            .expect("forged execution directory digest");
    directory.id = directory.semantic_digest.clone();
    for (namespace, object_id, bytes) in [
        (
            PRODUCTION_ARCHIVE_BLOB_NAMESPACE,
            blob.content_id.as_str(),
            serde_json::to_vec(&blob).expect("forged execution blob bytes"),
        ),
        (
            PRODUCTION_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            membership.id.as_str(),
            serde_json::to_vec(&membership).expect("forged execution membership bytes"),
        ),
        (
            PRODUCTION_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
            temporal.id.as_str(),
            serde_json::to_vec(&temporal).expect("forged execution temporal bytes"),
        ),
        (
            PRODUCTION_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            directory.id.as_str(),
            serde_json::to_vec(&directory).expect("forged execution directory bytes"),
        ),
    ] {
        store
            .store_production_archive_object(namespace, object_id, &bytes)
            .expect("store rehashed forged execution archive object");
    }
    directory
}

fn settled_state_for_archive() -> (ProductionState, ProductionExecutionId) {
    let (mut state, holder, site_id, facility_id) = base_state();
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("ID");
    let order = work_order("production:order:archive", &holder, &process_id, &site_id);
    let order_id = order.id.clone();
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:create:archive",
                ProductionOperation::CreateWorkOrder { work_order: order },
            ),
            SimTime::EPOCH,
        )
        .expect("archive work order");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:authorize:archive",
                ProductionOperation::AuthorizeWorkOrder {
                    work_order: order_id,
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize archive work");
    let execution = start_execution(
        &mut state,
        &holder,
        &facility_id,
        &WorkOrderId::new("production:order:archive").expect("ID"),
        "archive",
        PRODUCTION_RUNTIME_ID,
        vec![
            evidence(
                ProductionRequirementKind::LaborCapability,
                "customary-hand-milling",
                1,
            ),
            evidence(
                ProductionRequirementKind::Authorization,
                "household-authority",
                1,
            ),
        ],
        CapacityAllocationState::Reserved,
    )
    .expect("archive execution");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:advance:archive",
                ProductionOperation::AdvanceExecution {
                    execution: execution.clone(),
                    completed_units: 10,
                },
            ),
            SimTime::from_minutes(10),
        )
        .expect("archive work advance");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:complete:archive",
                ProductionOperation::CompleteExecution {
                    execution: execution.clone(),
                },
            ),
            SimTime::from_minutes(20),
        )
        .expect("archive work completion");
    let output = state.executions[&execution].output_requests[0].clone();
    let source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let certificate = state.executions[&execution].completion_certificate.clone();
    let resource_request = ResourceOperationRequestV1::Credit(output.resource_credit_request(
        source.clone(),
        certificate.clone(),
        SimTime::from_minutes(20),
    ));
    let request_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-request.v1", &resource_request)
            .expect("archive output request digest");
    let mut outcome = ResourceOperationOutcome {
        id: ResourceOperationOutcomeId::new(format!(
            "resource:outcome:{}:{}",
            output.operation_key.as_str().replace(':', "-"),
            &request_digest[..16]
        ))
        .expect("resource outcome ID"),
        revision: resource_revision(),
        operation_key: output.operation_key.clone(),
        request_digest,
        kind: ResourceOperationKind::Credit,
        status: ResourceOperationStatus::Applied,
        quantity: output.quantity,
        remainder: 0,
        result_ref: None,
        rejection_code: None,
        rejection_reason: None,
        exact_evidence: vec![source.clone()],
        semantic_digest: String::new(),
        sequence: 1,
    };
    outcome.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-outcome.v1", &outcome)
            .expect("archive output outcome digest");
    state
        .acknowledge_output(
            &ProductionOutputAcknowledgement {
                execution: execution.clone(),
                production_source: source.clone(),
                outcomes: vec![outcome],
            },
            SimTime::from_minutes(30),
        )
        .expect("archive output settlement");
    let participant = state.completion_participant_grants[&certificate.acquisition]
        [canwu_resource::PLUGIN_NAME]
        .clone();
    let mut consumed_grant = participant.grant;
    consumed_grant.state = CompletionGrantStateV1::Consumed;
    consumed_grant.revision = consumed_grant.revision.next().expect("consumed revision");
    let consumed_acquisition_revision =
        state.completion_acquisitions[&certificate.acquisition].revision;
    state
        .apply_completion_ingress(
            &ProductionCompletionIngressV1::AcknowledgeParticipantConsumed {
                acquisition: certificate.acquisition.clone(),
                expected_acquisition_revision: consumed_acquisition_revision,
                participant: canwu_resource::PLUGIN_NAME.to_owned(),
                provider_source: DomainRecordVersionRef {
                    record: canwu_resource::resource_runtime_reference().into_untyped(),
                    version: 1,
                    established_by: DomainRecordVersionSource::InitialScenario,
                },
                grant: consumed_grant.clone(),
            },
        )
        .expect("archive resource participant consumption");
    let mut completed_grant = consumed_grant;
    completed_grant.state = CompletionGrantStateV1::Completed;
    completed_grant.revision = completed_grant.revision.next().expect("completed revision");
    state
        .finalize_execution_resource_completion(
            &certificate.acquisition,
            &DomainRecordVersionRef {
                record: canwu_resource::resource_runtime_reference().into_untyped(),
                version: 1,
                established_by: DomainRecordVersionSource::InitialScenario,
            },
            &completed_grant,
        )
        .expect("archive resource participant completion");
    let wip_id =
        WorkInProgressId::new(format!("canwu.production:wip:{execution}")).expect("WIP ID");
    let wip = state
        .work_in_progress
        .get_mut(&wip_id)
        .expect("archive WIP");
    wip.recoverable_input_quantity = 7;
    wip.non_recoverable_waste_quantity = 3;
    state
        .facilities
        .get_mut(&facility_id)
        .expect("archive facility")
        .condition_per_mille = 725;
    state
        .rebuild_runtime_indexes()
        .expect("archive indexes rebuild");
    state.validate().expect("archive fixture validates");
    (state, execution)
}

#[test]
fn household_process_runs_while_machine_process_reports_exact_missing_constraints() {
    let (state, _, _, _) = base_state();
    let household =
        &state.processes[&ProcessRevisionId::new("production:household-process:v1").expect("ID")];
    let industrial =
        &state.processes[&ProcessRevisionId::new("production:industrial-process:v1").expect("ID")];
    let household_evidence = vec![
        evidence(
            ProductionRequirementKind::LaborCapability,
            "customary-hand-milling",
            1,
        ),
        evidence(
            ProductionRequirementKind::Authorization,
            "household-authority",
            1,
        ),
    ];
    assert!(
        state
            .blockers_for(household, &household_evidence)
            .is_empty()
    );

    let industrial_evidence = vec![
        evidence(ProductionRequirementKind::ToolsMachines, "steam-mill", 1),
        evidence(
            ProductionRequirementKind::TechnologyImplementation,
            "steam-mill-installed",
            1,
        ),
    ];
    let blockers = state.blockers_for(industrial, &industrial_evidence);
    let missing = blockers
        .iter()
        .map(|blocker| blocker.kind)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        missing,
        BTreeSet::from([
            ProductionRequirementKind::Energy,
            ProductionRequirementKind::Maintenance,
            ProductionRequirementKind::Access,
            ProductionRequirementKind::FinanceOrganization,
        ])
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn capacity_cannot_overlap_and_a_consumed_slot_releases_only_after_output_ack() {
    let (mut state, holder, site_id, facility_id) = base_state();
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("ID");
    for suffix in ["one", "two"] {
        let order = work_order(
            &format!("production:order:{suffix}"),
            &holder,
            &process_id,
            &site_id,
        );
        let order_id = order.id.clone();
        state
            .apply_operation(
                &command(
                    &state,
                    &holder,
                    &format!("production:create:{suffix}"),
                    ProductionOperation::CreateWorkOrder { work_order: order },
                ),
                SimTime::EPOCH,
            )
            .expect("work order should be created");
        state
            .apply_operation(
                &command(
                    &state,
                    &holder,
                    &format!("production:authorize:{suffix}"),
                    ProductionOperation::AuthorizeWorkOrder {
                        work_order: order_id,
                    },
                ),
                SimTime::EPOCH,
            )
            .expect("work order should be authorized");
    }
    let evidence = vec![
        evidence(
            ProductionRequirementKind::LaborCapability,
            "customary-hand-milling",
            1,
        ),
        evidence(
            ProductionRequirementKind::Authorization,
            "household-authority",
            1,
        ),
    ];
    let caller_consumed = start_execution(
        &mut state.clone(),
        &holder,
        &facility_id,
        &WorkOrderId::new("production:order:one").expect("ID"),
        "caller-consumed",
        "canwu.production:runtime:caller-consumed",
        evidence.clone(),
        CapacityAllocationState::Consumed,
    )
    .expect_err("callers may not skip the authoritative reservation transition");
    assert!(
        caller_consumed
            .message
            .contains("capacity allocation do not bind exactly")
    );

    let first = start_execution(
        &mut state,
        &holder,
        &facility_id,
        &WorkOrderId::new("production:order:one").expect("ID"),
        "one",
        "canwu.production:runtime:one",
        evidence.clone(),
        CapacityAllocationState::Reserved,
    )
    .expect("first slot should start");
    let started_allocation = state.executions[&first].allocations[0].clone();
    assert_eq!(
        state.capacity_allocations[&started_allocation].state,
        CapacityAllocationState::Consumed,
        "Phase 7 must consume the caller's reserved allocation when work starts"
    );
    let mut overlapping = state.clone();
    let error = start_execution(
        &mut overlapping,
        &holder,
        &facility_id,
        &WorkOrderId::new("production:order:two").expect("ID"),
        "two",
        "canwu.production:runtime:two",
        evidence,
        CapacityAllocationState::Reserved,
    )
    .expect_err("overlapping slot should be rejected");
    assert!(error.message.contains("overlap"));

    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:advance:one",
                ProductionOperation::AdvanceExecution {
                    execution: first.clone(),
                    completed_units: 10,
                },
            ),
            SimTime::from_minutes(10),
        )
        .expect("work should advance");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:complete:one",
                ProductionOperation::CompleteExecution {
                    execution: first.clone(),
                },
            ),
            SimTime::from_minutes(20),
        )
        .expect("work should complete before resource settlement");
    let allocation_id = state.executions[&first].allocations[0].clone();
    assert_eq!(
        state.capacity_allocations[&allocation_id].state,
        CapacityAllocationState::Consumed,
        "capacity remains consumed while output settlement is pending"
    );
    let output_request = state.executions[&first].output_requests[0].clone();
    let production_source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let resource_request =
        ResourceOperationRequestV1::Credit(output_request.resource_credit_request(
            production_source.clone(),
            state.executions[&first].completion_certificate.clone(),
            SimTime::from_minutes(30),
        ));
    let request_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-request.v1", &resource_request)
            .expect("resource output request digest");
    let mut outcome = ResourceOperationOutcome {
        id: ResourceOperationOutcomeId::new(format!(
            "resource:outcome:{}:{}",
            resource_request.operation_key().as_str().replace(':', "-"),
            &request_digest[..16]
        ))
        .expect("resource outcome ID"),
        revision: resource_revision(),
        operation_key: resource_request.operation_key(),
        request_digest,
        kind: ResourceOperationKind::Credit,
        status: ResourceOperationStatus::Applied,
        quantity: output_request.quantity,
        remainder: 0,
        result_ref: None,
        rejection_code: None,
        rejection_reason: None,
        exact_evidence: vec![production_source.clone()],
        semantic_digest: String::new(),
        sequence: 1,
    };
    outcome.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.operation-outcome.v1", &outcome)
            .expect("resource output outcome digest");
    state
        .acknowledge_output(
            &ProductionOutputAcknowledgement {
                execution: first.clone(),
                production_source,
                outcomes: vec![outcome],
            },
            SimTime::from_minutes(30),
        )
        .expect("exact resource outcome should settle production");
    assert_eq!(
        state.executions[&first].lifecycle,
        WorkOrderLifecycle::Settled
    );
    assert_eq!(
        state.capacity_allocations[&allocation_id].state,
        CapacityAllocationState::Released
    );
}

#[test]
fn multiple_outputs_settle_atomically_and_replay_exactly() {
    let (mut production, holder, _site, facility) = base_state();
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("process ID");
    let unit = production.processes[&process_id].outputs[0].unit.clone();
    production
        .processes
        .get_mut(&process_id)
        .expect("process")
        .outputs
        .push(ProductionOutputSpec {
            resource: ResourceDefinitionRevisionId::new("resource:bran:v1")
                .expect("byproduct resource"),
            unit,
            quantity: 2,
            quality_class: "feed-grade".to_owned(),
        });
    let order = work_order(
        "production:order:multiple-outputs",
        &holder,
        &process_id,
        &production.facilities[&facility].site,
    );
    let order_id = order.id.clone();
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:multiple-outputs:create",
                ProductionOperation::CreateWorkOrder { work_order: order },
            ),
            SimTime::EPOCH,
        )
        .expect("create multi-output order");
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:multiple-outputs:authorize",
                ProductionOperation::AuthorizeWorkOrder {
                    work_order: order_id.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize multi-output order");
    let execution = start_execution(
        &mut production,
        &holder,
        &facility,
        &order_id,
        "multiple-outputs",
        PRODUCTION_RUNTIME_ID,
        vec![
            evidence(
                ProductionRequirementKind::LaborCapability,
                "customary-hand-milling",
                1,
            ),
            evidence(
                ProductionRequirementKind::Authorization,
                "household-authority",
                1,
            ),
        ],
        CapacityAllocationState::Reserved,
    )
    .expect("start multi-output execution");
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:multiple-outputs:advance",
                ProductionOperation::AdvanceExecution {
                    execution: execution.clone(),
                    completed_units: 10,
                },
            ),
            SimTime::EPOCH,
        )
        .expect("advance multi-output execution");
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:multiple-outputs:complete",
                ProductionOperation::CompleteExecution {
                    execution: execution.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("complete multi-output work");
    let pending = production.executions[&execution].clone();
    assert_eq!(pending.output_requests.len(), 2);
    let mut resource = resource_state_for_execution(&production, &pending);
    let production_source = version::<ProductionRuntimeRecord>(PRODUCTION_RUNTIME_ID);
    let requests = pending
        .output_requests
        .iter()
        .map(|output| {
            output.resource_credit_request(
                production_source.clone(),
                pending.completion_certificate.clone(),
                SimTime::EPOCH,
            )
        })
        .collect::<Vec<_>>();
    let mut invalid_requests = requests.clone();
    invalid_requests[1].expected_account_revision = ResourceRevision::new(2).expect("revision");
    let before_failed_batch = resource.clone();
    assert!(
        resource
            .apply_production_output_batch(&invalid_requests)
            .is_err()
    );
    assert_eq!(
        resource, before_failed_batch,
        "a failed later output leg must roll back earlier credits and outcomes"
    );

    production.observation_dirty_index.clear();
    production.observation_due_index.clear();
    let mut scenario = scenario_with_production(production);
    scenario
        .domain_records
        .push(resource.into_record().expect("resource output root"));
    let production_plugin = ProductionPlugin;
    let resource_plugin = ResourcePlugin::default();
    let mut canwu = Canwu::new_with_plugins(216, scenario, &[&production_plugin, &resource_plugin])
        .expect("multi-output runtime");
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            PRODUCTION_OBSERVATION_WAKE_INGRESS,
            SimTime::EPOCH,
            serde_json::json!({ "reason": "resume-pending-output" }),
        ))
        .expect("resume pending output settlement");
    for boundary in 0..10 {
        settle_at_epoch(&mut canwu, &format!("multi-output boundary {boundary}"));
        if production_state(&canwu)
            .executions
            .get(&execution)
            .is_some_and(|execution| execution.lifecycle == WorkOrderLifecycle::Settled)
            && canwu_resource::resource_state(&canwu)
                .expect("resource state query")
                .expect("resource state")
                .1
                .external_completion_participants
                .participant(&pending.completion_certificate.acquisition)
                .is_some_and(|participant| {
                    participant.grant.state == CompletionGrantStateV1::Completed
                })
        {
            break;
        }
    }
    let settled = production_state(&canwu);
    assert_eq!(settled.executions[&execution].output_outcomes.len(), 2);
    assert_eq!(
        settled.executions[&execution].lifecycle,
        WorkOrderLifecycle::Settled
    );
    let (_, resources) = canwu_resource::resource_state(&canwu)
        .expect("resource state query")
        .expect("resource state");
    for output in &pending.output_requests {
        assert_eq!(resources.accounts[&output.account].balance, output.quantity);
        assert!(resources.outcomes.contains_key(&output.operation_key));
    }
    let replayed = Canwu::replay_from_journal(
        &[&production_plugin, &resource_plugin],
        &canwu.replay_journal(),
    )
    .expect("multi-output replay");
    assert_eq!(production_state(&replayed), settled);
    assert_eq!(
        canwu_resource::resource_state(&replayed)
            .expect("replayed resource state query")
            .expect("replayed resource state")
            .1,
        resources
    );
}

#[test]
fn admitted_facility_project_pins_its_process_revision_past_effective_until() {
    let (mut state, holder, site, facility) = base_state();
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("process ID");
    state
        .facilities
        .get_mut(&facility)
        .expect("facility")
        .lifecycle = FacilityLifecycle::Planned;
    state
        .processes
        .get_mut(&process_id)
        .expect("process")
        .effective_until = Some(SimTime::from_minutes(1));
    let project = facility_project(
        &mut state,
        &holder,
        &site,
        &facility,
        &process_id,
        "pinned-expiry",
        FacilityProjectKind::Construction,
        SimTime::EPOCH,
    );
    let project_id = project.id.clone();
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:pinned-expiry:create",
                ProductionOperation::CreateFacilityProject { project },
            ),
            SimTime::EPOCH,
        )
        .expect("admit project while its exact process revision is effective");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:pinned-expiry:authorize",
                ProductionOperation::AuthorizeFacilityProject {
                    project: project_id.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize admitted project");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:pinned-expiry:activate",
                ProductionOperation::AdvanceFacilityProject {
                    project: project_id.clone(),
                    completed_units: 1,
                },
            ),
            SimTime::EPOCH,
        )
        .expect("activate admitted project while its certificate is eligible");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:pinned-expiry:advance",
                ProductionOperation::AdvanceFacilityProject {
                    project: project_id.clone(),
                    completed_units: 9,
                },
            ),
            SimTime::from_minutes(2),
        )
        .expect("admitted project remains progressable after process expiry");
    assert_eq!(
        state.facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::Commissioning
    );
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:pinned-expiry:commission",
                ProductionOperation::AcceptFacilityCommissioning {
                    project: project_id.clone(),
                },
            ),
            SimTime::from_minutes(2),
        )
        .expect("commissioning remains progressable after process expiry");
    assert_eq!(
        state.facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::CompletionPending
    );
    assert!(
        !state
            .project_operation_outcome_reservations
            .contains_key(&project_id)
    );
    state.validate().expect("pinned project closure validates");
}

#[test]
fn construction_project_derives_the_commissioned_asset_at_its_certified_time() {
    let (mut state, holder, site_id, facility_id) = base_state();
    let facility = state.facilities.get_mut(&facility_id).expect("facility");
    facility.lifecycle = FacilityLifecycle::Planned;
    facility.condition_per_mille = 400;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("ID");
    let project = facility_project(
        &mut state,
        &holder,
        &site_id,
        &facility_id,
        &process_id,
        "construction",
        FacilityProjectKind::Construction,
        SimTime::from_minutes(40),
    );
    let project_id = project.id.clone();
    let completion_grant = project.production_completion_grant.clone();

    let mut forged_project = project.clone();
    let mut caller_result = state.facilities[&facility_id].clone();
    caller_result.generation = 2;
    caller_result.lifecycle = FacilityLifecycle::Operational;
    caller_result.condition_per_mille = 999;
    forged_project.resulting_asset = Some(caller_result);
    let mut forged_state = state.clone();
    assert!(
        forged_state
            .apply_operation(
                &command(
                    &forged_state,
                    &holder,
                    "production:construction:forged-result",
                    ProductionOperation::CreateFacilityProject {
                        project: forged_project,
                    },
                ),
                SimTime::from_minutes(40),
            )
            .is_err(),
        "construction must reject a caller-authored commissioned asset or condition"
    );

    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:construction:create",
                ProductionOperation::CreateFacilityProject { project },
            ),
            SimTime::from_minutes(40),
        )
        .expect("construction project creation");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:construction:authorize",
                ProductionOperation::AuthorizeFacilityProject {
                    project: project_id.clone(),
                },
            ),
            SimTime::from_minutes(40),
        )
        .expect("construction project authorization");

    let mut late_state = state.clone();
    assert!(
        late_state
            .apply_operation(
                &command(
                    &late_state,
                    &holder,
                    "production:construction:late-advance",
                    ProductionOperation::AdvanceFacilityProject {
                        project: project_id.clone(),
                        completed_units: 10,
                    },
                ),
                SimTime::from_minutes(41),
            )
            .is_err(),
        "project reserve consumption must occur at the certified eligibility time"
    );

    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:construction:advance",
                ProductionOperation::AdvanceFacilityProject {
                    project: project_id.clone(),
                    completed_units: 10,
                },
            ),
            SimTime::from_minutes(40),
        )
        .expect("construction should consume its reserve and finish work");
    assert_eq!(
        state.production_completion_grants[&completion_grant].state,
        CompletionGrantStateV1::Consumed
    );
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:construction:commission",
                ProductionOperation::AcceptFacilityCommissioning {
                    project: project_id.clone(),
                },
            ),
            SimTime::from_minutes(41),
        )
        .expect("construction commissioning");
    state
        .validate()
        .expect("pending construction must remain valid through the plugin candidate check");
    let pending = &state.facility_projects[&project_id];
    assert_eq!(
        pending.lifecycle,
        FacilityProjectLifecycle::CompletionPending
    );
    assert_eq!(state.facilities[&facility_id].generation, 1);
    assert_eq!(
        state.facilities[&facility_id].lifecycle,
        FacilityLifecycle::Planned
    );
    assert_eq!(state.facilities[&facility_id].condition_per_mille, 400);
    assert_eq!(
        state.production_completion_grants[&completion_grant].state,
        CompletionGrantStateV1::Consumed
    );
    assert!(!state.project_archive_due_index.contains(&project_id));
}

#[test]
fn canonical_commissioning_closes_resource_and_local_grants_before_project_archive() {
    let (mut state, holder, site_id, facility_id) = base_state();
    let facility = state.facilities.get_mut(&facility_id).expect("facility");
    facility.lifecycle = FacilityLifecycle::Planned;
    facility.condition_per_mille = 400;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("process");
    let project = facility_project(
        &mut state,
        &holder,
        &site_id,
        &facility_id,
        &process_id,
        "canonical-commissioning",
        FacilityProjectKind::Construction,
        SimTime::EPOCH,
    );
    let project_id = project.id.clone();
    let acquisition = project.completion_certificate.acquisition.clone();
    let local_grant = project.production_completion_grant.clone();
    let resource_grant = project.resource_completion_grant.clone();
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:canonical-project:create",
                ProductionOperation::CreateFacilityProject {
                    project: project.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("create canonical project fixture");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:canonical-project:authorize",
                ProductionOperation::AuthorizeFacilityProject {
                    project: project_id.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize canonical project fixture");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:canonical-project:advance",
                ProductionOperation::AdvanceFacilityProject {
                    project: project_id.clone(),
                    completed_units: 10,
                },
            ),
            SimTime::EPOCH,
        )
        .expect("advance canonical project fixture");
    let resource = resource_state_for_project(&state, &state.facility_projects[&project_id]);
    let mut scenario = scenario_with_production(state);
    scenario
        .domain_records
        .push(resource.into_record().expect("resource project root"));
    let production_plugin = ProductionPlugin;
    let resource_plugin = ResourcePlugin::default();
    let mut canwu = Canwu::new_with_plugins(212, scenario, &[&production_plugin, &resource_plugin])
        .expect("canonical project runtime");
    enqueue_tracked_production_operation(
        &mut canwu,
        &holder,
        1,
        "production:canonical-project:commission",
        ProductionOperation::AcceptFacilityCommissioning {
            project: project_id.clone(),
        },
    );
    for boundary in 0..8 {
        settle_at_epoch(
            &mut canwu,
            &format!("canonical project boundary {boundary}"),
        );
        if production_state(&canwu)
            .facility_projects
            .get(&project_id)
            .is_some_and(|project| project.lifecycle == FacilityProjectLifecycle::Completed)
        {
            break;
        }
    }

    let completed = production_state(&canwu);
    let completed_project = &completed.facility_projects[&project_id];
    assert_eq!(
        completed_project.lifecycle,
        FacilityProjectLifecycle::Completed
    );
    assert_eq!(completed.facilities[&facility_id].generation, 2);
    assert_eq!(
        completed.production_completion_grants[&local_grant].state,
        CompletionGrantStateV1::Completed
    );
    assert_eq!(
        completed.completion_participant_grants[&acquisition][canwu_resource::PLUGIN_NAME]
            .grant
            .state,
        CompletionGrantStateV1::Completed
    );
    assert_eq!(
        completed.completion_acquisitions[&acquisition].state,
        canwu_resource::CompletionLeaseAcquisitionStateV1::Released
    );
    assert!(completed.project_archive_due_index.contains(&project_id));
    let (_, authoritative_resource) = canwu_resource::resource_state(&canwu)
        .expect("resource query")
        .expect("resource state");
    assert_eq!(
        authoritative_resource
            .external_completion_participants
            .participant(&acquisition)
            .expect("completed resource participant")
            .grant
            .state,
        CompletionGrantStateV1::Completed
    );
    assert!(
        !authoritative_resource
            .external_completion_participants
            .target_locks
            .values()
            .any(|grant| grant == &resource_grant)
    );
    let commission_outcome = completed
        .operation_outcomes
        .values()
        .find(|outcome| outcome.project.as_ref() == Some(&project_id))
        .expect("canonical project outcome");
    assert_eq!(
        commission_outcome.disposition,
        ProductionOperationDisposition::Applied
    );
    let commission_outcome_id = commission_outcome.id.clone();
    let authentic_project_archive = completed
        .prepare_production_archive(1)
        .expect("authentic project archive fixture");
    let forged_result_store = TestProductionArchiveStore::default();
    let forged_result_directory = store_rehashed_forged_project_archive(
        &authentic_project_archive,
        &forged_result_store,
        |record| record.resulting_asset.condition_per_mille = 999,
    );
    assert!(
        authenticate_production_archive_directory(&forged_result_store, &forged_result_directory,)
            .is_err(),
        "rehashing every container must not authenticate a forged project result"
    );
    let forged_grant_store = TestProductionArchiveStore::default();
    let forged_grant_directory = store_rehashed_forged_project_archive(
        &authentic_project_archive,
        &forged_grant_store,
        |record| {
            record.production_completion_grant.state = CompletionGrantStateV1::Consumed;
        },
    );
    assert!(
        authenticate_production_archive_directory(&forged_grant_store, &forged_grant_directory)
            .is_err(),
        "rehashing every container must not authenticate incomplete completion closure"
    );
    let mut under_cap_pressure = completed.clone();
    let maximum_outcomes = ProductionLimitsV1::canonical().max_operation_outcomes;
    for index in 0..maximum_outcomes.saturating_sub(under_cap_pressure.operation_outcomes.len()) {
        let id = ProductionOperationOutcomeId::new(format!(
            "production:unrelated-cap-outcome:{index:05}"
        ))
        .expect("cap outcome ID");
        under_cap_pressure
            .operation_outcomes
            .insert(id.clone(), unrelated_outcome(id, &holder));
    }
    under_cap_pressure
        .validate()
        .expect("the exact operation-outcome cap remains valid");
    let prepared = under_cap_pressure
        .prepare_production_archive(1)
        .expect("project archive remains progressable under outcome cap pressure");
    assert_eq!(prepared.selected_projects, vec![project_id.clone()]);
    assert_eq!(prepared.blob.project_records[0].operation_outcomes.len(), 1);
    assert_eq!(
        prepared.blob.project_records[0].operation_outcomes[0].id,
        commission_outcome_id
    );
    let store = TestProductionArchiveStore::default();
    let archive_plugin = ProductionPlugin;
    let mut archive_runtime = Canwu::new_with_plugins(
        213,
        scenario_with_production(under_cap_pressure),
        &[&archive_plugin],
    )
    .expect("project archive runtime");
    enqueue_production_archive(&mut archive_runtime, &prepared, &store)
        .expect("verified canonical project archive");
    archive_runtime
        .step_canonical()
        .expect("project archive commit")
        .expect("project archive boundary");
    let archived = production_state(&archive_runtime);
    assert!(!archived.facility_projects.contains_key(&project_id));
    assert!(
        !archived
            .operation_outcomes
            .contains_key(&commission_outcome_id)
    );
    validate_production_archive(&store, &archived).expect("project archive provider authenticates");
}

#[test]
fn facility_generation_has_one_active_project_and_cannot_retire_while_owned() {
    let (mut state, holder, site_id, facility_id) = base_state();
    state
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .lifecycle = FacilityLifecycle::Planned;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("process");
    let project = facility_project(
        &mut state,
        &holder,
        &site_id,
        &facility_id,
        &process_id,
        "exclusive-project",
        FacilityProjectKind::Construction,
        SimTime::EPOCH,
    );
    let mut conflicting = project.clone();
    conflicting.id =
        FacilityProjectId::new("production:project:conflicting").expect("conflicting project ID");
    state
        .apply_operation(
            &command(
                &state,
                &holder,
                "production:exclusive-project:create",
                ProductionOperation::CreateFacilityProject { project },
            ),
            SimTime::EPOCH,
        )
        .expect("first project owns the generation");
    assert!(
        state
            .clone()
            .apply_operation(
                &command(
                    &state,
                    &holder,
                    "production:conflicting-project:create",
                    ProductionOperation::CreateFacilityProject {
                        project: conflicting,
                    },
                ),
                SimTime::EPOCH,
            )
            .is_err(),
        "a second nonterminal project cannot own the same facility generation"
    );
    assert!(
        state
            .clone()
            .apply_operation(
                &command(
                    &state,
                    &holder,
                    "production:exclusive-project:retire",
                    ProductionOperation::RetireFacility {
                        facility: facility_id,
                        expected_generation: 1,
                    },
                ),
                SimTime::EPOCH,
            )
            .is_err(),
        "retirement cannot strand a nonterminal project on its base generation"
    );
}

#[test]
fn degraded_facility_ticket_forks_into_continue_repair_or_defer_outcomes() {
    let (mut state, holder, site_id, facility_id) = base_state();
    state
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .lifecycle = FacilityLifecycle::Degraded;
    state
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .condition_per_mille = 600;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("ID");
    let mut order = work_order("production:order:degraded", &holder, &process_id, &site_id);
    order.lifecycle = WorkOrderLifecycle::Authorized;
    let order_id = order.id.clone();
    state.work_orders.insert(order.id.clone(), order);
    let ticket_id = canwu_api::DecisionTicketId::new(1);

    let resolve = |mut branch: ProductionState, choice| {
        branch
            .apply_operation(
                &command(
                    &branch,
                    &holder,
                    &format!("production:choice:{choice:?}"),
                    ProductionOperation::ResolveDegradedFacility {
                        work_order: order_id.clone(),
                        facility: facility_id.clone(),
                        choice,
                        decision_ticket: ticket_id,
                    },
                ),
                SimTime::EPOCH,
            )
            .expect("choice should apply");
        branch
    };
    let continued = resolve(state.clone(), DegradedFacilityChoice::ContinueDegraded);
    let mut repaired = resolve(state.clone(), DegradedFacilityChoice::StopForRepair);
    let deferred = resolve(state, DegradedFacilityChoice::DeferOrder);
    assert_eq!(continued.facilities[&facility_id].condition_per_mille, 550);
    assert_eq!(
        continued.work_orders[&order_id].lifecycle,
        WorkOrderLifecycle::Ready
    );
    assert_eq!(
        repaired.facilities[&facility_id].lifecycle,
        FacilityLifecycle::Repairing
    );
    let project = facility_project(
        &mut repaired,
        &holder,
        &site_id,
        &facility_id,
        &process_id,
        "repair",
        FacilityProjectKind::Repair,
        SimTime::from_minutes(60),
    );
    let project_id = project.id.clone();
    repaired
        .apply_operation(
            &command(
                &repaired,
                &holder,
                "production:repair:create",
                ProductionOperation::CreateFacilityProject { project },
            ),
            SimTime::from_minutes(60),
        )
        .expect("repair project should bind its certified authority");
    repaired
        .apply_operation(
            &command(
                &repaired,
                &holder,
                "production:repair:authorize",
                ProductionOperation::AuthorizeFacilityProject {
                    project: project_id.clone(),
                },
            ),
            SimTime::from_minutes(60),
        )
        .expect("repair project authorization");
    repaired
        .apply_operation(
            &command(
                &repaired,
                &holder,
                "production:repair:advance",
                ProductionOperation::AdvanceFacilityProject {
                    project: project_id.clone(),
                    completed_units: 10,
                },
            ),
            SimTime::from_minutes(60),
        )
        .expect("repair should consume its completion reserve and finish work");
    repaired
        .apply_operation(
            &command(
                &repaired,
                &holder,
                "production:repair:commission",
                ProductionOperation::AcceptFacilityCommissioning {
                    project: project_id.clone(),
                },
            ),
            SimTime::from_minutes(61),
        )
        .expect("repair should commission its authoritative derived result");
    assert_eq!(repaired.facilities[&facility_id].generation, 1);
    assert_eq!(
        repaired.facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::CompletionPending
    );
    assert!(!repaired.project_archive_due_index.contains(&project_id));
    assert_eq!(
        deferred.work_orders[&order_id].lifecycle,
        WorkOrderLifecycle::Authorized
    );
}

#[test]
fn persisted_decision_tickets_fork_trace_and_replay_all_degraded_choices() {
    let (mut state, holder, site_id, facility_id) = base_state();
    let facility = state.facilities.get_mut(&facility_id).expect("facility");
    facility.lifecycle = FacilityLifecycle::Degraded;
    facility.condition_per_mille = 600;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("ID");
    let mut order = work_order(
        "production:order:persisted-decision",
        &holder,
        &process_id,
        &site_id,
    );
    order.lifecycle = WorkOrderLifecycle::Authorized;
    let order_id = order.id.clone();
    state.work_orders.insert(order.id.clone(), order);
    state
        .materialize_observation_head(
            &ProductionObservationHeadKeyV1 {
                holder: holder.clone(),
                scope: site_id,
            },
            SimTime::EPOCH,
            EvidenceRef::DomainRecordVersion(version::<ProductionRuntimeRecord>(
                PRODUCTION_RUNTIME_ID,
            )),
        )
        .expect("decision observation cut");

    let plugin = ProductionPlugin;
    let mut canwu = Canwu::new_with_plugins(91, scenario_with_production(state), &[&plugin])
        .expect("production decision runtime");
    let ticket_id = canwu_api::DecisionTicketId::new(1);
    let controller = DecisionControllerBinding::new(
        "workshop-controller",
        DecisionPolicyIdentity::new(DecisionPolicyKind::Utility, "workshop-choice", "1"),
        DecisionAuthority::Actor {
            actor: PersonId::new(1),
        },
    );
    let mut ticket = degraded_facility_decision_ticket(
        &canwu,
        ticket_id,
        "workshop-controller",
        &holder,
        &order_id,
        &facility_id,
        None,
    )
    .expect("degraded facility decision ticket");
    for option in &mut ticket.options {
        for dimension in ["continue", "repair", "defer"] {
            option.utility_inputs.insert(
                dimension.to_owned(),
                i64::from(
                    (option.id == "continue_degraded" && dimension == "continue")
                        || (option.id == "stop_for_repair" && dimension == "repair")
                        || (option.id == "defer_order" && dimension == "defer"),
                ) * 100,
            );
        }
    }
    for (request_id, mutation) in [
        (
            DecisionRequestId::new(1),
            DecisionMutation::RegisterController { controller },
        ),
        (DecisionRequestId::new(2), DecisionMutation::Open { ticket }),
    ] {
        canwu
            .enqueue_decision(
                canwu.time(),
                0,
                DecisionIngressRequest::new(request_id, canwu.revision(), mutation),
            )
            .expect("decision ingress");
    }
    canwu
        .step_canonical()
        .expect("decision intake")
        .expect("decision intake boundary");

    for (dimension, expected_choice) in [
        ("continue", DegradedFacilityChoice::ContinueDegraded),
        ("repair", DegradedFacilityChoice::StopForRepair),
        ("defer", DegradedFacilityChoice::DeferOrder),
    ] {
        let mut branch = canwu.fork();
        let policy = WeightedUtilityPolicy::new(
            "workshop-choice",
            "1",
            UtilityProfile {
                weights: BTreeMap::from([(dimension.to_owned(), 1)]),
            },
        );
        assert!(matches!(
            branch
                .drive_decision(
                    branch.time(),
                    0,
                    DecisionRequestId::new(3),
                    Some(CommandRequestId::new(1)),
                    ticket_id,
                    &policy,
                )
                .expect("decision preparation"),
            DecisionEvaluation::Prepared(_)
        ));
        branch
            .step_canonical()
            .expect("decision resolution")
            .expect("decision resolution boundary");
        branch
            .step_canonical()
            .expect("production command ingress")
            .expect("production command boundary");
        let branch_state = production_state(&branch);
        let receipt = branch_state
            .decision_receipts
            .get(&ticket_id)
            .unwrap_or_else(|| {
                panic!(
                    "production decision receipt; attempts={:#?}; outcomes={:#?}",
                    branch.command_attempts(),
                    branch_state.operation_outcomes
                )
            });
        assert!(receipt.command_attempt_id.is_some());
        let trace = branch
            .decision_trace(receipt.trace_id)
            .expect("persisted decision trace");
        assert_eq!(trace.ticket_id, ticket_id);
        match expected_choice {
            DegradedFacilityChoice::ContinueDegraded => {
                assert_eq!(
                    branch_state.work_orders[&order_id].lifecycle,
                    WorkOrderLifecycle::Ready
                );
                assert_eq!(
                    branch_state.facilities[&facility_id].condition_per_mille,
                    550
                );
            }
            DegradedFacilityChoice::StopForRepair => assert_eq!(
                branch_state.facilities[&facility_id].lifecycle,
                FacilityLifecycle::Repairing
            ),
            DegradedFacilityChoice::DeferOrder => assert_eq!(
                branch_state.work_orders[&order_id].lifecycle,
                WorkOrderLifecycle::Authorized
            ),
        }
        let snapshot = branch.snapshot_json().expect("decision snapshot");
        assert_eq!(
            production_state(
                &from_production_snapshot_json(&snapshot, &[&plugin]).expect("decision restore"),
            ),
            branch_state
        );
        assert_eq!(
            production_state(
                &replay_production_from_journal(&[&plugin], &branch.replay_journal())
                    .expect("decision replay"),
            ),
            branch_state
        );
    }
}

#[test]
fn holder_relative_reports_use_persisted_cuts_instead_of_backdating_current_truth() {
    let (mut state, operator, site_id, facility_id) = base_state();
    let remote = KnowledgeHolderRef::Person(PersonId::new(2));
    let remote_grant_id =
        ProductionObserverGrantId::new("production:observer-grant:remote").expect("grant ID");
    state.observer_grants.insert(
        remote_grant_id.clone(),
        ProductionObserverGrant {
            id: remote_grant_id,
            holder: remote.clone(),
            sites: BTreeSet::from([site_id.clone()]),
            role: ProductionObservationRole::RemoteOwner,
            delay_minutes: 60,
        },
    );
    state
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .condition_per_mille = 900;
    state
        .materialize_observation_head(
            &ProductionObservationHeadKeyV1 {
                holder: remote.clone(),
                scope: site_id.clone(),
            },
            SimTime::from_minutes(60),
            EvidenceRef::DomainRecordVersion(version::<ProductionRuntimeRecord>(
                PRODUCTION_RUNTIME_ID,
            )),
        )
        .expect("remote historical cut");
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("ID");
    let mut order = work_order(
        "production:order:reported",
        &operator,
        &process_id,
        &site_id,
    );
    order.lifecycle = WorkOrderLifecycle::Running;
    order.quantity = 7;
    state.work_orders.insert(order.id.clone(), order);
    state
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .condition_per_mille = 300;
    state
        .materialize_observation_head(
            &ProductionObservationHeadKeyV1 {
                holder: operator.clone(),
                scope: site_id.clone(),
            },
            SimTime::from_minutes(120),
            EvidenceRef::DomainRecordVersion(version::<ProductionRuntimeRecord>(
                PRODUCTION_RUNTIME_ID,
            )),
        )
        .expect("operator current cut");
    let now = SimTime::from_minutes(120);
    let operator_report =
        production_report_from_state(&state, &operator, &site_id, now).expect("operator report");
    let remote_report =
        production_report_from_state(&state, &remote, &site_id, now).expect("remote report");
    let operator_order = operator_report
        .facts
        .iter()
        .find(|fact| fact.subject == "production:order:reported")
        .expect("operator order fact");
    assert_eq!(operator_order.quantity_low, 7);
    assert!(
        remote_report
            .facts
            .iter()
            .all(|fact| fact.subject != "production:order:reported"),
        "the delayed remote cut must not acquire an order created after it was observed"
    );
    assert_eq!(remote_report.observed_at, SimTime::from_minutes(60));
    let operator_facility = operator_report
        .facts
        .iter()
        .find(|fact| fact.subject == facility_id.as_str())
        .expect("operator facility fact");
    let remote_facility = remote_report
        .facts
        .iter()
        .find(|fact| fact.subject == facility_id.as_str())
        .expect("remote facility fact");
    assert_eq!(operator_facility.quantity_low, 300);
    assert_eq!(remote_facility.quantity_low, 800);
    assert_eq!(remote_facility.quantity_high, 1_000);
    assert_ne!(
        operator_report.canonical_digest,
        remote_report.canonical_digest
    );
}

#[test]
fn snapshot_restore_and_exact_replay_reproduce_reports_and_reject_a_forged_root() {
    let (mut state, holder, site_id, _) = base_state();
    state
        .materialize_observation_head(
            &ProductionObservationHeadKeyV1 {
                holder: holder.clone(),
                scope: site_id.clone(),
            },
            SimTime::EPOCH,
            EvidenceRef::DomainRecordVersion(version::<ProductionRuntimeRecord>(
                PRODUCTION_RUNTIME_ID,
            )),
        )
        .expect("initial observation cut");
    state.validate().expect("fixture should validate");
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
    let scenario = Scenario {
        start_time: SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![state.into_initial_record().expect("production root")],
    };
    let plugin = ProductionPlugin;
    let canwu = Canwu::new_with_plugins(41, scenario, &[&plugin])
        .expect("production runtime should initialize");
    let expected = production_report(&canwu, &holder, &site_id).expect("production report");
    let snapshot = canwu.snapshot_json().expect("snapshot should encode");
    let restored = from_production_snapshot_json(&snapshot, &[&plugin])
        .expect("production snapshot should restore exactly");
    let replayed = replay_production_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("production journal should replay exactly");
    assert_eq!(
        production_report(&restored, &holder, &site_id).expect("restored report"),
        expected
    );
    assert_eq!(
        production_report(&replayed, &holder, &site_id).expect("replayed report"),
        expected
    );
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );

    let mut forged: serde_json::Value =
        serde_json::from_str(&snapshot).expect("snapshot JSON should decode");
    assert!(tamper_observer_grant_key(&mut forged));
    let forged = serde_json::to_string(&forged).expect("forged snapshot should encode");
    assert!(
        from_production_snapshot_json(&forged, &[&plugin]).is_err(),
        "restore must reject a forged but syntactically valid production root"
    );
}

#[test]
fn phase_ten_only_stages_and_phase_eleven_atomically_commits_incidents() {
    let (mut state, _, _, facility_id) = base_state();
    let facility = state.facilities.get_mut(&facility_id).expect("facility");
    facility.incident_risk_per_mille = 1_000;
    facility.incident_max_severity_per_mille = 200;
    let plugin = ProductionPlugin;
    let mut canwu = Canwu::new_with_plugins(73, scenario_with_production(state), &[&plugin])
        .expect("production runtime");

    let descriptor = canwu
        .plugin_descriptors()
        .find(|descriptor| descriptor.name == PLUGIN_NAME)
        .expect("production descriptor");
    let phase_ten = descriptor
        .boundary_systems
        .iter()
        .find(|system| system.phase == BoundaryPhase::HistoricalCandidateEvaluation)
        .expect("incident evaluation system");
    let phase_eleven = descriptor
        .boundary_systems
        .iter()
        .find(|system| system.phase == BoundaryPhase::ConditionalTransitionCommit)
        .expect("incident conditional commit system");
    assert_eq!(
        phase_ten.random_streams,
        vec![production_incident_random_stream()]
    );
    assert_eq!(
        phase_ten.writes.len(),
        1,
        "phase 10 stages one owned mutation"
    );
    assert!(
        phase_eleven.writes.is_empty(),
        "the kernel, not an ordinary package writer, owns the Phase 11 atomic commit"
    );

    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            PRODUCTION_OBSERVATION_WAKE_INGRESS,
            canwu.time(),
            serde_json::json!({}),
        ))
        .expect("incident-driving ingress");
    canwu
        .step_canonical()
        .expect("incident boundary")
        .expect("incident work");
    let changed = production_state(&canwu);
    let receipt = changed
        .incident_receipts
        .values()
        .next()
        .expect("incident receipt");
    assert_eq!(receipt.random.stream, production_incident_random_stream());
    let RandomDrawAddress::OperationV1(trigger_address) = &receipt.random.trigger.address else {
        panic!("trigger draw must be operation addressed");
    };
    let RandomDrawAddress::OperationV1(severity_address) =
        &receipt.random.severity.as_ref().expect("severity").address
    else {
        panic!("severity draw must be operation addressed");
    };
    assert_eq!(trigger_address.draw_slot, 0);
    assert_eq!(severity_address.draw_slot, 1);
    assert_eq!(
        trigger_address.application_operation_id,
        receipt.operation_key
    );
    assert_eq!(
        severity_address.application_operation_id,
        receipt.operation_key
    );
    assert_eq!(canwu.random_draws().len(), 2);
    assert!(changed.facilities[&facility_id].condition_per_mille < 1_000);

    let snapshot = canwu.snapshot_json().expect("incident snapshot");
    let restored =
        from_production_snapshot_json(&snapshot, &[&plugin]).expect("incident snapshot restores");
    let replayed = replay_production_from_journal(&[&plugin], &canwu.replay_journal())
        .expect("incident journal replays");
    assert_eq!(production_state(&restored), changed);
    assert_eq!(production_state(&replayed), changed);

    let mut forged: serde_json::Value =
        serde_json::from_str(&snapshot).expect("snapshot JSON should decode");
    assert!(tamper_incident_random_value(&mut forged));
    assert!(
        from_production_snapshot_json(
            &serde_json::to_string(&forged).expect("forged snapshot"),
            &[&plugin],
        )
        .is_err(),
        "restore must reject a tampered incident draw"
    );
}

#[test]
fn active_production_continuation_resolves_compacted_resource_evidence_from_archive() {
    let (production, _, _, _) = base_state();
    let process = &production.processes
        [&ProcessRevisionId::new("production:household-process:v1").expect("process ID")];
    let mut input = resource_input(process, "compacted-continuation");
    canonicalize_archived_resource_input(&mut input);
    let mut resource = ResourceState::empty(ResourceLimitsV1::canonical()).expect("resource state");
    resource.allocation_legs.insert(
        input.allocation_leg.id.clone(),
        ResourceAllocationLeg {
            id: input.allocation_leg.id.clone(),
            revision: input.allocation_leg.revision,
            demand: ResourceDemandId::new("resource:compacted-demand").expect("demand ID"),
            demand_revision: resource_revision(),
            reservation: ResourceReservationId::new("resource:compacted-reservation")
                .expect("reservation ID"),
            account: input.allocation_leg.account.clone(),
            account_revision: input.allocation_leg.account_revision,
            resource_revision: input.allocation_leg.resource_revision.clone(),
            unit_revision: input.allocation_leg.unit_revision.clone(),
            quantity: input.allocation_leg.quantity,
            status: AllocationLegStatus::Consumed,
            priority: 0,
            due_at: SimTime::EPOCH,
            tie_break: ResourceTieBreakKey::new("resource:compacted-tie").expect("tie break"),
            admitted_sequence: 1,
            operation_key: ResourceOperationKey::new("resource:compacted-allocation")
                .expect("allocation operation"),
            semantic_digest: input.allocation_leg.semantic_digest.clone(),
        },
    );
    let (consumption, outcome) = archived_resource_input_payloads(&input);
    let records = vec![
        canwu_resource::ResourceTerminalArchiveRecordV1 {
            key: canwu_resource::ResourceTerminalRecordKeyV1::Consumption(
                input.consumption.id.clone(),
            ),
            operation_key: input.consumption_outcome.operation_key.clone(),
            quantity: input.quantity,
            remainder: 0,
            exact_evidence: vec![input.consumption.consumer_evidence.clone()],
            semantic_digest: input.consumption.semantic_digest.clone(),
            terminal_sequence: 1,
            payload: canwu_resource::ResourceTerminalArchivePayloadV1::Consumption(consumption),
        },
        canwu_resource::ResourceTerminalArchiveRecordV1 {
            key: canwu_resource::ResourceTerminalRecordKeyV1::Outcome(
                input.consumption_outcome.operation_key.clone(),
            ),
            operation_key: input.consumption_outcome.operation_key.clone(),
            quantity: input.quantity,
            remainder: 0,
            exact_evidence: outcome.exact_evidence.clone(),
            semantic_digest: input.consumption_outcome.semantic_digest.clone(),
            terminal_sequence: 2,
            payload: canwu_resource::ResourceTerminalArchivePayloadV1::Outcome(outcome),
        },
    ];
    let mut blob = canwu_resource::ResourceArchiveBlobV1 {
        format_version: 1,
        expected_source_root: "c".repeat(64),
        records: records.clone(),
        content_id: String::new(),
    };
    blob.content_id = canwu_resource::canonical_digest("canwu.resource.archive-blob.v1", &blob)
        .expect("resource blob digest");
    let mut membership = canwu_resource::ResourceArchiveMembershipPageV1 {
        id: String::new(),
        memberships: records
            .iter()
            .enumerate()
            .map(
                |(ordinal, record)| canwu_resource::ResourceArchiveMembershipV1 {
                    key: record.key.clone(),
                    blob_id: blob.content_id.clone(),
                    ordinal: u16::try_from(ordinal).expect("ordinal"),
                    terminal_sequence: record.terminal_sequence,
                    semantic_digest: record.semantic_digest.clone(),
                },
            )
            .collect(),
        semantic_digest: String::new(),
    };
    membership.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-membership-page.v1", &membership)
            .expect("membership digest");
    membership.id = membership.semantic_digest.clone();
    let mut temporal = canwu_resource::ResourceArchiveTemporalPageV1 {
        id: String::new(),
        entries: records
            .iter()
            .map(|record| canwu_resource::ResourceArchiveTemporalEntryV1 {
                terminal_sequence: record.terminal_sequence,
                key: record.key.clone(),
            })
            .collect(),
        semantic_digest: String::new(),
    };
    temporal.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-temporal-page.v1", &temporal)
            .expect("temporal digest");
    temporal.id = temporal.semantic_digest.clone();
    let mut directory = canwu_resource::ResourceArchiveIndexDirectoryV1 {
        id: String::new(),
        previous_root: None,
        membership_pages: vec![membership.id.clone()],
        temporal_pages: vec![temporal.id.clone()],
        blob_ids: vec![blob.content_id.clone()],
        archived_record_count: 2,
        semantic_digest: String::new(),
    };
    directory.semantic_digest =
        canwu_resource::canonical_digest("canwu.resource.archive-directory.v1", &directory)
            .expect("directory digest");
    directory.id = directory.semantic_digest.clone();
    let store = TestProductionArchiveStore::default();
    for (namespace, object_id, bytes) in [
        (
            canwu_resource::RESOURCE_ARCHIVE_BLOB_NAMESPACE,
            blob.content_id.as_str(),
            serde_json::to_vec(&blob).expect("blob bytes"),
        ),
        (
            canwu_resource::RESOURCE_ARCHIVE_MEMBERSHIP_PAGE_NAMESPACE,
            membership.id.as_str(),
            serde_json::to_vec(&membership).expect("membership bytes"),
        ),
        (
            canwu_resource::RESOURCE_ARCHIVE_TEMPORAL_PAGE_NAMESPACE,
            temporal.id.as_str(),
            serde_json::to_vec(&temporal).expect("temporal bytes"),
        ),
        (
            canwu_resource::RESOURCE_ARCHIVE_INDEX_DIRECTORY_NAMESPACE,
            directory.id.as_str(),
            serde_json::to_vec(&directory).expect("directory bytes"),
        ),
    ] {
        store
            .store_resource_archive_object(namespace, object_id, &bytes)
            .expect("store resource archive object");
    }
    resource.archive_head.directory_root = Some(directory.id);
    resource.archive_head.archived_record_count = 2;
    validate_production_resource_continuation(&resource, &store, &input)
        .expect("archive-backed continuation should resolve exact resource evidence");
    store.tamper_first_resource_blob();
    assert!(
        validate_production_resource_continuation(&resource, &store, &input).is_err(),
        "continuation restore must reject tampered archived resource evidence"
    );
}

#[test]
fn restored_active_project_advances_through_canonical_ingress_with_resource_archive() {
    let (mut production, holder, site_id, facility_id) = base_state();
    production
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .lifecycle = FacilityLifecycle::Planned;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("process");
    production
        .processes
        .get_mut(&process_id)
        .expect("process")
        .requirements
        .clear();
    let mut project = facility_project(
        &mut production,
        &holder,
        &site_id,
        &facility_id,
        &process_id,
        "canonical-archived-continuation",
        FacilityProjectKind::Construction,
        SimTime::EPOCH,
    );
    project.evidence.clear();
    canonicalize_archived_resource_input(&mut project.inputs[0]);

    let technique_spec_id = "technology:canonical-project-spec";
    let technique_revision_id = "technology:canonical-project-revision";
    let qualification_id = "technology:canonical-project-qualification";
    let technique_spec = canwu_technology::TechnologyCatalogRecord::Technique(
        canwu_technology::TechniqueSpecPayload {
            label: "Canonical archived continuation".to_owned(),
            function: "facility construction".to_owned(),
            requirements: Vec::new(),
            qualification_rules: Vec::new(),
        },
    )
    .into_initial_record(technique_spec_id)
    .expect("technique spec record");
    let technique_revision = canwu_technology::TechnologyCatalogRecord::Revision(
        canwu_technology::TechniqueRevisionPayload {
            label: "Canonical archived continuation revision".to_owned(),
            spec: version::<canwu_technology::TechniqueSpec>(technique_spec_id),
            parents: Vec::new(),
            parameters: Vec::new(),
            evaluator: "canwu.test.evaluator.v1".to_owned(),
            produced_by: None,
            execution_intent: None,
            discovery_evidence: Vec::new(),
        },
    )
    .into_initial_record(technique_revision_id)
    .expect("technique revision record");
    let mut qualification_payload =
        serde_json::to_value(canwu_technology::CapabilityQualificationPayload {
            holder: holder.clone(),
            operator: Some(EntityRef::Person(PersonId::new(1))),
            site: EntityRef::Territory(TerritoryId::new(1)),
            revision: version::<canwu_technology::TechniqueRevision>(technique_revision_id),
            operation: "facility-construction".to_owned(),
            reliability_per_mille: 1_000,
            attempts: Vec::new(),
            last_practiced_at: SimTime::EPOCH,
            valid_from: SimTime::EPOCH,
            valid_until: None,
            active: true,
        })
        .expect("qualification payload");
    qualification_payload
        .as_object_mut()
        .expect("qualification object")
        .insert(
            canwu_api::PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            serde_json::to_value(canwu_api::PayloadRequiredEvidenceContinuationV1::completed())
                .expect("qualification continuation"),
        );
    let qualification = canwu_api::DomainRecord {
        reference: TypedDomainRecordRef::<canwu_technology::CapabilityQualification>::new(
            qualification_id,
        )
        .into_untyped(),
        owner: canwu_technology::PLUGIN_NAME.to_owned(),
        class: canwu_api::DomainRecordClass::Record,
        version: 1,
        lifecycle: canwu_api::DomainRecordLifecycle::Active,
        payload: qualification_payload,
        references: vec![canwu_api::DomainReference {
            role: "domain".to_owned(),
            target: canwu_api::DomainReferenceTarget::Domain(
                version::<canwu_technology::TechniqueRevision>(technique_revision_id).record,
            ),
        }],
    };
    project.technology = TechnologyEvidenceBinding {
        technique_revision: version::<canwu_technology::TechniqueRevision>(technique_revision_id),
        capability_qualification: Some(version::<canwu_technology::CapabilityQualification>(
            qualification_id,
        )),
        implementation: None,
        adoption: None,
        semantic_digest: canwu_api::canonical_hash(
            "canwu.production.technology-binding.v1",
            &vec![technique_revision.clone(), qualification.clone()],
        )
        .expect("technology binding digest"),
    };
    let project_id = project.id.clone();
    let acquisition = project.completion_certificate.acquisition.clone();
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:archived-continuation:create",
                ProductionOperation::CreateFacilityProject {
                    project: project.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("create archived continuation project");
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:archived-continuation:authorize",
                ProductionOperation::AuthorizeFacilityProject {
                    project: project_id.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize archived continuation project");
    let reserved_outcomes =
        usize::try_from(production.project_operation_outcome_reservations[&project_id])
            .expect("project outcome reservation fits usize");
    let maximum_outcomes = ProductionLimitsV1::canonical().max_operation_outcomes;
    for index in
        0..maximum_outcomes.saturating_sub(production.operation_outcomes.len() + reserved_outcomes)
    {
        let id = ProductionOperationOutcomeId::new(format!(
            "production:active-project-cap-outcome:{index:05}"
        ))
        .expect("cap outcome ID");
        production
            .operation_outcomes
            .insert(id.clone(), unrelated_outcome(id, &holder));
    }
    assert_eq!(
        production.operation_outcomes.len() + reserved_outcomes,
        maximum_outcomes
    );
    production
        .validate()
        .expect("accepted project owns its remaining command outcome capacity");

    let mut resource = resource_state_for_project(&production, &project);
    let store = TestProductionArchiveStore::default();
    archive_resource_input(&mut resource, &store, &project.inputs[0]);
    resource
        .validate()
        .expect("archive-backed resource fixture");
    let mut scenario = scenario_with_production(production);
    scenario.domain_records.extend([
        resource.into_record().expect("resource archive root"),
        technique_spec,
        technique_revision,
        qualification,
    ]);
    let production_plugin = ProductionPlugin;
    let resource_plugin = ResourcePlugin::default();
    let technology_plugin = canwu_technology::TechnologyPlugin;
    let canwu = Canwu::new_with_plugins(
        214,
        scenario,
        &[&production_plugin, &resource_plugin, &technology_plugin],
    )
    .expect("archive-backed canonical runtime");
    let snapshot = canwu.snapshot_json().expect("archive-backed snapshot");
    let mut restored = from_production_snapshot_json_with_archives(
        &snapshot,
        &[&production_plugin, &resource_plugin, &technology_plugin],
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("archive-backed project restore");
    enqueue_tracked_production_operation(
        &mut restored,
        &holder,
        99,
        "production:active-project-cap-unrelated",
        ProductionOperation::RetireFacility {
            facility: facility_id.clone(),
            expected_generation: 1,
        },
    );
    settle_at_epoch(&mut restored, "unrelated cap-pressure rejection boundary");
    let canwu_api::CommandAttemptOutcome::Rejected { error } = &restored
        .command_attempts()
        .last()
        .expect("command attempt")
        .outcome
    else {
        panic!("unrelated operation must not consume accepted-project capacity");
    };
    assert_eq!(error.code, canwu_api::ErrorCode::ValueOutOfRange);
    assert!(error.message.contains("archive_backpressure"));
    enqueue_production_resource_continuation(&mut restored, &project_id, &store)
        .expect("authenticated continuation witness ingress");
    settle_at_epoch(&mut restored, "continuation witness boundary");
    assert!(
        production_state(&restored)
            .resource_continuation_witnesses
            .contains_key(&project_id)
    );
    let historical_witness =
        production_state(&restored).resource_continuation_witnesses[&project_id].clone();
    let (_, resource_before_later_archive) = canwu_resource::resource_state(&restored)
        .expect("resource state query before later archive")
        .expect("resource state before later archive");
    let later_resource_archive = resource_before_later_archive
        .prepare_resource_archive(1)
        .expect("unrelated later resource archive batch");
    canwu_resource::enqueue_resource_archive(&mut restored, &later_resource_archive, &store)
        .expect("enqueue unrelated later resource archive");
    settle_at_epoch(&mut restored, "unrelated later resource archive boundary");
    let (_, resource_after_later_archive) = canwu_resource::resource_state(&restored)
        .expect("resource state query after later archive")
        .expect("resource state after later archive");
    assert_ne!(
        resource_after_later_archive
            .archive_head
            .directory_root
            .as_deref(),
        Some(historical_witness.resource_archive_directory_root.as_str())
    );
    canwu_resource::authenticate_reachable_resource_archive_directory(
        &resource_after_later_archive,
        &store,
        &historical_witness.resource_archive_directory_root,
        historical_witness.resource_archive_record_count,
    )
    .expect("historical continuation root remains reachable");
    let later_archive_snapshot = restored
        .snapshot_json()
        .expect("snapshot with later unrelated resource archive");
    restored = from_production_snapshot_json_with_archives(
        &later_archive_snapshot,
        &[&production_plugin, &resource_plugin, &technology_plugin],
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("historical continuation witness restores under a later resource archive head");

    enqueue_tracked_production_operation(
        &mut restored,
        &holder,
        1,
        "production:archived-continuation:advance",
        ProductionOperation::AdvanceFacilityProject {
            project: project_id.clone(),
            completed_units: 10,
        },
    );
    settle_at_epoch(&mut restored, "archived continuation command boundary");
    settle_at_epoch(&mut restored, "archived continuation apply boundary");
    let advanced = production_state(&restored);
    assert_eq!(
        advanced.facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::Commissioning
    );
    assert_eq!(
        advanced.project_operation_outcome_reservations[&project_id],
        1
    );
    let outcome = advanced
        .operation_outcomes
        .values()
        .find(|outcome| outcome.project.as_ref() == Some(&project_id))
        .expect("canonical continuation outcome");
    assert_eq!(outcome.disposition, ProductionOperationDisposition::Applied);

    enqueue_tracked_production_operation(
        &mut restored,
        &holder,
        2,
        "production:archived-continuation:commission",
        ProductionOperation::AcceptFacilityCommissioning {
            project: project_id.clone(),
        },
    );
    for boundary in 0..8 {
        settle_at_epoch(
            &mut restored,
            &format!("archived continuation completion boundary {boundary}"),
        );
        if production_state(&restored)
            .facility_projects
            .get(&project_id)
            .is_some_and(|project| project.lifecycle == FacilityProjectLifecycle::Completed)
        {
            break;
        }
    }
    let completed = production_state(&restored);
    assert_eq!(
        completed.facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::Completed
    );
    assert!(
        !completed
            .project_operation_outcome_reservations
            .contains_key(&project_id)
    );
    assert!(
        !completed
            .resource_continuation_witnesses
            .contains_key(&project_id)
    );
    let completed_snapshot = restored
        .snapshot_json()
        .expect("completed hot project snapshot");
    let restored_completed = from_production_snapshot_json_with_archives(
        &completed_snapshot,
        &[&production_plugin, &resource_plugin, &technology_plugin],
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("completed hot project restores before archive");
    assert_eq!(
        production_state(&restored_completed).facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::Completed
    );

    let participant_archive_key =
        canwu_resource::ResourceTerminalRecordKeyV1::ExternalCompletionParticipant(
            acquisition.clone(),
        );
    for boundary in 0..4 {
        let (_, resource) = canwu_resource::resource_state(&restored)
            .expect("resource query while publishing completion report")
            .expect("resource state while publishing completion report");
        if resource
            .terminal_archive_candidates
            .values()
            .any(|candidate| candidate == &participant_archive_key)
        {
            break;
        }
        settle_at_epoch(
            &mut restored,
            &format!("completion report publication boundary {boundary}"),
        );
    }
    let (_, resource_before_participant_archive) = canwu_resource::resource_state(&restored)
        .expect("resource query before participant archive")
        .expect("resource state before participant archive");
    assert!(
        resource_before_participant_archive
            .terminal_archive_candidates
            .values()
            .any(|candidate| candidate == &participant_archive_key),
        "completed participant becomes archiveable only after its mandatory report is published"
    );
    let participant_archive = resource_before_participant_archive
        .prepare_resource_archive(
            resource_before_participant_archive
                .terminal_archive_candidates
                .len(),
        )
        .expect("prepare completed resource participant archive");
    assert!(
        participant_archive
            .selected
            .contains(&participant_archive_key)
    );
    canwu_resource::enqueue_resource_archive(&mut restored, &participant_archive, &store)
        .expect("enqueue completed resource participant archive");
    settle_at_epoch(
        &mut restored,
        "completed resource participant archive boundary",
    );
    let (_, resource_after_participant_archive) = canwu_resource::resource_state(&restored)
        .expect("resource query after participant archive")
        .expect("resource state after participant archive");
    assert!(
        resource_after_participant_archive
            .external_completion_participants
            .participant(&acquisition)
            .is_none(),
        "completed participant must leave hot authoritative capacity after archive commit"
    );
    let archived_participant_snapshot = restored
        .snapshot_json()
        .expect("snapshot with archived resource participant");
    let restored_archived_participant = from_production_snapshot_json_with_archives(
        &archived_participant_snapshot,
        &[&production_plugin, &resource_plugin, &technology_plugin],
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("completed project restores from the authenticated resource participant archive");
    assert_eq!(
        production_state(&restored_archived_participant).facility_projects[&project_id].lifecycle,
        FacilityProjectLifecycle::Completed
    );

    store.tamper_resource_blob_for_directory(&historical_witness.resource_archive_directory_root);
    assert!(
        from_production_snapshot_json_with_archives(
            &snapshot,
            &[&production_plugin, &resource_plugin, &technology_plugin],
            Rc::new(store.clone()),
            Rc::new(store.clone()),
        )
        .is_err(),
        "restored continuation must reject a tampered resource archive provider"
    );
}

#[test]
fn reserved_project_capacity_rejects_repeated_invalid_commands_without_using_outcomes() {
    let (mut production, holder, site_id, facility_id) = base_state();
    production
        .facilities
        .get_mut(&facility_id)
        .expect("facility")
        .lifecycle = FacilityLifecycle::Planned;
    let process_id = ProcessRevisionId::new("production:household-process:v1").expect("process");
    let project = facility_project(
        &mut production,
        &holder,
        &site_id,
        &facility_id,
        &process_id,
        "repeated-cap-rejection",
        FacilityProjectKind::Construction,
        SimTime::EPOCH,
    );
    let project_id = project.id.clone();
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:repeated-cap-rejection:create",
                ProductionOperation::CreateFacilityProject { project },
            ),
            SimTime::EPOCH,
        )
        .expect("create project");
    production
        .apply_operation(
            &command(
                &production,
                &holder,
                "production:repeated-cap-rejection:authorize",
                ProductionOperation::AuthorizeFacilityProject {
                    project: project_id.clone(),
                },
            ),
            SimTime::EPOCH,
        )
        .expect("authorize project");
    let reserved = usize::try_from(production.project_operation_outcome_reservations[&project_id])
        .expect("reserved outcome count");
    let limit = ProductionLimitsV1::canonical().max_operation_outcomes;
    for index in 0..limit.saturating_sub(reserved) {
        let id = ProductionOperationOutcomeId::new(format!(
            "production:repeated-cap-rejection:unrelated:{index:05}"
        ))
        .expect("outcome ID");
        production
            .operation_outcomes
            .insert(id.clone(), unrelated_outcome(id, &holder));
    }
    production
        .validate()
        .expect("project owns the remaining outcome capacity");
    let initial_outcomes = production.operation_outcomes.clone();
    let initial_reservation = production.project_operation_outcome_reservations.clone();
    let plugin = ProductionPlugin;
    let mut canwu = Canwu::new_with_plugins(215, scenario_with_production(production), &[&plugin])
        .expect("cap-pressure runtime");

    for request_id in 1..=2 {
        let state = production_state(&canwu);
        let outcome = canwu
            .process_command(CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PersonId::new(1)),
                    Command::Plugin {
                        plugin: canwu_production::PLUGIN_NAME.to_owned(),
                        command: PRODUCTION_COMMAND.to_owned(),
                        payload: serde_json::to_value(ProductionCommandEnvelope {
                            operation_id: ProductionOperationOutcomeId::new(format!(
                                "production:repeated-cap-rejection:attempt:{request_id}"
                            ))
                            .expect("attempt outcome ID"),
                            holder: holder.clone(),
                            expected_runtime_revision: state.revision,
                            operation: ProductionOperation::AuthorizeFacilityProject {
                                project: project_id.clone(),
                            },
                        })
                        .expect("command payload"),
                    },
                )
                .at_time(SimTime::EPOCH),
            ))
            .expect("invalid project command returns a structured command outcome");
        let canwu_api::CommandOutcome::Rejected { rejection } = outcome else {
            panic!("already-authorized project command must be rejected");
        };
        assert_eq!(rejection.error.code, canwu_api::ErrorCode::InvalidPayload);
        let after = production_state(&canwu);
        assert_eq!(after.operation_outcomes, initial_outcomes);
        assert_eq!(
            after.project_operation_outcome_reservations,
            initial_reservation
        );
    }
}

#[test]
fn archive_commit_survives_restart_and_authenticates_damage_waste_and_output_evidence() {
    let (mut state, execution) = settled_state_for_archive();
    let holder = state.work_orders[&state.executions[&execution].work_order]
        .holder
        .clone();
    let command = ProductionCommandEnvelope {
        operation_id: ProductionOperationOutcomeId::new(
            "production:archive:authoritative-complete",
        )
        .expect("archive command outcome ID"),
        holder,
        expected_runtime_revision: state.revision.saturating_sub(1),
        operation: ProductionOperation::CompleteExecution {
            execution: execution.clone(),
        },
    };
    let command_hash = canwu_api::canonical_hash("canwu.production.operation-input.v1", &command)
        .expect("archive command input hash");
    state.operation_outcomes.insert(
        command.operation_id.clone(),
        ProductionOperationOutcome {
            id: command.operation_id.clone(),
            canonical_input_hash: command_hash,
            command,
            disposition: ProductionOperationDisposition::Applied,
            work_order: None,
            execution: Some(execution.clone()),
            project: None,
            rejection_code: None,
            rejection_message: None,
            settled_at: SimTime::from_minutes(20),
        },
    );
    state.validate().expect("archive command origin validates");
    let prepared = state
        .prepare_production_archive(1)
        .expect("bounded archive preparation");
    assert_eq!(prepared.selected, vec![execution.clone()]);
    assert_eq!(prepared.blob.records[0].non_recoverable_waste_quantity, 3);
    assert_eq!(prepared.blob.records[0].recoverable_input_quantity, 7);
    assert_eq!(prepared.blob.records[0].output_outcome_digests.len(), 1);

    let forged_store = TestProductionArchiveStore::default();
    let forged_directory =
        store_rehashed_forged_execution_archive(&prepared, &forged_store, |record| {
            record.work_in_progress_record.completed_units -= 1;
        });
    assert!(
        authenticate_production_archive_directory(&forged_store, &forged_directory).is_err(),
        "rehashing every archive container must not authenticate a semantically forged execution"
    );

    let forged_command_store = TestProductionArchiveStore::default();
    let forged_command_directory =
        store_rehashed_forged_execution_archive(&prepared, &forged_command_store, |record| {
            record.operation_outcomes[0].canonical_input_hash = "f".repeat(64);
        });
    assert!(
        authenticate_production_archive_directory(
            &forged_command_store,
            &forged_command_directory,
        )
        .is_err(),
        "rehashing the record, blob, pages, and directory must not detach an archived outcome from its canonical command"
    );

    let store = TestProductionArchiveStore::default();
    let plugin = ProductionPlugin;
    let mut canwu = Canwu::new_with_plugins(121, scenario_with_production(state), &[&plugin])
        .expect("archive runtime");
    let receipt = enqueue_production_archive(&mut canwu, &prepared, &store)
        .expect("verified archive ingress");
    canwu
        .step_canonical()
        .expect("archive commit")
        .expect("archive boundary");
    let committed = production_state(&canwu);
    assert!(!committed.executions.contains_key(&execution));
    assert!(committed.work_in_progress.is_empty());
    assert!(committed.capacity_allocations.is_empty());
    assert!(committed.operation_outcomes.is_empty());
    assert!(committed.production_completion_grants.is_empty());
    assert!(committed.production_completion_certificates.is_empty());
    assert_eq!(committed.archive.archived_execution_count, 1);
    assert_eq!(committed.archive.pending_handles.len(), 1);
    validate_production_archive(&store, &committed).expect("archive head validates");

    let restart_snapshot = canwu.snapshot_json().expect("archive restart snapshot");
    assert!(
        from_production_snapshot_json(&restart_snapshot, &[&plugin]).is_err(),
        "archive-bearing restore must not skip provider authentication"
    );
    let mut restarted = from_production_snapshot_json_with_archives(
        &restart_snapshot,
        &[&plugin],
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("archive phase restart");
    assert_eq!(production_state(&restarted), committed);
    finalize_production_archive_retention(&mut restarted, &store, &receipt)
        .expect("archive retention acknowledgement");
    restarted
        .step_canonical()
        .expect("archive acknowledgement")
        .expect("archive acknowledgement boundary");
    let acknowledged = production_state(&restarted);
    assert!(acknowledged.archive.pending_handles.is_empty());
    assert_eq!(acknowledged.archive.maintenance_receipts.len(), 1);
    validate_production_archive(&store, &acknowledged).expect("acknowledged archive validates");

    let (mut chained, _) = settled_state_for_archive();
    chained.archive = acknowledged.archive.clone();
    let prepared_second = chained
        .prepare_production_archive(1)
        .expect("second archive batch");
    let second_commit = prepared_second
        .store_and_verify(&store)
        .expect("second archive verification");
    let second_root = second_commit.directory_root.clone();
    let mut pending = second_commit.retention;
    pending.phase = ProductionArchiveRetentionPhaseV1::Committed;
    pending.semantic_digest.clear();
    pending.semantic_digest =
        canwu_api::canonical_hash("canwu.production.archive-retention.v1", &pending)
            .expect("pending retention digest");
    chained.archive.directory_root = Some(second_root.clone());
    chained.archive.archived_execution_count += 1;
    chained.archive.committed_batch_count += 1;
    chained
        .archive
        .pending_handles
        .insert(pending.handle_id.clone(), pending);
    validate_production_archive(&store, &chained)
        .expect("archive provider should authenticate head, prior chain, and pending objects");
    store.sever_production_archive_prior_chain(&second_root);
    assert!(
        validate_production_archive(&store, &chained).is_err(),
        "restore must reject an archive head whose prior chain was severed"
    );

    let checkpoint = restarted
        .checkpoint_journal()
        .expect("archive checkpoint journal");
    let checkpoint_restored = from_production_checkpoint_journal_with_archives(
        checkpoint,
        &[&plugin],
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("archive checkpoint restore");
    let replayed = replay_production_from_journal_with_archives(
        &[&plugin],
        &restarted.replay_journal(),
        Rc::new(store.clone()),
        Rc::new(store.clone()),
    )
    .expect("archive journal replay");
    assert_eq!(production_state(&checkpoint_restored), acknowledged);
    assert_eq!(production_state(&replayed), acknowledged);

    store.tamper_production_blob_for_directory(
        acknowledged
            .archive
            .directory_root
            .as_deref()
            .expect("acknowledged archive root"),
    );
    assert!(
        validate_production_archive(&store, &acknowledged).is_err(),
        "archive validation must reject a tampered damage/waste terminal record"
    );
}

#[test]
fn holder_knowledge_cap_rejects_one_more_report_with_bounded_work() {
    let (mut state, holder, first_site, first_facility) = base_state();
    let site_limit = ProductionLimitsV1::canonical().max_reports_per_boundary;
    let mut sites = BTreeSet::from([first_site.clone()]);
    {
        let facility = state
            .facilities
            .get_mut(&first_facility)
            .expect("first cap facility");
        facility.incident_risk_per_mille = 1_000;
        facility.incident_max_severity_per_mille = 1;
    }
    for index in 1..site_limit {
        let site_id =
            ProductionSiteId::new(format!("production:cap-site:{index}")).expect("cap site ID");
        let facility_id = FacilityAssetId::new(format!("production:cap-facility:{index}"))
            .expect("cap facility ID");
        sites.insert(site_id.clone());
        state.sites.insert(
            site_id.clone(),
            ProductionSite {
                id: site_id.clone(),
                holder: holder.clone(),
                place: EntityRef::Territory(TerritoryId::new(1)),
                form: ProductionSiteForm::DistributedWorkshop,
                active: true,
            },
        );
        state.facilities.insert(
            facility_id.clone(),
            FacilityAsset {
                id: facility_id,
                site: site_id,
                generation: 1,
                lifecycle: FacilityLifecycle::Operational,
                condition_per_mille: 1_000,
                capacity: BTreeMap::new(),
                maintenance_evidence: Vec::new(),
                operational_stage_capacity_per_mille: 0,
                incident_risk_per_mille: 1_000,
                incident_max_severity_per_mille: 1,
            },
        );
    }
    state
        .observer_grants
        .values_mut()
        .next()
        .expect("cap observer grant")
        .sites = sites;
    let limit = ProductionLimitsV1::canonical().max_observation_records_per_holder;
    let plugin = ProductionPlugin;
    let mut canwu = Canwu::new_with_plugins(151, scenario_with_production(state), &[&plugin])
        .expect("knowledge cap runtime");
    for round in 0..17 {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                PRODUCTION_OBSERVATION_WAKE_INGRESS,
                canwu.time(),
                serde_json::json!({ "round": round }),
            ))
            .expect("knowledge cap wake");
        canwu
            .step_canonical()
            .expect("knowledge cap boundary")
            .expect("knowledge cap work");
    }
    assert_eq!(
        canwu
            .knowledge()
            .records
            .get(&holder)
            .expect("holder knowledge")
            .len(),
        limit,
        "the cap must reject rather than append a report"
    );
    assert!(canwu.events().iter().any(|event| {
        event.kind.plugin_identity()
            == Some((PLUGIN_NAME, "canwu.production.report_capacity_rejected.v1"))
    }));
    validate_production_runtime(&canwu).expect("knowledge cap remains restorable");
    let checkpoint = canwu
        .checkpoint_journal()
        .expect("knowledge cap checkpoint");
    let restored = from_production_checkpoint_journal(checkpoint, &[&plugin])
        .expect("knowledge cap checkpoint restore");
    assert_eq!(restored.snapshot(), canwu.snapshot());
}

fn tamper_observer_grant_key(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            if let Some(serde_json::Value::Object(grants)) = object.get_mut("observer_grants")
                && let Some(first) = grants.keys().next().cloned()
                && let Some(grant) = grants.remove(&first)
            {
                grants.insert("production:observer-grant:forged".to_owned(), grant);
                return true;
            }
            object.values_mut().any(tamper_observer_grant_key)
        }
        serde_json::Value::Array(values) => values.iter_mut().any(tamper_observer_grant_key),
        _ => false,
    }
}

fn tamper_incident_random_value(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            if object.contains_key("incident_receipts")
                && let Some(serde_json::Value::Object(receipts)) =
                    object.get_mut("incident_receipts")
                && let Some(serde_json::Value::Object(receipt)) = receipts.values_mut().next()
                && let Some(serde_json::Value::Object(random)) = receipt.get_mut("random")
                && let Some(serde_json::Value::Object(trigger)) = random.get_mut("trigger")
                && let Some(value) = trigger.get_mut("value")
            {
                *value = serde_json::json!(999);
                return true;
            }
            object.values_mut().any(tamper_incident_random_value)
        }
        serde_json::Value::Array(values) => values.iter_mut().any(tamper_incident_random_value),
        _ => false,
    }
}
