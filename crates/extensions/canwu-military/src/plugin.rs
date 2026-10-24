//! The plugin: registration, the registry, activation, system-name
//! encodings, and the host helpers.

use crate::command::{MilitaryCommandV1, admit_command};
use crate::ids::{
    CrossingId, EngagementId, ForceRecord, RegistryRecord, RulebookId, RulebookRecord,
    SCHEMA_VERSION, SiegeId, invalid, record_from, registry_reference, rulebook_reference,
};
use crate::report::{ReportItemV1, publish_engagement_reports, report_knowledge_schemas};
use crate::resolver::{ParametricStepResolver, StepResolver};
use crate::rulebook::CombatRulebookV1;
use crate::settle::{settle_engagements, wake_engagements};
use crate::{
    CombatLimitsV1, CrossingRecord, CrossingStateV1, EngagementRecord, EngagementStateV1,
    EngagementStepsRecord, EngagementStepsV1, EngagementSummaryRecord, EngagementSummaryV1,
    ForceStateV1, ForceStatus, PLUGIN_NAME, SiegeRecord, SiegeStateV1, force_reference,
};
use canwu_api::{
    BoundaryPhase, BoundarySystemContract, CanwuError, Command, DomainRecord, DomainRecordKind,
    DomainRecordLifecycle, DomainRecordMutationPolicy, DomainRecordRef, DomainRecordSchema,
    DomainRecordType, ErrorCode, IngressClass, KnowledgeHolderPolicy,
    KnowledgePublicationReservation, KnowledgeWriteGrant, PayloadSchema, PluginActionDescriptor,
    PluginIngressDescriptor, PluginIngressTarget, PluginRegistrar, RandomStreamKey, SimTime,
    SimulationPlugin, StateKey, StateVisibility, SystemCadence,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::marker::PhantomData;

/// The host action that carries every [`MilitaryCommandV1`].
pub const MILITARY_COMMAND: &str = "military_command_v1";
/// The internal packet that applies one admitted command in phase 7.
pub const MILITARY_APPLY_INGRESS: &str = "military_apply_v1";
/// The internal packet that steps the work due by its `up_to`.
pub const MILITARY_STEP_INGRESS: &str = "military_step_v1";

/// A command refused in phase 7: a recorded rejection; the boundary stands.
pub const MILITARY_REJECTION_EVENT: &str = "canwu.military.command_rejected.v1";
pub const ENGAGEMENT_OPENED_EVENT: &str = "canwu.military.engagement_opened.v1";
pub const ENGAGEMENT_CLOSED_EVENT: &str = "canwu.military.engagement_closed.v1";
/// An engagement that stopped stepping (an oversized step, a failing
/// resolver, too many draws, a missing input) until the host revises it.
pub const ENGAGEMENT_HELD_EVENT: &str = "canwu.military.engagement_held.v1";
pub const SIEGE_ENDED_EVENT: &str = "canwu.military.siege_ended.v1";
pub const CROSSING_WAVE_EVENT: &str = "canwu.military.crossing_wave.v1";
pub const REPORT_DROPPED_EVENT: &str = "canwu.military.report_dropped.v1";

/// The step terms the plugin itself adds.
pub const TERM_RESOLVER_CLAMPED: &str = "canwu.military:resolver-clamped";
pub const TERM_RESOLVER_FAILED: &str = "canwu.military:resolver-failed";
pub const TERM_PURSUIT: &str = "canwu.military:pursuit";

pub(crate) const PLUGIN_VERSION: &str = "0.20.0";
pub(crate) const SETTLE_SYSTEM: &str = "settle-engagements-v1";
pub(crate) const WAKE_SYSTEM: &str = "wake-engagements-on-cadence-v1";
pub(crate) const PUBLISH_SYSTEM: &str = "publish-engagement-reports-v1";
const SIGNAL_INFIX: &str = "/signal/";

/// A record retired at `at` (without a successor).
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RetireItemV1 {
    pub at: SimTime,
    pub record: DomainRecordRef,
}

/// The registry of open work and queues (record `canwu.military:registry`,
/// id `root`).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CombatRegistryV1 {
    pub schema_version: u32,
    pub rulebook: Option<RulebookId>,
    /// Registered forces not retired.
    pub force_count: u32,
    pub open_engagements: BTreeSet<EngagementId>,
    pub open_sieges: BTreeSet<SiegeId>,
    pub open_crossings: BTreeSet<CrossingId>,
    /// The earliest time open work falls due (a step, a siege day, a wave,
    /// a retirement, or a steps record to empty).
    pub earliest_due: Option<SimTime>,
    /// Ascending by (time, record).
    pub retire_queue: Vec<RetireItemV1>,
    /// Closed engagements whose steps record the next stepping boundary
    /// rewrites empty (their steps are in the journal).
    pub steps_to_empty: BTreeSet<EngagementId>,
    pub report_queue: Vec<ReportItemV1>,
    /// The sequence of the identifiers the plugin derives.
    pub next_sequence: u64,
}

impl CombatRegistryV1 {
    #[must_use]
    pub const fn new(rulebook: Option<RulebookId>) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            rulebook,
            force_count: 0,
            open_engagements: BTreeSet::new(),
            open_sieges: BTreeSet::new(),
            open_crossings: BTreeSet::new(),
            earliest_due: None,
            retire_queue: Vec::new(),
            steps_to_empty: BTreeSet::new(),
            report_queue: Vec::new(),
            next_sequence: 0,
        }
    }

    /// Checks the registry against the rulebook's limits.
    pub fn validate(&self, limits: &CombatLimitsV1) -> Result<(), CanwuError> {
        if self.schema_version != SCHEMA_VERSION
            || self.force_count > limits.forces
            || self.open_engagements.len() > limits.open_engagements as usize
            || self.open_sieges.len() > limits.open_sieges as usize
            || self.open_crossings.len() > limits.open_crossings as usize
            || self.report_queue.len() > limits.report_queue_bound()
            || self.retire_queue.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(invalid("military registry exceeds its limits"));
        }
        Ok(())
    }
}

/// The engagement engine plugin, generic over its step resolver.
///
/// Boundary handlers are `fn` pointers, so the resolver is a type: the
/// settle system is `settle::<R>`, which builds `R::default()` per step.
/// The plugin's semantic hash names `R::IDENTITY`, and activation refuses a
/// rulebook that names another resolver.
///
/// The host's choices (the wake cadence, the outcome-signal target, the
/// publication reservation) are in the plugin descriptor (the wake system's
/// contract, the settle system's name, the registrar's reservation), so a
/// run restores and replays only with a plugin configured the same way.
pub struct MilitaryPlugin<R: StepResolver = ParametricStepResolver> {
    step_cadence: Option<SystemCadence>,
    outcome_signal: Option<PluginIngressTarget>,
    reservation: Option<KnowledgePublicationReservation>,
    semantic_hash: String,
    resolver: PhantomData<fn() -> R>,
}

impl<R: StepResolver> Clone for MilitaryPlugin<R> {
    fn clone(&self) -> Self {
        Self {
            step_cadence: self.step_cadence.clone(),
            outcome_signal: self.outcome_signal.clone(),
            reservation: self.reservation,
            semantic_hash: self.semantic_hash.clone(),
            resolver: PhantomData,
        }
    }
}

impl<R: StepResolver> std::fmt::Debug for MilitaryPlugin<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MilitaryPlugin")
            .field("resolver", &R::IDENTITY)
            .field("step_cadence", &self.step_cadence)
            .field("outcome_signal", &self.outcome_signal)
            .field("reservation", &self.reservation)
            .finish_non_exhaustive()
    }
}

impl<R: StepResolver> Default for MilitaryPlugin<R> {
    fn default() -> Self {
        Self::new()
    }
}

/// The plugin with the parametric resolver and the daily wake cadence.
#[must_use]
pub fn military_plugin() -> MilitaryPlugin<ParametricStepResolver> {
    MilitaryPlugin::new()
}

impl<R: StepResolver> MilitaryPlugin<R> {
    /// The plugin with resolver `R`, waking on `SystemCadence::Daily`
    /// boundaries, with no outcome signal and no publication reservation.
    #[must_use]
    pub fn new() -> Self {
        let identity = R::IDENTITY;
        let text = format!(
            "canwu-military plugin semantic hash {PLUGIN_VERSION}: engagements {}@{}",
            identity.name, identity.version
        );
        Self {
            step_cadence: Some(SystemCadence::Daily),
            outcome_signal: None,
            reservation: None,
            semantic_hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
            resolver: PhantomData,
        }
    }

    /// Chooses who produces the step packet. With `Some(cadence)` the wake
    /// system (`wake-engagements-on-cadence-v1`, phase 7) schedules a step
    /// packet, due at once, in every boundary of `cadence` whose registry
    /// has work due, so the next boundary at the same instant steps it. With
    /// `None` the host drives stepping with the trusted `Step { up_to }`
    /// command, sent after its inputs in one admission order, which adds no
    /// boundary. Registration refuses `SystemCadence::EventDriven`.
    #[must_use]
    pub fn with_step_cadence(mut self, cadence: Option<SystemCadence>) -> Self {
        self.step_cadence = cadence;
        self
    }

    /// Sends each closed engagement's [`crate::EngagementOutcomeSignalV1`] as
    /// next-boundary plugin ingress to `packet_type` of `target_plugin`.
    ///
    /// The target must be an ingress that plugin registered: the engine
    /// finds an undeclared target only when the first signal is scheduled,
    /// and fails that boundary.
    #[must_use]
    pub fn with_outcome_signal(
        mut self,
        target_plugin: impl Into<String>,
        packet_type: impl Into<String>,
    ) -> Self {
        self.outcome_signal = Some(PluginIngressTarget {
            target_plugin: target_plugin.into(),
            packet_type: packet_type.into(),
        });
        self
    }

    /// Holds `reservation` of every boundary's knowledge publication budget
    /// for the report system, whatever runs before it.
    #[must_use]
    pub fn with_publication_reservation(
        mut self,
        reservation: KnowledgePublicationReservation,
    ) -> Self {
        self.reservation = Some(reservation);
        self
    }

    #[must_use]
    pub fn step_cadence(&self) -> Option<SystemCadence> {
        self.step_cadence.clone()
    }

    #[must_use]
    pub fn outcome_signal(&self) -> Option<&PluginIngressTarget> {
        self.outcome_signal.as_ref()
    }
}

/// The settle system's name, carrying the outcome-signal target.
fn settle_system_name(signal: Option<&PluginIngressTarget>) -> String {
    match signal {
        None => SETTLE_SYSTEM.to_owned(),
        Some(target) => format!(
            "{SETTLE_SYSTEM}{SIGNAL_INFIX}{}/{}",
            target.target_plugin, target.packet_type
        ),
    }
}

/// The outcome-signal target a settle system's name carries.
pub(crate) fn signal_target(system: &str) -> Result<Option<PluginIngressTarget>, CanwuError> {
    let Some(rest) = system.strip_prefix(SETTLE_SYSTEM) else {
        return Err(invalid("the military settle system has an unknown name"));
    };
    if rest.is_empty() {
        return Ok(None);
    }
    let Some((plugin, packet)) = rest
        .strip_prefix(SIGNAL_INFIX)
        .and_then(|target| target.split_once('/'))
    else {
        return Err(invalid(
            "the military settle system's name carries no target",
        ));
    };
    Ok(Some(PluginIngressTarget {
        target_plugin: plugin.to_owned(),
        packet_type: packet.to_owned(),
    }))
}

fn kind_key<T: DomainRecordType>() -> StateKey {
    DomainRecordSchema::for_type::<T>().state_key()
}

/// The state keys of the plugin's record kinds.
pub(crate) fn record_keys() -> Vec<StateKey> {
    let mut keys = vec![
        kind_key::<RulebookRecord>(),
        kind_key::<RegistryRecord>(),
        kind_key::<ForceRecord>(),
        kind_key::<EngagementRecord>(),
        kind_key::<EngagementStepsRecord>(),
        kind_key::<EngagementSummaryRecord>(),
        kind_key::<SiegeRecord>(),
        kind_key::<CrossingRecord>(),
    ];
    keys.sort();
    keys
}

/// The stream of keyed interception draws.
#[must_use]
pub fn crossing_interception_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN_NAME, "crossing-interception", 1)
}

/// The stream of a resolver's keyed contingency draws.
#[must_use]
pub fn step_contingency_stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN_NAME, "step-contingency", 1)
}

fn record_schemas() -> Vec<DomainRecordSchema> {
    let mut rulebook = DomainRecordSchema::for_type::<RulebookRecord>();
    rulebook.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    let mut force = DomainRecordSchema::for_type::<ForceRecord>();
    force.holder_policy = KnowledgeHolderPolicy::Allowed;
    let mut summary = DomainRecordSchema::for_type::<EngagementSummaryRecord>();
    summary.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
    vec![
        rulebook,
        DomainRecordSchema::for_type::<RegistryRecord>(),
        force,
        DomainRecordSchema::for_type::<EngagementRecord>(),
        DomainRecordSchema::for_type::<EngagementStepsRecord>(),
        summary,
        DomainRecordSchema::for_type::<SiegeRecord>(),
        DomainRecordSchema::for_type::<CrossingRecord>(),
    ]
}

impl<R: StepResolver> SimulationPlugin for MilitaryPlugin<R> {
    fn name(&self) -> &str {
        PLUGIN_NAME
    }

    fn version(&self) -> &str {
        PLUGIN_VERSION
    }

    fn semantic_hash(&self) -> &str {
        &self.semantic_hash
    }

    /// Decodes and validates the registry, the rulebook (hash, resolver
    /// identity, limits), and every active record against the limits;
    /// retired records are left to the kernel's envelope checks. It reads
    /// no host record version.
    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        validate_records::<R>(records)
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for schema in record_schemas() {
            registrar.register_record_schema(schema)?;
        }
        let schemas = report_knowledge_schemas();
        for schema in &schemas {
            registrar.register_knowledge_schema(schema.clone())?;
        }
        if let Some(reservation) = self.reservation {
            registrar.reserve_knowledge_publication(reservation)?;
        }
        registrar.register_command(
            PluginActionDescriptor {
                name: MILITARY_COMMAND.to_owned(),
                description: "Admit one engagement-engine command".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: vec![kind_key::<RegistryRecord>(), kind_key::<RulebookRecord>()]
                    .into_iter()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                writes: Vec::new(),
            },
            admit_command,
        )?;
        // Only the command handler and the wake system queue these packets;
        // hosts cannot author them.
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: MILITARY_APPLY_INGRESS.to_owned(),
            description: "Apply one admitted engagement-engine command".to_owned(),
            class: IngressClass::Decision,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_internal_ingress(PluginIngressDescriptor {
            name: MILITARY_STEP_INGRESS.to_owned(),
            description: "Step the engagements, sieges, and crossings due".to_owned(),
            class: IngressClass::ScheduledSystem,
            payload_schema: PayloadSchema::Any,
        })?;

        let step_target = PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: MILITARY_STEP_INGRESS.to_owned(),
        };
        let wake = match self.step_cadence.clone() {
            None => None,
            Some(SystemCadence::EventDriven) => {
                return Err(CanwuError::new(
                    ErrorCode::InvalidPluginRegistration,
                    "the military settle system is event-driven already; give with_step_cadence a cadence a host schedules",
                ));
            }
            Some(cadence) => {
                let mut contract = BoundarySystemContract::new(
                    WAKE_SYSTEM,
                    BoundaryPhase::DomainDeltaProposal,
                    cadence,
                );
                contract.reads = vec![kind_key::<RegistryRecord>()];
                contract.plugin_ingress_targets = vec![step_target.clone()];
                contract.visibility = StateVisibility::SameBoundary;
                Some(contract)
            }
        };

        let mut settle = BoundarySystemContract::new(
            settle_system_name(self.outcome_signal.as_ref()),
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        let mut reads = record_keys();
        reads.push(StateKey::core_ingress());
        reads.sort();
        settle.reads = reads;
        settle.writes = record_keys();
        settle.random_streams = vec![crossing_interception_stream(), step_contingency_stream()];
        settle.emits = vec![
            MILITARY_REJECTION_EVENT.to_owned(),
            ENGAGEMENT_OPENED_EVENT.to_owned(),
            ENGAGEMENT_CLOSED_EVENT.to_owned(),
            ENGAGEMENT_HELD_EVENT.to_owned(),
            SIEGE_ENDED_EVENT.to_owned(),
            CROSSING_WAVE_EVENT.to_owned(),
            REPORT_DROPPED_EVENT.to_owned(),
        ];
        settle.emits.sort();
        settle.plugin_ingress_targets = self.outcome_signal.iter().cloned().collect();
        settle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(settle, settle_engagements::<R>)?;
        if let Some(contract) = wake {
            registrar.register_boundary_system(contract, wake_engagements)?;
        }

        let mut publish = BoundarySystemContract::new(
            PUBLISH_SYSTEM,
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::EventDriven,
        );
        let mut reads = record_keys();
        reads.push(StateKey::core_person_availability());
        reads.sort();
        publish.reads = reads;
        publish.writes = vec![kind_key::<RegistryRecord>()];
        publish.knowledge_writes = schemas
            .iter()
            .map(|schema| KnowledgeWriteGrant {
                schema: schema.id.clone(),
                visibilities: vec![StateVisibility::SameBoundary],
            })
            .collect();
        publish.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(publish, publish_engagement_reports)
    }
}

/// Builds the tracked command for one [`MilitaryCommandV1`].
pub fn military_command(command: &MilitaryCommandV1) -> Result<Command, CanwuError> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: MILITARY_COMMAND.to_owned(),
        payload: serde_json::to_value(command)
            .map_err(|error| CanwuError::new(ErrorCode::InvalidPayload, error.to_string()))?,
    })
}

/// The initial-scenario records that install `rulebook` and `forces`: the
/// rulebook, the registry, and one record per force.
pub fn scenario_records(
    rulebook: &CombatRulebookV1,
    forces: &[ForceStateV1],
) -> Result<Vec<DomainRecord>, CanwuError> {
    rulebook.validate()?;
    let mut registry = CombatRegistryV1::new(Some(rulebook.id.clone()));
    registry.force_count = u32::try_from(forces.len()).unwrap_or(u32::MAX);
    registry.validate(&rulebook.limits)?;
    let mut records = vec![
        record_from(rulebook_reference(&rulebook.id), rulebook)?,
        record_from(registry_reference(), &registry)?,
    ];
    for force in forces {
        force.validate()?;
        force.validate_limits(rulebook)?;
        records.push(record_from(force_reference(&force.id), force)?);
    }
    Ok(records)
}

fn validate_records<R: StepResolver>(records: &[DomainRecord]) -> Result<(), CanwuError> {
    let mine = |kind: &DomainRecordKind| kind.namespace == crate::PLUGIN_NAMESPACE;
    let mut rulebook: Option<CombatRulebookV1> = None;
    let mut registry: Option<CombatRegistryV1> = None;
    for record in records.iter().filter(|record| mine(&record.reference.kind)) {
        let kind = &record.reference.kind;
        if kind.matches_type::<RulebookRecord>() {
            let decoded = record.decode_payload::<RulebookRecord>()?;
            decoded.validate()?;
            if !R::IDENTITY.matches(&decoded.resolver) {
                return Err(invalid(format!(
                    "the rulebook names resolver {}@{}, the plugin runs {}@{}",
                    decoded.resolver.name,
                    decoded.resolver.version,
                    R::IDENTITY.name,
                    R::IDENTITY.version
                )));
            }
            if rulebook.replace(decoded).is_some() {
                return Err(invalid("a run holds one military rulebook"));
            }
        } else if kind.matches_type::<RegistryRecord>() {
            if record.reference != registry_reference().into_untyped() {
                return Err(invalid("the military registry is misidentified"));
            }
            registry = Some(record.decode_payload::<RegistryRecord>()?);
        }
    }
    let others = records.iter().any(|record| {
        mine(&record.reference.kind)
            && !record.reference.kind.matches_type::<RulebookRecord>()
            && !record.reference.kind.matches_type::<RegistryRecord>()
    });
    let Some(rulebook) = rulebook else {
        if registry.is_some() || others {
            return Err(invalid("military records without an installed rulebook"));
        }
        return Ok(());
    };
    let registry = registry.ok_or_else(|| invalid("a military rulebook without its registry"))?;
    if registry.rulebook.as_ref() != Some(&rulebook.id) {
        return Err(invalid("the military registry names another rulebook"));
    }
    registry.validate(&rulebook.limits)?;
    let limits = &rulebook.limits;
    let mut active_forces = 0_u32;
    for record in records.iter().filter(|record| {
        mine(&record.reference.kind) && record.lifecycle == DomainRecordLifecycle::Active
    }) {
        let kind = &record.reference.kind;
        if kind.matches_type::<ForceRecord>() {
            let force: ForceStateV1 = record.decode_payload::<ForceRecord>()?;
            force.validate()?;
            force.validate_limits(&rulebook)?;
            if force.status == ForceStatus::Active {
                active_forces = active_forces.saturating_add(1);
            }
        } else if kind.matches_type::<EngagementRecord>() {
            let engagement: EngagementStateV1 = record.decode_payload::<EngagementRecord>()?;
            engagement.validate()?;
            for side in [&engagement.attacker, &engagement.defender] {
                if side.forces.len() > limits.forces_per_side as usize
                    || side.holders.len() > limits.holders_per_side as usize
                {
                    return Err(invalid("an engagement side exceeds the limits"));
                }
            }
        } else if kind.matches_type::<EngagementStepsRecord>() {
            let steps: EngagementStepsV1 = record.decode_payload::<EngagementStepsRecord>()?;
            if steps.schema_version != SCHEMA_VERSION
                || steps.steps.len() > crate::MAX_STEPS_PER_BOUNDARY as usize
            {
                return Err(invalid("an engagement steps record exceeds the limits"));
            }
        } else if kind.matches_type::<EngagementSummaryRecord>() {
            let summary: EngagementSummaryV1 =
                record.decode_payload::<EngagementSummaryRecord>()?;
            if summary.schema_version != SCHEMA_VERSION {
                return Err(invalid("an engagement summary has another schema"));
            }
        } else if kind.matches_type::<SiegeRecord>() {
            let siege: SiegeStateV1 = record.decode_payload::<SiegeRecord>()?;
            siege.validate()?;
        } else if kind.matches_type::<CrossingRecord>() {
            let crossing: CrossingStateV1 = record.decode_payload::<CrossingRecord>()?;
            crossing.validate()?;
        }
    }
    if active_forces != registry.force_count {
        return Err(invalid(
            "the military registry's force count disagrees with its forces",
        ));
    }
    Ok(())
}
