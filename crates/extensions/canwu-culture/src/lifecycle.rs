use crate::model::{
    CompiledCulturePlan, CulturalSignal, CulturalSignalBatch, CultureLifecycle, CultureState,
    DirtyPair, DirtySet, EffectEmissionCursor, EffectPersistence, LifecycleTransition,
    LifecycleTransitionKind, RetiredTargetTombstone, cause_ref_estimated_bytes,
    tombstone_estimated_bytes,
};
use canwu_api::{CanwuError, CauseRef, DomainRecord, ErrorCode, SimTime};
use std::collections::{BTreeMap, BTreeSet};

/// Mutable lifecycle state for one compiled culture plan.
///
/// The persisted state contains the lifecycle, schedule, dirty-work, and effect
/// cadence indexes. Population distributions remain owned by `canwu-society`;
/// hosts apply lifecycle eligibility there with
/// [`crate::synchronize_society_lifecycle`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CultureRuntime {
    budgets: crate::CultureBudgets,
    retirement: crate::RetirementPolicy,
    target_keys: BTreeMap<String, crate::TargetKey>,
    dormant_due_by_target: BTreeMap<String, u64>,
    state_bytes: usize,
    state: CultureState,
}

#[derive(Debug)]
pub(crate) struct CultureBoundaryDelta {
    boundary_index: u64,
    at: SimTime,
    projected_state_bytes: usize,
    target_updates: BTreeMap<String, crate::TargetLifecycle>,
    dormant_due_by_target: BTreeMap<String, u64>,
    tombstones: Vec<RetiredTargetTombstone>,
    transitions: Vec<LifecycleTransition>,
}

impl CultureBoundaryDelta {
    pub(crate) fn transitions(&self) -> &[LifecycleTransition] {
        &self.transitions
    }
}

impl CultureRuntime {
    #[must_use]
    pub fn new(plan: &CompiledCulturePlan) -> Self {
        Self::new_at(plan, SimTime::EPOCH)
    }

    #[must_use]
    pub fn new_at(plan: &CompiledCulturePlan, initial_time: SimTime) -> Self {
        let state = CultureState::from_plan_at(plan, initial_time);
        let state_bytes = state.estimated_bytes().unwrap_or(usize::MAX);
        Self {
            budgets: plan.budgets,
            retirement: plan.retirement,
            target_keys: plan.target_by_id.clone(),
            dormant_due_by_target: BTreeMap::new(),
            state_bytes,
            state,
        }
    }

    /// Restores every runtime index from a validated persisted state.
    pub fn from_state(plan: &CompiledCulturePlan, state: CultureState) -> Result<Self, CanwuError> {
        state.validate_against_plan(plan)?;
        let mut dormant_due_by_target = BTreeMap::new();
        for (due_at, targets) in &state.dormant_due {
            for target_id in targets {
                dormant_due_by_target.insert(target_id.clone(), *due_at);
            }
        }
        let state_bytes = state.estimated_bytes()?;
        Ok(Self {
            budgets: plan.budgets,
            retirement: plan.retirement,
            target_keys: plan.target_by_id.clone(),
            dormant_due_by_target,
            state_bytes,
            state,
        })
    }

    #[must_use]
    pub fn plan_hash(&self) -> &str {
        &self.state.plan_hash
    }

    #[must_use]
    pub fn state(&self) -> &CultureState {
        &self.state
    }

    #[must_use]
    pub fn snapshot_state(&self) -> CultureState {
        self.state.clone()
    }

    pub fn into_record(self, plan: &CompiledCulturePlan) -> Result<DomainRecord, CanwuError> {
        self.state.validate_against_plan(plan)?;
        self.state.into_record()
    }

    /// Returns the lifecycle entry for a target.
    #[must_use]
    pub fn target(&self, target_id: &str) -> Option<&crate::TargetLifecycle> {
        self.state.targets.get(target_id)
    }

    /// Returns the targets currently eligible for ordinary daily settlement.
    ///
    /// Dormant and retired targets are absent from this index. The current
    /// society adapter uses it for lifecycle synchronization; fully incremental
    /// aggregate and projection settlement remains a separate follow-up.
    #[must_use]
    pub fn active_target_ids(&self) -> &BTreeSet<String> {
        &self.state.hot_targets
    }

    /// Marks the affected cohort relationships for incremental settlement.
    pub fn mark_target_dirty(
        &mut self,
        plan: &CompiledCulturePlan,
        target_id: &str,
    ) -> Result<usize, CanwuError> {
        if plan.content_hash != self.state.plan_hash {
            return Err(lifecycle_error(
                "dirty-set plan hash does not match runtime plan",
            ));
        }
        if !self.state.hot_targets.contains(target_id) {
            return Err(lifecycle_error(format!(
                "inactive culture target {target_id} cannot be marked dirty"
            )));
        }
        let pairs = DirtySet::pairs_for_target(plan, target_id)?;
        let added_pairs = pairs
            .iter()
            .filter(|pair| !self.state.dirty_pairs.contains(**pair))
            .count();
        let additional_bytes = added_pairs
            .checked_mul(48)
            .ok_or_else(|| lifecycle_error("culture dirty-set byte estimate overflowed"))?;
        self.ensure_state_growth(additional_bytes)?;
        self.state.dirty_pairs.mark_pairs(pairs);
        self.state_bytes = self.state_bytes.saturating_add(additional_bytes);
        Ok(added_pairs)
    }

    /// Returns and clears the canonical worklist for one boundary.
    pub fn drain_dirty_pairs(&mut self) -> Vec<DirtyPair> {
        let pairs = self.state.dirty_pairs.drain_sorted();
        self.state_bytes = self
            .state_bytes
            .saturating_sub(pairs.len().saturating_mul(48));
        pairs
    }

    #[must_use]
    pub fn dirty_pair_count(&self) -> usize {
        self.state.dirty_pairs.len()
    }

    /// Records current engaged headcount for a target.
    ///
    /// A dormant target with newly engaged population becomes active in the
    /// same generation. A retired target requires explicit reactivation.
    pub fn set_engaged_headcount(
        &mut self,
        target_id: &str,
        engaged_headcount: u64,
        at: SimTime,
    ) -> Result<Option<LifecycleTransition>, CanwuError> {
        self.validate_mutation_time(at)?;
        let (was_dormant, state, generation) = {
            let target = self.target_mut(target_id)?;
            if target.state == CultureLifecycle::Retired {
                return Err(lifecycle_error(format!(
                    "retired culture target {target_id} cannot receive engagement"
                )));
            }
            let was_dormant = target.state == CultureLifecycle::Dormant;
            target.engaged_headcount = engaged_headcount;
            if engaged_headcount > 0 {
                target.quiet_boundaries = 0;
                target.last_active_at = at;
                target.last_work_at = Some(at);
                target.state = CultureLifecycle::Active;
                target.dormant_since_boundary = None;
            }
            (was_dormant, target.state, target.generation)
        };
        if engaged_headcount > 0 {
            self.state.hot_targets.insert(target_id.to_owned());
            self.cancel_dormant_schedule(target_id);
            self.state.latest_activity_at = at;
        }
        if was_dormant && state == CultureLifecycle::Active {
            self.refresh_state_bytes();
            return Ok(Some(LifecycleTransition {
                target_id: target_id.to_owned(),
                generation,
                kind: LifecycleTransitionKind::Reactivated,
                at,
            }));
        }
        Ok(None)
    }

    /// Records admitted work or an input that keeps the target hot.
    pub fn admit_work(
        &mut self,
        target_id: &str,
        at: SimTime,
    ) -> Result<Option<LifecycleTransition>, CanwuError> {
        self.validate_mutation_time(at)?;
        let (was_dormant, generation) = {
            let target = self.target_mut(target_id)?;
            if target.state == CultureLifecycle::Retired {
                return Err(lifecycle_error(format!(
                    "retired culture target {target_id} requires explicit reactivation"
                )));
            }
            let was_dormant = target.state == CultureLifecycle::Dormant;
            target.state = CultureLifecycle::Active;
            target.quiet_boundaries = 0;
            target.dormant_since_boundary = None;
            target.last_active_at = at;
            target.last_work_at = Some(at);
            (was_dormant, target.generation)
        };
        self.state.hot_targets.insert(target_id.to_owned());
        self.cancel_dormant_schedule(target_id);
        self.state.latest_activity_at = at;
        if was_dormant {
            self.refresh_state_bytes();
            return Ok(Some(LifecycleTransition {
                target_id: target_id.to_owned(),
                generation,
                kind: LifecycleTransitionKind::Reactivated,
                at,
            }));
        }
        Ok(None)
    }

    /// Settles one lifecycle boundary and returns transitions that must be
    /// recorded as authoritative evidence by the host plugin.
    pub fn settle_boundary(
        &mut self,
        at: SimTime,
        observations: &BTreeMap<String, LifecycleObservation>,
    ) -> Result<Vec<LifecycleTransition>, CanwuError> {
        let delta = self.prepare_boundary(at, observations)?;
        let transitions = delta.transitions.clone();
        self.apply_boundary_delta(delta);
        Ok(transitions)
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn prepare_boundary(
        &self,
        at: SimTime,
        observations: &BTreeMap<String, LifecycleObservation>,
    ) -> Result<CultureBoundaryDelta, CanwuError> {
        if observations
            .keys()
            .any(|target_id| !self.state.targets.contains_key(target_id))
        {
            return Err(lifecycle_error(
                "culture lifecycle observations contain an unknown target",
            ));
        }
        for (target_id, observation) in observations {
            if self
                .state
                .targets
                .get(target_id)
                .is_some_and(|target| target.state == CultureLifecycle::Retired)
                && (observation.engaged_headcount > 0
                    || observation.admitted_work
                    || observation.live_dependency)
            {
                return Err(lifecycle_error(format!(
                    "retired culture target {target_id} requires explicit reactivation"
                )));
            }
        }
        if at <= self.state.last_boundary_at {
            return Err(lifecycle_error(
                "culture lifecycle boundaries must advance simulation time",
            ));
        }
        if at < self.state.latest_activity_at {
            return Err(lifecycle_error(
                "culture boundary cannot precede already-recorded target activity",
            ));
        }
        let boundary_index = self
            .state
            .boundary_index
            .checked_add(1)
            .ok_or_else(|| lifecycle_error("culture lifecycle boundary index exhausted"))?;
        let mut transitions = Vec::new();
        let mut target_updates = BTreeMap::new();
        let mut dormant_due_by_target = BTreeMap::new();
        let mut tombstones = Vec::new();
        let retirement = self.retirement;
        let mut target_ids = self.state.hot_targets.clone();
        let due_targets = self
            .state
            .dormant_due
            .get(&boundary_index)
            .cloned()
            .unwrap_or_default();
        target_ids.extend(due_targets.iter().cloned());
        target_ids.extend(observations.keys().cloned());
        for target_id in target_ids {
            let observation = observations.get(&target_id).copied().unwrap_or_default();
            let mut target =
                self.state.targets.get(&target_id).cloned().ok_or_else(|| {
                    lifecycle_error(format!("unknown culture target {target_id}"))
                })?;
            if target.state == CultureLifecycle::Retired {
                continue;
            }
            let previous_state = target.state;
            target.engaged_headcount = observation.engaged_headcount;
            if observation.engaged_headcount > 0
                || observation.admitted_work
                || observation.live_dependency
            {
                target.state = CultureLifecycle::Active;
                target.quiet_boundaries = 0;
                target.dormant_since_boundary = None;
                target.last_active_at = at;
                if observation.admitted_work {
                    target.last_work_at = Some(at);
                }
                if previous_state == CultureLifecycle::Dormant {
                    transitions.push(LifecycleTransition {
                        target_id: target_id.clone(),
                        generation: target.generation,
                        kind: LifecycleTransitionKind::Reactivated,
                        at,
                    });
                }
            } else {
                if due_targets.contains(&target_id) && target.state == CultureLifecycle::Dormant {
                    target.quiet_boundaries = retirement.retired_after_boundaries;
                } else {
                    target.quiet_boundaries = target.quiet_boundaries.saturating_add(1);
                }
                if target.state == CultureLifecycle::Active
                    && target.quiet_boundaries >= retirement.dormant_after_boundaries
                {
                    target.state = CultureLifecycle::Dormant;
                    target.dormant_since_boundary = Some(boundary_index);
                    transitions.push(LifecycleTransition {
                        target_id: target_id.clone(),
                        generation: target.generation,
                        kind: LifecycleTransitionKind::BecameDormant,
                        at,
                    });
                }
                if target.state == CultureLifecycle::Dormant
                    && target.quiet_boundaries >= retirement.retired_after_boundaries
                {
                    target.state = CultureLifecycle::Retired;
                    target.dormant_since_boundary = None;
                    tombstones.push(RetiredTargetTombstone {
                        target_id: target_id.clone(),
                        generation: target.generation,
                        last_active_at: target.last_active_at,
                        retired_at: at,
                        reason: "quiet-window-expired".to_owned(),
                        policy_hash: self.state.plan_hash.clone(),
                        successor: None,
                        evidence: Vec::new(),
                    });
                    transitions.push(LifecycleTransition {
                        target_id: target_id.clone(),
                        generation: target.generation,
                        kind: LifecycleTransitionKind::Retired,
                        at,
                    });
                }
            }
            if target.state == CultureLifecycle::Dormant {
                let dormant_since = target.dormant_since_boundary.ok_or_else(|| {
                    lifecycle_error("dormant culture target has no origin boundary")
                })?;
                let dormant_span = retirement
                    .retired_after_boundaries
                    .saturating_sub(retirement.dormant_after_boundaries);
                let due_at = dormant_since
                    .checked_add(u64::from(dormant_span))
                    .ok_or_else(|| lifecycle_error("culture retirement schedule exhausted"))?;
                dormant_due_by_target.insert(target_id.clone(), due_at);
            }
            target_updates.insert(target_id, target);
        }

        let projected_tombstones = self
            .state
            .tombstones
            .len()
            .checked_add(tombstones.len())
            .ok_or_else(|| lifecycle_error("culture tombstone count overflowed"))?;
        if projected_tombstones > self.budgets.max_tombstones {
            return Err(lifecycle_error("culture tombstone budget exhausted"));
        }
        let mut added_bytes = 0_usize;
        let mut removed_bytes = 0_usize;
        for tombstone in &tombstones {
            if self
                .state
                .contains_tombstone(&tombstone.target_id, tombstone.generation)
            {
                return Err(lifecycle_error(format!(
                    "duplicate culture tombstone {}@{}",
                    tombstone.target_id, tombstone.generation
                )));
            }
            added_bytes = added_bytes
                .checked_add(tombstone_estimated_bytes(tombstone)?)
                .ok_or_else(|| lifecycle_error("culture tombstone size overflowed"))?;
        }

        let mut schedule_count_deltas = BTreeMap::<u64, i64>::new();
        let mut inactive_target_keys = BTreeSet::new();
        for (target_id, target) in &target_updates {
            let hot_entry_bytes = 64_usize.saturating_add(target_id.len());
            let was_hot = self.state.hot_targets.contains(target_id);
            let will_be_hot = target.state == CultureLifecycle::Active;
            if was_hot && !will_be_hot {
                removed_bytes = removed_bytes
                    .checked_add(hot_entry_bytes)
                    .ok_or_else(|| lifecycle_error("culture hot-index size overflowed"))?;
            } else if !was_hot && will_be_hot {
                added_bytes = added_bytes
                    .checked_add(hot_entry_bytes)
                    .ok_or_else(|| lifecycle_error("culture hot-index size overflowed"))?;
            }

            let schedule_entry_bytes = 64_usize.saturating_add(target_id.len());
            if let Some(current_due) = self.dormant_due_by_target.get(target_id) {
                removed_bytes = removed_bytes
                    .checked_add(schedule_entry_bytes)
                    .ok_or_else(|| lifecycle_error("culture schedule size overflowed"))?;
                *schedule_count_deltas.entry(*current_due).or_default() -= 1;
            }
            if let Some(next_due) = dormant_due_by_target.get(target_id) {
                added_bytes = added_bytes
                    .checked_add(schedule_entry_bytes)
                    .ok_or_else(|| lifecycle_error("culture schedule size overflowed"))?;
                *schedule_count_deltas.entry(*next_due).or_default() += 1;
            }

            if !will_be_hot {
                let target_key = self.target_key_for_id(target_id).ok_or_else(|| {
                    lifecycle_error(format!("unknown compiled culture target {target_id}"))
                })?;
                inactive_target_keys.insert(target_key);
            }
        }
        for (due_at, count_delta) in schedule_count_deltas {
            let current_count = self
                .state
                .dormant_due
                .get(&due_at)
                .map_or(0_usize, BTreeSet::len);
            let projected_count = i128::try_from(current_count)
                .ok()
                .and_then(|count| count.checked_add(i128::from(count_delta)))
                .ok_or_else(|| lifecycle_error("culture schedule count overflowed"))?;
            if projected_count < 0 {
                return Err(lifecycle_error("culture schedule count became negative"));
            }
            if current_count == 0 && projected_count > 0 {
                added_bytes = added_bytes
                    .checked_add(64)
                    .ok_or_else(|| lifecycle_error("culture schedule size overflowed"))?;
            } else if current_count > 0 && projected_count == 0 {
                removed_bytes = removed_bytes
                    .checked_add(64)
                    .ok_or_else(|| lifecycle_error("culture schedule size overflowed"))?;
            }
        }
        let removed_dirty_pairs = self
            .state
            .dirty_pairs
            .count_for_targets(&inactive_target_keys);
        removed_bytes = removed_bytes
            .checked_add(
                removed_dirty_pairs
                    .checked_mul(48)
                    .ok_or_else(|| lifecycle_error("culture dirty-set size overflowed"))?,
            )
            .ok_or_else(|| lifecycle_error("culture dirty-set size overflowed"))?;
        let projected_state_bytes = self
            .state_bytes
            .checked_sub(removed_bytes)
            .and_then(|bytes| bytes.checked_add(added_bytes))
            .ok_or_else(|| lifecycle_error("culture state byte estimate overflowed"))?;
        if projected_state_bytes > self.budgets.max_state_bytes {
            return Err(lifecycle_error(format!(
                "culture state growth exceeds byte budget: {projected_state_bytes} > {}",
                self.budgets.max_state_bytes
            )));
        }

        Ok(CultureBoundaryDelta {
            boundary_index,
            at,
            projected_state_bytes,
            target_updates,
            dormant_due_by_target,
            tombstones,
            transitions,
        })
    }

    pub(crate) fn apply_boundary_delta(&mut self, delta: CultureBoundaryDelta) {
        let projected_state_bytes = delta.projected_state_bytes;
        for tombstone in delta.tombstones {
            self.state.insert_tombstone(tombstone);
        }
        for (target_id, target) in delta.target_updates {
            self.cancel_dormant_schedule(&target_id);
            match target.state {
                CultureLifecycle::Active => {
                    self.state.hot_targets.insert(target_id.clone());
                }
                CultureLifecycle::Dormant | CultureLifecycle::Retired => {
                    self.state.hot_targets.remove(&target_id);
                    if let Some(target_key) = self.target_key_for_id(&target_id) {
                        self.state.dirty_pairs.remove_target(target_key);
                    }
                }
            }
            self.state.targets.insert(target_id, target);
        }
        for (target_id, due_at) in delta.dormant_due_by_target {
            self.state
                .dormant_due
                .entry(due_at)
                .or_default()
                .insert(target_id.clone());
            self.dormant_due_by_target.insert(target_id, due_at);
        }
        self.state.boundary_index = delta.boundary_index;
        self.state.last_boundary_at = delta.at;
        self.state.latest_activity_at = delta.at;
        self.state_bytes = projected_state_bytes;
    }

    /// Explicitly creates a new generation from a retired target tombstone.
    pub fn reactivate(
        &mut self,
        target_id: &str,
        at: SimTime,
    ) -> Result<LifecycleTransition, CanwuError> {
        self.validate_mutation_time(at)?;
        let additional_bytes = 64_usize.saturating_add(target_id.len());
        self.ensure_state_growth(additional_bytes)?;
        let generation = {
            let target = self.target_mut(target_id)?;
            if target.state != CultureLifecycle::Retired {
                return Err(lifecycle_error(format!(
                    "culture target {target_id} is not retired"
                )));
            }
            target.generation = target
                .generation
                .checked_add(1)
                .ok_or_else(|| lifecycle_error("culture target generation exhausted"))?;
            target.state = CultureLifecycle::Active;
            target.engaged_headcount = 0;
            target.quiet_boundaries = 0;
            target.dormant_since_boundary = None;
            target.last_active_at = at;
            target.last_work_at = Some(at);
            target.generation
        };
        self.state.hot_targets.insert(target_id.to_owned());
        self.cancel_dormant_schedule(target_id);
        self.state.latest_activity_at = at;
        self.state_bytes = self.state_bytes.saturating_add(additional_bytes);
        Ok(LifecycleTransition {
            target_id: target_id.to_owned(),
            generation,
            kind: LifecycleTransitionKind::Reactivated,
            at,
        })
    }

    /// Validates and emits one batch assembled from compiled effect bindings.
    fn emit_signal_batch(
        &self,
        id: impl Into<String>,
        emitted_at: SimTime,
        earliest_eligible_at: SimTime,
        signals: Vec<CulturalSignal>,
    ) -> Result<CulturalSignalBatch, CanwuError> {
        let id = id.into();
        if id.is_empty() || id.len() > self.budgets.max_text_bytes {
            return Err(lifecycle_error(
                "culture signal batch ID is empty or exceeds its byte budget",
            ));
        }
        if earliest_eligible_at < emitted_at {
            return Err(lifecycle_error(
                "culture signal earliest eligibility cannot precede emission",
            ));
        }
        if emitted_at < self.state.last_boundary_at {
            return Err(lifecycle_error(
                "culture signal emission cannot precede the latest lifecycle boundary",
            ));
        }
        if signals.is_empty() {
            return Err(lifecycle_error("culture signal batch cannot be empty"));
        }
        if signals.len() > self.budgets.max_signals_per_batch {
            return Err(lifecycle_error(format!(
                "culture signal batch exceeds budget: {} > {}",
                signals.len(),
                self.budgets.max_signals_per_batch
            )));
        }
        let mut batch = CulturalSignalBatch {
            id,
            plan_hash: self.state.plan_hash.clone(),
            emitted_at,
            earliest_eligible_at,
            signals,
        };
        batch.signals.sort_by(|left, right| {
            left.target_id
                .cmp(&right.target_id)
                .then(left.generation.cmp(&right.generation))
                .then(left.effect_id.cmp(&right.effect_id))
        });
        for signal in &batch.signals {
            let target = self.target(&signal.target_id).ok_or_else(|| {
                lifecycle_error(format!("unknown culture target {}", signal.target_id))
            })?;
            if target.generation != signal.generation || target.state != CultureLifecycle::Active {
                return Err(lifecycle_error(format!(
                    "signal {} does not target the active generation of {}",
                    signal.effect_id, signal.target_id
                )));
            }
            if signal.strength_per_mille > 1_000 {
                return Err(lifecycle_error(format!(
                    "signal {} strength exceeds 1000 per mille",
                    signal.effect_id
                )));
            }
            if signal.emitted_at != emitted_at {
                return Err(lifecycle_error(format!(
                    "signal {} timestamp does not match its batch",
                    signal.effect_id
                )));
            }
            if signal.scope.len() > self.budgets.max_fan_out {
                return Err(lifecycle_error(format!(
                    "signal {} scope exceeds fan-out budget",
                    signal.effect_id
                )));
            }
            if signal.evidence.len() > self.budgets.max_evidence_per_signal {
                return Err(lifecycle_error(format!(
                    "signal {} evidence exceeds budget",
                    signal.effect_id
                )));
            }
            for evidence in &signal.evidence {
                validate_cause_ref(evidence, self.budgets.max_text_bytes)?;
            }
            if signal.persistence == EffectPersistence::Commitment && signal.evidence.is_empty() {
                return Err(lifecycle_error(format!(
                    "commitment signal {} requires evidence",
                    signal.effect_id
                )));
            }
        }
        Ok(batch)
    }

    /// Builds a signal from a compiled effect binding, preventing callers from
    /// accidentally changing its target, persistence, scope, or cadence.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_effect(
        &mut self,
        plan: &CompiledCulturePlan,
        batch_id: impl Into<String>,
        effect: crate::EffectKey,
        strength_per_mille: u16,
        emitted_at: SimTime,
        earliest_eligible_at: SimTime,
        evidence: Vec<CauseRef>,
    ) -> Result<CulturalSignalBatch, CanwuError> {
        if plan.content_hash != self.state.plan_hash {
            return Err(lifecycle_error(
                "culture effect plan hash does not match runtime plan",
            ));
        }
        let binding = plan.effects.get(effect.get() as usize).ok_or_else(|| {
            lifecycle_error(format!("unknown culture effect key {}", effect.get()))
        })?;
        if binding.requires_evidence && evidence.is_empty() {
            return Err(lifecycle_error(format!(
                "culture effect {} requires evidence",
                binding.source_id
            )));
        }
        let target_id = &plan
            .targets
            .get(binding.target.get() as usize)
            .ok_or_else(|| lifecycle_error("compiled culture effect has an invalid target key"))?
            .source_id;
        let generation = self
            .target(target_id)
            .ok_or_else(|| lifecycle_error(format!("unknown culture target {target_id}")))?
            .generation;
        if let Some(cursor) = self.state.effect_emissions.get(&binding.source_id)
            && cursor.target_generation == generation
            && self
                .state
                .boundary_index
                .saturating_sub(cursor.boundary_index)
                < u64::from(binding.cadence_boundaries)
        {
            return Err(lifecycle_error(format!(
                "culture effect {} is not eligible at boundary {}",
                binding.source_id, self.state.boundary_index
            )));
        }
        let signal = CulturalSignal {
            effect_id: binding.source_id.clone(),
            target_id: target_id.clone(),
            generation,
            signal_kind: binding.signal_kind.clone(),
            persistence: binding.persistence,
            scope: binding.scope.clone(),
            strength_per_mille,
            emitted_at,
            evidence,
        };
        let batch =
            self.emit_signal_batch(batch_id, emitted_at, earliest_eligible_at, vec![signal])?;
        if !self.state.effect_emissions.contains_key(&binding.source_id) {
            let additional_bytes =
                160_usize.saturating_add(binding.source_id.len().saturating_mul(2));
            self.ensure_state_growth(additional_bytes)?;
            self.state_bytes = self.state_bytes.saturating_add(additional_bytes);
        }
        self.state.effect_emissions.insert(
            binding.source_id.clone(),
            EffectEmissionCursor {
                effect_id: binding.source_id.clone(),
                target_generation: generation,
                boundary_index: self.state.boundary_index,
            },
        );
        Ok(batch)
    }

    /// Attaches retained evidence to an already-created tombstone.
    pub fn attach_tombstone_evidence(
        &mut self,
        target_id: &str,
        generation: u64,
        evidence: impl IntoIterator<Item = CauseRef>,
    ) -> Result<(), CanwuError> {
        let tombstone_index = self
            .state
            .tombstones
            .iter()
            .position(|tombstone| {
                tombstone.target_id == target_id && tombstone.generation == generation
            })
            .ok_or_else(|| {
                lifecycle_error(format!("unknown tombstone {target_id}@{generation}"))
            })?;
        let mut unique = self.state.tombstones[tombstone_index].evidence.clone();
        let remaining_inputs = self
            .budgets
            .max_tombstone_evidence
            .saturating_sub(unique.len());
        let mut consumed = 0_usize;
        let mut additional_bytes = 0_usize;
        for cause in evidence {
            consumed = consumed
                .checked_add(1)
                .ok_or_else(|| lifecycle_error("tombstone evidence count overflowed"))?;
            if consumed > remaining_inputs {
                return Err(lifecycle_error(format!(
                    "tombstone {target_id}@{generation} evidence input exceeds budget"
                )));
            }
            validate_cause_ref(&cause, self.budgets.max_text_bytes)?;
            if !unique.contains(&cause) {
                additional_bytes = additional_bytes
                    .checked_add(cause_ref_estimated_bytes(&cause))
                    .ok_or_else(|| lifecycle_error("tombstone evidence size overflowed"))?;
                unique.push(cause);
            }
        }
        if unique.len() > self.budgets.max_tombstone_evidence {
            return Err(lifecycle_error(format!(
                "tombstone {target_id}@{generation} evidence exceeds budget"
            )));
        }
        self.ensure_state_growth(additional_bytes)?;
        self.state.tombstones[tombstone_index].evidence = unique;
        self.state_bytes = self.state_bytes.saturating_add(additional_bytes);
        Ok(())
    }

    fn target_mut(&mut self, target_id: &str) -> Result<&mut crate::TargetLifecycle, CanwuError> {
        self.state
            .targets
            .get_mut(target_id)
            .ok_or_else(|| lifecycle_error(format!("unknown culture target {target_id}")))
    }

    fn validate_mutation_time(&self, at: SimTime) -> Result<(), CanwuError> {
        if at < self.state.latest_activity_at {
            return Err(lifecycle_error(
                "culture lifecycle mutation cannot precede recorded activity",
            ));
        }
        Ok(())
    }

    fn ensure_state_growth(&self, additional: usize) -> Result<(), CanwuError> {
        let projected = self
            .state_bytes
            .checked_add(additional)
            .ok_or_else(|| lifecycle_error("culture state byte estimate overflowed"))?;
        if projected > self.budgets.max_state_bytes {
            return Err(lifecycle_error(format!(
                "culture state growth exceeds byte budget: {projected} > {}",
                self.budgets.max_state_bytes
            )));
        }
        Ok(())
    }

    fn refresh_state_bytes(&mut self) {
        self.state_bytes = self.state.estimated_bytes().unwrap_or(usize::MAX);
        debug_assert!(self.state_bytes <= self.budgets.max_state_bytes);
    }

    fn target_key_for_id(&self, target_id: &str) -> Option<crate::TargetKey> {
        self.target_keys.get(target_id).copied()
    }

    fn cancel_dormant_schedule(&mut self, target_id: &str) {
        let Some(due_at) = self.dormant_due_by_target.remove(target_id) else {
            return;
        };
        if let Some(targets) = self.state.dormant_due.get_mut(&due_at) {
            targets.remove(target_id);
            if targets.is_empty() {
                self.state.dormant_due.remove(&due_at);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LifecycleObservation {
    pub engaged_headcount: u64,
    pub admitted_work: bool,
    pub live_dependency: bool,
}

impl LifecycleObservation {
    #[must_use]
    pub const fn quiet() -> Self {
        Self {
            engaged_headcount: 0,
            admitted_work: false,
            live_dependency: false,
        }
    }
}

fn lifecycle_error(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

fn validate_cause_ref(cause: &CauseRef, max_text_bytes: usize) -> Result<(), CanwuError> {
    if let CauseRef::System(system) = cause
        && (system.is_empty() || system.len() > max_text_bytes)
    {
        return Err(lifecycle_error(
            "culture system evidence is empty or exceeds its byte budget",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CultureCohortDefinition, CultureDefinition, RetirementPolicy, compile_culture};
    use canwu_api::TerritoryId;

    fn runtime() -> CultureRuntime {
        let definition = CultureDefinition::builder("rights")
            .target("equality")
            .cohort(CultureCohortDefinition::new(
                "town",
                TerritoryId::new(1),
                100,
            ))
            .retirement(RetirementPolicy {
                dormant_after_boundaries: 2,
                retired_after_boundaries: 4,
            })
            .build()
            .expect("definition");
        let plan = compile_culture(&definition).expect("plan");
        CultureRuntime::new(&plan)
    }

    #[test]
    fn quiet_target_dormants_then_retires_without_losing_tombstone() {
        let mut runtime = runtime();
        let observations = BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
        assert!(
            runtime
                .settle_boundary(SimTime::from_minutes(1), &observations)
                .expect("boundary")
                .is_empty()
        );
        let transitions = runtime
            .settle_boundary(SimTime::from_minutes(2), &observations)
            .expect("boundary");
        assert_eq!(transitions[0].kind, LifecycleTransitionKind::BecameDormant);
        assert!(runtime.active_target_ids().is_empty());
        runtime
            .settle_boundary(SimTime::from_minutes(3), &observations)
            .expect("boundary");
        let transitions = runtime
            .settle_boundary(SimTime::from_minutes(4), &observations)
            .expect("boundary");
        assert_eq!(transitions[0].kind, LifecycleTransitionKind::Retired);
        assert_eq!(runtime.state.tombstones.len(), 1);
        assert_eq!(
            runtime.target("equality").expect("target").state,
            CultureLifecycle::Retired
        );
    }

    #[test]
    fn reactivation_creates_a_new_generation_and_old_signal_is_rejected() {
        let mut runtime = runtime();
        let observations = BTreeMap::from([("equality".to_owned(), LifecycleObservation::quiet())]);
        for minute in 1..=4 {
            runtime
                .settle_boundary(SimTime::from_minutes(minute), &observations)
                .expect("boundary");
        }
        let transition = runtime
            .reactivate("equality", SimTime::from_minutes(5))
            .expect("reactivate");
        assert_eq!(transition.generation, 2);
        assert!(runtime.active_target_ids().contains("equality"));
        let signal = CulturalSignal {
            effect_id: "pressure".to_owned(),
            target_id: "equality".to_owned(),
            generation: 1,
            signal_kind: "legitimacy_pressure".to_owned(),
            persistence: EffectPersistence::Level,
            scope: Vec::new(),
            strength_per_mille: 500,
            emitted_at: SimTime::from_minutes(5),
            evidence: Vec::new(),
        };
        assert!(
            runtime
                .emit_signal_batch(
                    "old",
                    SimTime::from_minutes(5),
                    SimTime::from_minutes(5),
                    vec![signal]
                )
                .is_err()
        );
    }
}
