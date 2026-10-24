//! Phase 7: the staged pass that applies admitted commands and, in a
//! boundary that admits a step packet, steps every engagement, siege, and
//! crossing due; and the optional wake system.

use crate::PLUGIN_NAME;
use crate::command::{AppliedCommandV1, StepPacketV1, add_minutes, apply};
use crate::crossing::{CrossingStateV1, CrossingStatus, WaveV1};
use crate::engagement::{
    Continuation, EngagementOutcomeKind, EngagementOutcomeSignalV1, EngagementOutcomeV1,
    EngagementStateV1, EngagementStatus, EngagementStepsV1, EngagementSummaryV1, ForceLossDeltaV1,
    HoldReason, RiverBand, Side, SideStateV1, SideStepV1, SideSummaryV1, StepV1,
};
use crate::force::{ComponentValuesV1, ForceStateV1, LossTallyV1, SideTermsV1};
use crate::ids::{
    ComponentId, CrossingId, EngagementId, ForceId, ProfileId, RegistryRecord, SCHEMA_VERSION,
    SiegeId, TermId, TerrainClassId, crossing_reference, digest, engagement_reference,
    engagement_steps_reference, engagement_summary_reference, force_reference, invalid,
    registry_reference, rulebook_reference, siege_reference,
};
use crate::plugin::{
    CROSSING_WAVE_EVENT, CombatRegistryV1, ENGAGEMENT_CLOSED_EVENT, ENGAGEMENT_HELD_EVENT,
    ENGAGEMENT_OPENED_EVENT, MILITARY_APPLY_INGRESS, MILITARY_REJECTION_EVENT,
    MILITARY_STEP_INGRESS, REPORT_DROPPED_EVENT, RetireItemV1, SIEGE_ENDED_EVENT, TERM_PURSUIT,
    TERM_RESOLVER_CLAMPED, TERM_RESOLVER_FAILED, crossing_interception_stream, signal_target,
    step_contingency_stream,
};
use crate::report::{ReportItemV1, ReportSubjectV1};
use crate::resolver::{
    ForecastSideV1, ResolverIdentity, SideOutcomeV1, SideStepInputV1, StepComponentV1, StepDraws,
    StepInputV1, StepResolver, breach_rate, forecast, intercept_bp, largest_remainder, pieces,
};
use crate::rulebook::{
    CombatRulebookV1, EngagementKind, FortUse, InterceptMode, KindRuleV1, MAX_DAYS_PER_BOUNDARY,
    TerrainRowV1,
};
use crate::siege::{SiegeEndCause, SiegeStateV1, SiegeStatus, StormRule};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryProposal, CanwuError, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef, DomainRecordType, EntityRef,
    ErrorCode, EvidenceRef, IngressId, IngressPayload, KnowledgeHolderRef, PluginIngressTarget,
    RandomDrawId, RandomOperationTarget, SimDuration, SimTime, SimulationView,
    TypedDomainRecordRef,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

const BP: i128 = 10_000;
const MAX_SUMMARY_BYTES: usize = 256;

#[derive(Clone, Debug)]
enum Staged {
    Write { payload: Value, base: Option<u64> },
    Retire { base: u64 },
}

pub(crate) enum RetireOutcome {
    Done,
    Deferred,
}

/// The writes of one phase-7 pass: every record a pass reads sees the
/// writes of earlier commands and steps of the same pass, and each record
/// receives one mutation at the end.
pub(crate) struct Run<'v, 'a> {
    pub view: &'v SimulationView<'a>,
    pub at: SimTime,
    pub resolver: ResolverIdentity,
    signal: Option<PluginIngressTarget>,
    records: BTreeMap<DomainRecordRef, Staged>,
    directives: Vec<BoundaryDirective>,
    registry: Option<CombatRegistryV1>,
    registry_base: Option<CombatRegistryV1>,
    rulebook: Option<Rc<CombatRulebookV1>>,
    loss_deltas: BTreeMap<ForceId, LossTallyV1>,
}

pub(crate) struct RunCheckpoint {
    records: BTreeMap<DomainRecordRef, Staged>,
    directives: usize,
    registry: Option<CombatRegistryV1>,
    rulebook: Option<Rc<CombatRulebookV1>>,
    loss_deltas: BTreeMap<ForceId, LossTallyV1>,
}

fn decode_value<T: DeserializeOwned>(value: &Value) -> Result<T, CanwuError> {
    serde_json::from_value(value.clone())
        .map_err(|error| invalid(format!("staged military record is invalid: {error}")))
}

fn encode_value<T: Serialize>(value: &T) -> Result<Value, CanwuError> {
    serde_json::to_value(value)
        .map_err(|error| invalid(format!("military record could not be encoded: {error}")))
}

fn bounded(mut text: String) -> String {
    if text.len() > MAX_SUMMARY_BYTES {
        let mut end = MAX_SUMMARY_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text.trim().to_owned()
}

impl<'v, 'a> Run<'v, 'a> {
    fn new(
        view: &'v SimulationView<'a>,
        at: SimTime,
        signal: Option<PluginIngressTarget>,
        resolver: ResolverIdentity,
    ) -> Result<Self, CanwuError> {
        let registry = view
            .typed_domain_record(&registry_reference())?
            .map(canwu_api::DomainRecord::decode_payload::<RegistryRecord>)
            .transpose()?;
        Ok(Self {
            view,
            at,
            resolver,
            signal,
            records: BTreeMap::new(),
            directives: Vec::new(),
            registry_base: registry.clone(),
            registry,
            rulebook: None,
            loss_deltas: BTreeMap::new(),
        })
    }

    pub(crate) fn checkpoint(&self) -> RunCheckpoint {
        RunCheckpoint {
            records: self.records.clone(),
            directives: self.directives.len(),
            registry: self.registry.clone(),
            rulebook: self.rulebook.clone(),
            loss_deltas: self.loss_deltas.clone(),
        }
    }

    pub(crate) fn restore(&mut self, checkpoint: RunCheckpoint) {
        self.records = checkpoint.records;
        self.directives.truncate(checkpoint.directives);
        self.registry = checkpoint.registry;
        self.rulebook = checkpoint.rulebook;
        self.loss_deltas = checkpoint.loss_deltas;
    }

    /// An active record as staged so far.
    pub(crate) fn get<T: DomainRecordType>(
        &self,
        reference: &TypedDomainRecordRef<T>,
    ) -> Result<Option<T::Payload>, CanwuError>
    where
        T::Payload: DeserializeOwned,
    {
        let key = reference.as_untyped();
        if let Some(staged) = self.records.get(key) {
            return match staged {
                Staged::Write { payload, .. } => decode_value(payload).map(Some),
                Staged::Retire { .. } => Ok(None),
            };
        }
        match self.view.domain_record(key)? {
            Some(record) if record.lifecycle == DomainRecordLifecycle::Active => {
                Ok(Some(record.decode_payload::<T>()?))
            }
            _ => Ok(None),
        }
    }

    /// Whether a record exists in any lifecycle.
    pub(crate) fn exists_any(&self, key: &DomainRecordRef) -> Result<bool, CanwuError> {
        Ok(self.records.contains_key(key) || self.view.domain_record(key)?.is_some())
    }

    fn is_staged(&self, key: &DomainRecordRef) -> bool {
        self.records.contains_key(key)
    }

    /// The version a record had when the boundary began.
    fn committed_version(&self, key: &DomainRecordRef) -> Result<Option<u64>, CanwuError> {
        Ok(self.view.domain_record(key)?.map(|record| record.version))
    }

    pub(crate) fn put<T: DomainRecordType>(
        &mut self,
        reference: &TypedDomainRecordRef<T>,
        payload: &T::Payload,
    ) -> Result<(), CanwuError>
    where
        T::Payload: Serialize,
    {
        let key = reference.as_untyped().clone();
        let value = encode_value(payload)?;
        if let Some(staged) = self.records.get_mut(&key) {
            return match staged {
                Staged::Write { payload, .. } => {
                    *payload = value;
                    Ok(())
                }
                Staged::Retire { .. } => Err(invalid("a retired military record was written")),
            };
        }
        let base = match self.view.domain_record(&key)? {
            Some(record) if record.lifecycle == DomainRecordLifecycle::Active => {
                Some(record.version)
            }
            Some(_) => return Err(invalid("a retired military record was written")),
            None => None,
        };
        self.records.insert(
            key,
            Staged::Write {
                payload: value,
                base,
            },
        );
        Ok(())
    }

    pub(crate) fn create<T: DomainRecordType>(
        &mut self,
        reference: &TypedDomainRecordRef<T>,
        payload: &T::Payload,
    ) -> Result<(), CanwuError>
    where
        T::Payload: Serialize,
    {
        if self.exists_any(reference.as_untyped())? {
            return Err(CanwuError::new(
                ErrorCode::DuplicateDomainRecord,
                format!("military record {} exists already", reference.as_untyped()),
            ));
        }
        self.records.insert(
            reference.as_untyped().clone(),
            Staged::Write {
                payload: encode_value(payload)?,
                base: None,
            },
        );
        Ok(())
    }

    /// Retires an active record without a successor; a record this pass
    /// already wrote waits for a later pass.
    fn retire(&mut self, key: DomainRecordRef) -> Result<RetireOutcome, CanwuError> {
        if self.is_staged(&key) {
            return Ok(RetireOutcome::Deferred);
        }
        if let Some(record) = self.view.domain_record(&key)?
            && record.lifecycle == DomainRecordLifecycle::Active
        {
            let base = record.version;
            self.records.insert(key, Staged::Retire { base });
        }
        Ok(RetireOutcome::Done)
    }

    pub(crate) fn registry(&self) -> Option<&CombatRegistryV1> {
        self.registry.as_ref()
    }

    pub(crate) fn registry_slot(&mut self) -> &mut Option<CombatRegistryV1> {
        &mut self.registry
    }

    pub(crate) fn registry_mut(&mut self) -> Result<&mut CombatRegistryV1, CanwuError> {
        self.registry.as_mut().ok_or_else(|| {
            CanwuError::new(ErrorCode::InvalidPayload, "no combat rulebook is installed")
        })
    }

    pub(crate) fn rulebook(&mut self) -> Result<Option<Rc<CombatRulebookV1>>, CanwuError> {
        if let Some(rulebook) = &self.rulebook {
            return Ok(Some(Rc::clone(rulebook)));
        }
        let Some(id) = self
            .registry
            .as_ref()
            .and_then(|registry| registry.rulebook.clone())
        else {
            return Ok(None);
        };
        let Some(rulebook) = self.get(&rulebook_reference(&id))? else {
            return Ok(None);
        };
        let rulebook = Rc::new(rulebook);
        self.rulebook = Some(Rc::clone(&rulebook));
        Ok(Some(rulebook))
    }

    pub(crate) fn forget_rulebook(&mut self) {
        self.rulebook = None;
    }

    fn emit(&mut self, event_type: &str, summary: String, affected: Vec<EntityRef>) {
        self.directives.push(BoundaryDirective::Emit {
            event_type: event_type.to_owned(),
            summary: bounded(summary),
            affected,
        });
    }

    fn reject(&mut self, command: &crate::MilitaryCommandV1, error: &CanwuError) {
        let name = serde_json::to_value(command)
            .ok()
            .and_then(|value| {
                value
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        self.emit(
            MILITARY_REJECTION_EVENT,
            format!("{:?}: {name}: {}", error.code, error.message),
            Vec::new(),
        );
    }

    fn add_losses(&mut self, force: &ForceId, tally: &LossTallyV1) {
        if !tally.is_zero() {
            self.loss_deltas
                .entry(force.clone())
                .or_default()
                .add(tally);
        }
    }

    /// Queues a report: a progress item replaces the subject's queued
    /// progress item and drops the oldest progress item past the cap; a
    /// closing item is kept up to the queue's bound.
    fn queue_report(
        &mut self,
        rulebook: &CombatRulebookV1,
        subject: ReportSubjectV1,
        closing: bool,
    ) -> Result<(), CanwuError> {
        let at = self.at;
        let limits = &rulebook.limits;
        let registry = self.registry_mut()?;
        let mut dropped = Vec::new();
        if closing {
            registry.report_queue.push(ReportItemV1 {
                subject,
                closing,
                boundary_at: at,
            });
            while registry.report_queue.len() > limits.report_queue_bound() {
                let oldest = registry
                    .report_queue
                    .iter()
                    .position(|item| !item.closing)
                    .unwrap_or(0);
                dropped.push(registry.report_queue.remove(oldest));
            }
        } else {
            registry
                .report_queue
                .retain(|item| item.closing || item.subject != subject);
            let progress = registry
                .report_queue
                .iter()
                .filter(|item| !item.closing)
                .count();
            if progress >= limits.report_queue as usize
                && let Some(oldest) = registry.report_queue.iter().position(|item| !item.closing)
            {
                dropped.push(registry.report_queue.remove(oldest));
            }
            registry.report_queue.push(ReportItemV1 {
                subject,
                closing,
                boundary_at: at,
            });
        }
        for item in dropped {
            self.emit(
                REPORT_DROPPED_EVENT,
                format!("report on {:?} dropped from a full queue", item.subject),
                Vec::new(),
            );
        }
        Ok(())
    }

    /// The registry's earliest due work, recomputed from the open records.
    fn refresh_earliest_due(&mut self) -> Result<(), CanwuError> {
        let Some(registry) = self.registry.clone() else {
            return Ok(());
        };
        let mut due: Option<SimTime> = None;
        let mut take = |at: SimTime| {
            due = Some(due.map_or(at, |due| due.min(at)));
        };
        for id in &registry.open_engagements {
            if let Some(engagement) = self.get(&engagement_reference(id))?
                && engagement.status == EngagementStatus::Open
            {
                take(engagement.next_step_at);
            }
        }
        for id in &registry.open_sieges {
            if let Some(siege) = self.get(&siege_reference(id))?
                && siege.is_open()
            {
                take(siege.next_day_at);
            }
        }
        for id in &registry.open_crossings {
            if let Some(crossing) = self.get(&crossing_reference(id))?
                && crossing.is_open()
            {
                take(crossing.next_wave_at);
            }
        }
        if let Some(first) = registry.retire_queue.first() {
            take(first.at);
        }
        if !registry.steps_to_empty.is_empty() {
            take(self.at);
        }
        if let Some(registry) = self.registry.as_mut() {
            registry.earliest_due = due;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<BoundaryProposal, CanwuError> {
        if !self.records.is_empty() || self.registry != self.registry_base {
            self.refresh_earliest_due()?;
        }
        if self.registry != self.registry_base
            && let Some(registry) = self.registry.clone()
        {
            self.put(&registry_reference(), &registry)?;
        }
        let mut directives = Vec::with_capacity(self.records.len() + self.directives.len());
        for (key, staged) in self.records {
            let mutation = match staged {
                Staged::Write { payload, base } => match base {
                    None => DomainRecordMutation::Create {
                        record: DomainRecordDraft::new(key, payload),
                    },
                    Some(expected_version) => {
                        if self
                            .view
                            .domain_record(&key)?
                            .is_some_and(|record| record.payload == payload)
                        {
                            continue;
                        }
                        DomainRecordMutation::Update {
                            record: DomainRecordDraft::new(key, payload),
                            expected_version,
                        }
                    }
                },
                Staged::Retire { base } => DomainRecordMutation::Retire {
                    record: key,
                    expected_version: base,
                    successor: None,
                },
            };
            directives.push(BoundaryDirective::MutateRecord {
                mutation,
                summary: "Write one engagement-engine record".to_owned(),
            });
        }
        directives.extend(self.directives);
        Ok(BoundaryProposal {
            directives,
            ..BoundaryProposal::default()
        })
    }
}

/// Queues a record's retirement at `at`, in (time, record) order.
pub(crate) fn queue_retire(registry: &mut CombatRegistryV1, at: SimTime, record: DomainRecordRef) {
    let item = RetireItemV1 { at, record };
    if let Err(position) = registry.retire_queue.binary_search(&item) {
        registry.retire_queue.insert(position, item);
    }
}

fn retire_at(rulebook: &CombatRulebookV1, at: SimTime) -> Option<SimTime> {
    at.checked_add(SimDuration::minutes(
        i64::try_from(rulebook.limits.retire_after_minutes).unwrap_or(i64::MAX),
    ))
}

// ------------------------------------------------------------------ systems

/// The settle system (`settle-engagements-v1`, phase 7, event-driven).
pub(crate) fn settle_engagements<R: StepResolver>(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let signal = signal_target(&context.system)?;
    let mut applies = Vec::new();
    let mut steps = Vec::new();
    for id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*id)? else {
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
        if plugin != PLUGIN_NAME {
            continue;
        }
        if packet_type == MILITARY_APPLY_INGRESS {
            applies.push(decode_value::<AppliedCommandV1>(payload)?);
        } else if packet_type == MILITARY_STEP_INGRESS {
            steps.push((*id, decode_value::<StepPacketV1>(payload)?));
        }
    }
    if applies.is_empty() && steps.is_empty() {
        return Ok(BoundaryProposal::default());
    }
    let mut run = Run::new(view, context.at, signal, R::IDENTITY)?;
    for applied in &applies {
        let checkpoint = run.checkpoint();
        if let Err(error) = apply(&mut run, &applied.issuer, &applied.command) {
            run.restore(checkpoint);
            run.reject(&applied.command, &error);
        }
    }
    // Two step packets in one cut: the draws cite the first admitted, and
    // the boundary steps to the larger `up_to`.
    if let Some((packet, _)) = steps.first() {
        let up_to = steps
            .iter()
            .map(|(_, step)| step.up_to)
            .max()
            .unwrap_or(context.at);
        step_due::<R>(&mut run, *packet, up_to.min(context.at))?;
    }
    run.finish()
}

/// The wake system (`wake-engagements-on-cadence-v1`, phase 7, the host's
/// cadence): schedules a step packet, due at once, when work is due.
pub(crate) fn wake_engagements(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(record) = view.typed_domain_record(&registry_reference())? else {
        return Ok(BoundaryProposal::default());
    };
    let registry = record.decode_payload::<RegistryRecord>()?;
    if registry.earliest_due.is_none_or(|due| due > context.at) || !view.can_schedule_ingress() {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::SchedulePluginIngress {
            target_plugin: PLUGIN_NAME.to_owned(),
            after: SimDuration::ZERO,
            packet_type: MILITARY_STEP_INGRESS.to_owned(),
            priority: 0,
            payload: encode_value(&StepPacketV1 { up_to: context.at })?,
            affected: Vec::new(),
        }],
        ..BoundaryProposal::default()
    })
}

fn step_due<R: StepResolver>(
    run: &mut Run<'_, '_>,
    packet: IngressId,
    up: SimTime,
) -> Result<(), CanwuError> {
    let Some(rulebook) = run.rulebook()? else {
        return Ok(());
    };
    upkeep(run, up)?;
    let Some(registry) = run.registry().cloned() else {
        return Ok(());
    };
    for id in &registry.open_engagements {
        step_engagement::<R>(run, &rulebook, id, packet, up)?;
    }
    for id in &registry.open_sieges {
        advance_siege(run, &rulebook, id, up)?;
    }
    for id in &registry.open_crossings {
        advance_crossing(run, &rulebook, id, packet, up)?;
    }
    Ok(())
}

/// Empties the steps records of closed engagements and retires what is due.
fn upkeep(run: &mut Run<'_, '_>, up: SimTime) -> Result<(), CanwuError> {
    let (empty, due) = {
        let registry = run.registry_mut()?;
        let due: Vec<RetireItemV1> = registry
            .retire_queue
            .iter()
            .take_while(|item| item.at <= up)
            .cloned()
            .collect();
        (
            registry.steps_to_empty.iter().cloned().collect::<Vec<_>>(),
            due,
        )
    };
    let at = run.at;
    for id in empty {
        let reference = engagement_steps_reference(&id);
        if run.is_staged(reference.as_untyped()) {
            continue;
        }
        if let Some(mut steps) = run.get(&reference)?
            && !steps.steps.is_empty()
        {
            steps.steps.clear();
            steps.boundary_at = at;
            run.put(&reference, &steps)?;
        }
        run.registry_mut()?.steps_to_empty.remove(&id);
    }
    for item in due {
        if matches!(run.retire(item.record.clone())?, RetireOutcome::Done) {
            run.registry_mut()?
                .retire_queue
                .retain(|queued| queued != &item);
        }
    }
    Ok(())
}

// ------------------------------------------------------------- engagements

/// What opens an engagement, by command or by a siege or crossing.
pub(crate) struct OpenSpec {
    pub id: EngagementId,
    pub kind: EngagementKind,
    pub profile: ProfileId,
    pub site: EntityRef,
    pub fort: Option<(u8, bool)>,
    pub terrain: Option<TerrainClassId>,
    pub siege: Option<SiegeId>,
    pub crossing: Option<CrossingId>,
    pub attackers: Vec<ForceId>,
    pub defenders: Vec<ForceId>,
    pub holders: [Vec<KnowledgeHolderRef>; 2],
    pub inputs: [Option<(crate::ConditionInputV1, SimTime)>; 2],
    pub environment: Option<(crate::EnvironmentInputV1, SimTime)>,
    pub men_caps: [Option<u64>; 2],
    pub opened_at: SimTime,
}

/// The men-weighted floor mean of the forces' terms (traits intersected),
/// cohesion, and shock, and their men.
fn aggregate(forces: &[&ForceStateV1]) -> (SideTermsV1, u32, u32, u64) {
    let weights: Vec<u128> = forces.iter().map(|force| u128::from(force.men())).collect();
    let total: u128 = weights.iter().sum();
    let mean = |value: &dyn Fn(&ForceStateV1) -> u32| -> u32 {
        if forces.is_empty() {
            return 0;
        }
        let (sum, weight) = if total == 0 {
            (
                forces
                    .iter()
                    .map(|force| u128::from(value(force)))
                    .sum::<u128>(),
                forces.len() as u128,
            )
        } else {
            (
                forces
                    .iter()
                    .zip(&weights)
                    .map(|(force, weight)| u128::from(value(force)) * weight)
                    .sum::<u128>(),
                total,
            )
        };
        u32::try_from(sum / weight.max(1)).unwrap_or(u32::MAX)
    };
    let mut traits = forces
        .first()
        .map(|force| force.terms.traits.clone())
        .unwrap_or_default();
    for force in forces.iter().skip(1) {
        traits.retain(|trait_| force.terms.traits.contains(trait_));
    }
    let terms = SideTermsV1 {
        cohesion_cap: mean(&|force| force.terms.cohesion_cap),
        morale_break: mean(&|force| force.terms.morale_break),
        morale_sensitivity: mean(&|force| force.terms.morale_sensitivity),
        battle_command: mean(&|force| force.terms.battle_command),
        assessment: mean(&|force| force.terms.assessment),
        credibility: mean(&|force| force.terms.credibility),
        traits,
    };
    (
        terms,
        mean(&ForceStateV1::cohesion),
        mean(&|force| force.battle.morale_shock),
        u64::try_from(total).unwrap_or(u64::MAX),
    )
}

/// A side's components in (force id, component id) order with the men
/// that fight: all present men, or the side's cap split by largest
/// remainder.
fn side_parts(
    forces: &BTreeMap<ForceId, ForceStateV1>,
    side: &[ForceId],
    cap: Option<u64>,
) -> Vec<(ForceId, ComponentId, u64)> {
    let mut ids: Vec<&ForceId> = side.iter().collect();
    ids.sort();
    let mut parts = Vec::new();
    for id in ids {
        let Some(force) = forces.get(id) else {
            continue;
        };
        if !force.is_active() {
            continue;
        }
        for (component, state) in &force.components {
            if state.present > 0 {
                parts.push((id.clone(), component.clone(), state.present));
            }
        }
    }
    let men = parts
        .iter()
        .fold(0_u64, |sum, part| sum.saturating_add(part.2));
    if let Some(cap) = cap
        && cap < men
    {
        let weights: Vec<u64> = parts.iter().map(|part| part.2).collect();
        for (part, share) in parts.iter_mut().zip(largest_remainder(cap, &weights)) {
            part.2 = share;
        }
        parts.retain(|part| part.2 > 0);
    }
    parts
}

fn component_values<'f>(
    forces: &'f BTreeMap<ForceId, ForceStateV1>,
    force: &ForceId,
    component: &ComponentId,
) -> Option<&'f ComponentValuesV1> {
    forces
        .get(force)?
        .components
        .get(component)
        .map(|state| &state.values)
}

/// The fortification a step reads: the engagement's, opened by its siege's
/// breach or gate, as the kind uses it.
fn effective_fort(
    run: &Run<'_, '_>,
    kind_rule: &KindRuleV1,
    engagement: &EngagementStateV1,
    at: SimTime,
) -> Result<Option<(u8, bool)>, CanwuError> {
    let Some((grade, mut open)) = engagement.fort else {
        return Ok(None);
    };
    if let Some(siege) = &engagement.siege
        && let Some(state) = run.get(&siege_reference(siege))?
    {
        open = state.breach_open || state.gate_open_at(at);
    }
    Ok(match kind_rule.fort {
        FortUse::None => None,
        FortUse::Site => Some((grade, open)),
        FortUse::SiteHalved => Some((grade, true)),
    })
}

fn load_forces(
    run: &Run<'_, '_>,
    ids: impl Iterator<Item = ForceId>,
) -> Result<BTreeMap<ForceId, ForceStateV1>, CanwuError> {
    let mut forces = BTreeMap::new();
    for id in ids {
        if let Some(force) = run.get(&force_reference(&id))? {
            forces.insert(id, force);
        }
    }
    Ok(forces)
}

fn write_forces(
    run: &mut Run<'_, '_>,
    forces: &BTreeMap<ForceId, ForceStateV1>,
) -> Result<(), CanwuError> {
    for (id, force) in forces {
        run.put(&force_reference(id), force)?;
    }
    Ok(())
}

fn affected(engagement: &EngagementStateV1) -> Vec<EntityRef> {
    let mut entities: Vec<EntityRef> = engagement
        .forces()
        .map(|id| EntityRef::Domain(force_reference(id).into_untyped()))
        .collect();
    entities.push(engagement.site.clone());
    entities.sort();
    entities.dedup();
    entities
}

pub(crate) fn open_engagement(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    spec: OpenSpec,
) -> Result<(), CanwuError> {
    let kind_rule = rulebook
        .kinds
        .get(&spec.kind)
        .ok_or_else(|| invalid("the engagement kind has no rule"))?;
    let profile = rulebook
        .step_profiles
        .get(&spec.profile)
        .ok_or_else(|| invalid("the step profile is unknown"))?;
    let (Some(attacker_input), Some(defender_input), Some(environment)) = (
        spec.inputs[0].clone(),
        spec.inputs[1].clone(),
        spec.environment.clone(),
    ) else {
        return Err(invalid(
            "an engagement opens with both inputs and an environment",
        ));
    };
    let terrain = spec
        .terrain
        .clone()
        .or_else(|| environment.0.terrain.clone())
        .or_else(|| kind_rule.terrain.clone())
        .ok_or_else(|| invalid("an engagement opens on a terrain"))?;
    let mut forces = load_forces(run, spec.attackers.iter().chain(&spec.defenders).cloned())?;
    for force in forces.values_mut() {
        force.recover(rulebook, spec.opened_at);
        force.battle.engaged_in = Some(spec.id.clone());
    }
    for id in &spec.attackers {
        if let Some(force) = forces.get_mut(id) {
            force.battle.posture = Some(attacker_input.0.posture.clone());
        }
    }
    for id in &spec.defenders {
        if let Some(force) = forces.get_mut(id) {
            force.battle.posture = Some(defender_input.0.posture.clone());
        }
    }
    let attacker_condition = attacker_input.0.clone();
    let defender_condition = defender_input.0.clone();
    let side = |ids: &[ForceId], holders: &[KnowledgeHolderRef], input, cap: Option<u64>| {
        let members: Vec<&ForceStateV1> = ids.iter().filter_map(|id| forces.get(id)).collect();
        let (_, cohesion, shock, men) = aggregate(&members);
        SideStateV1 {
            forces: ids.to_vec(),
            holders: holders.to_vec(),
            input: Some(input),
            morale_shock: shock,
            cohesion,
            posture_after: Continuation::Continue,
            losses: LossTallyV1::default(),
            opened_men: cap.map_or(men, |cap| cap.min(men)),
            men_cap: cap,
            withdraw_ordered: false,
        }
    };
    let attacker = side(
        &spec.attackers,
        &spec.holders[0],
        attacker_input,
        spec.men_caps[0],
    );
    let defender = side(
        &spec.defenders,
        &spec.holders[1],
        defender_input,
        spec.men_caps[1],
    );
    let forecast = {
        let parts = [
            side_parts(&forces, &spec.attackers, spec.men_caps[0]),
            side_parts(&forces, &spec.defenders, spec.men_caps[1]),
        ];
        let members = [
            spec.attackers
                .iter()
                .filter_map(|id| forces.get(id))
                .collect::<Vec<_>>(),
            spec.defenders
                .iter()
                .filter_map(|id| forces.get(id))
                .collect::<Vec<_>>(),
        ];
        let terms = [aggregate(&members[0]).0, aggregate(&members[1]).0];
        let state = [&attacker, &defender];
        let conditions = [&attacker_condition, &defender_condition];
        let sides: Vec<ForecastSideV1<'_>> = (0..2)
            .map(|index| {
                let input = conditions[index];
                ForecastSideV1 {
                    components: parts[index]
                        .iter()
                        .filter_map(|(force, component, present)| {
                            component_values(&forces, force, component)
                                .map(|values| (*present, values))
                        })
                        .collect(),
                    terms: &terms[index],
                    condition: input,
                    cohesion: state[index].cohesion,
                    morale: u32::from(input.morale_per_mille)
                        .saturating_sub(state[index].morale_shock),
                }
            })
            .collect();
        let fort = match kind_rule.fort {
            FortUse::None => None,
            FortUse::Site => spec.fort,
            FortUse::SiteHalved => spec.fort.map(|(grade, _)| (grade, true)),
        };
        let fortified = fort.is_some_and(|(grade, open)| grade > 0 && !open);
        forecast(
            rulebook,
            &terrain,
            &environment.0,
            fortified,
            &sides[0],
            &sides[1],
        )?
    };
    let engagement = EngagementStateV1 {
        schema_version: SCHEMA_VERSION,
        id: spec.id.clone(),
        host_revision: 0,
        kind: spec.kind,
        profile: spec.profile.clone(),
        site: spec.site.clone(),
        fort: spec.fort,
        terrain: spec.terrain.clone(),
        siege: spec.siege.clone(),
        crossing: spec.crossing.clone(),
        attacker,
        defender,
        environment: Some(environment),
        forecast,
        opened_at: spec.opened_at,
        last_step_at: spec.opened_at,
        next_step_at: add_minutes(spec.opened_at, profile.step_minutes)?,
        steps_resolved: 0,
        status: EngagementStatus::Open,
        outcome: None,
    };
    engagement.validate()?;
    run.create(&engagement_reference(&spec.id), &engagement)?;
    run.create(
        &engagement_steps_reference(&spec.id),
        &EngagementStepsV1 {
            schema_version: SCHEMA_VERSION,
            engagement: spec.id.clone(),
            boundary_at: run.at,
            steps: Vec::new(),
        },
    )?;
    write_forces(run, &forces)?;
    run.registry_mut()?.open_engagements.insert(spec.id.clone());
    run.emit(
        ENGAGEMENT_OPENED_EVENT,
        format!("{:?} engagement {} opened", spec.kind, spec.id),
        affected(&engagement),
    );
    Ok(())
}

/// Adds co-located free forces to one side and recomposes its cohesion and
/// shock.
pub(crate) fn join_side(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    engagement: &mut EngagementStateV1,
    side: Side,
    joining: Vec<ForceStateV1>,
) -> Result<(), CanwuError> {
    let at = run.at;
    let posture = engagement
        .side(side)
        .input
        .as_ref()
        .map(|(input, _)| input.posture.clone());
    let mut members = load_forces(run, engagement.side(side).forces.iter().cloned())?;
    let mut added = 0_u64;
    for mut force in joining {
        force.recover(rulebook, at);
        force.battle.engaged_in = Some(engagement.id.clone());
        force.battle.posture.clone_from(&posture);
        added = added.saturating_add(force.men());
        engagement.side_mut(side).forces.push(force.id.clone());
        run.put(&force_reference(&force.id), &force)?;
        members.insert(force.id.clone(), force);
    }
    let all: Vec<&ForceStateV1> = members.values().collect();
    let (_, cohesion, shock, _) = aggregate(&all);
    let state = engagement.side_mut(side);
    state.cohesion = cohesion;
    state.morale_shock = shock;
    state.opened_men = state.opened_men.saturating_add(added);
    Ok(())
}

/// How an engagement closes.
pub(crate) struct CloseInfo {
    pub kind: EngagementOutcomeKind,
    pub routed: [Vec<ForceId>; 2],
    pub site_fell: bool,
    pub at: SimTime,
}

enum StepResult {
    Applied {
        step: StepV1,
        close: Option<CloseInfo>,
    },
    Held {
        reason: HoldReason,
        step: Option<StepV1>,
    },
    /// A side has no men left: the engagement closes without a step.
    Ended(CloseInfo),
}

/// The plugin's accessor for a resolver's keyed draws.
struct Draws<'v, 'a> {
    view: &'v SimulationView<'a>,
    packet: IngressId,
    operation: String,
    target: RandomOperationTarget,
    limit: usize,
    used: BTreeSet<u32>,
    ids: Vec<RandomDrawId>,
    exceeded: bool,
}

impl StepDraws for Draws<'_, '_> {
    fn draw(
        &mut self,
        side: Side,
        participant: u16,
        purpose: u8,
        upper_exclusive: u64,
    ) -> Result<(u64, RandomDrawId), CanwuError> {
        if self.ids.len() >= self.limit {
            self.exceeded = true;
            return Err(CanwuError::new(
                ErrorCode::ValueOutOfRange,
                "a step asked for more draws than the rulebook allows",
            ));
        }
        if participant >= 128 {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "a draw participant must be below 128",
            ));
        }
        let slot = u32::try_from(side.index()).unwrap_or(1) * 32_768
            + u32::from(participant) * 256
            + u32::from(purpose);
        if !self.used.insert(slot) {
            return Err(CanwuError::new(
                ErrorCode::InvalidRandomDraw,
                "a draw slot was asked for twice in one step",
            ));
        }
        let draw = self.view.random_draw_for_operation(
            &step_contingency_stream(),
            EvidenceRef::Ingress(self.packet),
            "contingency",
            &self.operation,
            self.target.clone(),
            slot,
            upper_exclusive,
            "step-contingency",
        )?;
        self.ids.push(draw.draw_id);
        Ok((draw.sample.value, draw.draw_id))
    }
}

/// One side's applied outcome of a step.
struct SidePlan {
    casualties: u64,
    killed: u64,
    wounded_left: u64,
    pursuit: u64,
    cohesion_after: u32,
    shock: u32,
    continuation: Continuation,
    step: SideStepV1,
}

fn plan_side(
    rulebook: &CombatRulebookV1,
    terrain: &TerrainRowV1,
    night: bool,
    state: &SideStateV1,
    terms: &SideTermsV1,
    morale: u32,
    out: &SideOutcomeV1,
    men: u64,
    walled: bool,
    age: u32,
) -> (SidePlan, bool) {
    let inflicted = out.inflicted_per_mille.min(1_000);
    let casualties = out.casualties.min(men);
    let killed = out.killed.min(casualties);
    let cohesion_after = out.cohesion_after.min(terms.cohesion_cap.min(1_000));
    let shock = out.morale_shock;
    let morale_after = morale.saturating_sub(shock);
    let clamped = inflicted != out.inflicted_per_mille
        || casualties != out.casualties
        || killed != out.killed
        || cohesion_after != out.cohesion_after
        || morale_after != out.morale_after;
    let rule = &rulebook.break_rule;
    let mut continuation = if walled {
        Continuation::Continue
    } else if cohesion_after < rule.rout_cohesion_below
        || morale_after.saturating_add(rule.rout_morale_margin) < terms.morale_break
    {
        Continuation::Rout
    } else if morale_after < terms.morale_break {
        Continuation::Withdraw
    } else {
        Continuation::Continue
    };
    if state.withdraw_ordered && continuation == Continuation::Continue {
        continuation = Continuation::Withdraw;
    }
    let wounded = casualties - killed;
    let (pursuit, wounded_left) = if continuation == Continuation::Rout {
        let pursuit_bp = if night {
            i128::from(terrain.pursuit_bp) * i128::from(rulebook.night.pursuit_bp) / BP
        } else {
            i128::from(terrain.pursuit_bp)
        };
        let engaged = i128::from(out.lines.engaged.min(men));
        let captured =
            (engaged * pursuit_bp * i128::from(rulebook.pursuit.captured_per_mille_at_full))
                / 10_000_000;
        let pursuit = u64::try_from(captured.max(0))
            .unwrap_or(u64::MAX)
            .min(men - casualties);
        let left = u64::try_from(
            u128::from(wounded) * u128::from(rulebook.pursuit.wounded_left_per_mille) / 1_000,
        )
        .unwrap_or(0)
        .min(wounded);
        (pursuit, left)
    } else {
        (0, 0)
    };
    (
        SidePlan {
            casualties,
            killed,
            wounded_left,
            pursuit,
            cohesion_after,
            shock,
            continuation,
            step: SideStepV1 {
                lines: out.lines,
                inflicted_per_mille: inflicted,
                killed,
                wounded: wounded - wounded_left,
                captured: pursuit + wounded_left,
                dispersed: 0,
                cohesion_after,
                morale_after,
                morale_shock: shock,
                posture_after: continuation,
                condition_age_steps: age,
            },
        },
        clamped,
    )
}

/// Splits a side's losses over its components by largest remainder and
/// applies them.
fn allocate_side(
    run: &mut Run<'_, '_>,
    forces: &mut BTreeMap<ForceId, ForceStateV1>,
    parts: &[(ForceId, ComponentId, u64)],
    plan: &SidePlan,
) -> LossTallyV1 {
    let weights: Vec<u64> = parts.iter().map(|part| part.2).collect();
    let casualties = largest_remainder(plan.casualties, &weights);
    let killed = largest_remainder(plan.killed, &casualties);
    let wounded: Vec<u64> = casualties
        .iter()
        .zip(&killed)
        .map(|(all, killed)| all - killed)
        .collect();
    let left = largest_remainder(plan.wounded_left, &wounded);
    let remaining: Vec<u64> = weights
        .iter()
        .zip(&casualties)
        .map(|(present, lost)| present.saturating_sub(*lost))
        .collect();
    let pursuit = largest_remainder(plan.pursuit, &remaining);
    let mut total = LossTallyV1::default();
    for (index, (force_id, component_id, _)) in parts.iter().enumerate() {
        let tally = LossTallyV1 {
            killed: killed[index],
            wounded: wounded[index] - left[index],
            captured: left[index] + pursuit[index],
            dispersed: 0,
        };
        if let Some(component) = forces
            .get_mut(force_id)
            .and_then(|force| force.components.get_mut(component_id))
        {
            component.present = component.present.saturating_sub(tally.total());
            component.losses.add(&tally);
        }
        run.add_losses(force_id, &tally);
        total.add(&tally);
    }
    total
}

fn empty_step<R: StepResolver>(
    index: u32,
    at: SimTime,
    draws: Vec<RandomDrawId>,
) -> Result<StepV1, CanwuError> {
    Ok(StepV1 {
        index,
        at,
        attacker: SideStepV1::default(),
        defender: SideStepV1::default(),
        attacker_channels: Vec::new(),
        defender_channels: Vec::new(),
        condition_digest: String::new(),
        environment_digest: String::new(),
        terms: vec![TermId::new(TERM_RESOLVER_FAILED)?],
        draws,
        resolver: R::IDENTITY.owned(),
    })
}

fn oversized(rulebook: &CombatRulebookV1, step: &StepV1) -> Result<bool, CanwuError> {
    let bytes = serde_json::to_vec(step)
        .map_err(|error| invalid(format!("a step could not be encoded: {error}")))?;
    Ok(bytes.len() > rulebook.limits.step_bytes as usize)
}

fn minutes_between(from: SimTime, to: SimTime) -> i64 {
    to.checked_sub(from).map_or(0, SimDuration::as_minutes)
}

fn one_step<R: StepResolver>(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    engagement: &mut EngagementStateV1,
    forces: &mut BTreeMap<ForceId, ForceStateV1>,
    packet: IngressId,
    version: u64,
    at: SimTime,
) -> Result<StepResult, CanwuError> {
    let held = |reason| Ok(StepResult::Held { reason, step: None });
    let (Some(kind_rule), Some(profile)) = (
        rulebook.kinds.get(&engagement.kind),
        rulebook.step_profiles.get(&engagement.profile),
    ) else {
        return held(HoldReason::MissingInput);
    };
    let (
        Some((attacker_input, attacker_at)),
        Some((defender_input, defender_at)),
        Some((environment, _)),
    ) = (
        engagement.attacker.input.clone(),
        engagement.defender.input.clone(),
        engagement.environment.clone(),
    )
    else {
        return held(HoldReason::MissingInput);
    };
    let Some(terrain_class) = engagement
        .terrain
        .clone()
        .or_else(|| environment.terrain.clone())
        .or_else(|| kind_rule.terrain.clone())
    else {
        return held(HoldReason::MissingInput);
    };
    let Some(terrain) = rulebook.terrain.get(&terrain_class) else {
        return held(HoldReason::MissingInput);
    };
    let fort = effective_fort(run, kind_rule, engagement, at)?;
    let parts = [
        side_parts(
            forces,
            &engagement.attacker.forces,
            engagement.attacker.men_cap,
        ),
        side_parts(
            forces,
            &engagement.defender.forces,
            engagement.defender.men_cap,
        ),
    ];
    let men: [u64; 2] = [
        parts[0]
            .iter()
            .fold(0_u64, |sum, part| sum.saturating_add(part.2)),
        parts[1]
            .iter()
            .fold(0_u64, |sum, part| sum.saturating_add(part.2)),
    ];
    if men[0] == 0 {
        return Ok(StepResult::Ended(CloseInfo {
            kind: EngagementOutcomeKind::Repulsed,
            routed: [engagement.attacker.forces.clone(), Vec::new()],
            site_fell: false,
            at,
        }));
    }
    if men[1] == 0 {
        return Ok(StepResult::Ended(CloseInfo {
            kind: EngagementOutcomeKind::AttackerPrevailed,
            routed: [Vec::new(), engagement.defender.forces.clone()],
            site_fell: engagement.kind == EngagementKind::Assault,
            at,
        }));
    }
    let terms = [
        aggregate(
            &engagement
                .attacker
                .forces
                .iter()
                .filter_map(|id| forces.get(id))
                .collect::<Vec<_>>(),
        )
        .0,
        aggregate(
            &engagement
                .defender
                .forces
                .iter()
                .filter_map(|id| forces.get(id))
                .collect::<Vec<_>>(),
        )
        .0,
    ];
    let morale = [
        u32::from(attacker_input.morale_per_mille).saturating_sub(engagement.attacker.morale_shock),
        u32::from(defender_input.morale_per_mille).saturating_sub(engagement.defender.morale_shock),
    ];
    let step_minutes = i64::from(profile.step_minutes.max(1));
    let age = |since: SimTime| {
        u32::try_from(minutes_between(since, at) / step_minutes).unwrap_or(u32::MAX)
    };
    let ages = [age(attacker_at), age(defender_at)];
    let index = engagement.steps_resolved;
    let mut draws = Draws {
        view: run.view,
        packet,
        operation: format!("contingency:{}:{index}", engagement.id),
        target: RandomOperationTarget::DomainRecord {
            record: engagement_reference(&engagement.id).into_untyped(),
            version,
        },
        limit: rulebook.limits.draws_per_step as usize,
        used: BTreeSet::new(),
        ids: Vec::new(),
        exceeded: false,
    };
    let result = {
        let components = |side: usize| -> Vec<StepComponentV1<'_>> {
            parts[side]
                .iter()
                .filter_map(|(force, component, present)| {
                    let values = component_values(forces, force, component)?;
                    let (force, component) =
                        forces.get_key_value(force).and_then(|(force, state)| {
                            state
                                .components
                                .get_key_value(component)
                                .map(|(component, _)| (force, component))
                        })?;
                    Some(StepComponentV1 {
                        force,
                        component,
                        present: *present,
                        values,
                    })
                })
                .collect()
        };
        let input = StepInputV1 {
            rulebook,
            kind: engagement.kind,
            kind_rule,
            profile,
            step_index: index,
            attacker: SideStepInputV1 {
                components: components(0),
                terms: terms[0].clone(),
                condition: &attacker_input,
                condition_age_steps: ages[0],
                cohesion: engagement.attacker.cohesion,
                morale: morale[0],
                attacking: true,
            },
            defender: SideStepInputV1 {
                components: components(1),
                terms: terms[1].clone(),
                condition: &defender_input,
                condition_age_steps: ages[1],
                cohesion: engagement.defender.cohesion,
                morale: morale[1],
                attacking: false,
            },
            environment: &environment,
            terrain_class: &terrain_class,
            terrain,
            fort,
        };
        R::default().resolve(&input, &mut draws)
    };
    let draw_ids = std::mem::take(&mut draws.ids);
    let exceeded = draws.exceeded;
    let outcome = match result {
        Ok(outcome) if !exceeded => outcome,
        _ => {
            let reason = if exceeded {
                HoldReason::DrawLimit
            } else {
                HoldReason::ResolverFailed
            };
            if draw_ids.is_empty() {
                return held(reason);
            }
            let step = empty_step::<R>(index, at, draw_ids)?;
            if oversized(rulebook, &step)? {
                return held(HoldReason::StepOversized);
            }
            return Ok(StepResult::Held {
                reason,
                step: Some(step),
            });
        }
    };
    let walled = fort
        .is_some_and(|(grade, open)| !open && grade >= rulebook.break_rule.walled_holds_from_grade);
    let (attacker_plan, attacker_clamped) = plan_side(
        rulebook,
        terrain,
        environment.night,
        &engagement.attacker,
        &terms[0],
        morale[0],
        &outcome.attacker,
        men[0],
        false,
        ages[0],
    );
    let (defender_plan, defender_clamped) = plan_side(
        rulebook,
        terrain,
        environment.night,
        &engagement.defender,
        &terms[1],
        morale[1],
        &outcome.defender,
        men[1],
        walled,
        ages[1],
    );
    let mut step_terms = outcome.terms.clone();
    if attacker_clamped || defender_clamped {
        step_terms.push(TermId::new(TERM_RESOLVER_CLAMPED)?);
    }
    if attacker_plan.pursuit > 0 || defender_plan.pursuit > 0 {
        step_terms.push(TermId::new(TERM_PURSUIT)?);
    }
    let step = StepV1 {
        index,
        at,
        attacker: attacker_plan.step.clone(),
        defender: defender_plan.step.clone(),
        attacker_channels: attacker_input.channels.clone(),
        defender_channels: defender_input.channels.clone(),
        condition_digest: digest(&(&attacker_input, &defender_input))?,
        environment_digest: digest(&environment)?,
        terms: step_terms,
        draws: draw_ids,
        resolver: R::IDENTITY.owned(),
    };
    // Holding after the resolver drew is safe: the held engagement is
    // written in this boundary, so its record version (the draws' target)
    // changes, and a later step draws fresh addresses, never this step's
    // with other evidence.
    if oversized(rulebook, &step)? {
        return held(HoldReason::StepOversized);
    }
    for (side, plan, side_parts) in [
        (Side::Attacker, &attacker_plan, &parts[0]),
        (Side::Defender, &defender_plan, &parts[1]),
    ] {
        let tally = allocate_side(run, forces, side_parts, plan);
        let state = engagement.side_mut(side);
        state.losses.add(&tally);
        state.cohesion = plan.cohesion_after;
        state.morale_shock = state.morale_shock.saturating_add(plan.shock);
        state.posture_after = plan.continuation;
        let shock = state.morale_shock;
        for id in &state.forces {
            if let Some(force) = forces.get_mut(id) {
                force.battle.cohesion = Some(plan.cohesion_after.min(force.terms.cohesion_cap));
                force.battle.morale_shock = shock;
                force.battle.last_fought_at = Some(at);
            }
        }
    }
    let routed = |side: &SideStateV1, plan: &SidePlan| {
        if plan.continuation == Continuation::Rout {
            side.forces.clone()
        } else {
            Vec::new()
        }
    };
    let close = if attacker_plan.continuation != Continuation::Continue {
        Some(EngagementOutcomeKind::Repulsed)
    } else if defender_plan.continuation != Continuation::Continue {
        Some(EngagementOutcomeKind::AttackerPrevailed)
    } else {
        None
    };
    let close = close.map(|kind| CloseInfo {
        kind,
        routed: [
            routed(&engagement.attacker, &attacker_plan),
            routed(&engagement.defender, &defender_plan),
        ],
        site_fell: engagement.kind == EngagementKind::Assault
            && kind == EngagementOutcomeKind::AttackerPrevailed,
        at,
    });
    Ok(StepResult::Applied { step, close })
}

fn step_engagement<R: StepResolver>(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    id: &EngagementId,
    packet: IngressId,
    up: SimTime,
) -> Result<(), CanwuError> {
    let reference = engagement_reference(id);
    let Some(mut engagement) = run.get(&reference)? else {
        run.registry_mut()?.open_engagements.remove(id);
        return Ok(());
    };
    if engagement.status != EngagementStatus::Open || engagement.next_step_at > up {
        return Ok(());
    }
    let Some(profile) = rulebook.step_profiles.get(&engagement.profile).cloned() else {
        return Ok(());
    };
    let max_minutes = rulebook
        .kinds
        .get(&engagement.kind)
        .map_or(u32::MAX, |rule| rule.max_minutes);
    let step_minutes = i64::from(profile.step_minutes.max(1));
    let elapsed = minutes_between(engagement.last_step_at, up) / step_minutes;
    let count = elapsed.min(i64::from(profile.max_steps_per_boundary));
    let Some(version) = run.committed_version(reference.as_untyped())? else {
        return Ok(());
    };
    let mut forces = load_forces(
        run,
        engagement.forces().cloned().collect::<Vec<_>>().into_iter(),
    )?;
    let mut steps = Vec::new();
    let mut close = None;
    let mut hold = None;
    for _ in 0..count {
        let at = add_minutes(engagement.last_step_at, profile.step_minutes)?;
        match one_step::<R>(
            run,
            rulebook,
            &mut engagement,
            &mut forces,
            packet,
            version,
            at,
        )? {
            StepResult::Applied { step, close: ends } => {
                steps.push(step);
                engagement.last_step_at = at;
                engagement.steps_resolved += 1;
                if let Some(ends) = ends {
                    close = Some(ends);
                    break;
                }
            }
            StepResult::Held { reason, step } => {
                if let Some(step) = step {
                    steps.push(step);
                    engagement.last_step_at = at;
                    engagement.steps_resolved += 1;
                }
                hold = Some(reason);
                break;
            }
            StepResult::Ended(ends) => {
                close = Some(ends);
                break;
            }
        }
        if minutes_between(engagement.opened_at, at) >= i64::from(max_minutes) {
            close = Some(CloseInfo {
                kind: EngagementOutcomeKind::Stalled,
                routed: [Vec::new(), Vec::new()],
                site_fell: false,
                at,
            });
            break;
        }
    }
    engagement.next_step_at = add_minutes(engagement.last_step_at, profile.step_minutes)?;
    if !steps.is_empty() {
        run.put(
            &engagement_steps_reference(id),
            &EngagementStepsV1 {
                schema_version: SCHEMA_VERSION,
                engagement: id.clone(),
                boundary_at: run.at,
                steps,
            },
        )?;
    }
    if let Some(reason) = hold {
        engagement.status = EngagementStatus::Held { reason };
        run.emit(
            ENGAGEMENT_HELD_EVENT,
            format!("engagement {id} held: {reason:?}"),
            affected(&engagement),
        );
    }
    if let Some(close) = close {
        return close_engagement(run, rulebook, engagement, close, Some(forces));
    }
    write_forces(run, &forces)?;
    run.put(&reference, &engagement)?;
    run.queue_report(rulebook, ReportSubjectV1::Engagement(id.clone()), false)
}

/// Closes an engagement: its summary, its forces freed, its retirement and
/// report queued, the event and the optional signal, and the effect on its
/// siege or crossing.
pub(crate) fn close_engagement(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    mut engagement: EngagementStateV1,
    close: CloseInfo,
    forces: Option<BTreeMap<ForceId, ForceStateV1>>,
) -> Result<(), CanwuError> {
    let mut forces = match forces {
        Some(forces) => forces,
        None => load_forces(
            run,
            engagement.forces().cloned().collect::<Vec<_>>().into_iter(),
        )?,
    };
    let id = engagement.id.clone();
    engagement.status = EngagementStatus::Closed;
    engagement.outcome = Some(EngagementOutcomeV1 {
        kind: close.kind,
        closed_at: close.at,
        site_fell: close.site_fell,
    });
    for force in forces.values_mut() {
        if force.battle.engaged_in.as_ref() == Some(&id) {
            force.battle.engaged_in = None;
            force.battle.posture = None;
        }
    }
    write_forces(run, &forces)?;
    let summary = EngagementSummaryV1 {
        schema_version: SCHEMA_VERSION,
        engagement: id.clone(),
        kind: engagement.kind,
        site: engagement.site.clone(),
        opened_at: engagement.opened_at,
        closed_at: close.at,
        steps: engagement.steps_resolved,
        outcome: close.kind,
        site_fell: close.site_fell,
        attacker: SideSummaryV1 {
            forces: engagement.attacker.forces.clone(),
            losses: engagement.attacker.losses,
            routed: close.routed[0].clone(),
        },
        defender: SideSummaryV1 {
            forces: engagement.defender.forces.clone(),
            losses: engagement.defender.losses,
            routed: close.routed[1].clone(),
        },
        siege: engagement.siege.clone(),
        crossing: engagement.crossing.clone(),
    };
    run.create(&engagement_summary_reference(&id), &summary)?;
    run.put(&engagement_reference(&id), &engagement)?;
    {
        let retire = retire_at(rulebook, close.at);
        let registry = run.registry_mut()?;
        registry.open_engagements.remove(&id);
        registry.steps_to_empty.insert(id.clone());
        if let Some(retire) = retire {
            queue_retire(registry, retire, engagement_reference(&id).into_untyped());
            queue_retire(
                registry,
                retire,
                engagement_steps_reference(&id).into_untyped(),
            );
        }
    }
    run.queue_report(rulebook, ReportSubjectV1::Engagement(id.clone()), true)?;
    if let Some(siege) = engagement.siege.clone() {
        siege_after_fight(run, rulebook, &siege, &engagement, &close)?;
    }
    if let Some(crossing) = engagement.crossing.clone() {
        let reference = crossing_reference(&crossing);
        if let Some(mut state) = run.get(&reference)?
            && state.engagement.as_ref() == Some(&id)
        {
            state.engagement = None;
            state.next_wave_at = state.next_wave_at.max(close.at);
            run.put(&reference, &state)?;
        }
    }
    run.emit(
        ENGAGEMENT_CLOSED_EVENT,
        format!("engagement {id} closed {:?}", close.kind),
        affected(&engagement),
    );
    if let Some(target) = run.signal.clone() {
        let force_losses = engagement
            .forces()
            .map(|force| ForceLossDeltaV1 {
                force: force.clone(),
                losses: run.loss_deltas.get(force).copied().unwrap_or_default(),
            })
            .collect();
        run.directives
            .push(BoundaryDirective::SchedulePluginIngress {
                target_plugin: target.target_plugin,
                after: SimDuration::ZERO,
                packet_type: target.packet_type,
                priority: 0,
                payload: encode_value(&EngagementOutcomeSignalV1 {
                    summary,
                    force_losses,
                })?,
                affected: Vec::new(),
            });
    }
    Ok(())
}

// ------------------------------------------------------------------ sieges

fn siege_after_fight(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    id: &SiegeId,
    engagement: &EngagementStateV1,
    close: &CloseInfo,
) -> Result<(), CanwuError> {
    let reference = siege_reference(id);
    let Some(mut siege) = run.get(&reference)? else {
        return Ok(());
    };
    if !siege.is_open() {
        return Ok(());
    }
    if siege.engagement.as_ref() == Some(&engagement.id) {
        siege.engagement = None;
    }
    let end = match (engagement.kind, close.kind) {
        (EngagementKind::Assault, EngagementOutcomeKind::AttackerPrevailed) => Some((
            SiegeStatus::Fell,
            if siege.breach_open {
                SiegeEndCause::Breach
            } else if siege.gate_open_at(close.at) {
                SiegeEndCause::GateOpened
            } else {
                SiegeEndCause::Storm
            },
        )),
        (EngagementKind::Assault, EngagementOutcomeKind::Repulsed)
            if !close.routed[0].is_empty() =>
        {
            Some((SiegeStatus::Lifted, SiegeEndCause::Relieved))
        }
        (EngagementKind::Sortie, EngagementOutcomeKind::AttackerPrevailed)
            if close.routed[1].is_empty() =>
        {
            // The sortie broke the siege lines without routing them: the
            // guns re-emplace from now.
            siege.bombardment_from = None;
            siege.emplacement_from = Some(close.at);
            None
        }
        (
            EngagementKind::Sortie | EngagementKind::Relief,
            EngagementOutcomeKind::AttackerPrevailed,
        ) => Some((SiegeStatus::Lifted, SiegeEndCause::Relieved)),
        _ => None,
    };
    match end {
        Some((status, cause)) => end_siege(run, rulebook, &mut siege, status, cause, close.at),
        None => run.put(&reference, &siege),
    }
}

/// Ends a siege: its forces released, a fall dispersing the defenders still
/// at the site, its retirement and closing report queued.
pub(crate) fn end_siege(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    siege: &mut SiegeStateV1,
    status: SiegeStatus,
    cause: SiegeEndCause,
    at: SimTime,
) -> Result<(), CanwuError> {
    siege.status = status;
    siege.end_cause = Some(cause);
    siege.ended_at = Some(at);
    siege.assault_ordered = false;
    siege.sortie_ordered = false;
    let mut entities = Vec::new();
    for (id, defender) in siege
        .besiegers
        .iter()
        .map(|id| (id, false))
        .chain(siege.defenders.iter().map(|id| (id, true)))
    {
        let reference = force_reference(id);
        let Some(mut force) = run.get(&reference)? else {
            continue;
        };
        if force
            .battle
            .siege_role
            .as_ref()
            .is_some_and(|role| role.siege() == &siege.id)
        {
            force.battle.siege_role = None;
        }
        if status == SiegeStatus::Fell
            && defender
            && force.is_active()
            && force.location.is_at(&siege.site)
        {
            let mut dispersed = LossTallyV1::default();
            for component in force.components.values_mut() {
                let tally = LossTallyV1 {
                    dispersed: component.present,
                    ..LossTallyV1::default()
                };
                component.losses.add(&tally);
                dispersed.add(&tally);
                component.present = 0;
            }
            force.battle.dispersed = true;
            run.add_losses(id, &dispersed);
        }
        entities.push(EntityRef::Domain(reference.as_untyped().clone()));
        run.put(&reference, &force)?;
    }
    entities.push(siege.site.clone());
    entities.sort();
    entities.dedup();
    {
        let retire = retire_at(rulebook, at);
        let registry = run.registry_mut()?;
        registry.open_sieges.remove(&siege.id);
        if let Some(retire) = retire {
            queue_retire(registry, retire, siege_reference(&siege.id).into_untyped());
        }
    }
    run.queue_report(rulebook, ReportSubjectV1::Siege(siege.id.clone()), true)?;
    run.emit(
        SIEGE_ENDED_EVENT,
        format!("siege {} ended {status:?} by {cause:?}", siege.id),
        entities,
    );
    run.put(&siege_reference(&siege.id), siege)
}

/// The next free identifier for a fight a siege or crossing opens.
fn derive_id(
    run: &mut Run<'_, '_>,
    base: &str,
    kind: EngagementKind,
) -> Result<Option<EngagementId>, CanwuError> {
    for _ in 0..8 {
        let sequence = {
            let registry = run.registry_mut()?;
            let sequence = registry.next_sequence;
            registry.next_sequence = sequence.saturating_add(1);
            sequence
        };
        let Ok(id) = EngagementId::new(format!("{base}/{}-{sequence}", kind.slug())) else {
            return Ok(None);
        };
        if !run.exists_any(engagement_reference(&id).as_untyped())? {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn has_capacity(run: &Run<'_, '_>, rulebook: &CombatRulebookV1) -> bool {
    run.registry().is_some_and(|registry| {
        registry.open_engagements.len() < rulebook.limits.open_engagements as usize
    })
}

fn present_forces(
    run: &Run<'_, '_>,
    ids: &[ForceId],
    site: Option<&EntityRef>,
) -> Result<Vec<ForceStateV1>, CanwuError> {
    let mut forces = Vec::new();
    for id in ids {
        if let Some(force) = run.get(&force_reference(id))?
            && force.is_active()
            && force.men() > 0
            && site.is_none_or(|site| force.location.is_at(site))
        {
            forces.push(force);
        }
    }
    Ok(forces)
}

fn advance_siege(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    id: &SiegeId,
    up: SimTime,
) -> Result<(), CanwuError> {
    let reference = siege_reference(id);
    let Some(mut siege) = run.get(&reference)? else {
        run.registry_mut()?.open_sieges.remove(id);
        return Ok(());
    };
    let mut days = 0;
    while siege.is_open() && siege.next_day_at <= up && days < MAX_DAYS_PER_BOUNDARY {
        let day = siege.next_day_at;
        siege_day(run, rulebook, &mut siege, day)?;
        if siege.is_open() {
            siege.next_day_at = add_minutes(day, rulebook.siege.day_minutes)?;
        }
        days += 1;
    }
    if days > 0 && siege.is_open() {
        run.put(&reference, &siege)?;
        run.queue_report(rulebook, ReportSubjectV1::Siege(id.clone()), false)?;
    }
    Ok(())
}

fn siege_day(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    siege: &mut SiegeStateV1,
    day: SimTime,
) -> Result<(), CanwuError> {
    let besiegers = present_forces(run, &siege.besiegers, None)?;
    let defenders = present_forces(run, &siege.defenders, Some(&siege.site))?;
    if siege.engagement.is_none() {
        if defenders.is_empty() {
            return end_siege(
                run,
                rulebook,
                siege,
                SiegeStatus::Fell,
                SiegeEndCause::Abandoned,
                day,
            );
        }
        if besiegers.is_empty() {
            return end_siege(
                run,
                rulebook,
                siege,
                SiegeStatus::Lifted,
                SiegeEndCause::Withdrawn,
                day,
            );
        }
    }
    let rules = &rulebook.siege;
    let besieger_men = besiegers
        .iter()
        .fold(0_u64, |sum, force| sum.saturating_add(force.men()));
    let land = u64::from(siege.profile.accesses);
    let water = u64::from(siege.profile.water_accesses);
    let mut sealed = land.min(besieger_men / u64::from(rules.men_per_access_sealed.max(1)));
    if siege
        .naval
        .as_ref()
        .is_some_and(|naval| naval.control_bp >= rules.water_access_control_bp)
    {
        sealed += water;
    }
    siege.investment_bp = u32::try_from(sealed * 10_000 / (land + water).max(1)).unwrap_or(10_000);
    let components: Vec<(u64, &ComponentValuesV1)> = besiegers
        .iter()
        .flat_map(|force| {
            force
                .components
                .values()
                .map(|component| (component.present, &component.values))
        })
        .collect();
    siege.pieces = components.iter().fold(0_u64, |sum, (present, values)| {
        sum.saturating_add(pieces(*present, values))
    });
    if siege.pieces > 0 && siege.emplacement_from.is_none() {
        siege.emplacement_from = Some(day);
    }
    if let Some(from) = siege.emplacement_from
        && siege.bombardment_from.is_none()
    {
        let needed = siege
            .environment
            .as_ref()
            .and_then(|(environment, _)| rules.emplacement_days.get(&environment.ground))
            .copied()
            .unwrap_or(0);
        let ready = from.checked_add(SimDuration::minutes(
            i64::from(needed) * i64::from(rules.day_minutes),
        ));
        if ready.is_some_and(|ready| day >= ready) {
            siege.bombardment_from = Some(day);
        }
    }
    if siege.bombardment_from.is_some() && !siege.breach_open {
        let progress = breach_rate(
            components.iter().copied(),
            siege.work_bp,
            siege.profile.repair_per_day,
        );
        siege.breach_progress = siege
            .breach_progress
            .saturating_add(u32::try_from(progress).unwrap_or(u32::MAX));
        if siege.breach_progress >= siege.breach_threshold {
            siege.breach_open = true;
        }
    }
    if siege.engagement.is_some() {
        return Ok(());
    }
    let free = besiegers
        .iter()
        .chain(&defenders)
        .all(|force| force.battle.engaged_in.is_none());
    if !free {
        return Ok(());
    }
    let storm = siege.assault_ordered
        || (siege.storm_on == StormRule::BreachOrGate
            && (siege.breach_open || siege.gate_open_at(day)));
    if storm {
        if open_siege_fight(
            run,
            rulebook,
            siege,
            EngagementKind::Assault,
            &besiegers,
            &defenders,
            day,
        )? {
            siege.assault_ordered = false;
        }
    } else if siege.sortie_ordered
        && open_siege_fight(
            run,
            rulebook,
            siege,
            EngagementKind::Sortie,
            &besiegers,
            &defenders,
            day,
        )?
    {
        siege.sortie_ordered = false;
        let ready = add_minutes(day, rules.sortie_cooldown_minutes)?;
        for defender in &defenders {
            let reference = force_reference(&defender.id);
            if let Some(mut force) = run.get(&reference)? {
                force.battle.sortie_ready_at = Some(ready);
                run.put(&reference, &force)?;
            }
        }
    }
    Ok(())
}

fn open_siege_fight(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    siege: &mut SiegeStateV1,
    kind: EngagementKind,
    besiegers: &[ForceStateV1],
    defenders: &[ForceStateV1],
    day: SimTime,
) -> Result<bool, CanwuError> {
    let (Some(besieging), Some(defending), Some(environment)) = (
        siege.inputs[0].clone(),
        siege.inputs[1].clone(),
        siege.environment.clone(),
    ) else {
        return Ok(false);
    };
    if !has_capacity(run, rulebook) {
        return Ok(false);
    }
    let Some(id) = derive_id(run, siege.id.as_str(), kind)? else {
        return Ok(false);
    };
    let ids = |forces: &[ForceStateV1]| forces.iter().map(|force| force.id.clone()).collect();
    let gate = siege.gate_open_at(day);
    let rules = &rulebook.siege;
    let spec = if kind == EngagementKind::Assault {
        OpenSpec {
            id: id.clone(),
            kind,
            profile: rulebook.default_profile.clone(),
            site: siege.site.clone(),
            fort: Some((siege.profile.grade, siege.breach_open || gate)),
            terrain: Some(if siege.breach_open {
                rules.breach_terrain.clone()
            } else {
                rules.assault_terrain.clone()
            }),
            siege: Some(siege.id.clone()),
            crossing: None,
            attackers: ids(besiegers),
            defenders: ids(defenders),
            holders: siege.holders.clone(),
            inputs: [Some(besieging), Some(defending)],
            environment: Some(environment),
            men_caps: [None, None],
            opened_at: day,
        }
    } else {
        OpenSpec {
            id: id.clone(),
            kind,
            profile: rulebook.default_profile.clone(),
            site: siege.site.clone(),
            fort: None,
            terrain: Some(rules.sortie_terrain.clone()),
            siege: Some(siege.id.clone()),
            crossing: None,
            attackers: ids(defenders),
            defenders: ids(besiegers),
            holders: [siege.holders[1].clone(), siege.holders[0].clone()],
            inputs: [Some(defending), Some(besieging)],
            environment: Some(environment),
            men_caps: [None, None],
            opened_at: day,
        }
    };
    open_engagement(run, rulebook, spec)?;
    siege.engagement = Some(id);
    Ok(true)
}

// --------------------------------------------------------------- crossings

/// Ends a crossing: its force released, its retirement and closing report
/// queued.
pub(crate) fn finish_crossing(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    crossing: &mut CrossingStateV1,
    status: CrossingStatus,
    at: SimTime,
) -> Result<(), CanwuError> {
    crossing.status = status;
    crossing.ended_at = Some(at);
    let reference = force_reference(&crossing.force);
    if let Some(mut force) = run.get(&reference)?
        && force.battle.crossing.as_ref() == Some(&crossing.id)
    {
        force.battle.crossing = None;
        run.put(&reference, &force)?;
    }
    {
        let retire = retire_at(rulebook, at);
        let registry = run.registry_mut()?;
        registry.open_crossings.remove(&crossing.id);
        if let Some(retire) = retire {
            queue_retire(
                registry,
                retire,
                crossing_reference(&crossing.id).into_untyped(),
            );
        }
    }
    run.queue_report(
        rulebook,
        ReportSubjectV1::Crossing(crossing.id.clone()),
        true,
    )?;
    run.put(&crossing_reference(&crossing.id), crossing)
}

fn advance_crossing(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    id: &CrossingId,
    packet: IngressId,
    up: SimTime,
) -> Result<(), CanwuError> {
    let reference = crossing_reference(id);
    let Some(mut crossing) = run.get(&reference)? else {
        run.registry_mut()?.open_crossings.remove(id);
        return Ok(());
    };
    let Some(version) = run.committed_version(reference.as_untyped())? else {
        return Ok(());
    };
    let mut waves = 0;
    while crossing.is_open() && crossing.next_wave_at <= up && waves < MAX_DAYS_PER_BOUNDARY {
        let at = crossing.next_wave_at;
        if !crossing_wave(run, rulebook, &mut crossing, packet, version, at)? {
            break;
        }
        if crossing.is_open() {
            crossing.next_wave_at = add_minutes(at, rulebook.crossing.wave_minutes)?;
        }
        waves += 1;
    }
    if waves > 0 && crossing.is_open() {
        run.put(&reference, &crossing)?;
        run.queue_report(rulebook, ReportSubjectV1::Crossing(id.clone()), false)?;
    }
    Ok(())
}

fn crossing_wave(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    crossing: &mut CrossingStateV1,
    packet: IngressId,
    version: u64,
    at: SimTime,
) -> Result<bool, CanwuError> {
    // A wave waits, `next_wave_at` held, while the crossing's fight is open
    // or no environment input is set.
    if crossing.engagement.is_some() || crossing.environment.is_none() {
        return Ok(false);
    }
    let force = run.get(&force_reference(&crossing.force))?;
    let Some(force) = force.filter(|force| force.is_active() && force.men() > 0) else {
        finish_crossing(run, rulebook, crossing, CrossingStatus::Complete, at)?;
        return Ok(true);
    };
    let men = force.men();
    let remaining = men.saturating_sub(crossing.landed);
    if remaining == 0 {
        finish_crossing(run, rulebook, crossing, CrossingStatus::Complete, at)?;
        return Ok(true);
    }
    let Some((environment, _)) = crossing.environment.clone() else {
        return Ok(false);
    };
    let rules = &rulebook.crossing;
    let wave = remaining.min(rules.wave_men);
    let wave = match environment.river {
        RiverBand::Closed => 0,
        RiverBand::Hard => {
            u64::try_from(u128::from(wave) * u128::from(rules.hard_river_wave_bp) / 10_000)
                .unwrap_or(0)
        }
        RiverBand::Open => wave,
    };
    if wave == 0 {
        // A closed river: this wave's slot passes.
        return Ok(true);
    }
    let inputs = crossing.inputs[0].clone().zip(crossing.inputs[1].clone());
    let fight_possible =
        inputs.is_some() && has_capacity(run, rulebook) && force.battle.engaged_in.is_none();
    let mut interceptor = None;
    if let Some(input) = &crossing.interceptor
        && input.control_bp >= rules.intercept.min_control_bp
        && input.force != crossing.force
        && let Some(state) = run.get(&force_reference(&input.force))?
        && state.is_active()
        && state.men() > 0
        && state.battle.engaged_in.is_none()
    {
        interceptor = Some(input.clone());
    }
    let index = crossing.waves_sent;
    let mut intercept = None;
    let mut draw = None;
    let mut intercepted = false;
    if let Some(input) = &interceptor
        && fight_possible
    {
        let score = intercept_bp(
            &rules.intercept,
            input.control_bp,
            input.scouts,
            &environment.concealment,
            crossing.night,
        );
        intercept = Some(score);
        intercepted = match rules.intercept.mode {
            InterceptMode::Threshold => score > i64::from(rules.intercept.threshold_bp),
            InterceptMode::KeyedDraw => {
                // The wave's last refusable step: nothing after it refuses.
                let drawn = run.view.random_draw_for_operation(
                    &crossing_interception_stream(),
                    EvidenceRef::Ingress(packet),
                    "interception",
                    &format!("interception:{}:{index}", crossing.id),
                    RandomOperationTarget::DomainRecord {
                        record: crossing_reference(&crossing.id).into_untyped(),
                        version,
                    },
                    index,
                    10_000,
                    "crossing-interception",
                )?;
                draw = Some(drawn.draw_id);
                i64::try_from(drawn.sample.value).unwrap_or(i64::MAX) < score
            }
        };
    }
    crossing.waves_sent += 1;
    let mut engagement = None;
    let terrain = Some(rules.terrain.clone());
    if intercepted && let (Some(input), Some((own, theirs))) = (&interceptor, inputs.clone()) {
        if let Some(id) = derive_id(run, crossing.id.as_str(), EngagementKind::FerryFight)? {
            open_engagement(
                run,
                rulebook,
                OpenSpec {
                    id: id.clone(),
                    kind: EngagementKind::FerryFight,
                    profile: rulebook.default_profile.clone(),
                    site: crossing.from.clone(),
                    fort: None,
                    terrain: terrain.clone(),
                    siege: None,
                    crossing: Some(crossing.id.clone()),
                    attackers: vec![input.force.clone()],
                    defenders: vec![crossing.force.clone()],
                    holders: [crossing.holders[1].clone(), crossing.holders[0].clone()],
                    inputs: [Some(theirs), Some(own)],
                    environment: crossing.environment.clone(),
                    men_caps: [None, Some(wave)],
                    opened_at: at,
                },
            )?;
            engagement = Some(id);
        }
    } else {
        crossing.landed = crossing.landed.saturating_add(wave).min(men);
        let defenders = present_forces(run, &crossing.defenders, Some(&crossing.to))?;
        let defenders: Vec<ForceId> = defenders
            .into_iter()
            .filter(ForceStateV1::is_free)
            .map(|force| force.id)
            .collect();
        if !defenders.is_empty()
            && fight_possible
            && let Some((mut own, theirs)) = inputs
            && let Some(id) = derive_id(run, crossing.id.as_str(), EngagementKind::BridgeheadFight)?
        {
            own.0.defense_halved = crossing.landed < rules.bridgehead_secure_men;
            open_engagement(
                run,
                rulebook,
                OpenSpec {
                    id: id.clone(),
                    kind: EngagementKind::BridgeheadFight,
                    profile: rulebook.default_profile.clone(),
                    site: crossing.to.clone(),
                    fort: None,
                    terrain,
                    siege: None,
                    crossing: Some(crossing.id.clone()),
                    attackers: vec![crossing.force.clone()],
                    defenders,
                    holders: crossing.holders.clone(),
                    inputs: [Some(own), Some(theirs)],
                    environment: crossing.environment.clone(),
                    men_caps: [Some(crossing.landed), None],
                    opened_at: at,
                },
            )?;
            engagement = Some(id);
        }
    }
    crossing.engagement.clone_from(&engagement);
    crossing.bridgehead_secure = crossing.landed >= rules.bridgehead_secure_men;
    crossing.last_wave = Some(WaveV1 {
        index,
        at,
        men: wave,
        intercepted,
        intercept_bp: intercept,
        draw,
        engagement,
    });
    run.emit(
        CROSSING_WAVE_EVENT,
        format!(
            "crossing {} wave {index}: {wave} men, {}",
            crossing.id,
            if intercepted { "intercepted" } else { "landed" }
        ),
        vec![EntityRef::Domain(
            force_reference(&crossing.force).into_untyped(),
        )],
    );
    if crossing.landed >= men {
        finish_crossing(run, rulebook, crossing, CrossingStatus::Complete, at)?;
        return Ok(true);
    }
    Ok(true)
}
