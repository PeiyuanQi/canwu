//! Plugin-driven culture lifecycle settlement.
//!
//! [`CultureBoundaryPlugin`] settles the culture lifecycle inside the engine
//! instead of the host. Its Monthly phase-7 system
//! [`CULTURE_LIFECYCLE_SYSTEM`] consumes admitted [`CULTURE_EXPOSURE_INGRESS`]
//! batches and accepted institutional decisions, calls
//! [`crate::settle_culture_society_boundary`] against the boundary snapshot of
//! `canwu.society:state`, persists `canwu.culture:state`, hands one
//! [`canwu_society::SocietyLifecycleDeltaV1`] per transitioned target to the
//! society plugin as next-boundary plugin ingress, and emits
//! [`crate::CulturalSignalBatch`] records through the compiled effect bindings
//! as next-boundary [`CULTURAL_SIGNAL_INGRESS`].
//!
//! Delivery order: within one boundary all phase-7 systems read the same
//! boundary snapshot and run in plugin-name order, so `canwu-culture` runs
//! before `canwu-society` but neither observes the other's phase-7 writes.
//! Every hand-off is therefore next-boundary: the society delta is admitted
//! at the next boundary, queued by the society intake, and applied by the
//! following Daily society settlement; signal batches are admitted at the
//! next boundary as evidence for their consumers.
//!
//! Reconciliation: the committed culture lifecycle is the desired society
//! state, and culture never rolls a step back. A transitioned target's delta
//! is sent at its transition. At every later lifecycle boundary culture
//! compares each target with the society snapshot and re-sends the delta
//! that brings society to the committed lifecycle: missing compiled bindings
//! of an active target, remaining compiled rules of a dormant or retired
//! target, and the remaining dynamic state of a retired target once society
//! no longer depends on it. Society applies deltas idempotently, so a
//! duplicate sent while one is still in flight is harmless, and a delta the
//! society refused or whose release it blocked is retried a month later.

use crate::lifecycle::{CultureRuntime, LifecycleObservation};
use crate::model::{
    CompiledCulturePlan, CulturalSignalBatch, CultureDefinition, CultureLifecycle,
    CultureStateRecord, LifecycleTransition, culture_state_reference,
};
use crate::society::{compiled_target_bindings, settle_culture_society_boundary};
use crate::{PLUGIN_NAME, compile_culture, society_lifecycle_delta};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProposal,
    BoundarySystemContract, CanwuError, CauseRef, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordLifecycle, DomainRecordMutation, DomainRecordMutationPolicy,
    DomainRecordSchema, DomainRecordType, DomainValueKindClass, ErrorCode, EvidenceRef,
    IngressClass, IngressId, IngressPayload, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginIngressDescriptor, PluginIngressTarget, PluginRegistrar, SimDuration, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, TypedDomainRecordRef,
};
use canwu_society::{
    PolicyDecision, SOCIETY_LIFECYCLE_DELTA_INGRESS, SocietyLifecycleDeltaV1, SocietyState,
    SocietyStateRecord, SocietyTargetBindings, society_policy_decision_state_key,
    society_state_reference,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Public culture ingress carrying one [`CultureExposureSignalBatch`].
pub const CULTURE_EXPOSURE_INGRESS: &str = "culture_exposure_v1";
/// Internal, self-addressed culture ingress carrying one emitted
/// [`CulturalSignalBatch`]. Consumers cite its ingress ID and verify the
/// producer with `SimulationView::plugin_ingress_payload_matches`.
pub const CULTURAL_SIGNAL_INGRESS: &str = "cultural_signal_batch_v1";
/// Monthly phase-7 lifecycle settlement system.
pub const CULTURE_LIFECYCLE_SYSTEM: &str = "culture_lifecycle_settle_v1";
/// Event-driven phase-12 system queuing admitted exposure batches.
pub const CULTURE_EXPOSURE_INTAKE_SYSTEM: &str = "culture_exposure_intake_v1";
/// Event emitted for an exposure batch rejected at admission or settlement.
pub const CULTURE_EXPOSURE_REJECTED_EVENT: &str = "culture_exposure_rejected_v1";
/// Event emitted for every lifecycle transition the plugin settles.
pub const CULTURE_LIFECYCLE_TRANSITION_EVENT: &str = "culture_lifecycle_transition_v1";
/// Event emitted when a lifecycle boundary is rejected and leaves culture
/// state unchanged.
pub const CULTURE_LIFECYCLE_REJECTED_EVENT: &str = "culture_lifecycle_rejected_v1";
/// Semantic identity of [`CultureBoundaryPlugin`].
pub const BOUNDARY_SEMANTIC_HASH: &str =
    "a158ba392ebe8f2f0d79ff38d2bfaa8ff5e76ec43cd185927203411bc0811e5e";
/// Most admitted exposure batches that may wait for a lifecycle boundary; the
/// intake rejects further batches with a [`CULTURE_EXPOSURE_REJECTED_EVENT`].
pub const MAX_CULTURE_EXPOSURE_QUEUE: usize = 4_096;

const MAX_EXPOSURE_SCOPE: usize = 1_024;
const MAX_EXPOSURE_EVIDENCE: usize = 64;

/// Resolved exposure to one culture target generation, produced by an
/// information or correspondence provider after access and interpretation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CultureExposureSignalBatch {
    pub target_id: String,
    /// Generation of the target the exposure was resolved against; a batch
    /// citing another generation is rejected.
    pub target_generation: u64,
    /// Compiled cohort IDs reached, sorted and unique; empty means every
    /// cohort.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cohort_scope: Vec<String>,
    /// Interpretation fidelity, 1 to 1,000 per mille.
    pub fidelity_per_mille: u16,
    /// Existing evidence for the exposure; at least one reference.
    pub evidence: Vec<EvidenceRef>,
    /// The batch is not settled at a lifecycle boundary with a smaller ID.
    pub earliest_boundary: BoundaryId,
}

impl CultureExposureSignalBatch {
    fn validate_shape(&self) -> Result<(), String> {
        if self.target_id.is_empty() || self.target_id != self.target_id.trim() {
            return Err("exposure target ID is empty or not canonical".to_owned());
        }
        if self.target_generation == 0 {
            return Err("exposure target generation must be positive".to_owned());
        }
        if self.fidelity_per_mille == 0 || self.fidelity_per_mille > 1_000 {
            return Err("exposure fidelity must be between 1 and 1000 per mille".to_owned());
        }
        if self.cohort_scope.len() > MAX_EXPOSURE_SCOPE
            || self.cohort_scope.iter().any(String::is_empty)
            || self.cohort_scope.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("exposure cohort scope must be sorted, unique, and bounded".to_owned());
        }
        if self.evidence.is_empty() || self.evidence.len() > MAX_EXPOSURE_EVIDENCE {
            return Err("exposure requires between 1 and 64 evidence references".to_owned());
        }
        Ok(())
    }
}

/// The culture definition the boundary plugin compiles at every lifecycle
/// boundary. It is installed by the scenario and never mutated.
pub struct CultureDefinitionRecord;

impl DomainRecordType for CultureDefinitionRecord {
    type Payload = CultureDefinition;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.culture";
    const NAME: &'static str = "definition";
}

#[must_use]
pub fn culture_definition_reference() -> TypedDomainRecordRef<CultureDefinitionRecord> {
    TypedDomainRecordRef::new("root")
}

/// Encodes a definition as the scenario record the boundary plugin compiles.
///
/// # Errors
///
/// Returns an error when the definition does not compile.
pub fn culture_definition_record(
    definition: &CultureDefinition,
) -> Result<DomainRecord, CanwuError> {
    compile_culture(definition)?;
    let draft = DomainRecordDraft::from_typed(culture_definition_reference(), definition)?;
    Ok(DomainRecord {
        reference: draft.reference,
        owner: PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: Vec::new(),
    })
}

pub struct CultureExposureQueueRecord;

impl DomainRecordType for CultureExposureQueueRecord {
    type Payload = CultureExposureQueue;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.culture";
    const NAME: &'static str = "exposure-queue";
}

#[must_use]
pub fn culture_exposure_queue_reference() -> TypedDomainRecordRef<CultureExposureQueueRecord> {
    TypedDomainRecordRef::new("root")
}

/// One admitted exposure batch awaiting a lifecycle boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedCultureExposure {
    pub ingress: IngressId,
    pub admitted_at: BoundaryId,
    pub batch: CultureExposureSignalBatch,
}

/// Admitted exposure batches in admission order. The phase-12 intake appends;
/// the Monthly lifecycle system consumes every eligible entry it observes.
/// Only the intake writes entries: an initial scenario cannot seed a
/// non-empty queue.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CultureExposureQueue {
    pub schema_version: u32,
    pub entries: Vec<QueuedCultureExposure>,
}

impl CultureExposureQueue {
    pub const SCHEMA_VERSION: u32 = 1;

    /// # Errors
    ///
    /// Returns `InvalidDomainRecord` when the schema, identities, admission
    /// order, or a batch shape is invalid.
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(invalid("unsupported culture exposure queue version"));
        }
        if self.entries.len() > MAX_CULTURE_EXPOSURE_QUEUE {
            return Err(invalid("culture exposure queue exceeds its capacity"));
        }
        let mut seen = BTreeSet::new();
        let mut previous = None;
        for entry in &self.entries {
            if !seen.insert(entry.ingress)
                || previous.is_some_and(|boundary| entry.admitted_at < boundary)
            {
                return Err(invalid("culture exposure queue contains an invalid entry"));
            }
            entry.batch.validate_shape().map_err(invalid)?;
            previous = Some(entry.admitted_at);
        }
        Ok(())
    }
}

/// Plugin-driven culture lifecycle: registers the culture record schemas,
/// the exposure ingress, and the lifecycle boundary system.
///
/// The scenario installs [`culture_definition_record`], the culture state
/// (`CultureRuntime::new_at(..).into_record(..)`), and a society state with
/// [`crate::install_into_society`] applied. Register [`canwu_society::SocietyPlugin`]
/// alongside: it stays the only writer of `canwu.society:state` and applies
/// the delivered lifecycle deltas. A host using this plugin must not also call
/// [`crate::settle_culture_society_boundary`] for the same run.
#[derive(Clone, Copy, Debug, Default)]
pub struct CultureBoundaryPlugin;

impl SimulationPlugin for CultureBoundaryPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        BOUNDARY_SEMANTIC_HASH
    }

    #[allow(clippy::too_many_lines)]
    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        crate::plugin::register_culture_state(registrar)?;

        let mut definition = DomainRecordSchema::for_record::<CultureDefinitionRecord>();
        definition.mutation_policy = DomainRecordMutationPolicy::CreateOnly;
        definition.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("id".to_owned(), required(PayloadValueType::String)),
                ("targets".to_owned(), optional(PayloadValueType::Array)),
                ("cohorts".to_owned(), optional(PayloadValueType::Array)),
                ("channels".to_owned(), optional(PayloadValueType::Array)),
                ("transitions".to_owned(), optional(PayloadValueType::Array)),
                ("effects".to_owned(), optional(PayloadValueType::Array)),
                ("institutions".to_owned(), optional(PayloadValueType::Array)),
                ("budgets".to_owned(), optional(PayloadValueType::Object)),
                ("retirement".to_owned(), optional(PayloadValueType::Object)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(definition)?;

        let mut queue = DomainRecordSchema::for_record::<CultureExposureQueueRecord>();
        queue.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("entries".to_owned(), required(PayloadValueType::Array)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(queue)?;

        registrar.register_ingress(PluginIngressDescriptor {
            name: CULTURE_EXPOSURE_INGRESS.to_owned(),
            description: "Admit one resolved exposure to a culture target generation".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([
                    ("target_id".to_owned(), required(PayloadValueType::String)),
                    (
                        "target_generation".to_owned(),
                        required(PayloadValueType::Integer),
                    ),
                    ("cohort_scope".to_owned(), optional(PayloadValueType::Array)),
                    (
                        "fidelity_per_mille".to_owned(),
                        required(PayloadValueType::Integer),
                    ),
                    ("evidence".to_owned(), required(PayloadValueType::Array)),
                    (
                        "earliest_boundary".to_owned(),
                        required(PayloadValueType::Integer),
                    ),
                ]),
                allow_additional: false,
            },
        })?;
        // Only the lifecycle system schedules signal batches; the host cannot
        // author one.
        let _signal_permit = registrar.register_internal_ingress(PluginIngressDescriptor {
            name: CULTURAL_SIGNAL_INGRESS.to_owned(),
            description: "Emit one bounded cultural signal batch for downstream consumers"
                .to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Object {
                properties: BTreeMap::from([
                    ("id".to_owned(), required(PayloadValueType::String)),
                    ("plan_hash".to_owned(), required(PayloadValueType::String)),
                    ("emitted_at".to_owned(), required(PayloadValueType::Integer)),
                    (
                        "earliest_eligible_at".to_owned(),
                        required(PayloadValueType::Integer),
                    ),
                    ("signals".to_owned(), required(PayloadValueType::Array)),
                ]),
                allow_additional: false,
            },
        })?;

        let mut intake = BoundarySystemContract::new(
            CULTURE_EXPOSURE_INTAKE_SYSTEM,
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        intake.reads = vec![
            culture_state_key(),
            exposure_queue_key(),
            StateKey::core_commands(),
            StateKey::core_domain_records(),
            StateKey::core_events(),
            StateKey::core_evidence(),
            StateKey::core_ingress(),
        ];
        intake.writes = vec![exposure_queue_key()];
        intake.emits = vec![CULTURE_EXPOSURE_REJECTED_EVENT.to_owned()];
        intake.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(intake, intake_culture_exposure)?;

        let mut settle = BoundarySystemContract::new(
            CULTURE_LIFECYCLE_SYSTEM,
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Monthly,
        );
        settle.reads = vec![
            culture_state_key(),
            definition_key(),
            exposure_queue_key(),
            StateKey::new(SocietyStateRecord::NAMESPACE, SocietyStateRecord::NAME),
            society_policy_decision_state_key(),
        ];
        settle.writes = vec![culture_state_key(), exposure_queue_key()];
        settle.emits = vec![
            CULTURE_EXPOSURE_REJECTED_EVENT.to_owned(),
            CULTURE_LIFECYCLE_REJECTED_EVENT.to_owned(),
            CULTURE_LIFECYCLE_TRANSITION_EVENT.to_owned(),
        ];
        settle.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: canwu_society::PLUGIN_NAME.to_owned(),
            packet_type: SOCIETY_LIFECYCLE_DELTA_INGRESS.to_owned(),
        }];
        settle.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(settle, settle_culture_lifecycle)
    }

    fn validate_activation(&self, records: &[DomainRecord]) -> Result<(), CanwuError> {
        let find = |reference: canwu_api::DomainRecordRef| {
            records.iter().find(|record| record.reference == reference)
        };
        let plan = find(culture_definition_reference().into_untyped())
            .map(|record| {
                record
                    .decode_payload::<CultureDefinitionRecord>()
                    .and_then(|definition| compile_culture(&definition))
            })
            .transpose()?;
        if let (Some(plan), Some(state)) = (&plan, find(culture_state_reference().into_untyped())) {
            state
                .decode_payload::<CultureStateRecord>()?
                .validate_against_plan(plan)?;
        }
        if let Some(queue) = find(culture_exposure_queue_reference().into_untyped()) {
            queue
                .decode_payload::<CultureExposureQueueRecord>()?
                .validate()?;
        }
        Ok(())
    }
}

/// Queues every admitted exposure batch whose shape and evidence are valid;
/// rejects the rest with a [`CULTURE_EXPOSURE_REJECTED_EVENT`].
fn intake_culture_exposure(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let (mut queue, queue_record) = load_exposure_queue(view)?;
    let culture_active = view
        .typed_domain_record(&culture_state_reference())?
        .is_some_and(DomainRecord::is_active);
    let mut directives = Vec::new();
    let mut changed = false;
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
        if plugin != PLUGIN_NAME || packet_type != CULTURE_EXPOSURE_INGRESS {
            continue;
        }
        if !culture_active {
            directives.push(rejection_event(*ingress_id, "culture state is not active"));
            continue;
        }
        if queue.entries.len() >= MAX_CULTURE_EXPOSURE_QUEUE {
            directives.push(rejection_event(*ingress_id, "the exposure queue is full"));
            continue;
        }
        let Ok(batch) = serde_json::from_value::<CultureExposureSignalBatch>(payload.clone())
        else {
            directives.push(rejection_event(*ingress_id, "malformed_payload"));
            continue;
        };
        if let Err(reason) = batch.validate_shape() {
            directives.push(rejection_event(*ingress_id, &reason));
            continue;
        }
        let mut evidence_available = true;
        for reference in &batch.evidence {
            evidence_available &= view.evidence_exists(reference)?;
        }
        if !evidence_available {
            directives.push(rejection_event(
                *ingress_id,
                "exposure evidence is unavailable",
            ));
            continue;
        }
        queue.entries.push(QueuedCultureExposure {
            ingress: *ingress_id,
            admitted_at: context.boundary_id,
            batch,
        });
        changed = true;
    }
    if changed {
        queue.validate()?;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: queue_mutation(&queue, queue_record.as_ref())?,
            summary: "Queued admitted culture exposure".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Settles one Monthly culture lifecycle boundary inside the engine.
///
/// A domain rejection of the lifecycle step (for example an exhausted
/// tombstone budget or a conflicting society binding) emits
/// [`CULTURE_LIFECYCLE_REJECTED_EVENT`] and leaves culture state and the
/// exposure queue unchanged, so the boundary still commits.
fn settle_culture_lifecycle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(state_record) = view.typed_domain_record(&culture_state_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let Some(definition_record) = view.typed_domain_record(&culture_definition_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    // Owner-authorized maintenance may retire the persisted culture state;
    // a retired root is inert.
    if !state_record.is_active() || !definition_record.is_active() {
        return Ok(BoundaryProposal::default());
    }
    let definition = definition_record.decode_payload::<CultureDefinitionRecord>()?;
    let plan = compile_culture(&definition)?;
    let state = state_record.decode_payload::<CultureStateRecord>()?;
    let runtime = CultureRuntime::from_state(&plan, state)?;
    // A lifecycle boundary must advance time; a repeated Monthly boundary at
    // the same instant leaves culture state and the exposure queue untouched.
    if context.at <= runtime.state().last_boundary_at
        || context.at < runtime.state().latest_activity_at
    {
        return Ok(BoundaryProposal::default());
    }
    let society = load_society_snapshot(view)?;
    let (queue, queue_record) = load_exposure_queue(view)?;
    let mut settled =
        match settle_lifecycle(view, context, &plan, runtime, society.as_ref(), &queue) {
            Ok(settled) => settled,
            Err(error) if error.code == ErrorCode::InvalidDomainRecord => {
                return Ok(BoundaryProposal {
                    directives: vec![BoundaryDirective::Emit {
                        event_type: CULTURE_LIFECYCLE_REJECTED_EVENT.to_owned(),
                        summary: format!("Culture lifecycle boundary rejected: {}", error.message),
                        affected: Vec::new(),
                    }],
                    ..BoundaryProposal::default()
                });
            }
            Err(error) => return Err(error),
        };

    let mut directives = std::mem::take(&mut settled.rejections);
    directives.push(BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Update {
            record: DomainRecordDraft::from_typed(
                culture_state_reference(),
                settled.runtime.state(),
            )?,
            expected_version: state_record.version,
        },
        summary: "Settled culture lifecycle boundary".to_owned(),
    });
    if settled.remaining.len() != queue.entries.len() {
        let queue = CultureExposureQueue {
            schema_version: CultureExposureQueue::SCHEMA_VERSION,
            entries: settled.remaining.clone(),
        };
        directives.push(BoundaryDirective::MutateRecord {
            mutation: queue_mutation(&queue, queue_record.as_ref())?,
            summary: "Consumed culture exposure".to_owned(),
        });
    }
    for transition in &settled.transitions {
        directives.push(BoundaryDirective::Emit {
            event_type: CULTURE_LIFECYCLE_TRANSITION_EVENT.to_owned(),
            summary: format!(
                "Culture target {} generation {} {:?}",
                transition.target_id, transition.generation, transition.kind
            ),
            affected: Vec::new(),
        });
    }
    if let Some(society) = &society {
        directives.extend(society_delta_directives(&plan, &settled, society)?);
    }
    for batch in settled.batches {
        directives.push(BoundaryDirective::ScheduleIngress {
            after: SimDuration::ZERO,
            packet_type: CULTURAL_SIGNAL_INGRESS.to_owned(),
            priority: 0,
            payload: serde_json::to_value(&batch).map_err(|error| encoding_error(&error))?,
            affected: Vec::new(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct SettledLifecycle {
    runtime: CultureRuntime,
    remaining: Vec<QueuedCultureExposure>,
    rejections: Vec<BoundaryDirective>,
    transitions: Vec<LifecycleTransition>,
    batches: Vec<CulturalSignalBatch>,
    live_targets: BTreeSet<String>,
}

/// Schedules one society lifecycle delta per target, so a release the
/// society owner has to block cannot discard another target's lifecycle
/// change. Transitioned targets send their transition delta; every other
/// target is reconciled against the society snapshot.
fn society_delta_directives(
    plan: &CompiledCulturePlan,
    settled: &SettledLifecycle,
    society: &SocietyState,
) -> Result<Vec<BoundaryDirective>, CanwuError> {
    let mut by_target = BTreeMap::<&str, Vec<LifecycleTransition>>::new();
    for transition in &settled.transitions {
        by_target
            .entry(transition.target_id.as_str())
            .or_default()
            .push(transition.clone());
    }
    let mut deltas = BTreeMap::new();
    for (target_id, transitions) in &by_target {
        deltas.insert(
            (*target_id).to_owned(),
            society_lifecycle_delta(plan, transitions)?,
        );
    }
    for (target_id, delta) in
        reconciliation_deltas(plan, &settled.runtime, society, &settled.live_targets)?
    {
        deltas.entry(target_id).or_insert(delta);
    }
    deltas
        .values()
        .map(|delta| {
            Ok(BoundaryDirective::SchedulePluginIngress {
                target_plugin: canwu_society::PLUGIN_NAME.to_owned(),
                after: SimDuration::ZERO,
                packet_type: SOCIETY_LIFECYCLE_DELTA_INGRESS.to_owned(),
                priority: 0,
                payload: serde_json::to_value(delta).map_err(|error| encoding_error(&error))?,
                affected: Vec::new(),
            })
        })
        .collect()
}

/// Returns, per target whose society bindings disagree with the committed
/// culture lifecycle, the delta that reconciles them (see the module docs).
fn reconciliation_deltas(
    plan: &CompiledCulturePlan,
    runtime: &CultureRuntime,
    society: &SocietyState,
    live_targets: &BTreeSet<String>,
) -> Result<BTreeMap<String, SocietyLifecycleDeltaV1>, CanwuError> {
    let mut deltas = BTreeMap::new();
    let distributed = society
        .distributions
        .values()
        .map(|distribution| distribution.target_id.as_str())
        .collect::<BTreeSet<_>>();
    for target in &plan.targets {
        let target_id = &target.source_id;
        let Some(lifecycle) = runtime.target(target_id) else {
            continue;
        };
        if !society.targets.contains_key(target_id) {
            continue;
        }
        let bindings = compiled_target_bindings(plan, target_id)?;
        let rules_present = bindings
            .rules
            .iter()
            .any(|rule| society.transition_rules.contains_key(&rule.id));
        let delta =
            match lifecycle.state {
                CultureLifecycle::Active => {
                    // Install only what is missing, so a present binding with
                    // different content is never re-sent as a conflict.
                    let missing = SocietyTargetBindings {
                        target_id: target_id.clone(),
                        rules: bindings
                            .rules
                            .into_iter()
                            .filter(|rule| !society.transition_rules.contains_key(&rule.id))
                            .collect(),
                        alignments: bindings
                            .alignments
                            .into_iter()
                            .filter(|alignment| {
                                !society.institutional_alignments.contains_key(&alignment.id)
                            })
                            .collect(),
                    };
                    let installable = missing
                        .rules
                        .iter()
                        .flat_map(|rule| &rule.affected_cohorts)
                        .chain(
                            missing
                                .alignments
                                .iter()
                                .flat_map(|alignment| &alignment.affected_cohorts),
                        )
                        .all(|cohort| society.cohorts.contains_key(cohort));
                    ((!missing.rules.is_empty() || !missing.alignments.is_empty()) && installable)
                        .then(|| SocietyLifecycleDeltaV1 {
                            installs: vec![missing],
                            ..SocietyLifecycleDeltaV1::default()
                        })
                }
                CultureLifecycle::Dormant => rules_present.then(|| SocietyLifecycleDeltaV1 {
                    deactivations: vec![bindings],
                    ..SocietyLifecycleDeltaV1::default()
                }),
                CultureLifecycle::Retired => {
                    let dynamic = bindings.alignments.iter().any(|alignment| {
                        society.institutional_alignments.contains_key(&alignment.id)
                    }) || distributed.contains(target_id.as_str());
                    (rules_present || (dynamic && !live_targets.contains(target_id))).then(|| {
                        SocietyLifecycleDeltaV1 {
                            deactivations: vec![bindings],
                            releases: BTreeSet::from([target_id.clone()]),
                            ..SocietyLifecycleDeltaV1::default()
                        }
                    })
                }
            };
        if let Some(delta) = delta {
            deltas.insert(target_id.clone(), delta);
        }
    }
    Ok(deltas)
}

fn settle_lifecycle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    plan: &CompiledCulturePlan,
    mut runtime: CultureRuntime,
    society: Option<&SocietyState>,
    queue: &CultureExposureQueue,
) -> Result<SettledLifecycle, CanwuError> {
    let mut rejections = Vec::new();
    let mut exposed_targets = BTreeSet::new();
    let mut remaining = Vec::new();
    for entry in &queue.entries {
        if entry.batch.earliest_boundary > context.boundary_id {
            remaining.push(entry.clone());
            continue;
        }
        match check_exposure(plan, &runtime, &entry.batch) {
            Ok(()) => {
                exposed_targets.insert(entry.batch.target_id.clone());
            }
            Err(reason) => rejections.push(rejection_event(entry.ingress, &reason)),
        }
    }
    let signals = society_signals(view, plan, society)?;
    let observations = lifecycle_observations(&runtime, &signals, &exposed_targets);
    let transitions = match society {
        Some(society) => {
            let mut staged = society.clone();
            settle_culture_society_boundary(
                plan,
                &mut runtime,
                &mut staged,
                context.at,
                &observations,
            )?
        }
        None => runtime.settle_boundary(context.at, &observations)?,
    };
    let batches = emit_eligible_effects(plan, &mut runtime, context, &observations, society)?;
    Ok(SettledLifecycle {
        runtime,
        remaining,
        rejections,
        transitions,
        batches,
        live_targets: signals.live,
    })
}

fn check_exposure(
    plan: &CompiledCulturePlan,
    runtime: &CultureRuntime,
    batch: &CultureExposureSignalBatch,
) -> Result<(), String> {
    let Some(target) = runtime.target(&batch.target_id) else {
        return Err(format!("unknown culture target {}", batch.target_id));
    };
    if target.state == CultureLifecycle::Retired {
        return Err(format!(
            "retired culture target {} requires explicit reactivation",
            batch.target_id
        ));
    }
    if target.generation != batch.target_generation {
        return Err(format!(
            "exposure cites generation {} of target {}, current generation is {}",
            batch.target_generation, batch.target_id, target.generation
        ));
    }
    if batch.cohort_scope.len() > plan.budgets.max_fan_out
        || batch
            .cohort_scope
            .iter()
            .any(|cohort| !plan.cohort_by_id.contains_key(cohort))
    {
        return Err("exposure cohort scope names an unknown cohort or exceeds fan-out".to_owned());
    }
    if batch.evidence.len() > plan.budgets.max_evidence_per_signal {
        return Err("exposure evidence exceeds the plan evidence budget".to_owned());
    }
    Ok(())
}

/// What the society snapshot says about each culture target.
///
/// * engaged headcount counts non-neutral society buckets of the target;
/// * a decided target has an accepted institutional decision on one of its
///   culture-compiled alignments that society has not applied yet;
/// * a live target is one society still depends on: a rule or alignment the
///   plan did not compile for the target, a culture alignment with live
///   values or any stored institutional decision (decision components are
///   permanent, so such an alignment cannot be released), an active influence
///   edge or organization, or a policy on the target.
#[derive(Default)]
struct SocietySignals {
    engaged: BTreeMap<String, u64>,
    decided: BTreeSet<String>,
    live: BTreeSet<String>,
}

fn society_signals(
    view: &SimulationView<'_>,
    plan: &CompiledCulturePlan,
    society: Option<&SocietyState>,
) -> Result<SocietySignals, CanwuError> {
    let mut signals = SocietySignals::default();
    let Some(society) = society else {
        return Ok(signals);
    };
    for distribution in society.distributions.values() {
        let Some(target) = society.targets.get(&distribution.target_id) else {
            continue;
        };
        let count = distribution
            .buckets
            .iter()
            .filter(|bucket| bucket.profile != target.neutral_profile)
            .fold(0_u64, |total, bucket| {
                total.saturating_add(bucket.headcount)
            });
        let entry = signals
            .engaged
            .entry(distribution.target_id.clone())
            .or_default();
        *entry = entry.saturating_add(count);
    }
    let mut compiled_rules = BTreeMap::<String, BTreeSet<String>>::new();
    let mut compiled_alignments = BTreeMap::<String, BTreeSet<String>>::new();
    for target in &plan.targets {
        let bindings = compiled_target_bindings(plan, &target.source_id)?;
        compiled_rules.insert(
            target.source_id.clone(),
            bindings.rules.into_iter().map(|rule| rule.id).collect(),
        );
        compiled_alignments.insert(
            target.source_id.clone(),
            bindings
                .alignments
                .into_iter()
                .map(|alignment| alignment.id)
                .collect(),
        );
    }
    let compiled = |sets: &BTreeMap<String, BTreeSet<String>>, target: &str, id: &str| {
        sets.get(target).is_some_and(|ids| ids.contains(id))
    };
    for (id, rule) in &society.transition_rules {
        if !compiled(&compiled_rules, &rule.target_id, id) {
            signals.live.insert(rule.target_id.clone());
        }
    }
    for (id, alignment) in &society.institutional_alignments {
        let culture_owned = compiled(&compiled_alignments, &alignment.target_id, id);
        if !culture_owned
            || alignment.support_per_mille > 0
            || alignment.enforcement_per_mille > 0
            || alignment.access_grant_per_mille > 0
            || alignment.authorized_actor.is_some()
        {
            signals.live.insert(alignment.target_id.clone());
        }
        if culture_owned
            && let Some(value) = view.component(
                &society_policy_decision_state_key(),
                &alignment.institution,
                &alignment.id,
            )?
        {
            signals.live.insert(alignment.target_id.clone());
            let decision: PolicyDecision =
                serde_json::from_value(value.clone()).map_err(|error| {
                    invalid(format!(
                        "stored institutional decision is malformed: {error}"
                    ))
                })?;
            if decision.decision_version > alignment.last_decision_version {
                signals.decided.insert(alignment.target_id.clone());
            }
        }
    }
    for edge in society.influence_edges.values().filter(|edge| edge.active) {
        signals.live.insert(edge.target_id.clone());
    }
    for organization in society
        .organizations
        .values()
        .filter(|organization| organization.active)
    {
        signals.live.insert(organization.target_id.clone());
    }
    for policy in society.policies.values() {
        signals.live.insert(policy.target_id.clone());
    }
    Ok(signals)
}

/// Derives one lifecycle observation per non-retired target that is not
/// quiet: engagement from society buckets, admitted work from a settled
/// exposure or an unapplied accepted decision, and a live dependency from
/// [`SocietySignals::live`], so retirement is never attempted while society
/// still depends on the target.
fn lifecycle_observations(
    runtime: &CultureRuntime,
    signals: &SocietySignals,
    exposed_targets: &BTreeSet<String>,
) -> BTreeMap<String, LifecycleObservation> {
    let mut observations = BTreeMap::new();
    for (target_id, lifecycle) in runtime.state().targets() {
        if lifecycle.state == CultureLifecycle::Retired {
            continue;
        }
        let observation = LifecycleObservation {
            engaged_headcount: signals.engaged.get(target_id).copied().unwrap_or(0),
            admitted_work: exposed_targets.contains(target_id)
                || signals.decided.contains(target_id),
            live_dependency: signals.live.contains(target_id),
        };
        if observation != LifecycleObservation::quiet() {
            observations.insert(target_id.clone(), observation);
        }
    }
    observations
}

/// Emits one batch per compiled effect whose target is active and whose
/// cadence is due. Strength is the target's engaged share of the compiled
/// cohorts' society headcount; the settling boundary is the evidence.
fn emit_eligible_effects(
    plan: &CompiledCulturePlan,
    runtime: &mut CultureRuntime,
    context: &BoundaryContext,
    observations: &BTreeMap<String, LifecycleObservation>,
    society: Option<&SocietyState>,
) -> Result<Vec<CulturalSignalBatch>, CanwuError> {
    let population = society.map_or(0_u64, |society| {
        plan.cohorts
            .iter()
            .filter_map(|cohort| society.cohorts.get(&cohort.source_id))
            .fold(0_u64, |total, cohort| {
                total.saturating_add(cohort.headcount)
            })
    });
    let mut batches = Vec::new();
    for effect in &plan.effects {
        let Some(target) = plan.targets.get(effect.target.get() as usize) else {
            return Err(invalid("compiled culture effect has an invalid target key"));
        };
        let Some(lifecycle) = runtime.target(&target.source_id) else {
            continue;
        };
        if lifecycle.state != CultureLifecycle::Active {
            continue;
        }
        let generation = lifecycle.generation;
        if runtime
            .state()
            .effect_emissions
            .get(&effect.source_id)
            .is_some_and(|cursor| {
                cursor.target_generation == generation
                    && runtime
                        .state()
                        .boundary_index()
                        .saturating_sub(cursor.boundary_index)
                        < u64::from(effect.cadence_boundaries)
            })
        {
            continue;
        }
        let engaged = observations
            .get(&target.source_id)
            .map_or(0, |observation| observation.engaged_headcount);
        let strength = if population == 0 {
            0
        } else {
            u16::try_from(
                (u128::from(engaged.min(population)) * 1_000 / u128::from(population)).min(1_000),
            )
            .unwrap_or(1_000)
        };
        batches.push(runtime.emit_effect(
            plan,
            format!("culture:{}:{}", context.boundary_id, effect.key.get()),
            effect.key,
            strength,
            context.at,
            context.at,
            vec![CauseRef::Boundary(context.boundary_id)],
        )?);
    }
    Ok(batches)
}

fn load_society_snapshot(view: &SimulationView<'_>) -> Result<Option<SocietyState>, CanwuError> {
    let Some(record) = view.typed_domain_record(&society_state_reference())? else {
        return Ok(None);
    };
    let mut state = record.decode_payload::<SocietyStateRecord>()?;
    state.canonicalize()?;
    state.validate()?;
    Ok(Some(state))
}

fn load_exposure_queue(
    view: &SimulationView<'_>,
) -> Result<(CultureExposureQueue, Option<DomainRecord>), CanwuError> {
    let Some(record) = view.typed_domain_record(&culture_exposure_queue_reference())? else {
        return Ok((
            CultureExposureQueue {
                schema_version: CultureExposureQueue::SCHEMA_VERSION,
                entries: Vec::new(),
            },
            None,
        ));
    };
    let queue = record.decode_payload::<CultureExposureQueueRecord>()?;
    queue.validate()?;
    // Only the intake writes queue entries. A non-empty queue whose current
    // version the initial scenario established was never admitted.
    if !queue.entries.is_empty()
        && view
            .current_domain_record_version(&record.reference)?
            .is_none_or(|version| {
                version.established_by == canwu_api::DomainRecordVersionSource::InitialScenario
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "an initial scenario cannot seed admitted culture exposure",
        ));
    }
    Ok((queue, Some(record.clone())))
}

fn queue_mutation(
    queue: &CultureExposureQueue,
    record: Option<&DomainRecord>,
) -> Result<DomainRecordMutation, CanwuError> {
    let draft = DomainRecordDraft::from_typed(culture_exposure_queue_reference(), queue)?;
    Ok(match record {
        Some(record) => DomainRecordMutation::Update {
            record: draft,
            expected_version: record.version,
        },
        None => DomainRecordMutation::Create { record: draft },
    })
}

fn rejection_event(ingress: IngressId, reason: &str) -> BoundaryDirective {
    BoundaryDirective::Emit {
        event_type: CULTURE_EXPOSURE_REJECTED_EVENT.to_owned(),
        summary: format!("Rejected culture exposure ingress {ingress}: {reason}"),
        affected: Vec::new(),
    }
}

fn culture_state_key() -> StateKey {
    StateKey::new(CultureStateRecord::NAMESPACE, CultureStateRecord::NAME)
}

fn definition_key() -> StateKey {
    StateKey::new(
        CultureDefinitionRecord::NAMESPACE,
        CultureDefinitionRecord::NAME,
    )
}

fn exposure_queue_key() -> StateKey {
    StateKey::new(
        CultureExposureQueueRecord::NAMESPACE,
        CultureExposureQueueRecord::NAME,
    )
}

const fn required(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: true,
    }
}

const fn optional(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: false,
    }
}

fn encoding_error(error: &serde_json::Error) -> CanwuError {
    CanwuError::new(
        ErrorCode::InvalidPayload,
        format!("culture boundary payload could not be encoded: {error}"),
    )
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
