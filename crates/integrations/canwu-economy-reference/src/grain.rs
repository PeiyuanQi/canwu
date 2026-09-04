//! A deterministic fourteen-month grain composition running on the real Canwu kernel.
//!
//! The harness intentionally drives every mutable operation through tracked command or
//! plugin ingress.  Initial scenario records contain only starting domain state; allocation,
//! completion leases, transport progress, consumption, harvest credit, force acknowledgement,
//! and monthly closing are all persisted by their owning plugins.

use crate::{
    DeliveryDispositionV1, EconomyCommandV1, EconomyDeliveryAttemptId, EconomyDeliveryAttemptV1,
    EconomyObservationGrantId, EconomyObservationGrantV1, EconomyOperationId, EconomyOperationV1,
    EconomyProfileId, EconomyProfileV1, EconomyReferencePlugin, EconomyReferenceStateV1,
    EconomyRouteObservationId, EconomyRouteObservationV1, EconomyRouteProviderPayloadV1,
    EconomyRouteProviderRecordId, EconomyRuleRevisionId, GrainDecision, LocalEconomyId,
    LocalEconomyV1, MonthlyEconomyEvidenceV1, MonthlyEconomyFrameV1,
    PopulationConsumptionProfileV1, PriceEvidenceApplicabilityV1, ProjectionProviderRegistryV1,
    ProjectionQueryResultV1, ProjectionScopeBindingV1, ReliefPolicyV1,
    RequisitionPolicyV1 as EconomyRequisitionPolicyV1, ResourceProjectionSourceV1,
    SeasonalHarvestProfileV1, economy_command, economy_reference_runtime_reference,
    economy_reference_state, economy_route_provider_reference,
};
use canwu_api::{
    BoundaryRequest, Canwu, CanwuError, Command, CommandAuthority, CommandEnvelope, CommandRequest,
    CommandRequestId, DecisionAction, DecisionAuthority, DecisionContext,
    DecisionControllerBinding, DecisionIngressRequest, DecisionMutation, DecisionOption,
    DecisionPolicyIdentity, DecisionPolicyKind, DecisionRequestId, DecisionTicketDraft,
    DecisionTicketId, DomainRecordRef, DomainRecordVersionRef, EntityRef, EvidenceRef, Issuer,
    KnowledgeHolderRef, PersonId, PluginIngressRequest, PolicyDecision, Scenario, SimDuration,
    SimTime, SimulationPlugin, SystemCadence, canonical_hash,
};
use canwu_economy_reference_content::{compile_content_pack, synthetic_grain_fixture};
use canwu_force_supply_reference::{
    DueRequirementStateV1, FORCE_RESOURCE_OUTCOME_INGRESS, FORCE_SUPPLY_COMMAND,
    ForceAcceptedTransferEvidenceV1, ForceCommandEnvelopeV1, ForceCompletionOperationV1,
    ForceConsumptionIntent, ForceConsumptionIntentId, ForceConsumptionIntentStatus,
    ForceObservationRole, ForceObserverGrantId, ForceObserverGrantV1, ForceOperationId,
    ForceOperationV1, ForceStockCustodyBindingV1, ForceSupplyReferencePlugin,
    ForceSupplyRuntimeRecord, ForceSupplyStateV1, ReferenceForce, ReferenceForceId,
    ResourceOutcomePacketV1, SupplyResourceKind, enqueue_force_archive,
    finalize_force_archive_retention, force_supply_command, force_supply_runtime_reference,
};
use canwu_resource::{
    ActivateCompletionLeaseV1, AllocationLegStatus, CompleteExternalCompletionParticipantGrantV1,
    CompletionCapacityGrantId, CompletionCapacityPartitionV1, CompletionCapacityRecipeV1,
    CompletionLeaseAcquisitionId, CompletionLeaseActivationCertificateV1, CompletionLockedTargetV1,
    CompletionPolicyClassV1, ConsumeExternalCompletionParticipantGrantV1, DemandStatus,
    EligibilityEnvelopeV1, GrantCompletionCapacityV1, PartialFulfillmentPolicy,
    PrepareCompletionCapacityV1, PrepareExternalCompletionParticipantGrantV1,
    RequestCompletionLeaseV1, RequestExternalCompletionParticipantGrantV1, ResourceAccount,
    ResourceAccountId, ResourceAdapterOperationV1, ResourceAllocationLegVersionV1,
    ResourceAllocationObservationV1, ResourceAllocationRequestV1, ResourceArchiveRetentionHandleV1,
    ResourceArchiveRetentionPhaseV1, ResourceArchiveStore, ResourceCommandV1,
    ResourceConsumptionId, ResourceConsumptionObservationV1, ResourceConsumptionRequestV1,
    ResourceConsumptionVersionV1, ResourceCreditRequestV1, ResourceCreditSourceV1,
    ResourceDefinitionId, ResourceDefinitionRevision, ResourceDemand, ResourceDemandId,
    ResourceDemandObservationV1, ResourceError, ResourceFulfillmentObservationV1, ResourceLimitsV1,
    ResourceObservationHeadId, ResourceObservationHeadV1, ResourceOperationKey,
    ResourceOperationOutcomeVersionV1, ResourceOperationRequestV1, ResourceOperationStatus,
    ResourcePlugin, ResourceQualityId, ResourceReportGrantId, ResourceReportGrantV1,
    ResourceRevision, ResourceScopeId, ResourceState, ResourceStockObservationV1,
    ResourceSubmitDemandRequestV1, ResourceTieBreakKey, ResourceTransferDispositionRequestV1,
    ResourceTransferDispositionV1, ResourceTransferId, ResourceTransferObservationV1,
    ResourceTransferProgressRequestV1, ResourceTransferStartRequestV1,
    ResourceTransportAcceptanceV1, ResourceUnitRevision, RunBudgetRevisionV1, TransferProgressV1,
    TransportExecutionLink, enqueue_resource_adapter_operation, enqueue_resource_allocation,
    enqueue_resource_archive, enqueue_resource_completion_operation,
    finalize_resource_archive_retention, resource_command, resource_operation_outcome,
    resource_state,
};
use canwu_routing::{
    PlanningSnapshot, RoutingConnection, RoutingConnectionRef, RoutingEndpoint,
    RoutingEndpointKind, RoutingNetwork, RoutingNodeRef, RoutingPolicy, RoutingRequest,
    TransferMode, TraversalModel, plan_route,
};
use canwu_transport::{
    CapacityBooking, CapacityBookingId, CapacityBookingStatus, Handoff, HandoffId,
    ItineraryRevision, ItineraryRevisionId, ItineraryRevisionReason, ReconciliationOutcome,
    TransportExecution, TransportExecutionId,
};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::rc::Rc;

const MONTH: SimDuration = SimDuration::days(30);
const DAY: SimDuration = SimDuration::days(1);
const SHIPMENT_QUANTITY: u64 = 1_200;
const CIVILIAN_NEED: u64 = 320;
const RELIEF_TARGET: u64 = 120;
const HARVEST_MONTH: u16 = 10;
const HARVEST_BASE: u64 = 5_000;
const OPENING_GRAIN: u64 = 4_300;
const ARCHIVE_TRIGGER: usize = 96;
const ARCHIVE_BATCH: usize = 64;

const NS_TRANSFER_BEGIN: &str = "canwu.economy-reference.grain.transfer.begin";
const NS_TRANSFER_ACCEPT: &str = "canwu.economy-reference.grain.transfer.accept";
const FORCE_COMPLETION_NAMESPACE: &str = "canwu.force-supply-reference:requisition";
const NS_CIVILIAN_CONSUME: &str = "canwu.economy-reference.grain.consume.civilian";
const NS_RELIEF_CONSUME: &str = "canwu.economy-reference.grain.consume.relief";
const NS_HARVEST_CREDIT: &str = "canwu.economy-reference.grain.credit.harvest";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GrainLoopSummary {
    pub frames: Vec<MonthlyEconomyFrameV1>,
    pub final_stock: u64,
    pub final_population_wellbeing_per_mille: u16,
    pub final_force_readiness_per_mille: u16,
    pub final_cooperation_per_mille: u16,
    pub total_harvest: u64,
    pub transport_executions: usize,
    pub closed_route_months: Vec<u16>,
    pub rerouted_months: Vec<u16>,
    pub conservation_closing: u128,
    pub checkpoint_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrainLoopError {
    Canwu(String),
    Resource(String),
    Routing(String),
    Transport(String),
    Missing(String),
    Rejected(String),
    ArithmeticOverflow,
}

impl Display for GrainLoopError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Canwu(message)
            | Self::Resource(message)
            | Self::Routing(message)
            | Self::Transport(message)
            | Self::Missing(message)
            | Self::Rejected(message) => formatter.write_str(message),
            Self::ArithmeticOverflow => formatter.write_str("grain harness arithmetic overflowed"),
        }
    }
}

impl std::error::Error for GrainLoopError {}

impl From<CanwuError> for GrainLoopError {
    fn from(value: CanwuError) -> Self {
        Self::Canwu(value.to_string())
    }
}

impl From<ResourceError> for GrainLoopError {
    fn from(value: ResourceError) -> Self {
        Self::Resource(value.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GrainArchiveObjectV1 {
    namespace: String,
    object_id: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct GrainArchiveBundleV1 {
    objects: Vec<GrainArchiveObjectV1>,
    retention: BTreeMap<String, ResourceArchiveRetentionHandleV1>,
    package_retention:
        BTreeMap<String, canwu_force_supply_reference::PackageArchiveRetentionHandleV1>,
    available: bool,
}

#[derive(Clone)]
struct GrainArchiveStore {
    objects: RefCell<BTreeMap<(String, String), Vec<u8>>>,
    retention: RefCell<BTreeMap<String, ResourceArchiveRetentionHandleV1>>,
    package_retention:
        RefCell<BTreeMap<String, canwu_force_supply_reference::PackageArchiveRetentionHandleV1>>,
    available: RefCell<bool>,
}

impl Default for GrainArchiveStore {
    fn default() -> Self {
        Self {
            objects: RefCell::new(BTreeMap::new()),
            retention: RefCell::new(BTreeMap::new()),
            package_retention: RefCell::new(BTreeMap::new()),
            available: RefCell::new(true),
        }
    }
}

impl GrainArchiveStore {
    fn bundle(&self) -> GrainArchiveBundleV1 {
        GrainArchiveBundleV1 {
            objects: self
                .objects
                .borrow()
                .iter()
                .map(|((namespace, object_id), bytes)| GrainArchiveObjectV1 {
                    namespace: namespace.clone(),
                    object_id: object_id.clone(),
                    bytes: bytes.clone(),
                })
                .collect(),
            retention: self.retention.borrow().clone(),
            package_retention: self.package_retention.borrow().clone(),
            available: *self.available.borrow(),
        }
    }

    fn from_bundle(bundle: GrainArchiveBundleV1) -> Self {
        Self {
            objects: RefCell::new(
                bundle
                    .objects
                    .into_iter()
                    .map(|object| ((object.namespace, object.object_id), object.bytes))
                    .collect(),
            ),
            retention: RefCell::new(bundle.retention),
            package_retention: RefCell::new(bundle.package_retention),
            available: RefCell::new(bundle.available),
        }
    }

    fn require_available(&self) -> Result<(), ResourceError> {
        if *self.available.borrow() {
            Ok(())
        } else {
            Err(ResourceError::Capacity(
                "grain archive storage is unavailable".to_owned(),
            ))
        }
    }
}

impl canwu_force_supply_reference::PackageArchiveStore for GrainArchiveStore {
    fn store_package_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), CanwuError> {
        self.require_available().map_err(|error| {
            CanwuError::new(canwu_api::ErrorCode::InvalidArchive, error.to_string())
        })?;
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_package_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        self.require_available().map_err(|error| {
            CanwuError::new(canwu_api::ErrorCode::InvalidArchive, error.to_string())
        })?;
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_package_archive_retention(
        &self,
        handle: &canwu_force_supply_reference::PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        self.package_retention
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn load_package_archive_retention(
        &self,
        handle_id: &str,
    ) -> Result<Option<canwu_force_supply_reference::PackageArchiveRetentionHandleV1>, CanwuError>
    {
        Ok(self.package_retention.borrow().get(handle_id).cloned())
    }

    fn finalize_package_archive_retention(
        &self,
        finalized: &canwu_force_supply_reference::PackageArchiveRetentionHandleV1,
    ) -> Result<(), CanwuError> {
        let mut handles = self.package_retention.borrow_mut();
        let handle = handles.get_mut(&finalized.id).ok_or_else(|| {
            CanwuError::new(
                canwu_api::ErrorCode::InvalidArchive,
                "package archive retention handle is unavailable",
            )
        })?;
        if handle.expected_source_root != finalized.expected_source_root
            || handle.directory_root != finalized.directory_root
            || handle.object_ids != finalized.object_ids
        {
            return Err(CanwuError::new(
                canwu_api::ErrorCode::InvalidArchive,
                "package archive finalized a different retention closure",
            ));
        }
        *handle = finalized.clone();
        Ok(())
    }
}

impl canwu_api::PluginArchiveObjectProvider for GrainArchiveStore {
    fn load_plugin_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, CanwuError> {
        canwu_force_supply_reference::PackageArchiveStore::load_package_archive_object(
            self, namespace, object_id,
        )
    }
}

impl ResourceArchiveStore for GrainArchiveStore {
    fn store_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
        bytes: &[u8],
    ) -> Result<(), ResourceError> {
        self.require_available()?;
        self.objects
            .borrow_mut()
            .insert((namespace.to_owned(), object_id.to_owned()), bytes.to_vec());
        Ok(())
    }

    fn load_resource_archive_object(
        &self,
        namespace: &str,
        object_id: &str,
    ) -> Result<Option<Vec<u8>>, ResourceError> {
        self.require_available()?;
        Ok(self
            .objects
            .borrow()
            .get(&(namespace.to_owned(), object_id.to_owned()))
            .cloned())
    }

    fn persist_resource_archive_retention(
        &self,
        handle: &ResourceArchiveRetentionHandleV1,
    ) -> Result<(), ResourceError> {
        self.require_available()?;
        self.retention
            .borrow_mut()
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn finalize_resource_archive_retention(
        &self,
        handle_id: &str,
        phase: ResourceArchiveRetentionPhaseV1,
    ) -> Result<(), ResourceError> {
        self.require_available()?;
        let mut retention = self.retention.borrow_mut();
        let handle = retention.get_mut(handle_id).ok_or_else(|| {
            ResourceError::NotFound("grain archive retention handle is unavailable".to_owned())
        })?;
        handle.phase = phase;
        Ok(())
    }
}

/// Real Canwu-backed grain integration harness.
pub struct GrainHarness {
    canwu: Canwu,
    archive_store: Rc<GrainArchiveStore>,
    economy: LocalEconomyId,
    force: ReferenceForceId,
    grain_revision: canwu_resource::ResourceDefinitionRevisionId,
    unit_revision: canwu_resource::ResourceUnitRevisionId,
    granary_account: ResourceAccountId,
    army_account: ResourceAccountId,
    scope: ResourceScopeId,
}

#[derive(Debug, Deserialize, Serialize)]
struct GrainHarnessSnapshotV1 {
    canwu_snapshot_json: String,
    archive: GrainArchiveBundleV1,
}

#[derive(Debug, Deserialize, Serialize)]
struct GrainHarnessJournalV1 {
    canwu_replay_journal_json: String,
    archive: GrainArchiveBundleV1,
}

/// Backward-compatible public example name.
pub type SyntheticGrainLoop = GrainHarness;

impl GrainHarness {
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Result<Self, GrainLoopError> {
        let compiled = compile_content_pack(&synthetic_grain_fixture())?;
        let mut force_state = ForceSupplyStateV1::from_compiled_content(compiled.clone())?;
        let food_profile = force_state
            .profiles
            .values()
            .find(|profile| {
                profile
                    .requirements
                    .iter()
                    .any(|requirement| requirement.kind == SupplyResourceKind::Food)
            })
            .cloned()
            .ok_or_else(|| GrainLoopError::Missing("compiled food force profile".to_owned()))?;
        let food = food_profile
            .requirements
            .iter()
            .find(|requirement| requirement.kind == SupplyResourceKind::Food)
            .cloned()
            .ok_or_else(|| GrainLoopError::Missing("compiled food requirement".to_owned()))?;

        let manager = holder(1);
        let force_holder = holder(2);
        let scope = ResourceScopeId::new("canwu.economy-reference:scope:river-granary")?;
        let granary_account = ResourceAccountId::new("canwu.economy-reference:account:z-granary")?;
        let army_account = ResourceAccountId::new("canwu.economy-reference:account:a-field-force")?;

        let mut resources = ResourceState::empty(ResourceLimitsV1::canonical())?;
        resources.install_run_budget(completion_budget(&manager, &force_holder)?)?;
        resources.install_unit(ResourceUnitRevision {
            id: food.unit_revision.clone(),
            revision: ResourceRevision::INITIAL,
            symbol: "synthetic-basket".to_owned(),
            scale_numerator: 1,
            scale_denominator: 1,
            semantic_digest: digest_label("grain unit"),
        })?;
        resources.install_definition(ResourceDefinitionRevision {
            id: food.resource_revision.clone(),
            resource: ResourceDefinitionId::new("canwu.economy-reference:resource:grain")?,
            revision: ResourceRevision::INITIAL,
            canonical_unit: food.unit_revision.clone(),
            quality: ResourceQualityId::new("canwu.economy-reference:quality:staple")?,
            scope: scope.clone(),
            effective_from: SimTime::EPOCH,
            effective_until: None,
            process_suitability: BTreeSet::from([
                "canwu.economy-reference:process:seasonal-harvest".to_owned(),
            ]),
            semantic_digest: digest_label("grain definition"),
        })?;
        resources.install_opening_account(ResourceAccount {
            id: granary_account.clone(),
            revision: ResourceRevision::INITIAL,
            custodian: manager.clone(),
            resource_revision: food.resource_revision.clone(),
            unit_revision: food.unit_revision.clone(),
            balance: OPENING_GRAIN,
            capacity: Some(60_000),
            protected_floor_policy: None,
            closed: false,
        })?;
        resources.install_opening_account(ResourceAccount {
            id: army_account.clone(),
            revision: ResourceRevision::INITIAL,
            custodian: force_holder.clone(),
            resource_revision: food.resource_revision.clone(),
            unit_revision: food.unit_revision.clone(),
            balance: 0,
            capacity: Some(20_000),
            protected_floor_policy: None,
            closed: false,
        })?;
        resources.install_report_grant(ResourceReportGrantV1 {
            id: ResourceReportGrantId::new("canwu.economy-reference:report-grant:granary-manager")?,
            holder: manager.clone(),
            scope: scope.clone(),
            accounts: BTreeSet::from([granary_account.clone()]),
            demands: (1..=14)
                .flat_map(|month| {
                    ["civilian", "relief", "force-dispatch"].map(move |label| (month, label))
                })
                .map(|(month, label)| grain_demand_id(month, label))
                .collect::<Result<BTreeSet<_>, _>>()?,
            include_transfer_details: true,
            confidence_per_mille: 1_000,
            cadence_minutes: u64::try_from(DAY.as_minutes())
                .map_err(|_| GrainLoopError::ArithmeticOverflow)?,
            delay_minutes: 0,
        })?;
        for (id, holder, confidence) in [
            (
                "canwu.economy-reference:report-grant:force-quartermaster",
                force_holder.clone(),
                1_000,
            ),
            (
                "canwu.economy-reference:report-grant:remote-commander",
                holder(4),
                800,
            ),
        ] {
            resources.install_report_grant(ResourceReportGrantV1 {
                id: ResourceReportGrantId::new(id)?,
                holder,
                scope: scope.clone(),
                accounts: BTreeSet::from([army_account.clone()]),
                demands: (1..=14)
                    .map(|month| grain_demand_id(month, "force-issue"))
                    .collect::<Result<BTreeSet<_>, _>>()?,
                include_transfer_details: true,
                confidence_per_mille: confidence,
                cadence_minutes: u64::try_from(DAY.as_minutes())
                    .map_err(|_| GrainLoopError::ArithmeticOverflow)?,
                delay_minutes: if confidence == 1_000 {
                    0
                } else {
                    u64::try_from(DAY.as_minutes())
                        .map_err(|_| GrainLoopError::ArithmeticOverflow)?
                },
            })?;
        }

        let rule = compiled
            .model_cards
            .values()
            .flat_map(|card| card.rule_revisions.iter())
            .next()
            .ok_or_else(|| GrainLoopError::Missing("compiled economy rule".to_owned()))?;
        let rule = EconomyRuleRevisionId::new(rule.as_str())?;
        let profile_id = EconomyProfileId::new("canwu.economy-reference:profile:grain-v1")?;
        let interpretation_rule = rule.clone();
        let profile = EconomyProfileV1 {
            id: profile_id.clone(),
            revision: 1,
            synthetic: true,
            compiled_content_hash: compiled.content_hash.clone(),
            definition_ids: compiled
                .definitions
                .keys()
                .map(|id| id.as_str().to_owned())
                .collect(),
            model_card_ids: compiled
                .model_cards
                .keys()
                .map(|id| id.as_str().to_owned())
                .collect(),
            consumption: PopulationConsumptionProfileV1 {
                monthly_need: CIVILIAN_NEED,
                shortage_wellbeing_cost_per_unit: 1,
                relief_wellbeing_gain_per_unit: 1,
                rule_revision: rule.clone(),
            },
            harvest: SeasonalHarvestProfileV1 {
                harvest_month: HARVEST_MONTH,
                base_output: HARVEST_BASE,
                seed_floor: 180,
                minimum_environment_per_mille: 500,
                rule_revision: rule.clone(),
            },
            relief: ReliefPolicyV1 {
                monthly_target: RELIEF_TARGET,
                rule_revision: rule.clone(),
            },
            requisition: EconomyRequisitionPolicyV1 {
                cooperation_cost_per_use: 80,
                next_harvest_penalty_per_mille: 120,
                rule_revision: rule,
            },
            price_applicability: PriceEvidenceApplicabilityV1::NotApplicable,
            interpretation_rules: BTreeSet::from([interpretation_rule]),
            semantic_digest: String::new(),
        }
        .seal()?;
        let economy = LocalEconomyId::new("canwu.economy-reference:economy:river-valley")?;
        let local = LocalEconomyV1 {
            id: economy.clone(),
            revision: 1,
            manager: manager.clone(),
            scope: scope.clone(),
            profile: profile_id.clone(),
            month: 0,
            population_wellbeing_per_mille: 1_000,
            cooperation_per_mille: 900,
            pending_harvest_penalty_per_mille: 0,
            latest_decision: GrainDecision::Balanced,
        };
        let mut economy_state =
            EconomyReferenceStateV1::default().with_compiled_content(compiled)?;
        economy_state
            .configure_completion_authority(force_holder.clone(), FORCE_COMPLETION_NAMESPACE)?;
        economy_state.profiles.insert(profile_id, profile);
        economy_state.local_economies.insert(economy.clone(), local);
        economy_state
            .resilience_postures
            .insert(economy.clone(), "keep_buffer".to_owned());
        let observation_grant = EconomyObservationGrantV1 {
            id: EconomyObservationGrantId::new(
                "canwu.economy-reference:observation-grant:river-dispatch",
            )?,
            holder: manager.clone(),
            scopes: BTreeSet::from([scope.clone()]),
            delay_minutes: 0,
            confidence_per_mille: 1_000,
        };
        economy_state
            .observation_grants
            .insert(observation_grant.id.clone(), observation_grant.clone());
        for scope in &observation_grant.scopes {
            economy_state.observation_grant_by_holder_scope.insert(
                crate::holder_scope_index_key(&observation_grant.holder, scope)?,
                observation_grant.id.clone(),
            );
        }

        let force = ReferenceForceId::new("canwu.force-supply-reference:force:river-garrison")?;
        let first_food_due = SimTime::EPOCH + MONTH + MONTH + DAY + DAY;
        let deferred_non_food_due = SimTime::EPOCH + SimDuration::days(450);
        let force_due = food_profile
            .requirements
            .iter()
            .map(|requirement| {
                let next_due = if requirement.id == food.id {
                    first_food_due
                } else {
                    deferred_non_food_due
                };
                (
                    requirement.id.clone(),
                    DueRequirementStateV1 {
                        requirement: requirement.id.clone(),
                        next_due,
                        persisted_remainder_minutes: 0,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        force_state.forces.insert(
            force.clone(),
            ReferenceForce {
                id: force.clone(),
                revision: 1,
                holder: force_holder.clone(),
                profile: food_profile.id.clone(),
                active: true,
                readiness_per_mille: 900,
                fatigue_per_mille: 100,
                cohesion_per_mille: 850,
                disease_per_mille: 0,
                desertion_per_mille: 0,
                supply_posture: "wait_for_supply".to_owned(),
                due: force_due.clone(),
                blocked_by_active_requisition: None,
            },
        );
        for due in force_due.values() {
            force_state
                .due_index
                .entry(due.next_due)
                .or_default()
                .insert((force.clone(), due.requirement.clone()));
        }
        force_state.configure_completion_authority(force_holder.clone())?;
        for grant in [
            ForceObserverGrantV1 {
                id: ForceObserverGrantId::new("canwu.force-supply-reference:grant:quartermaster")?,
                holder: force_holder.clone(),
                force: force.clone(),
                role: ForceObservationRole::WarehouseCustodian,
                observation_delay_minutes: 0,
                confidence_per_mille: 1_000,
            },
            ForceObserverGrantV1 {
                id: ForceObserverGrantId::new(
                    "canwu.force-supply-reference:grant:remote-commander",
                )?,
                holder: holder(4),
                force: force.clone(),
                role: ForceObservationRole::RemoteCommander,
                observation_delay_minutes: u64::try_from(DAY.as_minutes())
                    .map_err(|_| GrainLoopError::ArithmeticOverflow)?,
                confidence_per_mille: 800,
            },
        ] {
            force_state
                .observation_grants
                .insert(grant.id.clone(), grant);
        }
        force_state.validate()?;

        let scenario = Scenario::new(
            SimTime::EPOCH,
            vec![
                EntityRef::Person(PersonId::new(1)),
                EntityRef::Person(PersonId::new(2)),
                EntityRef::Person(PersonId::new(3)),
                EntityRef::Person(PersonId::new(4)),
            ],
        )
        .with_domain_records(vec![
            resources.into_record()?,
            economy_state.into_initial_record()?,
            force_state.into_initial_record()?,
        ]);
        let mut canwu = new_canwu(0x0047_5241_494e, scenario)?;
        let archive_store = Rc::new(GrainArchiveStore::default());
        canwu.set_plugin_archive_object_provider(archive_store.clone());
        register_grain_decision_controller(&mut canwu)?;
        Ok(Self {
            canwu,
            archive_store,
            economy,
            force,
            grain_revision: food.resource_revision,
            unit_revision: food.unit_revision,
            granary_account,
            army_account,
            scope,
        })
    }

    #[must_use]
    pub const fn canwu(&self) -> &Canwu {
        &self.canwu
    }

    #[doc(hidden)]
    pub const fn canwu_mut(&mut self) -> &mut Canwu {
        &mut self.canwu
    }

    pub fn snapshot_json(&self) -> Result<String, GrainLoopError> {
        serde_json::to_string(&GrainHarnessSnapshotV1 {
            canwu_snapshot_json: self.canwu.snapshot_json()?,
            archive: self.archive_store.bundle(),
        })
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))
    }

    pub fn checkpoint_journal_json(&self) -> Result<String, GrainLoopError> {
        self.replay_journal_json()
    }

    pub fn replay_journal_json(&self) -> Result<String, GrainLoopError> {
        serde_json::to_string(&GrainHarnessJournalV1 {
            canwu_replay_journal_json: serde_json::to_string(&self.canwu.replay_journal())
                .map_err(|error| GrainLoopError::Canwu(error.to_string()))?,
            archive: self.archive_store.bundle(),
        })
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))
    }

    pub fn from_snapshot_json(json: &str) -> Result<Self, GrainLoopError> {
        let saved: GrainHarnessSnapshotV1 =
            serde_json::from_str(json).map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        let archive_store = Rc::new(GrainArchiveStore::from_bundle(saved.archive));
        let mut canwu = load_snapshot(&saved.canwu_snapshot_json)?;
        canwu.set_plugin_archive_object_provider(archive_store.clone());
        canwu_resource::validate_resource_runtime_with_archive_store(
            &canwu,
            archive_store.as_ref(),
        )?;
        validate_reference_archive_stores(&canwu, archive_store.as_ref())?;
        let mut harness = Self::from_canwu(canwu)?;
        harness.archive_store = archive_store.clone();
        harness
            .canwu
            .set_plugin_archive_object_provider(archive_store);
        Ok(harness)
    }

    pub fn replay_from_journal_json(json: &str) -> Result<Self, GrainLoopError> {
        let saved: GrainHarnessJournalV1 =
            serde_json::from_str(json).map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        let archive_store = Rc::new(GrainArchiveStore::from_bundle(saved.archive));
        let canwu = replay_journal(&saved.canwu_replay_journal_json, archive_store.clone())?;
        canwu_resource::validate_resource_runtime_with_archive_store(
            &canwu,
            archive_store.as_ref(),
        )?;
        validate_reference_archive_stores(&canwu, archive_store.as_ref())?;
        let mut harness = Self::from_canwu(canwu)?;
        harness.archive_store = archive_store.clone();
        harness
            .canwu
            .set_plugin_archive_object_provider(archive_store);
        Ok(harness)
    }

    #[must_use]
    pub fn archive_object_count(&self) -> usize {
        self.archive_store.objects.borrow().len()
    }

    pub fn set_archive_storage_available(&mut self, available: bool) {
        *self.archive_store.available.borrow_mut() = available;
    }

    pub fn archive_reference_history(&mut self) -> Result<(), GrainLoopError> {
        let force = self.force_state()?;
        if !force.terminal_receipts.is_empty() {
            let prepared = force.prepare_force_archive(ARCHIVE_BATCH)?;
            let receipt =
                enqueue_force_archive(&mut self.canwu, &prepared, self.archive_store.as_ref())?;
            self.settle_current(&[])?;
            finalize_force_archive_retention(
                &mut self.canwu,
                self.archive_store.as_ref(),
                &receipt,
            )?;
            self.settle_current(&[])?;
        }
        let (_, economy) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let has_archive_candidates = economy.frames.values().any(|frames| !frames.is_empty())
            || economy
                .observation_heads
                .values()
                .any(|heads| heads.len() > 1)
            || !economy.route_observations.is_empty()
            || !economy.price_observations.is_empty()
            || economy
                .delivery_attempts
                .values()
                .any(|attempt| attempt.disposition != DeliveryDispositionV1::Pending)
            || !economy.externality_outcomes.is_empty()
            || !economy.outcomes.is_empty();
        if has_archive_candidates {
            let prepared = economy.prepare_economy_archive(ARCHIVE_BATCH)?;
            let receipt = crate::enqueue_economy_archive(
                &mut self.canwu,
                &prepared,
                self.archive_store.as_ref(),
            )?;
            self.settle_current(&[])?;
            crate::finalize_economy_archive_retention(
                &mut self.canwu,
                self.archive_store.as_ref(),
                &receipt,
            )?;
            self.settle_current(&[])?;
        }
        Ok(())
    }

    pub fn fork(&self) -> Result<Self, GrainLoopError> {
        let mut fork = Self::from_canwu(self.canwu.fork())?;
        fork.archive_store = self.archive_store.clone();
        fork.canwu
            .set_plugin_archive_object_provider(fork.archive_store.clone());
        Ok(fork)
    }

    fn from_canwu(mut canwu: Canwu) -> Result<Self, GrainLoopError> {
        let (_, resources) = resource_state(&canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let (_, economy_state) = economy_reference_state(&canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let force_record = canwu
            .typed_domain_record(&force_supply_runtime_reference())
            .ok_or_else(|| GrainLoopError::Missing("force runtime".to_owned()))?;
        let force_state = force_record.decode_payload::<ForceSupplyRuntimeRecord>()?;
        let economy = economy_state
            .local_economies
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| GrainLoopError::Missing("local economy".to_owned()))?;
        let local = &economy_state.local_economies[&economy];
        let force = force_state
            .forces
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| GrainLoopError::Missing("reference force".to_owned()))?;
        let army_account = ResourceAccountId::new("canwu.economy-reference:account:a-field-force")?;
        let granary_account = ResourceAccountId::new("canwu.economy-reference:account:z-granary")?;
        let grain_revision = resources
            .accounts
            .get(&army_account)
            .map(|account| account.resource_revision.clone())
            .ok_or_else(|| GrainLoopError::Missing("army account".to_owned()))?;
        let definition = &resources.definitions[&grain_revision];
        let archive_store = Rc::new(GrainArchiveStore::default());
        canwu.set_plugin_archive_object_provider(archive_store.clone());
        Ok(Self {
            canwu,
            archive_store,
            economy,
            force,
            grain_revision,
            unit_revision: definition.canonical_unit.clone(),
            granary_account,
            army_account,
            scope: local.scope.clone(),
        })
    }

    #[allow(clippy::too_many_lines)]
    pub fn advance_month(
        &mut self,
        decision: GrainDecision,
    ) -> Result<MonthlyEconomyFrameV1, GrainLoopError> {
        let month = self
            .current_month()?
            .checked_add(1)
            .ok_or(GrainLoopError::ArithmeticOverflow)?;
        let month_start = SimTime::EPOCH + SimDuration::days(30 * i64::from(month - 1));
        if self.canwu.time() < month_start {
            self.settle_at(month_start, &[SystemCadence::Daily])?;
        }
        let decision = self.record_decision(month, decision)?;
        // Seasonal river access is observed every month, including months
        // without a shipment.  The first two months are closed; month three
        // opens again but the delivery below demonstrates a sudden crossing
        // failure and an evidence-backed reroute of that same attempt.
        self.record_route_availability(month, month >= 3, true)?;
        self.record_g5_decision(month, decision)?;
        let resilience_decision = self.resilience_decision()?;
        if month >= 3 {
            self.record_force_decision(month, decision)?;
        }
        let force_decision = if month >= 3 {
            self.force_posture_decision()?
        } else {
            decision
        };
        let month_end = SimTime::EPOCH + SimDuration::days(30 * i64::from(month));
        let mut force_operation = if month > 3 {
            self.service_force_if_due(month, month_end, force_decision)?
        } else {
            None
        };

        // All three uses are manager-owned demands for the same exact grain
        // revision and become due at the same simulation instant. One
        // canonical allocation ingress therefore decides scarcity by persisted
        // priority/tie-break data rather than by Rust control flow.
        let manager = holder(1);
        let civilian = self.submit_demand(
            month,
            "civilian",
            manager.clone(),
            CIVILIAN_NEED,
            decision_priority(resilience_decision, "civilian"),
            None,
        )?;
        let relief = self.submit_demand(
            month,
            "relief",
            manager.clone(),
            RELIEF_TARGET,
            decision_priority(resilience_decision, "relief"),
            None,
        )?;
        let force_demand = self.submit_demand(
            month,
            "force-dispatch",
            manager.clone(),
            SHIPMENT_QUANTITY,
            decision_priority(resilience_decision, "force"),
            None,
        )?;
        self.allocate_competing(month, &manager)?;
        let civilian_leg = self.reserved_allocation(&civilian)?;
        let relief_leg = self.reserved_allocation(&relief)?;
        let force_leg = self.reserved_allocation(&force_demand)?;

        if month >= 3 {
            if let Some(force_leg) = force_leg
                && month == 3
            {
                self.deliver_to_force(month, true, &force_demand, force_leg)?;
            }
            if month == 3 {
                force_operation = self.service_force_if_due(month, month_end, force_decision)?;
            }
        } else {
            self.cancel_demand(month, "force-before-readiness", &force_demand)?;
        }
        if let Some(leg) = civilian_leg {
            self.consume_local(month, "civilian", NS_CIVILIAN_CONSUME, &civilian, &leg)?;
        }
        if let Some(leg) = relief_leg {
            self.consume_local(month, "relief", NS_RELIEF_CONSUME, &relief, &leg)?;
        }
        self.advance_daily_to(month_end)?;
        let harvest_credit = if ((month - 1) % 12) + 1 == HARVEST_MONTH {
            Some(self.credit_harvest(month)?)
        } else {
            None
        };
        let source_versions = vec![
            self.current_exact(resource_runtime_reference_untyped())?,
            self.current_exact(force_supply_runtime_reference().into_untyped())?,
        ];
        self.submit_economy_at(
            EconomyOperationV1::CloseMonth {
                economy: self.economy.clone(),
                evidence: MonthlyEconomyEvidenceV1 {
                    civilian_demand: civilian,
                    relief_demand: relief,
                    force_demand,
                    harvest_credit,
                    force_operation,
                    source_versions,
                },
            },
            month_end,
            &[SystemCadence::Monthly],
        )?;
        self.record_resource_observations(month)?;
        let (_, state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let frame = state.frames[&self.economy]
            .last()
            .cloned()
            .ok_or_else(|| GrainLoopError::Missing("monthly frame".to_owned()))?;
        self.archive_if_needed()?;
        Ok(frame)
    }

    pub fn run_fourteen_months(
        mut self,
        decisions: impl IntoIterator<Item = GrainDecision>,
    ) -> Result<GrainLoopSummary, GrainLoopError> {
        let decisions: Vec<_> = decisions.into_iter().collect();
        if decisions.len() != 14 {
            return Err(GrainLoopError::Missing(
                "the reference case requires exactly fourteen decisions".to_owned(),
            ));
        }
        for (index, decision) in decisions.into_iter().enumerate() {
            self.advance_month(decision).map_err(|error| {
                GrainLoopError::Canwu(format!("month {} failed: {error}", index + 1))
            })?;
        }
        self.summary()
    }

    pub fn summary(&self) -> Result<GrainLoopSummary, GrainLoopError> {
        let (_, resources) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        resources.validate_conservation()?;
        let (_, economies) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let force_record = self
            .canwu
            .typed_domain_record(&force_supply_runtime_reference())
            .ok_or_else(|| GrainLoopError::Missing("force runtime".to_owned()))?;
        let forces = force_record.decode_payload::<ForceSupplyRuntimeRecord>()?;
        let frames = economies
            .frames
            .get(&self.economy)
            .cloned()
            .unwrap_or_default();
        let local = &economies.local_economies[&self.economy];
        let force = &forces.forces[&self.force];
        let final_stock = resources
            .accounts
            .values()
            .try_fold(0_u64, |sum, account| sum.checked_add(account.balance))
            .ok_or(GrainLoopError::ArithmeticOverflow)?;
        let conservation_closing = u128::from(final_stock)
            + resources
                .transfers
                .values()
                .filter(|transfer| {
                    !matches!(
                        transfer.state,
                        canwu_resource::ResourceTransferState::Accepted
                            | canwu_resource::ResourceTransferState::Lost
                            | canwu_resource::ResourceTransferState::ExternalOutflowSettled
                            | canwu_resource::ResourceTransferState::Cancelled
                            | canwu_resource::ResourceTransferState::Returned
                    )
                })
                .map(|transfer| u128::from(transfer.quantity))
                .sum::<u128>();
        Ok(GrainLoopSummary {
            total_harvest: frames.iter().map(|frame| frame.harvest_output).sum(),
            transport_executions: economies.delivery_attempts.len(),
            closed_route_months: economies
                .route_observations
                .values()
                .filter(|observation| {
                    observation.id.as_str().contains(":route:primary:") && !observation.reachable
                })
                .filter_map(|observation| route_month_from_id(observation.id.as_str()))
                .collect(),
            rerouted_months: economies
                .delivery_attempts
                .values()
                .filter(|attempt| {
                    attempt.execution.revisions.iter().any(|revision| {
                        matches!(revision.reason, ItineraryRevisionReason::Disaster { .. })
                    })
                })
                .filter_map(|attempt| delivery_month_from_id(attempt.id.as_str()))
                .collect(),
            frames,
            final_stock,
            final_population_wellbeing_per_mille: local.population_wellbeing_per_mille,
            final_force_readiness_per_mille: force.readiness_per_mille,
            final_cooperation_per_mille: local.cooperation_per_mille,
            conservation_closing,
            checkpoint_hash: canonical_hash(
                "canwu.economy-reference.grain.checkpoint.v1",
                &self.canwu.snapshot(),
            )?,
        })
    }

    fn current_month(&self) -> Result<u16, GrainLoopError> {
        let (_, state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        Ok(state.local_economies[&self.economy].month)
    }

    #[allow(clippy::too_many_lines)]
    fn record_resource_observations(&mut self, month: u16) -> Result<(), GrainLoopError> {
        let provider_source =
            self.current_exact(economy_reference_runtime_reference().into_untyped())?;
        let grant_ids = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?
            .1
            .report_grants
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for grant_id in grant_ids {
            let (_, state) = resource_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
            let grant = state.report_grants[&grant_id].clone();
            let revision = state
                .observation_head_by_grant
                .get(&grant_id)
                .and_then(|id| state.observation_heads.get(id))
                .map_or(Ok(ResourceRevision::INITIAL), |head| head.revision.next())?;
            let stock = grant
                .accounts
                .iter()
                .filter_map(|account_id| {
                    let account = state.accounts.get(account_id)?;
                    let definition = state.definitions.get(&account.resource_revision)?;
                    let quantities = state.account_quantities(account_id).ok()?;
                    Some(ResourceStockObservationV1 {
                        account: account_id.clone(),
                        scope: definition.scope.clone(),
                        known_minimum: account.balance,
                        known_maximum: account.balance,
                        reserved: quantities.reserved,
                        protected: quantities.protected,
                    })
                })
                .collect();
            let demands = grant
                .demands
                .iter()
                .filter_map(|demand_id| state.demands.get(demand_id))
                .map(|demand| ResourceDemandObservationV1 {
                    demand: demand.id.clone(),
                    requested: demand.requested,
                    fulfilled: demand.fulfilled,
                    remainder: demand.requested.saturating_sub(demand.fulfilled),
                    status: demand.status,
                    rejection_reason: demand.rejection_reason.clone(),
                })
                .collect();
            let allocations = state
                .allocation_legs
                .values()
                .filter(|leg| {
                    leg.status != AllocationLegStatus::Reserved
                        && grant.accounts.contains(&leg.account)
                        && grant.demands.contains(&leg.demand)
                })
                .map(|leg| ResourceAllocationObservationV1 {
                    allocation: leg.id.clone(),
                    exact: leg.into(),
                    status: leg.status,
                })
                .collect();
            let fulfillments = state
                .fulfillments
                .values()
                .filter(|fulfillment| {
                    grant.demands.contains(&fulfillment.demand)
                        && fulfillment.allocation_legs.iter().all(|allocation| {
                            state.allocation_legs.get(allocation).is_none_or(|leg| {
                                grant.accounts.contains(&leg.account)
                                    && grant.demands.contains(&leg.demand)
                            })
                        })
                })
                .map(|fulfillment| ResourceFulfillmentObservationV1 {
                    fulfillment: fulfillment.id.clone(),
                    consumed: fulfillment.consumed_quantity,
                    remainder: fulfillment.remainder,
                    rejection_reason: fulfillment.rejection_reason.clone(),
                })
                .collect();
            let transfers = if grant.include_transfer_details {
                state
                    .transfers
                    .values()
                    .filter(|transfer| {
                        grant.accounts.contains(&transfer.source)
                            || transfer
                                .destination
                                .as_ref()
                                .is_some_and(|account| grant.accounts.contains(account))
                    })
                    .map(|transfer| ResourceTransferObservationV1 {
                        transfer: transfer.id.clone(),
                        state: transfer.state,
                        quantity: transfer.quantity,
                        escrow: transfer.escrow,
                        accepted: transfer.accepted,
                        lost: transfer.lost,
                        returned: transfer.returned,
                        external_outflow: transfer.external_outflow,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let consumptions = if grant.include_transfer_details {
                state
                    .consumptions
                    .values()
                    .filter(|consumption| {
                        grant.accounts.contains(&consumption.account)
                            && grant.demands.contains(&consumption.demand)
                    })
                    .map(|consumption| ResourceConsumptionObservationV1 {
                        consumption: consumption.id.clone(),
                        exact: ResourceConsumptionVersionV1::from(consumption),
                        demand: consumption.demand.clone(),
                        status: consumption.status,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let head = ResourceObservationHeadV1 {
                id: ResourceObservationHeadId::new(format!(
                    "canwu.economy-reference:observation:{}",
                    grant_id.as_str().replace(':', "-")
                ))?,
                revision,
                provider_plugin: crate::PLUGIN_NAME.to_owned(),
                provider_version: env!("CARGO_PKG_VERSION").to_owned(),
                provider_semantic_hash: crate::ECONOMY_SEMANTIC_HASH.to_owned(),
                provider_source: provider_source.clone(),
                holder: grant.holder,
                grant: grant_id.clone(),
                provider_state_revision: state.state_revision,
                observed_at: self.canwu.time(),
                confidence_per_mille: grant.confidence_per_mille,
                stock,
                demands,
                allocations,
                fulfillments,
                transfers,
                consumptions,
                source_versions: vec![provider_source.clone()],
                semantic_digest: String::new(),
            }
            .seal()?;
            self.adapter_resource(
                provider_source.clone(),
                ResourceOperationRequestV1::RecordObservation(
                    canwu_resource::ResourceObservationRequestV1 {
                        operation_key: operation_key(
                            month,
                            &format!("observe-{}", grant_id.as_str().replace(':', "-")),
                        )?,
                        head,
                    },
                ),
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn record_decision(
        &mut self,
        month: u16,
        decision: GrainDecision,
    ) -> Result<GrainDecision, GrainLoopError> {
        let ticket_id = DecisionTicketId::new(u64::from(month));
        let policy = grain_decision_policy();
        let (_, economy_state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let options = [
            GrainDecision::ReliefFirst,
            GrainDecision::ForceFirst,
            GrainDecision::Balanced,
            GrainDecision::RequisitionForForce,
        ]
        .into_iter()
        .map(|option| -> Result<DecisionOption, GrainLoopError> {
            let command = economy_command(&EconomyCommandV1 {
                holder: holder(1),
                operation_id: EconomyOperationId::new(format!(
                    "canwu.economy-reference:operation:grain-decision:{month:02}:{}",
                    decision_option_id(option),
                ))?,
                expected_runtime_revision: economy_state.revision,
                operation: EconomyOperationV1::SelectDecision {
                    economy: self.economy.clone(),
                    decision: option,
                    selection: crate::EconomyDecisionSelectionV1 {
                        ticket: ticket_id,
                        option_id: decision_option_id(option).to_owned(),
                    },
                },
            })
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
            Ok(DecisionOption {
                action: DecisionAction::Command {
                    command: serde_json::to_value(command)
                        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?,
                },
                utility_inputs: BTreeMap::from([
                    (
                        "population_wellbeing".to_owned(),
                        i64::from(decision_priority(option, "civilian")),
                    ),
                    (
                        "force_readiness".to_owned(),
                        i64::from(decision_priority(option, "force")),
                    ),
                    (
                        "relief_coverage".to_owned(),
                        i64::from(decision_priority(option, "relief")),
                    ),
                ]),
                metadata: serde_json::json!({ "grain_decision": decision_option_id(option) }),
                ..DecisionOption::new(decision_option_id(option), decision_option_label(option))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let context = DecisionContext::new(
            "canwu.economy-reference.grain-allocation.v1",
            serde_json::json!({
                "month": month,
                "resource": self.current_exact(resource_runtime_reference_untyped())?,
                "economy": self.current_exact(economy_reference_runtime_reference().into_untyped())?,
                "force": self.current_exact(force_supply_runtime_reference().into_untyped())?,
            }),
        );
        self.canwu.enqueue_decision(
            self.canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(10_000 + u64::from(month) * 2),
                self.canwu.revision(),
                DecisionMutation::Open {
                    ticket: DecisionTicketDraft {
                        id: ticket_id,
                        definition: "canwu.economy-reference.grain-allocation".to_owned(),
                        decision_maker: EntityRef::Person(PersonId::new(1)),
                        assigned_controller: grain_decision_controller_id().to_owned(),
                        summary: format!("Choose the conserved grain allocation for month {month}"),
                        context,
                        options: options.clone(),
                        deadline: Some(self.canwu.time()),
                    },
                },
            ),
        )?;
        self.settle_current(&[])?;
        let command_request_id = CommandRequestId::new(40_000 + u64::from(month));
        let selected_command = options
            .iter()
            .find(|option| option.id == decision_option_id(decision))
            .and_then(|option| match &option.action {
                DecisionAction::Command { command } => Some(command.clone()),
                DecisionAction::None => None,
            })
            .ok_or_else(|| GrainLoopError::Missing("selected grain command option".to_owned()))?;
        let selected_command: Command = serde_json::from_value(selected_command)
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        let decision_revision = self.canwu.revision();
        self.canwu.enqueue_decision(
            self.canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(10_001 + u64::from(month) * 2),
                decision_revision,
                DecisionMutation::Resolve {
                    ticket_id,
                    expected_version: 1,
                    controller_id: grain_decision_controller_id().to_owned(),
                    policy,
                    decision: PolicyDecision::selected(
                        decision_option_id(decision),
                        "Manager selected the month's persisted grain priority",
                    ),
                    command_request_id: Some(command_request_id),
                },
            )
            .with_command(CommandRequest::new(
                command_request_id,
                decision_revision,
                CommandEnvelope::new(
                    Issuer::Human(grain_decision_controller_id().to_owned()),
                    selected_command,
                )
                .with_authority(CommandAuthority::for_actor(PersonId::new(1)))
                .at_time(self.canwu.time()),
            )),
        )?;
        self.settle_current(&[])?;
        self.settle_current(&[])?;
        let ticket = self
            .canwu
            .decision_ticket(ticket_id)
            .ok_or_else(|| GrainLoopError::Missing("resolved grain ticket".to_owned()))?;
        let canwu_api::DecisionTicketState::Resolved { option_id, .. } = &ticket.state else {
            return Err(GrainLoopError::Rejected(format!(
                "grain decision did not resolve authoritatively: state={:?}, attempt={:?}",
                ticket.state,
                self.canwu
                    .decision_attempt(DecisionRequestId::new(10_001 + u64::from(month) * 2))
            )));
        };
        grain_decision_from_option(option_id)
    }

    fn record_force_decision(
        &mut self,
        month: u16,
        decision: GrainDecision,
    ) -> Result<(), GrainLoopError> {
        let holder = holder(2);
        let detached = self.force_state()?.decision_ticket(&holder, &self.force)?;
        let selected = match decision {
            GrainDecision::RequisitionForForce => "requisition_locally",
            GrainDecision::ForceFirst => "advance_immediately",
            GrainDecision::ReliefFirst | GrainDecision::Balanced => "wait_for_supply",
        };
        let force_revision = self.force_state()?.revision;
        let options = [
            ("wait_for_supply", "Wait for supply"),
            ("advance_immediately", "Advance immediately"),
            ("requisition_locally", "Requisition locally"),
        ]
        .into_iter()
        .map(|(id, label)| -> Result<DecisionOption, GrainLoopError> {
            let command = force_supply_command(&ForceCommandEnvelopeV1 {
                operation_id: ForceOperationId::new(format!(
                    "canwu.force-supply-reference:decision:month-{month:02}:{id}"
                ))?,
                holder: holder.clone(),
                expected_runtime_revision: force_revision,
                operation: ForceOperationV1::SelectSupplyPosture {
                    force: self.force.clone(),
                    posture: id.to_owned(),
                    decision: canwu_force_supply_reference::ForceDecisionSelectionV1 {
                        ticket: DecisionTicketId::new(10_000 + u64::from(month)),
                        option_id: id.to_owned(),
                    },
                },
            })
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
            Ok(DecisionOption {
                action: DecisionAction::Command {
                    command: serde_json::to_value(command)
                        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?,
                },
                metadata: serde_json::json!({ "force_choice": id }),
                ..DecisionOption::new(id, label)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let context = DecisionContext::new(
            "canwu.force-supply-reference.supply-choice.v1",
            serde_json::json!({
                "holder": holder,
                "force": self.force,
                "force_revision": detached.force_revision,
                "holder_facts_digest": detached.holder_facts_digest,
                "provider": self.current_exact(force_supply_runtime_reference().into_untyped())?,
            }),
        );
        self.record_resolved_ticket(
            DecisionTicketId::new(10_000 + u64::from(month)),
            20_000 + u64::from(month) * 2,
            "canwu.force-supply-reference.supply-choice",
            EntityRef::Person(PersonId::new(2)),
            force_decision_controller_id(),
            force_decision_policy(),
            format!("Choose the force-supply posture for month {month}"),
            context,
            &options,
            selected,
            "Commander selected a persisted force-supply option",
        )
    }

    fn record_g5_decision(
        &mut self,
        month: u16,
        decision: GrainDecision,
    ) -> Result<(), GrainLoopError> {
        let manager = holder(1);
        let registry = ProjectionProviderRegistryV1::new([ProjectionScopeBindingV1 {
            holder: manager.clone(),
            scope: self.scope.clone(),
            resource: ResourceProjectionSourceV1 {
                grant: ResourceReportGrantId::new(
                    "canwu.economy-reference:report-grant:granary-manager",
                )?,
            },
            production: None,
            force: None,
            semantic_digest: String::new(),
        }
        .seal()
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?])
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        let projection = registry.project(&self.canwu, &manager, &self.scope);
        let projection_digest = match &projection {
            ProjectionQueryResultV1::Available(value) => value.price_pressure.input_digest.clone(),
            ProjectionQueryResultV1::Unavailable(value) => value.digest.clone(),
        };
        let selected = match decision {
            GrainDecision::ReliefFirst => "release_reserves",
            GrainDecision::Balanced => "keep_buffer",
            GrainDecision::RequisitionForForce => "ration",
            GrainDecision::ForceFirst => "dispatch_remote_transfer",
        };
        let (_, economy_state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let economy_revision = economy_state.revision;
        let options = [
            ("release_reserves", "Release reserves"),
            ("keep_buffer", "Keep the buffer"),
            ("ration", "Ration local stock"),
            ("dispatch_remote_transfer", "Dispatch a remote transfer"),
        ]
        .into_iter()
        .map(|(id, label)| -> Result<DecisionOption, GrainLoopError> {
            let command = economy_command(&EconomyCommandV1 {
                holder: holder(1),
                operation_id: EconomyOperationId::new(format!(
                    "canwu.economy-reference:decision:month-{month:02}:{id}"
                ))?,
                expected_runtime_revision: economy_revision,
                operation: EconomyOperationV1::SelectResiliencePosture {
                    economy: self.economy.clone(),
                    posture: id.to_owned(),
                    selection: crate::EconomyDecisionSelectionV1 {
                        ticket: DecisionTicketId::new(20_000 + u64::from(month)),
                        option_id: id.to_owned(),
                    },
                },
            })
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
            Ok(DecisionOption {
                action: DecisionAction::Command {
                    command: serde_json::to_value(command)
                        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?,
                },
                metadata: serde_json::json!({ "resilience_choice": id }),
                ..DecisionOption::new(id, label)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let context = DecisionContext::new(
            "canwu.economy-reference.local-resilience.v1",
            serde_json::json!({
                "holder": manager,
                "scope": self.scope,
                "projection_digest": projection_digest,
                "projection": projection,
                "economy": self.current_exact(economy_reference_runtime_reference().into_untyped())?,
            }),
        );
        self.record_resolved_ticket(
            DecisionTicketId::new(20_000 + u64::from(month)),
            30_000 + u64::from(month) * 2,
            "canwu.economy-reference.local-resilience",
            EntityRef::Person(PersonId::new(1)),
            grain_decision_controller_id(),
            grain_decision_policy(),
            format!("Choose the local resilience response for month {month}"),
            context,
            &options,
            selected,
            "Manager selected a persisted projection-backed resilience option",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record_resolved_ticket(
        &mut self,
        ticket_id: DecisionTicketId,
        request_base: u64,
        definition: &str,
        decision_maker: EntityRef,
        controller_id: &str,
        policy: DecisionPolicyIdentity,
        summary: String,
        context: DecisionContext,
        options: &[DecisionOption],
        selected: &str,
        rationale: &str,
    ) -> Result<(), GrainLoopError> {
        let command_request_id = CommandRequestId::new(request_base + 100_000);
        let selected_command = options
            .iter()
            .find(|option| option.id == selected)
            .and_then(|option| match &option.action {
                DecisionAction::Command { command } => Some(command.clone()),
                DecisionAction::None => None,
            })
            .ok_or_else(|| {
                GrainLoopError::Missing("selected decision command option".to_owned())
            })?;
        let selected_command: Command = serde_json::from_value(selected_command)
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        let EntityRef::Person(decision_actor) = decision_maker.clone() else {
            return Err(GrainLoopError::Missing(
                "reference decision command requires a responsible person".to_owned(),
            ));
        };
        self.canwu.enqueue_decision(
            self.canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(request_base),
                self.canwu.revision(),
                DecisionMutation::Open {
                    ticket: DecisionTicketDraft {
                        id: ticket_id,
                        definition: definition.to_owned(),
                        decision_maker,
                        assigned_controller: controller_id.to_owned(),
                        summary,
                        context,
                        options: options.to_owned(),
                        deadline: Some(self.canwu.time()),
                    },
                },
            ),
        )?;
        self.settle_current(&[])?;
        let decision_revision = self.canwu.revision();
        self.canwu.enqueue_decision(
            self.canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(request_base + 1),
                decision_revision,
                DecisionMutation::Resolve {
                    ticket_id,
                    expected_version: 1,
                    controller_id: controller_id.to_owned(),
                    policy,
                    decision: PolicyDecision::selected(selected, rationale),
                    command_request_id: Some(command_request_id),
                },
            )
            .with_command(CommandRequest::new(
                command_request_id,
                decision_revision,
                CommandEnvelope::new(Issuer::Human(controller_id.to_owned()), selected_command)
                    .with_authority(CommandAuthority::for_actor(decision_actor))
                    .at_time(self.canwu.time()),
            )),
        )?;
        self.settle_current(&[])?;
        let ticket = self
            .canwu
            .decision_ticket(ticket_id)
            .ok_or_else(|| GrainLoopError::Missing("resolved reference ticket".to_owned()))?;
        if !matches!(
            ticket.state,
            canwu_api::DecisionTicketState::Resolved { .. }
        ) {
            return Err(GrainLoopError::Rejected(format!(
                "reference decision did not resolve: state={:?}, attempt={:?}",
                ticket.state,
                self.canwu
                    .decision_attempt(DecisionRequestId::new(request_base + 1))
            )));
        }
        Ok(())
    }

    fn archive_if_needed(&mut self) -> Result<(), GrainLoopError> {
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        if state.terminal_archive_candidates.len() < ARCHIVE_TRIGGER {
            return Ok(());
        }
        let prepared = state
            .prepare_resource_archive(state.terminal_archive_candidates.len().min(ARCHIVE_BATCH))?;
        let receipt =
            enqueue_resource_archive(&mut self.canwu, &prepared, self.archive_store.as_ref())?;
        self.settle_current(&[])?;
        finalize_resource_archive_retention(
            &mut self.canwu,
            self.archive_store.as_ref(),
            &receipt,
        )?;
        self.settle_current(&[])
    }

    fn submit_demand(
        &mut self,
        month: u16,
        label: &str,
        requester: KnowledgeHolderRef,
        quantity: u64,
        priority: i32,
        override_class: Option<&str>,
    ) -> Result<ResourceDemandId, GrainLoopError> {
        let id = grain_demand_id(month, label)?;
        let due_at = self.canwu.time();
        self.submit_resource(
            requester.clone(),
            ResourceOperationRequestV1::SubmitDemand(ResourceSubmitDemandRequestV1 {
                operation_key: operation_key(month, &format!("demand-{label}"))?,
                demand: ResourceDemand {
                    id: id.clone(),
                    revision: ResourceRevision::INITIAL,
                    requester,
                    resource_revision: self.grain_revision.clone(),
                    unit_revision: self.unit_revision.clone(),
                    requested: quantity,
                    fulfilled: 0,
                    minimum_useful: 1,
                    partial_fulfillment: PartialFulfillmentPolicy::AcceptPartial,
                    alternative_group: None,
                    due_at,
                    expires_at: due_at + MONTH,
                    priority,
                    tie_break: ResourceTieBreakKey::new(format!(
                        "canwu.economy-reference:tie:{month:02}:{label}"
                    ))?,
                    admitted_sequence: 0,
                    protected_floor_policy: None,
                    protection_override_class: override_class.map(str::to_owned),
                    status: DemandStatus::Open,
                    rejection_reason: None,
                },
            }),
        )?;
        Ok(id)
    }

    fn allocate_competing(
        &mut self,
        month: u16,
        requester: &KnowledgeHolderRef,
    ) -> Result<(), GrainLoopError> {
        let (_, before) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let now = self.canwu.time();
        enqueue_resource_allocation(
            &mut self.canwu,
            now,
            requester,
            &ResourceAllocationRequestV1 {
                operation_key: operation_key(month, "allocate-competing")?,
                expected_state_revision: before.state_revision,
                at: now,
                candidate_limit: 32,
            },
        )?;
        self.settle_current(&[])
    }

    fn reserved_allocation(
        &self,
        demand: &ResourceDemandId,
    ) -> Result<Option<ResourceAllocationLegVersionV1>, GrainLoopError> {
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        Ok(state
            .allocation_legs
            .values()
            .rfind(|leg| &leg.demand == demand && leg.status == AllocationLegStatus::Reserved)
            .map(Into::into))
    }

    fn cancel_demand(
        &mut self,
        month: u16,
        label: &str,
        demand: &ResourceDemandId,
    ) -> Result<(), GrainLoopError> {
        self.submit_resource(
            holder(1),
            ResourceOperationRequestV1::CancelDemand(
                canwu_resource::ResourceCancelDemandRequestV1 {
                    operation_key: operation_key(month, label)?,
                    demand: demand.clone(),
                    expected_demand_revision: self.demand_revision(demand)?,
                },
            ),
        )
    }

    fn allocate(
        &mut self,
        month: u16,
        label: &str,
        requester: &KnowledgeHolderRef,
        demand: &ResourceDemandId,
    ) -> Result<ResourceAllocationLegVersionV1, GrainLoopError> {
        let (_, before) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let now = self.canwu.time();
        enqueue_resource_allocation(
            &mut self.canwu,
            now,
            requester,
            &ResourceAllocationRequestV1 {
                operation_key: operation_key(month, &format!("allocate-{label}"))?,
                expected_state_revision: before.state_revision,
                at: now,
                candidate_limit: 32,
            },
        )?;
        self.settle_current(&[])?;
        let (_, after) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        after
            .allocation_legs
            .values()
            .rfind(|leg| &leg.demand == demand && leg.status == AllocationLegStatus::Reserved)
            .map(Into::into)
            .ok_or_else(|| {
                GrainLoopError::Missing(format!(
                    "allocation for {demand}: demand={:?}, army_account={:?}, force_issue_outcomes={:?}",
                    after.demands.get(demand),
                    after.accounts.get(&self.army_account),
                    after
                        .outcomes
                        .values()
                        .filter(|outcome| outcome.operation_key.as_str().contains("force-issue"))
                        .collect::<Vec<_>>()
                ))
            })
    }

    #[allow(clippy::too_many_lines)]
    fn deliver_to_force(
        &mut self,
        month: u16,
        reroute: bool,
        demand: &ResourceDemandId,
        leg: ResourceAllocationLegVersionV1,
    ) -> Result<(), GrainLoopError> {
        let quantity = leg.quantity;
        let begin_key = operation_key(month, "transfer-begin")?;
        let begin_certificate = self.activate_lease(
            month,
            "transfer-begin",
            holder(1),
            NS_TRANSFER_BEGIN,
            begin_key.clone(),
            vec![
                CompletionLockedTargetV1::Account {
                    id: leg.account.clone(),
                    revision: leg.account_revision,
                },
                CompletionLockedTargetV1::AllocationLeg {
                    id: leg.id.clone(),
                    revision: leg.revision,
                },
                CompletionLockedTargetV1::Demand {
                    id: demand.clone(),
                    revision: self.demand_revision(demand)?,
                },
            ],
            self.canwu.time(),
        )?;
        let transfer =
            ResourceTransferId::new(format!("canwu.economy-reference:transfer:month-{month:02}"))?;
        let expected_account_revision = leg.account_revision;
        self.submit_resource(
            holder(1),
            ResourceOperationRequestV1::BeginTransfer(ResourceTransferStartRequestV1 {
                operation_key: begin_key,
                transfer_id: transfer.clone(),
                allocation: leg,
                expected_account_revision,
                destination: Some(self.army_account.clone()),
                at: self.canwu.time(),
                completion_certificate: begin_certificate,
            }),
        )?;

        let mut execution = self.transport_execution(month, reroute)?;
        let transport_source = self.current_exact(resource_runtime_reference_untyped())?;
        let mut booking = CapacityBooking::new(
            CapacityBookingId(u64::from(month)),
            execution.id,
            self.grain_revision.as_str().to_owned(),
            self.canwu.time(),
            self.canwu.time() + MONTH,
            quantity,
            decision_priority(GrainDecision::ForceFirst, "force"),
        )
        .map_err(transport_error)?;
        booking
            .allocation_evidence
            .push(EvidenceRef::DomainRecordVersion(transport_source.clone()));
        booking
            .transition(CapacityBookingStatus::Confirmed, self.canwu.time())
            .map_err(transport_error)?;
        booking
            .transition(CapacityBookingStatus::Consumed, self.canwu.time())
            .map_err(transport_error)?;
        execution.bookings.push(booking);
        let attempt_version =
            self.current_exact(economy_reference_runtime_reference().into_untyped())?;
        execution.delivery_attempt = Some(attempt_version.clone());
        execution
            .begin_saga(
                attempt_version,
                format!("canwu.economy-reference:delivery:month-{month:02}"),
            )
            .map_err(|error| GrainLoopError::Transport(format!("{error:?}")))?;
        if reroute {
            execution
                .start_current_leg(self.canwu.time())
                .map_err(transport_error)?;
            execution
                .fail_current_leg("river crossing closed".to_owned(), self.canwu.time())
                .map_err(transport_error)?;
            let alternate = route_plan(self.canwu.time(), month, true)?;
            execution
                .reroute(
                    ItineraryRevision {
                        id: ItineraryRevisionId(2),
                        predecessor: Some(ItineraryRevisionId(1)),
                        plan: alternate,
                        planned_at: self.canwu.time(),
                        valid_from: self.canwu.time(),
                        reason: ItineraryRevisionReason::Disaster {
                            explanation: "river crossing closed; using ridge road".to_owned(),
                        },
                        superseded_at: None,
                        evidence: Vec::new(),
                    },
                    self.canwu.time(),
                )
                .map_err(transport_error)?;
        }
        self.record_delivery(
            month,
            &transfer,
            DeliveryDispositionV1::Pending,
            &execution,
            vec![transport_source],
        )?;
        let evidence = self.current_exact(economy_reference_runtime_reference().into_untyped())?;
        let link = active_transport_link(&execution)?;
        self.adapter_resource(
            evidence.clone(),
            ResourceOperationRequestV1::AdvanceTransfer(ResourceTransferProgressRequestV1 {
                operation_key: operation_key(month, "transfer-in-transit")?,
                transfer: transfer.clone(),
                expected_transfer_revision: ResourceRevision::INITIAL,
                progress: TransferProgressV1::InTransit,
                transport: link,
                transport_evidence: evidence,
            }),
        )?;

        loop {
            execution
                .start_current_leg(self.canwu.time())
                .map_err(transport_error)?;
            self.settle_at(self.canwu.time() + DAY, &[SystemCadence::Daily])?;
            let active = execution
                .active_itinerary_revision
                .ok_or_else(|| GrainLoopError::Missing("active itinerary".to_owned()))?;
            let endpoint = execution
                .revisions
                .iter()
                .find(|revision| revision.id == active)
                .and_then(|revision| revision.plan.legs.get(execution.current_leg_index))
                .map(|route_leg| route_leg.to.as_str().to_owned())
                .ok_or_else(|| GrainLoopError::Missing("transport route leg".to_owned()))?;
            let arrived = execution
                .complete_current_leg(self.canwu.time(), endpoint.clone())
                .map_err(transport_error)?;
            if arrived {
                break;
            }
            let active_legs: Vec<_> = execution
                .legs
                .iter()
                .filter(|item| item.itinerary_revision == active)
                .collect();
            let from = active_legs[execution.current_leg_index - 1].id;
            let to = active_legs[execution.current_leg_index].id;
            execution
                .record_handoff(Handoff {
                    id: HandoffId(
                        u64::from(month) * 10
                            + u64::try_from(execution.current_leg_index)
                                .map_err(|_| GrainLoopError::ArithmeticOverflow)?,
                    ),
                    from_leg: from,
                    to_leg: to,
                    from_custodian: "river-carrier".to_owned(),
                    to_custodian: "garrison-carrier".to_owned(),
                    at: self.canwu.time(),
                    location: endpoint,
                    evidence: Vec::new(),
                })
                .map_err(transport_error)?;
        }
        self.record_delivery(
            month,
            &transfer,
            DeliveryDispositionV1::Pending,
            &execution,
            vec![self.current_exact(resource_runtime_reference_untyped())?],
        )?;
        let arrival_evidence =
            self.current_exact(economy_reference_runtime_reference().into_untyped())?;
        let arrival_link = active_transport_link(&execution)?;
        let transfer_revision = self.transfer_revision(&transfer)?;
        self.adapter_resource(
            arrival_evidence.clone(),
            ResourceOperationRequestV1::AdvanceTransfer(ResourceTransferProgressRequestV1 {
                operation_key: operation_key(month, "transfer-arrival")?,
                transfer: transfer.clone(),
                expected_transfer_revision: transfer_revision,
                progress: TransferProgressV1::ArrivalPending,
                transport: arrival_link.clone(),
                transport_evidence: arrival_evidence.clone(),
            }),
        )?;
        let accept_key = operation_key(month, "transfer-accept")?;
        let transfer_revision = self.transfer_revision(&transfer)?;
        let destination_revision = self.account_revision(&self.army_account)?;
        let accept_certificate = self.activate_lease(
            month,
            "transfer-accept",
            holder(1),
            NS_TRANSFER_ACCEPT,
            accept_key.clone(),
            vec![
                CompletionLockedTargetV1::Transfer {
                    id: transfer.clone(),
                    revision: transfer_revision,
                },
                CompletionLockedTargetV1::Account {
                    id: self.army_account.clone(),
                    revision: destination_revision,
                },
                CompletionLockedTargetV1::ExternalRecord {
                    version: arrival_evidence.clone(),
                },
            ],
            self.canwu.time(),
        )?;
        let acceptance = ResourceTransportAcceptanceV1 {
            evidence: arrival_evidence.clone(),
            execution: arrival_link,
            destination: self.army_account.clone(),
            quantity,
            accepted_at: self.canwu.time(),
            semantic_digest: String::new(),
        }
        .seal()?;
        self.adapter_resource(
            arrival_evidence,
            ResourceOperationRequestV1::CompleteTransfer(ResourceTransferDispositionRequestV1 {
                operation_key: accept_key,
                transfer: transfer.clone(),
                expected_transfer_revision: transfer_revision,
                at: self.canwu.time(),
                disposition: ResourceTransferDispositionV1::Accept {
                    destination: self.army_account.clone(),
                    expected_destination_revision: destination_revision,
                    acceptance,
                },
                exact_transport_evidence: Some(
                    self.current_exact(economy_reference_runtime_reference().into_untyped())?,
                ),
                completion_certificate: accept_certificate,
            }),
        )?;
        execution
            .reconcile_information(ReconciliationOutcome::Success)
            .map_err(transport_error)?;
        self.record_delivery(
            month,
            &transfer,
            DeliveryDispositionV1::Accepted,
            &execution,
            vec![self.current_exact(resource_runtime_reference_untyped())?],
        )
    }

    fn record_route_availability(
        &mut self,
        month: u16,
        primary_reachable: bool,
        alternate_reachable: bool,
    ) -> Result<(), GrainLoopError> {
        let source = self.current_exact(resource_runtime_reference_untyped())?;
        let field_scope = ResourceScopeId::new("canwu.economy-reference:scope:field-force")?;
        for (route, reachable) in [
            ("primary", primary_reachable),
            ("alternate", alternate_reachable),
        ] {
            let provider_id = EconomyRouteProviderRecordId::new(format!(
                "canwu.economy-reference:route-provider:{route}:month-{month:02}"
            ))?;
            self.submit_economy(EconomyOperationV1::PublishRouteProvider {
                payload: EconomyRouteProviderPayloadV1 {
                    id: provider_id.clone(),
                    provider_plugin: crate::PLUGIN_NAME.to_owned(),
                    route_key: route.to_owned(),
                    holder: holder(1),
                    target_scope: self.scope.clone(),
                    source_scope: field_scope.clone(),
                    observed_at: self.canwu.time(),
                    reachable,
                    delay_minutes: 0,
                    confidence_per_mille: 1_000,
                    source_versions: vec![source.clone()],
                    semantic_digest: String::new(),
                },
            })?;
            let provider_source =
                self.current_exact(economy_route_provider_reference(&provider_id).into_untyped())?;
            self.submit_economy(EconomyOperationV1::RecordRouteObservation {
                observation: EconomyRouteObservationV1 {
                    id: EconomyRouteObservationId::new(format!(
                        "canwu.economy-reference:route:{route}:month-{month:02}"
                    ))?,
                    route_key: route.to_owned(),
                    holder: holder(1),
                    target_scope: self.scope.clone(),
                    source_scope: field_scope.clone(),
                    observed_at: self.canwu.time(),
                    reachable,
                    delay_minutes: 0,
                    confidence_per_mille: 1_000,
                    provider_source,
                    source_versions: vec![source.clone()],
                    semantic_digest: String::new(),
                },
            })?;
        }
        Ok(())
    }

    fn service_force_if_due(
        &mut self,
        month: u16,
        month_end: SimTime,
        decision: GrainDecision,
    ) -> Result<Option<ForceOperationId>, GrainLoopError> {
        let force_state = self.force_state()?;
        let force = &force_state.forces[&self.force];
        let food_requirement = force_state.profiles[&force.profile]
            .requirements
            .iter()
            .find(|requirement| requirement.kind == SupplyResourceKind::Food)
            .ok_or_else(|| GrainLoopError::Missing("force food requirement".to_owned()))?;
        let requirement_id = food_requirement.id.clone();
        let force_due = force
            .due
            .get(&requirement_id)
            .map(|due| due.next_due)
            .ok_or_else(|| GrainLoopError::Missing("force food cadence".to_owned()))?;
        if force_due > month_end {
            return Ok(None);
        }
        if self.canwu.time() < force_due {
            self.advance_daily_to(force_due)?;
        }
        let service_at = force_due.max(self.canwu.time());
        let force_issue_quantity = self
            .force_state()?
            .due_requirements(service_at, 64)?
            .into_iter()
            .find(|(force, requirement, ..)| force == &self.force && requirement == &requirement_id)
            .map(|(_, _, _, _, quantity)| quantity)
            .ok_or_else(|| GrainLoopError::Missing("force food due-work item".to_owned()))?;
        self.release_foreign_force_reservations(month)?;
        let force_stock = resource_state(&self.canwu)?
            .and_then(|(_, state)| {
                state.accounts.get(&self.army_account).map(|account| {
                    let reserved = state
                        .reservations
                        .values()
                        .filter(|reservation| {
                            reservation.account == self.army_account
                                && reservation.status == canwu_resource::ReservationStatus::Active
                        })
                        .map(|reservation| reservation.quantity)
                        .sum::<u64>();
                    account.balance.saturating_sub(reserved)
                })
            })
            .ok_or_else(|| GrainLoopError::Missing("force stock account".to_owned()))?;
        let issue_quantity = force_issue_quantity.min(force_stock);
        if issue_quantity == 0 {
            return Ok(None);
        }
        let issue =
            self.submit_demand(month, "force-issue", holder(2), issue_quantity, 200, None)?;
        let issue_leg = self.allocate(month, "force-issue", &holder(2), &issue)?;
        self.consume_for_force(month, &issue, &issue_leg, force_due, service_at, decision)
    }

    fn release_foreign_force_reservations(&mut self, month: u16) -> Result<(), GrainLoopError> {
        let force_holder = holder(2);
        let cancellations = {
            let (_, state) = resource_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
            let mut cancellations = BTreeMap::new();
            for reservation in state.reservations.values().filter(|reservation| {
                reservation.account == self.army_account
                    && reservation.status == canwu_resource::ReservationStatus::Active
            }) {
                let Some(leg) = state.allocation_legs.get(&reservation.allocation_leg) else {
                    continue;
                };
                let Some(demand) = state.demands.get(&leg.demand) else {
                    continue;
                };
                if demand.requester != force_holder
                    && matches!(
                        demand.status,
                        DemandStatus::Open | DemandStatus::PartiallyFulfilled
                    )
                {
                    cancellations.insert(
                        demand.id.clone(),
                        (demand.revision, demand.requester.clone()),
                    );
                }
            }
            cancellations
        };
        for (ordinal, (demand, (revision, requester))) in cancellations.into_iter().enumerate() {
            self.submit_resource(
                requester,
                ResourceOperationRequestV1::CancelDemand(
                    canwu_resource::ResourceCancelDemandRequestV1 {
                        operation_key: operation_key(
                            month,
                            &format!("release-force-custody-{ordinal}"),
                        )?,
                        demand,
                        expected_demand_revision: revision,
                    },
                ),
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn consume_for_force(
        &mut self,
        month: u16,
        demand: &ResourceDemandId,
        leg: &ResourceAllocationLegVersionV1,
        scheduled_due: SimTime,
        service_at: SimTime,
        decision: GrainDecision,
    ) -> Result<Option<ForceOperationId>, GrainLoopError> {
        let force_source = self.current_exact(force_supply_runtime_reference().into_untyped())?;
        let force_state = self.force_state()?;
        let force = force_state.forces[&self.force].clone();
        let profile = &force_state.profiles[&force.profile];
        let requirement = profile
            .requirements
            .iter()
            .find(|requirement| requirement.kind == SupplyResourceKind::Food)
            .ok_or_else(|| GrainLoopError::Missing("force food requirement".to_owned()))?;
        let key = operation_key(month, "consume-force")?;
        let economy_target = (decision == GrainDecision::RequisitionForForce)
            .then(|| self.current_exact(economy_reference_runtime_reference().into_untyped()))
            .transpose()?;
        let certificate = self.activate_force_lease(
            month,
            demand,
            leg,
            &key,
            force_source.clone(),
            economy_target.as_ref(),
            service_at,
        )?;
        let intent_id = ForceConsumptionIntentId::new(format!(
            "canwu.force-supply-reference:intent:grain-month-{month:02}"
        ))?;
        let consumption_id = ResourceConsumptionId::new(format!(
            "canwu.force-supply-reference:consumption:grain-month-{month:02}"
        ))?;
        let resource = resource_state(&self.canwu)?
            .map(|(_, state)| state)
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let destination = resource
            .accounts
            .get(&leg.account)
            .ok_or_else(|| GrainLoopError::Missing("force destination account".to_owned()))?;
        let accepted = resource
            .transfers
            .values()
            .filter(|transfer| {
                transfer.state == canwu_resource::ResourceTransferState::Accepted
                    && transfer.destination.as_ref() == Some(&leg.account)
                    && transfer.accepted >= leg.quantity
            })
            .max_by_key(|transfer| transfer.terminal_sequence);
        let accepted_transfer = accepted
            .map(|accepted| {
                let mut evidence = ForceAcceptedTransferEvidenceV1 {
                    transfer: accepted.id.clone(),
                    transfer_revision: accepted.revision,
                    destination: leg.account.clone(),
                    accepted_quantity: accepted.accepted,
                    transport: accepted.transport.clone().ok_or_else(|| {
                        GrainLoopError::Missing(
                            "accepted force transfer transport custody".to_owned(),
                        )
                    })?,
                    acceptance_source: accepted.exact_evidence.last().cloned().ok_or_else(
                        || GrainLoopError::Missing("accepted force transfer evidence".to_owned()),
                    )?,
                    semantic_digest: String::new(),
                };
                evidence.semantic_digest =
                    canonical_hash("canwu.force-supply.accepted-transfer.v1", &evidence)?;
                Ok::<_, GrainLoopError>(evidence)
            })
            .transpose()?;
        let mut stock_custody = ForceStockCustodyBindingV1 {
            destination_account: leg.account.clone(),
            destination_custodian: destination.custodian.clone(),
            accepted_transfer,
            semantic_digest: String::new(),
        };
        stock_custody.semantic_digest =
            canonical_hash("canwu.force-supply.stock-custody.v1", &stock_custody)?;
        let mut intent = ForceConsumptionIntent {
            id: intent_id.clone(),
            revision: 1,
            force: self.force.clone(),
            expected_force_runtime_revision: force_state.revision,
            expected_force_revision: force.revision,
            requirement: requirement.id.clone(),
            scheduled_due,
            due_at: service_at,
            due_count: 0,
            requested_quantity: 0,
            allocation: leg.clone(),
            stock_custody,
            resource_operation_key: key.clone(),
            consumption_id: consumption_id.clone(),
            requisition_policy: (decision == GrainDecision::RequisitionForForce)
                .then(|| force_state.requisition_policies.keys().next().cloned())
                .flatten(),
            completion_certificate: certificate.clone(),
            status: ForceConsumptionIntentStatus::PendingResourceConsumption,
            resource_outcome: None,
            resource_outcome_source: None,
            consequence: None,
            semantic_digest: String::new(),
        };
        intent.semantic_digest = canonical_hash("canwu.force-supply.intent.v1", &intent)?;
        let operation = self.submit_force(ForceOperationV1::SubmitConsumptionIntent { intent })?;
        if !self.force_state()?.intents.contains_key(&intent_id) {
            return Err(GrainLoopError::Missing(format!(
                "force consumption intent was rejected before resource ingress: operation={operation}, outcome={:?}",
                self.force_state()?.outcomes.get(&operation),
            )));
        }
        self.settle_current(&[])?;
        self.settle_current(&[])?;
        // The force command ingress is admitted first, then the force runtime
        // mutation dispatches the canonical resource adapter ingress in the
        // following boundary.  Drain that third boundary before resolving the
        // resource outcome so the adapter sees the persisted intent-bearing
        // force provider record (rather than a pre-intent head).
        self.settle_current(&[])?;
        let outcome = resource_operation_outcome(&self.canwu, &key).ok_or_else(|| {
            GrainLoopError::Missing(format!(
                "force resource outcome: operation={key}, recent={:?}",
                resource_state(&self.canwu)
                    .ok()
                    .flatten()
                    .map(|(_, state)| state
                        .outcomes
                        .values()
                        .rev()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()),
            ))
        })?;
        let force_after = self.force_state()?;
        if !force_after.intents.contains_key(&intent_id) {
            return Err(GrainLoopError::Missing(format!(
                "force intent {intent_id} was not persisted before resource acknowledgement; outcome: {:?}",
                force_after.outcomes.get(&operation)
            )));
        }
        self.completion(ResourceOperationRequestV1::Completion(
            canwu_resource::ResourceCompletionOperationV1::CompleteExternalParticipant(
                CompleteExternalCompletionParticipantGrantV1 {
                    acquisition: certificate.acquisition.clone(),
                    operation_key: key.clone(),
                },
            ),
        ))?;
        let resource_source = self.current_exact(resource_runtime_reference_untyped())?;
        self.canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                canwu_force_supply_reference::PLUGIN_NAME,
                FORCE_RESOURCE_OUTCOME_INGRESS,
                self.canwu.time(),
                serde_json::to_value(ResourceOutcomePacketV1 {
                    intent: intent_id.clone(),
                    authoritative_resource_state: resource_source,
                    outcome_id: outcome.id,
                })
                .map_err(|error| GrainLoopError::Canwu(error.to_string()))?,
            ))?;
        self.settle_current(&[])?;
        if decision == GrainDecision::RequisitionForForce {
            self.settle_current(&[])?;
            let saga = self
                .force_state()?
                .sagas
                .values()
                .find(|saga| saga.intent == intent_id)
                .map(|saga| saga.id.clone())
                .ok_or_else(|| {
                    GrainLoopError::Missing("requisition saga acknowledgement".to_owned())
                })?;
            self.submit_force(ForceOperationV1::FinalizeRequisition { saga })?;
            self.settle_current(&[])?;
        }
        Ok(Some(operation))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn activate_force_lease(
        &mut self,
        month: u16,
        demand: &ResourceDemandId,
        leg: &ResourceAllocationLegVersionV1,
        operation_key: &ResourceOperationKey,
        force_target: DomainRecordVersionRef,
        economy_target: Option<&DomainRecordVersionRef>,
        eligibility_time: SimTime,
    ) -> Result<CompletionLeaseActivationCertificateV1, GrainLoopError> {
        let acquisition = CompletionLeaseAcquisitionId::new(format!(
            "canwu.force-supply-reference:completion-acquisition:grain-month-{month:02}"
        ))?;
        let force_grant = CompletionCapacityGrantId::new(format!(
            "canwu.force-supply-reference:completion-grant:force:grain-month-{month:02}"
        ))?;
        let resource_grant = CompletionCapacityGrantId::new(format!(
            "canwu.force-supply-reference:completion-grant:resource:grain-month-{month:02}"
        ))?;
        let economy_grant = economy_target
            .as_ref()
            .map(|_| {
                CompletionCapacityGrantId::new(format!(
                    "canwu.force-supply-reference:completion-grant:economy:grain-month-{month:02}"
                ))
            })
            .transpose()?;
        let recipe = CompletionCapacityRecipeV1 {
            receipts: canwu_resource::MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
            mutations: 16,
            reports_per_holder: 1,
            holders: 1,
            bytes: 8_192,
        };
        let envelope = EligibilityEnvelopeV1::new(
            vec![force_target.clone()],
            BTreeMap::new(),
            BTreeSet::new(),
            Vec::new(),
            economy_target.cloned().into_iter().collect(),
        )?;
        let mut expected_participants = BTreeSet::from([
            canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
            canwu_resource::PLUGIN_NAME.to_owned(),
        ]);
        if economy_target.is_some() {
            expected_participants.insert(crate::PLUGIN_NAME.to_owned());
        }
        self.submit_force(ForceOperationV1::Completion {
            operation: ForceCompletionOperationV1::Acquire(RequestCompletionLeaseV1 {
                id: acquisition.clone(),
                operation_key: operation_key.clone(),
                holder: holder(2),
                operation_namespace: FORCE_COMPLETION_NAMESPACE.to_owned(),
                eligibility_time,
                eligibility_envelope: envelope.clone(),
                recipe: recipe.clone(),
                expected_participants,
                policy_class: CompletionPolicyClassV1::Guaranteed,
            }),
        })?;

        let force_state = self.force_state()?;
        force_state
            .completion_leases
            .acquisitions
            .get(&acquisition)
            .ok_or_else(|| {
                GrainLoopError::Missing(format!(
                    "force completion acquisition was rejected: {:?}",
                    force_state.outcomes.values().next_back()
                ))
            })?;

        let mut resource_targets = vec![
            CompletionLockedTargetV1::Account {
                id: leg.account.clone(),
                revision: leg.account_revision,
            },
            CompletionLockedTargetV1::AllocationLeg {
                id: leg.id.clone(),
                revision: leg.revision,
            },
            CompletionLockedTargetV1::Demand {
                id: demand.clone(),
                revision: self.demand_revision(demand)?,
            },
        ];
        resource_targets.sort();
        let envelope_digest = envelope.digest;
        let grant_boundary = self.current_boundary();
        let initial_force = self.force_state()?;
        let coordinator_revision =
            initial_force.completion_leases.acquisitions[&acquisition].revision;
        let coordinator_source =
            self.current_exact(force_supply_runtime_reference().into_untyped())?;
        let mut resource_grant_request = RequestExternalCompletionParticipantGrantV1 {
            coordinator_plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
            coordinator_source: coordinator_source.clone(),
            coordinator_acquisition_revision: coordinator_revision,
            acquisition: acquisition.clone(),
            operation_key: operation_key.clone(),
            holder: holder(2),
            operation_namespace: FORCE_COMPLETION_NAMESPACE.to_owned(),
            eligibility_time,
            eligibility_envelope_digest: envelope_digest.clone(),
            recipe: recipe.clone(),
            policy_class: CompletionPolicyClassV1::Guaranteed,
            grant_id: resource_grant.clone(),
            target_versions: resource_targets.clone(),
            current_boundary: grant_boundary,
        };
        if let (Some(grant), Some(target)) = (economy_grant.as_ref(), economy_target) {
            self.enqueue_economy_as(
                holder(2),
                EconomyOperationV1::GrantCompletionParticipant {
                    request: RequestExternalCompletionParticipantGrantV1 {
                        coordinator_plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
                        coordinator_source: coordinator_source.clone(),
                        coordinator_acquisition_revision: coordinator_revision,
                        acquisition: acquisition.clone(),
                        operation_key: operation_key.clone(),
                        holder: holder(2),
                        operation_namespace: FORCE_COMPLETION_NAMESPACE.to_owned(),
                        eligibility_time,
                        eligibility_envelope_digest: envelope_digest.clone(),
                        recipe: recipe.clone(),
                        policy_class: CompletionPolicyClassV1::Guaranteed,
                        grant_id: grant.clone(),
                        target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                            version: target.clone(),
                        }],
                        current_boundary: grant_boundary,
                    },
                },
                self.canwu.time(),
            )?;
            self.settle_current(&[])?;
        }
        // The resource owner derives its fixed TTL from this field. Bind it to
        // the boundary at which its ingress is actually admitted, rather than
        // the earlier economy-owner request construction boundary.
        resource_grant_request.current_boundary = self
            .current_boundary()
            .checked_add(1)
            .ok_or(GrainLoopError::ArithmeticOverflow)?;
        let now = self.canwu.time();
        enqueue_resource_completion_operation(
            &mut self.canwu,
            now,
            &canwu_resource::ResourceCompletionOperationV1::GrantExternalParticipant(
                resource_grant_request,
            ),
        )?;
        self.settle_current(&[])?;

        let (_, resources) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let resource_participant =
            resources.external_completion_participants.grants[&acquisition].clone();
        let resource_owner_source = self.current_exact(resource_runtime_reference_untyped())?;
        let mut grant_acknowledgements = vec![
            ForceOperationV1::Completion {
                operation: ForceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                    grant_id: force_grant.clone(),
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: coordinator_revision,
                    owner_plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
                    target_versions: vec![CompletionLockedTargetV1::ExternalRecord {
                        version: force_target,
                    }],
                    current_boundary: self.current_boundary(),
                }),
            },
            ForceOperationV1::Completion {
                operation: ForceCompletionOperationV1::AcknowledgeExternalParticipant {
                    owner_source: resource_owner_source,
                    participant: resource_participant,
                },
            },
        ];
        if economy_grant.is_some() {
            let (_, economy) = economy_reference_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
            let economy_participant = economy.completion_participants[&acquisition].clone();
            let economy_owner_source =
                self.current_exact(economy_reference_runtime_reference().into_untyped())?;
            grant_acknowledgements.push(ForceOperationV1::Completion {
                operation: ForceCompletionOperationV1::AcknowledgeExternalParticipant {
                    owner_source: economy_owner_source,
                    participant: economy_participant,
                },
            });
        }
        self.submit_force_batch(grant_acknowledgements)?;

        let coordinator_source =
            self.current_exact(force_supply_runtime_reference().into_untyped())?;
        if economy_grant.is_some() {
            let (_, economy) = economy_reference_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
            self.enqueue_economy_as(
                holder(2),
                EconomyOperationV1::PrepareCompletionParticipant {
                    request: PrepareExternalCompletionParticipantGrantV1 {
                        coordinator_source: coordinator_source.clone(),
                        acquisition: acquisition.clone(),
                        expected_grant_revision: economy.completion_participants[&acquisition]
                            .grant
                            .revision,
                        current_boundary: self.current_boundary(),
                        eligibility_envelope_digest: envelope_digest.clone(),
                    },
                },
                self.canwu.time(),
            )?;
            self.settle_current(&[])?;
        }
        let (_, resources) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let now = self.canwu.time();
        let prepare_boundary = self.current_boundary();
        enqueue_resource_completion_operation(
            &mut self.canwu,
            now,
            &canwu_resource::ResourceCompletionOperationV1::PrepareExternalParticipant(
                PrepareExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    acquisition: acquisition.clone(),
                    expected_grant_revision: resources.external_completion_participants.grants
                        [&acquisition]
                        .grant
                        .revision,
                    current_boundary: prepare_boundary,
                    eligibility_envelope_digest: envelope_digest.clone(),
                },
            ),
        )?;
        self.settle_current(&[])?;

        let (_, resources) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let resource_participant =
            resources.external_completion_participants.grants[&acquisition].clone();
        let resource_owner_source = self.current_exact(resource_runtime_reference_untyped())?;
        let state = self.force_state()?;
        let mut expected_activation_revision = state.completion_leases.acquisitions[&acquisition]
            .revision
            .next()?;
        let expected_force_grant_revision = state.completion_leases.grants[&force_grant]
            .revision
            .next()?;
        let mut prepare_acknowledgements = vec![
            ForceOperationV1::Completion {
                operation: ForceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    grant: force_grant.clone(),
                    expected_grant_revision: state.completion_leases.grants[&force_grant].revision,
                    current_boundary: self.current_boundary(),
                    eligibility_envelope_digest: envelope_digest.clone(),
                }),
            },
            ForceOperationV1::Completion {
                operation: ForceCompletionOperationV1::AcknowledgeExternalParticipant {
                    owner_source: resource_owner_source,
                    participant: resource_participant,
                },
            },
        ];
        expected_activation_revision = expected_activation_revision.next()?;
        if economy_grant.is_some() {
            let (_, economy) = economy_reference_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
            let economy_participant = economy.completion_participants[&acquisition].clone();
            let economy_owner_source =
                self.current_exact(economy_reference_runtime_reference().into_untyped())?;
            prepare_acknowledgements.push(ForceOperationV1::Completion {
                operation: ForceCompletionOperationV1::AcknowledgeExternalParticipant {
                    owner_source: economy_owner_source,
                    participant: economy_participant,
                },
            });
            expected_activation_revision = expected_activation_revision.next()?;
        }
        prepare_acknowledgements.push(ForceOperationV1::Completion {
            operation: ForceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: expected_activation_revision,
                grant: force_grant.clone(),
                expected_grant_revision: expected_force_grant_revision,
                at: self.canwu.time(),
                current_boundary: self.current_boundary(),
                eligibility_envelope_digest: envelope_digest,
            }),
        });
        self.submit_force_batch(prepare_acknowledgements)?;
        let force_state = self.force_state()?;
        let certificate = force_state
            .completion_leases
            .certificates
            .get(&acquisition)
            .cloned()
            .ok_or_else(|| {
                GrainLoopError::Missing(format!(
                    "force completion activation was rejected: acquisition={:?}, grants={:?}, outcome={:?}",
                    force_state.completion_leases.acquisitions.get(&acquisition),
                    force_state.completion_leases.grants,
                    force_state.outcomes.values().next_back()
                ))
            })?;
        let coordinator_participant = force_state
            .completion_participant_grants
            .get(&acquisition)
            .and_then(|participants| participants.get(canwu_resource::PLUGIN_NAME))
            .ok_or_else(|| {
                GrainLoopError::Missing(
                    "force coordinator lost its exact resource completion participant".to_owned(),
                )
            })?;
        let (_, resource_state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let resource_participant = resource_state
            .external_completion_participants
            .participant(&acquisition)
            .ok_or_else(|| {
                GrainLoopError::Missing(
                    "resource owner lost its exact force completion participant".to_owned(),
                )
            })?;
        if coordinator_participant.grant != resource_participant.grant
            || coordinator_participant.certificate != resource_participant.certificate
        {
            return Err(GrainLoopError::Missing(format!(
                "force/resource completion participant drift before activation consumption: coordinator={coordinator_participant:?}, owner={resource_participant:?}"
            )));
        }
        let coordinator_source =
            self.current_exact(force_supply_runtime_reference().into_untyped())?;
        self.completion(ResourceOperationRequestV1::Completion(
            canwu_resource::ResourceCompletionOperationV1::ConsumeExternalParticipant(
                ConsumeExternalCompletionParticipantGrantV1 {
                    coordinator_source,
                    certificate: certificate.clone(),
                    at: eligibility_time,
                },
            ),
        ))?;
        Ok(certificate)
    }

    fn consume_local(
        &mut self,
        month: u16,
        label: &str,
        namespace: &str,
        demand: &ResourceDemandId,
        leg: &ResourceAllocationLegVersionV1,
    ) -> Result<(), GrainLoopError> {
        let operation_key = operation_key(month, &format!("consume-{label}"))?;
        let consumption_id = ResourceConsumptionId::new(format!(
            "canwu.economy-reference:consumption:{label}:month-{month:02}"
        ))?;
        let account_revision = self.account_revision(&leg.account)?;
        let consumption_intent_id = canwu_resource::ResourceConsumptionIntentId::new(format!(
            "canwu.economy-reference:resource-consumption-intent:{label}:month-{month:02}"
        ))?;
        let intent = canwu_resource::ResourceConsumptionIntentV1 {
            id: consumption_intent_id.clone(),
            provider_plugin: crate::PLUGIN_NAME.to_owned(),
            demand: demand.clone(),
            demand_revision: self.demand_revision(demand)?,
            allocation: leg.clone(),
            account: leg.account.clone(),
            expected_account_revision: account_revision,
            consumption_id: consumption_id.clone(),
            operation_key: operation_key.clone(),
            quantity: leg.quantity,
            status: canwu_resource::ResourceConsumptionIntentStatusV1::Authorized,
            semantic_digest: String::new(),
        }
        .seal()?;
        self.submit_economy(EconomyOperationV1::AuthorizeResourceConsumption { intent })?;
        let evidence = self.current_exact(economy_reference_runtime_reference().into_untyped())?;
        let certificate = self.activate_lease(
            month,
            label,
            holder(1),
            namespace,
            operation_key.clone(),
            vec![
                CompletionLockedTargetV1::Account {
                    id: leg.account.clone(),
                    revision: account_revision,
                },
                CompletionLockedTargetV1::AllocationLeg {
                    id: leg.id.clone(),
                    revision: leg.revision,
                },
                CompletionLockedTargetV1::Demand {
                    id: demand.clone(),
                    revision: self.demand_revision(demand)?,
                },
                CompletionLockedTargetV1::ExternalRecord {
                    version: evidence.clone(),
                },
            ],
            self.canwu.time(),
        )?;
        self.adapter_resource(
            evidence.clone(),
            ResourceOperationRequestV1::Consume(ResourceConsumptionRequestV1 {
                operation_key: operation_key.clone(),
                consumption_id,
                allocation: leg.clone(),
                expected_account_revision: account_revision,
                consumer_evidence: evidence,
                at: self.canwu.time(),
                completion_certificate: certificate,
            }),
        )?;
        let outcome = resource_operation_outcome(&self.canwu, &operation_key)
            .filter(|outcome| outcome.status == ResourceOperationStatus::Applied)
            .ok_or_else(|| {
                GrainLoopError::Missing(format!("{label} consumption outcome was not applied"))
            })?
            .clone();
        let authoritative_resource_state =
            self.current_exact(resource_runtime_reference_untyped())?;
        self.submit_economy(EconomyOperationV1::RetireResourceConsumption {
            intent: consumption_intent_id,
            authoritative_resource_state,
            outcome_id: outcome.id,
        })?;
        Ok(())
    }

    fn credit_harvest(
        &mut self,
        month: u16,
    ) -> Result<ResourceOperationOutcomeVersionV1, GrainLoopError> {
        let (_, economy) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let local = &economy.local_economies[&self.economy];
        let quantity = HARVEST_BASE
            .saturating_mul(u64::from(local.cooperation_per_mille))
            .saturating_mul(u64::from(
                1_000_u16.saturating_sub(local.pending_harvest_penalty_per_mille),
            ))
            / 1_000_000;
        let source = self.current_exact(economy_reference_runtime_reference().into_untyped())?;
        let key = operation_key(month, "harvest-credit")?;
        let account_revision = self.account_revision(&self.granary_account)?;
        let certificate = self.activate_lease(
            month,
            "harvest",
            holder(1),
            NS_HARVEST_CREDIT,
            key.clone(),
            vec![
                CompletionLockedTargetV1::Account {
                    id: self.granary_account.clone(),
                    revision: account_revision,
                },
                CompletionLockedTargetV1::ExternalRecord {
                    version: source.clone(),
                },
            ],
            self.canwu.time(),
        )?;
        self.adapter_resource(
            source.clone(),
            ResourceOperationRequestV1::Credit(ResourceCreditRequestV1 {
                operation_key: key.clone(),
                account: self.granary_account.clone(),
                expected_account_revision: account_revision,
                resource_revision: self.grain_revision.clone(),
                unit_revision: self.unit_revision.clone(),
                quantity,
                // The grain fixture has no separate production runtime record;
                // model the seasonal harvest as an evidence-qualified inflow
                // from the economy reference while the production extension
                // remains independently testable and replaceable.
                source: ResourceCreditSourceV1::ExternalInflow(EvidenceRef::DomainRecordVersion(
                    source,
                )),
                at: self.canwu.time(),
                completion_certificate: certificate,
            }),
        )?;
        resource_operation_outcome(&self.canwu, &key)
            .filter(|outcome| outcome.status == ResourceOperationStatus::Applied)
            .ok_or_else(|| GrainLoopError::Missing("harvest credit outcome".to_owned()))
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn activate_lease(
        &mut self,
        month: u16,
        label: &str,
        authority: KnowledgeHolderRef,
        namespace: &str,
        operation_key: ResourceOperationKey,
        targets: Vec<CompletionLockedTargetV1>,
        eligibility_time: SimTime,
    ) -> Result<CompletionLeaseActivationCertificateV1, GrainLoopError> {
        let acquisition =
            CompletionLeaseAcquisitionId::new(format!("grain:lease:{label}:{month:02}"))?;
        let resource_grant =
            CompletionCapacityGrantId::new(format!("grain:grant:{label}:{month:02}"))?;
        let (resource_targets, economy_targets): (Vec<_>, Vec<_>) =
            targets.into_iter().partition(|target| {
                !matches!(
                    target,
                    CompletionLockedTargetV1::ExternalRecord { version }
                        if version.record.kind.namespace == crate::PLUGIN_NAMESPACE
                )
            });
        let (resource_targets, force_targets): (Vec<_>, Vec<_>) =
            resource_targets.into_iter().partition(|target| {
                !matches!(
                    target,
                    CompletionLockedTargetV1::ExternalRecord { version }
                        if version.record.kind.namespace
                            == canwu_force_supply_reference::PLUGIN_NAMESPACE
                )
            });
        let economy_grant = (!economy_targets.is_empty())
            .then(|| {
                CompletionCapacityGrantId::new(format!("grain:grant:{label}:economy:{month:02}"))
            })
            .transpose()?;
        let force_grant = (!force_targets.is_empty())
            .then(|| {
                CompletionCapacityGrantId::new(format!("grain:grant:{label}:force:{month:02}"))
            })
            .transpose()?;
        let exact_evidence = resource_targets
            .iter()
            .chain(economy_targets.iter())
            .chain(force_targets.iter())
            .filter_map(|target| match target {
                CompletionLockedTargetV1::ExternalRecord { version } => Some(version.clone()),
                _ => None,
            })
            .collect();
        let mut participants = BTreeSet::from([canwu_resource::PLUGIN_NAME.to_owned()]);
        if economy_grant.is_some() {
            participants.insert(crate::PLUGIN_NAME.to_owned());
        }
        if force_grant.is_some() {
            participants.insert(canwu_force_supply_reference::PLUGIN_NAME.to_owned());
        }
        let resource_targets_snapshot = resource_targets.clone();
        self.completion(ResourceOperationRequestV1::Completion(
            canwu_resource::ResourceCompletionOperationV1::Acquire(
                canwu_resource::RequestCompletionLeaseV1 {
                    id: acquisition.clone(),
                    operation_key,
                    holder: authority,
                    operation_namespace: namespace.to_owned(),
                    eligibility_time,
                    eligibility_envelope: EligibilityEnvelopeV1::new(
                        exact_evidence,
                        BTreeMap::new(),
                        BTreeSet::new(),
                        Vec::new(),
                        Vec::new(),
                    )?,
                    recipe: CompletionCapacityRecipeV1 {
                        receipts: canwu_resource::MAX_COMPLETION_RECEIPTS_PER_LIFECYCLE,
                        mutations: 4,
                        reports_per_holder: 0,
                        holders: 0,
                        bytes: 2_048,
                    },
                    expected_participants: participants,
                    policy_class: CompletionPolicyClassV1::Guaranteed,
                },
            ),
        ))?;
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let acquisition_revision = state
            .completion_leases
            .acquisitions
            .get(&acquisition)
            .map(|value| value.revision)
            .ok_or_else(|| {
                GrainLoopError::Missing(format!(
                    "completion acquisition {acquisition} was not persisted; latest outcome: {:?}",
                    state
                        .outcomes
                        .values()
                        .max_by_key(|outcome| outcome.sequence)
                ))
            })?;
        self.completion(ResourceOperationRequestV1::Completion(
            canwu_resource::ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                grant_id: resource_grant.clone(),
                acquisition: acquisition.clone(),
                expected_acquisition_revision: acquisition_revision,
                owner_plugin: canwu_resource::PLUGIN_NAME.to_owned(),
                target_versions: resource_targets,
                current_boundary: self.current_boundary(),
            }),
        ))?;
        if let Some(economy_grant) = &economy_grant {
            let (_, state) = resource_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
            self.completion(ResourceOperationRequestV1::Completion(
                canwu_resource::ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                    grant_id: economy_grant.clone(),
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    owner_plugin: crate::PLUGIN_NAME.to_owned(),
                    target_versions: economy_targets,
                    current_boundary: self.current_boundary(),
                }),
            ))?;
        }
        if let Some(force_grant) = &force_grant {
            let (_, state) = resource_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
            self.completion(ResourceOperationRequestV1::Completion(
                canwu_resource::ResourceCompletionOperationV1::Grant(GrantCompletionCapacityV1 {
                    grant_id: force_grant.clone(),
                    acquisition: acquisition.clone(),
                    expected_acquisition_revision: state.completion_leases.acquisitions
                        [&acquisition]
                        .revision,
                    owner_plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
                    target_versions: force_targets,
                    current_boundary: self.current_boundary(),
                }),
            ))?;
        }
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        let envelope_digest = state.completion_leases.acquisitions[&acquisition]
            .eligibility_envelope
            .digest
            .clone();
        let resource_grant_revision = state
            .completion_leases
            .grants
            .get(&resource_grant)
            .map(|value| value.revision)
            .ok_or_else(|| {
                GrainLoopError::Missing(format!(
                    "completion grant {resource_grant} was not persisted; latest outcome: {:?}",
                    (
                        state
                            .outcomes
                            .values()
                            .max_by_key(|outcome| outcome.sequence),
                        resource_targets_snapshot
                    )
                ))
            })?;
        self.completion(ResourceOperationRequestV1::Completion(
            canwu_resource::ResourceCompletionOperationV1::Prepare(PrepareCompletionCapacityV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition]
                    .revision,
                grant: resource_grant.clone(),
                expected_grant_revision: resource_grant_revision,
                current_boundary: self.current_boundary(),
                eligibility_envelope_digest: envelope_digest.clone(),
            }),
        ))?;
        if let Some(economy_grant) = &economy_grant {
            let (_, state) = resource_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
            self.completion(ResourceOperationRequestV1::Completion(
                canwu_resource::ResourceCompletionOperationV1::Prepare(
                    PrepareCompletionCapacityV1 {
                        acquisition: acquisition.clone(),
                        expected_acquisition_revision: state.completion_leases.acquisitions
                            [&acquisition]
                            .revision,
                        grant: economy_grant.clone(),
                        expected_grant_revision: state.completion_leases.grants[economy_grant]
                            .revision,
                        current_boundary: self.current_boundary(),
                        eligibility_envelope_digest: envelope_digest.clone(),
                    },
                ),
            ))?;
        }
        if let Some(force_grant) = &force_grant {
            let (_, state) = resource_state(&self.canwu)?
                .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
            self.completion(ResourceOperationRequestV1::Completion(
                canwu_resource::ResourceCompletionOperationV1::Prepare(
                    PrepareCompletionCapacityV1 {
                        acquisition: acquisition.clone(),
                        expected_acquisition_revision: state.completion_leases.acquisitions
                            [&acquisition]
                            .revision,
                        grant: force_grant.clone(),
                        expected_grant_revision: state.completion_leases.grants[force_grant]
                            .revision,
                        current_boundary: self.current_boundary(),
                        eligibility_envelope_digest: envelope_digest.clone(),
                    },
                ),
            ))?;
        }
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        self.completion(ResourceOperationRequestV1::Completion(
            canwu_resource::ResourceCompletionOperationV1::Activate(ActivateCompletionLeaseV1 {
                acquisition: acquisition.clone(),
                expected_acquisition_revision: state.completion_leases.acquisitions[&acquisition]
                    .revision,
                grant: resource_grant.clone(),
                expected_grant_revision: state.completion_leases.grants[&resource_grant].revision,
                at: self.canwu.time(),
                current_boundary: self.current_boundary(),
                eligibility_envelope_digest: envelope_digest,
            }),
        ))?;
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        state
            .completion_leases
            .certificates
            .get(&acquisition)
            .cloned()
            .ok_or_else(|| GrainLoopError::Missing("activated completion certificate".to_owned()))
    }

    fn completion(&mut self, request: ResourceOperationRequestV1) -> Result<(), GrainLoopError> {
        let ResourceOperationRequestV1::Completion(operation) = request else {
            unreachable!()
        };
        let now = self.canwu.time();
        enqueue_resource_completion_operation(&mut self.canwu, now, &operation)?;
        self.settle_current(&[])
    }

    fn submit_resource(
        &mut self,
        subject: KnowledgeHolderRef,
        request: ResourceOperationRequestV1,
    ) -> Result<(), GrainLoopError> {
        let command = resource_command(&ResourceCommandV1 {
            subject: subject.clone(),
            request,
        })
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        self.process_command(subject, command)?;
        self.settle_current(&[])?;
        self.settle_current(&[])
    }

    fn adapter_resource(
        &mut self,
        provider_source: DomainRecordVersionRef,
        request: ResourceOperationRequestV1,
    ) -> Result<(), GrainLoopError> {
        let provider_plugin = self
            .canwu
            .domain_record_version(&provider_source)
            .ok_or_else(|| GrainLoopError::Missing("resource adapter provider body".to_owned()))?
            .owner;
        let adapter_operation_key = request.operation_key();
        let now = self.canwu.time();
        enqueue_resource_adapter_operation(
            &mut self.canwu,
            now,
            &ResourceAdapterOperationV1 {
                provider_plugin,
                provider_source,
                request,
            },
        )?;
        self.settle_current(&[])?;
        self.settle_current(&[]).map_err(|error| {
            GrainLoopError::Canwu(format!(
                "resource adapter operation {adapter_operation_key} failed: {error}"
            ))
        })
    }

    fn submit_economy(&mut self, operation: EconomyOperationV1) -> Result<(), GrainLoopError> {
        let now = self.canwu.time();
        self.submit_economy_at(operation, now, &[])
    }

    fn enqueue_economy_as(
        &mut self,
        subject: KnowledgeHolderRef,
        operation: EconomyOperationV1,
        at: SimTime,
    ) -> Result<(), GrainLoopError> {
        let (_, state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let command = economy_command(&EconomyCommandV1 {
            holder: subject.clone(),
            operation_id: EconomyOperationId::new(format!(
                "canwu.economy-reference:operation:{}:{}",
                self.canwu.revision(),
                at.as_minutes(),
            ))?,
            expected_runtime_revision: state.revision,
            operation,
        })
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        self.process_command_at(subject, command, at)
    }

    fn submit_economy_at(
        &mut self,
        operation: EconomyOperationV1,
        at: SimTime,
        cadences: &[SystemCadence],
    ) -> Result<(), GrainLoopError> {
        let (_, state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        let command = economy_command(&EconomyCommandV1 {
            holder: holder(1),
            operation_id: EconomyOperationId::new(format!(
                "canwu.economy-reference:operation:{}:{}",
                self.canwu.revision(),
                at.as_minutes(),
            ))?,
            expected_runtime_revision: state.revision,
            operation,
        })
        .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
        self.process_command_at(holder(1), command, at)?;
        self.settle_at(at, cadences)?;
        self.settle_at(at, cadences)
    }

    fn submit_force(
        &mut self,
        operation: ForceOperationV1,
    ) -> Result<ForceOperationId, GrainLoopError> {
        let state = self.force_state()?;
        let operation_id = ForceOperationId::new(format!(
            "canwu.force-supply-reference:operation:{}:{}",
            self.canwu.revision(),
            self.canwu.time().as_minutes(),
        ))?;
        let command = Command::Plugin {
            plugin: canwu_force_supply_reference::PLUGIN_NAME.to_owned(),
            command: FORCE_SUPPLY_COMMAND.to_owned(),
            payload: serde_json::to_value(ForceCommandEnvelopeV1 {
                operation_id: operation_id.clone(),
                holder: holder(2),
                expected_runtime_revision: state.revision,
                operation,
            })
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?,
        };
        self.process_command(holder(2), command)?;
        self.settle_current(&[])?;
        self.settle_current(&[])?;
        Ok(operation_id)
    }

    fn submit_force_batch(
        &mut self,
        operations: Vec<ForceOperationV1>,
    ) -> Result<Vec<ForceOperationId>, GrainLoopError> {
        let runtime_revision = self.force_state()?.revision;
        let request_revision = self.canwu.revision();
        let now = self.canwu.time();
        let mut ids = Vec::with_capacity(operations.len());
        for (index, operation) in operations.into_iter().enumerate() {
            let ordinal = u64::try_from(index).map_err(|_| GrainLoopError::ArithmeticOverflow)?;
            let operation_id = ForceOperationId::new(format!(
                "canwu.force-supply-reference:operation:{request_revision}:{}:batch-{ordinal}",
                now.as_minutes(),
            ))?;
            let command = force_supply_command(&ForceCommandEnvelopeV1 {
                operation_id: operation_id.clone(),
                holder: holder(2),
                expected_runtime_revision: runtime_revision,
                operation,
            })
            .map_err(|error| GrainLoopError::Canwu(error.to_string()))?;
            let KnowledgeHolderRef::Person(actor) = holder(2) else {
                unreachable!()
            };
            self.canwu.enqueue_command(
                now,
                0,
                CommandRequest::new(
                    CommandRequestId::new(request_revision.saturating_add(ordinal + 1)),
                    request_revision.saturating_add(ordinal),
                    CommandEnvelope::new(Issuer::Actor(actor), command).at_time(now),
                ),
            )?;
            ids.push(operation_id);
        }
        self.settle_current(&[])?;
        self.settle_current(&[])?;
        Ok(ids)
    }

    fn process_command(
        &mut self,
        issuer: KnowledgeHolderRef,
        command: Command,
    ) -> Result<(), GrainLoopError> {
        self.process_command_at(issuer, command, self.canwu.time())
    }

    #[allow(clippy::needless_pass_by_value)]
    fn process_command_at(
        &mut self,
        issuer: KnowledgeHolderRef,
        command: Command,
        at: SimTime,
    ) -> Result<(), GrainLoopError> {
        let KnowledgeHolderRef::Person(actor) = issuer else {
            return Err(GrainLoopError::Rejected(
                "grain harness uses person authority".to_owned(),
            ));
        };
        self.canwu.enqueue_command(
            at,
            0,
            CommandRequest::new(
                CommandRequestId::new(self.canwu.revision().saturating_add(1)),
                self.canwu.revision(),
                CommandEnvelope::new(Issuer::Actor(actor), command).at_time(at),
            ),
        )?;
        Ok(())
    }

    fn record_delivery(
        &mut self,
        month: u16,
        transfer: &ResourceTransferId,
        disposition: DeliveryDispositionV1,
        execution: &TransportExecution,
        source_versions: Vec<DomainRecordVersionRef>,
    ) -> Result<(), GrainLoopError> {
        self.submit_economy(EconomyOperationV1::RecordDeliveryAttempt {
            attempt: EconomyDeliveryAttemptV1 {
                id: EconomyDeliveryAttemptId::new(format!(
                    "canwu.economy-reference:delivery-attempt:month-{month:02}"
                ))?,
                economy: self.economy.clone(),
                resource_transfer: transfer.clone(),
                source_scope: self.scope.clone(),
                target_scope: ResourceScopeId::new("canwu.economy-reference:scope:field-force")?,
                disposition,
                execution: execution.clone(),
                recorded_at: self.canwu.time(),
                source_versions,
                semantic_digest: String::new(),
            },
        })
    }

    fn transport_execution(
        &self,
        month: u16,
        _reroute: bool,
    ) -> Result<TransportExecution, GrainLoopError> {
        let mut execution = TransportExecution::new(TransportExecutionId(u64::from(month)), None);
        execution
            .install_initial_itinerary(ItineraryRevision {
                id: ItineraryRevisionId(1),
                predecessor: None,
                plan: route_plan(self.canwu.time(), month, false)?,
                planned_at: self.canwu.time(),
                valid_from: self.canwu.time(),
                reason: ItineraryRevisionReason::Initial,
                superseded_at: None,
                evidence: Vec::new(),
            })
            .map_err(transport_error)?;
        Ok(execution)
    }

    fn advance_daily_to(&mut self, target: SimTime) -> Result<(), GrainLoopError> {
        while self.canwu.time() < target {
            let next = (self.canwu.time() + DAY).min(target);
            self.settle_at(next, &[SystemCadence::Daily])?;
        }
        Ok(())
    }

    fn settle_current(&mut self, cadences: &[SystemCadence]) -> Result<(), GrainLoopError> {
        self.settle_at(self.canwu.time(), cadences)
    }

    fn settle_at(&mut self, at: SimTime, cadences: &[SystemCadence]) -> Result<(), GrainLoopError> {
        let mut request = BoundaryRequest::at(at);
        for cadence in cadences {
            request = request.with_cadence(cadence.clone());
        }
        self.canwu.settle_boundary(request)?;
        Ok(())
    }

    #[allow(clippy::needless_pass_by_value)]
    fn current_exact(
        &self,
        reference: DomainRecordRef,
    ) -> Result<DomainRecordVersionRef, GrainLoopError> {
        self.canwu
            .current_domain_record_version(&reference)?
            .ok_or_else(|| {
                GrainLoopError::Missing(format!(
                    "exact current domain-record provenance is unavailable for {reference}"
                ))
            })
    }

    fn current_boundary(&self) -> u64 {
        self.canwu
            .boundaries()
            .last()
            .map_or(1, |boundary| boundary.id.get())
    }

    fn force_state(&self) -> Result<ForceSupplyStateV1, GrainLoopError> {
        self.canwu
            .typed_domain_record(&force_supply_runtime_reference())
            .ok_or_else(|| GrainLoopError::Missing("force runtime".to_owned()))?
            .decode_payload::<ForceSupplyRuntimeRecord>()
            .map_err(Into::into)
    }

    fn force_posture_decision(&self) -> Result<GrainDecision, GrainLoopError> {
        match self.force_state()?.forces[&self.force]
            .supply_posture
            .as_str()
        {
            "wait_for_supply" => Ok(GrainDecision::ReliefFirst),
            "advance_immediately" => Ok(GrainDecision::ForceFirst),
            "requisition_locally" => Ok(GrainDecision::RequisitionForForce),
            posture => Err(GrainLoopError::Rejected(format!(
                "resolved force decision produced unsupported posture {posture}"
            ))),
        }
    }

    fn resilience_decision(&self) -> Result<GrainDecision, GrainLoopError> {
        let (_, state) = economy_reference_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("economy runtime".to_owned()))?;
        match state
            .resilience_postures
            .get(&self.economy)
            .map(String::as_str)
        {
            Some("release_reserves") => Ok(GrainDecision::ReliefFirst),
            Some("keep_buffer") => Ok(GrainDecision::Balanced),
            Some("ration") => Ok(GrainDecision::RequisitionForForce),
            Some("dispatch_remote_transfer") => Ok(GrainDecision::ForceFirst),
            Some(posture) => Err(GrainLoopError::Rejected(format!(
                "resolved resilience decision produced unsupported posture {posture}"
            ))),
            None => Err(GrainLoopError::Missing(
                "resolved resilience posture".to_owned(),
            )),
        }
    }

    fn account_revision(&self, id: &ResourceAccountId) -> Result<ResourceRevision, GrainLoopError> {
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        state
            .accounts
            .get(id)
            .map(|account| account.revision)
            .ok_or_else(|| GrainLoopError::Missing(format!("resource account {id}")))
    }

    fn transfer_revision(
        &self,
        id: &ResourceTransferId,
    ) -> Result<ResourceRevision, GrainLoopError> {
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        state
            .transfers
            .get(id)
            .map(|transfer| transfer.revision)
            .ok_or_else(|| GrainLoopError::Missing(format!("resource transfer {id}")))
    }

    fn demand_revision(&self, id: &ResourceDemandId) -> Result<ResourceRevision, GrainLoopError> {
        let (_, state) = resource_state(&self.canwu)?
            .ok_or_else(|| GrainLoopError::Missing("resource runtime".to_owned()))?;
        state
            .demands
            .get(id)
            .map(|demand| demand.revision)
            .ok_or_else(|| GrainLoopError::Missing(format!("resource demand {id}")))
    }
}

fn validate_reference_archive_stores(
    canwu: &Canwu,
    store: &GrainArchiveStore,
) -> Result<(), GrainLoopError> {
    canwu_force_supply_reference::validate_force_supply_runtime_with_archive_store(canwu, store)?;
    crate::validate_economy_reference_runtime_with_archive_store(canwu, store)?;
    Ok(())
}

fn new_canwu(seed: u64, scenario: Scenario) -> Result<Canwu, CanwuError> {
    let economy_kind = economy_reference_runtime_reference().into_untyped().kind;
    let force_kind = force_supply_runtime_reference().into_untyped().kind;
    let resource = ResourcePlugin::new([economy_kind, force_kind]);
    let economy = EconomyReferencePlugin;
    let force = ForceSupplyReferencePlugin;
    let plugins: [&dyn SimulationPlugin; 3] = [&resource, &economy, &force];
    Canwu::new_with_plugins(seed, scenario, &plugins)
}

fn register_grain_decision_controller(canwu: &mut Canwu) -> Result<(), CanwuError> {
    for (request_id, controller, policy, actor) in [
        (
            9_998,
            force_decision_controller_id(),
            force_decision_policy(),
            PersonId::new(2),
        ),
        (
            9_999,
            grain_decision_controller_id(),
            grain_decision_policy(),
            PersonId::new(1),
        ),
    ] {
        canwu.enqueue_decision(
            canwu.time(),
            0,
            DecisionIngressRequest::new(
                DecisionRequestId::new(request_id),
                canwu.revision(),
                DecisionMutation::RegisterController {
                    controller: DecisionControllerBinding::new(
                        controller,
                        policy,
                        DecisionAuthority::Actor { actor },
                    ),
                },
            ),
        )?;
        canwu.settle_boundary(BoundaryRequest::at(canwu.time()))?;
    }
    Ok(())
}

fn grain_decision_controller_id() -> &'static str {
    "canwu.economy-reference:controller:grain-manager"
}

fn grain_decision_policy() -> DecisionPolicyIdentity {
    DecisionPolicyIdentity::new(
        DecisionPolicyKind::Human,
        "canwu.economy-reference:policy:grain-manager",
        "1",
    )
}

fn force_decision_controller_id() -> &'static str {
    "canwu.force-supply-reference:controller:field-commander"
}

fn force_decision_policy() -> DecisionPolicyIdentity {
    DecisionPolicyIdentity::new(
        DecisionPolicyKind::Human,
        "canwu.force-supply-reference:policy:field-commander",
        "1",
    )
}

const fn decision_option_id(decision: GrainDecision) -> &'static str {
    match decision {
        GrainDecision::ReliefFirst => "relief_first",
        GrainDecision::ForceFirst => "force_first",
        GrainDecision::Balanced => "balanced",
        GrainDecision::RequisitionForForce => "requisition_for_force",
    }
}

fn grain_decision_from_option(option: &str) -> Result<GrainDecision, GrainLoopError> {
    match option {
        "relief_first" => Ok(GrainDecision::ReliefFirst),
        "force_first" => Ok(GrainDecision::ForceFirst),
        "balanced" => Ok(GrainDecision::Balanced),
        "requisition_for_force" => Ok(GrainDecision::RequisitionForForce),
        _ => Err(GrainLoopError::Rejected(
            "resolved grain ticket selected an unknown option".to_owned(),
        )),
    }
}

const fn decision_option_label(decision: GrainDecision) -> &'static str {
    match decision {
        GrainDecision::ReliefFirst => "Relief first",
        GrainDecision::ForceFirst => "Force first",
        GrainDecision::Balanced => "Balanced rationing",
        GrainDecision::RequisitionForForce => "Requisition for the force",
    }
}

fn load_snapshot(json: &str) -> Result<Canwu, CanwuError> {
    let economy_kind = economy_reference_runtime_reference().into_untyped().kind;
    let force_kind = force_supply_runtime_reference().into_untyped().kind;
    let resource = ResourcePlugin::new([economy_kind, force_kind]);
    let economy = EconomyReferencePlugin;
    let force = ForceSupplyReferencePlugin;
    Canwu::from_snapshot_json_with_plugins(json, &[&resource, &economy, &force])
}

fn replay_journal(json: &str, archive_store: Rc<GrainArchiveStore>) -> Result<Canwu, CanwuError> {
    let economy_kind = economy_reference_runtime_reference().into_untyped().kind;
    let force_kind = force_supply_runtime_reference().into_untyped().kind;
    let resource = ResourcePlugin::new([economy_kind, force_kind]);
    let economy = EconomyReferencePlugin;
    let force = ForceSupplyReferencePlugin;
    let journal: canwu_api::ReplayJournal = serde_json::from_str(json).map_err(|error| {
        CanwuError::new(canwu_api::ErrorCode::InvalidPayload, error.to_string())
    })?;
    Canwu::replay_from_journal_with_archive_provider(
        &[&resource, &economy, &force],
        &journal,
        archive_store,
    )
}

fn completion_budget(
    manager: &KnowledgeHolderRef,
    force: &KnowledgeHolderRef,
) -> Result<RunBudgetRevisionV1, ResourceError> {
    let partitions = [
        (manager.clone(), NS_TRANSFER_BEGIN),
        (manager.clone(), NS_TRANSFER_ACCEPT),
        (manager.clone(), NS_CIVILIAN_CONSUME),
        (manager.clone(), NS_HARVEST_CREDIT),
        (force.clone(), FORCE_COMPLETION_NAMESPACE),
        (manager.clone(), NS_RELIEF_CONSUME),
    ]
    .into_iter()
    .map(
        |(authority, operation_namespace)| CompletionCapacityPartitionV1 {
            authority,
            operation_namespace: operation_namespace.to_owned(),
            guaranteed_units: 100_000,
            reserved_pending_slots: 4,
            maximum_burst_units: 50_000,
            request_token_capacity: 16,
            request_token_refill_minutes: 1,
            reacquire_cooldown_minutes: 1,
            root_acquisition_cap_per_sim_time: 4,
            guaranteed_max_wait_boundaries: 4,
        },
    )
    .collect();
    RunBudgetRevisionV1 {
        revision: ResourceRevision::INITIAL,
        total_completion_units: 1_000_000,
        shared_pending_slots: 8,
        partitions,
        semantic_digest: String::new(),
    }
    .seal()
}

fn route_plan(
    at: SimTime,
    month: u16,
    alternate: bool,
) -> Result<canwu_routing::RoutePlan, GrainLoopError> {
    let origin = RoutingNodeRef::new("river-granary");
    let relay = RoutingNodeRef::new(if alternate {
        "ridge-post"
    } else {
        "river-port"
    });
    let destination = RoutingNodeRef::new("field-force");
    let mode = if alternate {
        TransferMode::RoadVehicle
    } else {
        TransferMode::RiverBoat
    };
    let version = format!(
        "grain-topology-{month:02}-{}",
        if alternate { "alternate" } else { "primary" }
    );
    let network = RoutingNetwork::new(
        version.clone(),
        vec![
            RoutingEndpoint {
                id: origin.clone(),
                kind: RoutingEndpointKind::Settlement,
            },
            RoutingEndpoint {
                id: relay.clone(),
                kind: RoutingEndpointKind::RelayStation,
            },
            RoutingEndpoint {
                id: destination.clone(),
                kind: RoutingEndpointKind::MilitaryPosition,
            },
        ],
        vec![
            RoutingConnection {
                id: RoutingConnectionRef::new(format!("grain-leg-{month:02}-1")),
                from: origin.clone(),
                to: relay.clone(),
                mode,
                traversal: TraversalModel::Fixed { duration: DAY },
                available_from: Some(at),
                available_until: Some(at + MONTH),
                risk_per_mille: if alternate { 120 } else { 40 },
                resource_cost: if alternate { 18 } else { 8 },
            },
            RoutingConnection {
                id: RoutingConnectionRef::new(format!("grain-leg-{month:02}-2")),
                from: relay,
                to: destination.clone(),
                mode: TransferMode::RoadVehicle,
                traversal: TraversalModel::Fixed { duration: DAY },
                available_from: Some(at),
                available_until: Some(at + MONTH),
                risk_per_mille: 60,
                resource_cost: 10,
            },
        ],
    )
    .map_err(|error| GrainLoopError::Routing(format!("{error:?}")))?;
    let snapshot = PlanningSnapshot {
        observer: "river-granary-dispatch".to_owned(),
        observed_at: at,
        valid_until: Some(at + MONTH),
        knowledge_cut: format!("grain-route-month-{month:02}"),
        topology_version: version,
        timetable_version: None,
        network,
    };
    plan_route(
        &snapshot,
        &RoutingRequest {
            origin,
            destination,
            departure_at: at,
            policy: RoutingPolicy::default(),
        },
    )
    .map_err(|error| GrainLoopError::Routing(format!("{error:?}")))
}

fn active_transport_link(
    execution: &TransportExecution,
) -> Result<TransportExecutionLink, GrainLoopError> {
    let itinerary = execution
        .active_itinerary_revision
        .ok_or_else(|| GrainLoopError::Missing("active itinerary".to_owned()))?;
    let leg = execution
        .legs
        .iter()
        .find(|leg| {
            leg.itinerary_revision == itinerary
                && leg.leg_index == execution.current_leg_index.min(1)
        })
        .or_else(|| {
            execution
                .legs
                .iter()
                .rev()
                .find(|leg| leg.itinerary_revision == itinerary)
        })
        .ok_or_else(|| GrainLoopError::Missing("active transport leg".to_owned()))?;
    Ok(TransportExecutionLink {
        execution: execution.id,
        itinerary_revision: itinerary,
        leg_execution: Some(leg.id),
        handoff: execution.handoffs.last().map(|value| value.id),
        capacity_booking: execution.bookings.last().map(|booking| booking.id),
    })
}

fn resource_runtime_reference_untyped() -> DomainRecordRef {
    canwu_resource::resource_runtime_reference().into_untyped()
}

fn holder(value: u64) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(value))
}

fn operation_key(month: u16, label: &str) -> Result<ResourceOperationKey, ResourceError> {
    ResourceOperationKey::new(format!(
        "canwu.economy-reference:grain:{label}:month-{month:02}"
    ))
}

fn grain_demand_id(month: u16, label: &str) -> Result<ResourceDemandId, ResourceError> {
    ResourceDemandId::new(format!(
        "canwu.economy-reference:demand:{label}:month-{month:02}"
    ))
}

fn decision_priority(decision: GrainDecision, label: &str) -> i32 {
    match (decision, label) {
        (GrainDecision::ReliefFirst, "relief")
        | (GrainDecision::ForceFirst | GrainDecision::RequisitionForForce, "force") => 120,
        (_, "civilian") => 100,
        (_, "relief") => 90,
        _ => 80,
    }
}

fn route_month_from_id(value: &str) -> Option<u16> {
    value.rsplit_once("month-")?.1.parse().ok()
}

fn delivery_month_from_id(value: &str) -> Option<u16> {
    route_month_from_id(value)
}

fn digest_label(label: &str) -> String {
    blake3::hash(label.as_bytes()).to_hex().to_string()
}

#[allow(clippy::needless_pass_by_value)]
fn transport_error(error: canwu_transport::TransportError) -> GrainLoopError {
    GrainLoopError::Transport(format!("{error:?}"))
}
