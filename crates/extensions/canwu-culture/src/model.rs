use crate::PLUGIN_NAME;
use canwu_api::{
    CanwuError, CauseRef, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordType, DomainValueKindClass, EntityRef, ErrorCode, SimTime,
    TerritoryId, TypedDomainRecordRef,
};
use canwu_society::{DispositionProfile, TransitionWeights};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CULTURE_SCHEMA_VERSION: u32 = 1;
pub const CULTURE_PLAN_VERSION: u32 = 1;
pub const CULTURE_PLAN_HASH_DOMAIN: &str = "canwu.culture.compiled-plan.v1";

pub struct CultureStateRecord;

impl DomainRecordType for CultureStateRecord {
    type Payload = CultureState;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.culture";
    const NAME: &'static str = "state";
}

#[must_use]
pub fn culture_state_reference() -> TypedDomainRecordRef<CultureStateRecord> {
    TypedDomainRecordRef::new("root")
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TargetKey(u32);

impl TargetKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CohortKey(u32);

impl CohortKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ChannelKey(u32);

impl ChannelKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TransitionKey(u32);

impl TransitionKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct EffectKey(u32);

impl EffectKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct InstitutionKey(u32);

impl InstitutionKey {
    pub(crate) const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CultureBudgets {
    pub max_targets: usize,
    pub max_cohorts: usize,
    pub max_channels: usize,
    pub max_transitions: usize,
    pub max_effects: usize,
    pub max_institutions: usize,
    pub max_fan_out: usize,
    #[serde(default = "default_max_signals_per_batch")]
    pub max_signals_per_batch: usize,
    #[serde(default = "default_max_evidence_per_signal")]
    pub max_evidence_per_signal: usize,
    #[serde(default = "default_max_tombstone_evidence")]
    pub max_tombstone_evidence: usize,
    #[serde(default = "default_max_tombstones")]
    pub max_tombstones: usize,
    #[serde(default = "default_max_text_bytes")]
    pub max_text_bytes: usize,
    #[serde(default = "default_max_state_bytes")]
    pub max_state_bytes: usize,
    pub max_memory_bytes: usize,
}

impl CultureBudgets {
    #[must_use]
    pub const fn conservative() -> Self {
        Self {
            max_targets: 10_000,
            max_cohorts: 10_000,
            max_channels: 20_000,
            max_transitions: 50_000,
            max_effects: 20_000,
            max_institutions: 10_000,
            max_fan_out: 128,
            max_signals_per_batch: 256,
            max_evidence_per_signal: 64,
            max_tombstone_evidence: 256,
            max_tombstones: 100_000,
            max_text_bytes: 4_096,
            max_state_bytes: 64 * 1024 * 1024,
            max_memory_bytes: 64 * 1024 * 1024,
        }
    }
}

const fn default_max_signals_per_batch() -> usize {
    256
}

const fn default_max_evidence_per_signal() -> usize {
    64
}

const fn default_max_tombstone_evidence() -> usize {
    256
}

const fn default_max_tombstones() -> usize {
    100_000
}

const fn default_max_text_bytes() -> usize {
    4_096
}

const fn default_max_state_bytes() -> usize {
    64 * 1024 * 1024
}

impl Default for CultureBudgets {
    fn default() -> Self {
        Self::conservative()
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetirementPolicy {
    pub dormant_after_boundaries: u32,
    pub retired_after_boundaries: u32,
}

impl RetirementPolicy {
    #[must_use]
    pub const fn after_quiet_boundaries(boundaries: u32) -> Self {
        Self {
            dormant_after_boundaries: boundaries,
            retired_after_boundaries: boundaries.saturating_mul(3),
        }
    }

    fn validate(self) -> Result<(), CanwuError> {
        if self.dormant_after_boundaries == 0 || self.retired_after_boundaries == 0 {
            return Err(invalid(
                "retirement quiet-boundary windows must be greater than zero",
            ));
        }
        if self.retired_after_boundaries < self.dormant_after_boundaries {
            return Err(invalid(
                "retired quiet-boundary window cannot precede dormant window",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CultureTargetDefinition {
    pub id: String,
    pub parent: Option<String>,
    pub neutral_profile: DispositionProfile,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl CultureTargetDefinition {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent: None,
            neutral_profile: DispositionProfile::neutral(),
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CultureCohortDefinition {
    pub id: String,
    pub territory: TerritoryId,
    pub headcount: u64,
    #[serde(default)]
    pub classification: BTreeMap<String, String>,
}

impl CultureCohortDefinition {
    #[must_use]
    pub fn new(id: impl Into<String>, territory: TerritoryId, headcount: u64) -> Self {
        Self {
            id: id.into(),
            territory,
            headcount,
            classification: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChannelSpec {
    pub id: String,
    pub source_cohort_id: Option<String>,
    pub target_cohort_id: String,
    pub target_id: String,
    pub reach_per_mille: u16,
    pub trust_per_mille: u16,
    pub interpretation_fidelity_per_mille: u16,
    pub delay_boundaries: u32,
    pub capacity: u64,
    pub active: bool,
}

impl ChannelSpec {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        target_cohort_id: impl Into<String>,
        target_id: impl Into<String>,
        reach_per_mille: u16,
        trust_per_mille: u16,
    ) -> Self {
        Self {
            id: id.into(),
            source_cohort_id: None,
            target_cohort_id: target_cohort_id.into(),
            target_id: target_id.into(),
            reach_per_mille,
            trust_per_mille,
            interpretation_fidelity_per_mille: 1_000,
            delay_boundaries: 0,
            capacity: u64::MAX,
            active: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionSpec {
    pub id: String,
    pub target_id: String,
    #[serde(default)]
    pub affected_cohorts: BTreeSet<String>,
    pub from: DispositionProfile,
    pub to: DispositionProfile,
    pub base_rate_per_million: u32,
    pub weights: TransitionWeights,
}

impl TransitionSpec {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        target_id: impl Into<String>,
        from: DispositionProfile,
        to: DispositionProfile,
        base_rate_per_million: u32,
        weights: TransitionWeights,
    ) -> Self {
        Self {
            id: id.into(),
            target_id: target_id.into(),
            affected_cohorts: BTreeSet::new(),
            from,
            to,
            base_rate_per_million,
            weights,
        }
    }

    #[must_use]
    pub fn awareness_from_influence(
        id: impl Into<String>,
        target_id: impl Into<String>,
        base_rate_per_million: u32,
    ) -> Self {
        Self::new(
            id,
            target_id,
            DispositionProfile::neutral(),
            DispositionProfile {
                awareness: canwu_society::AwarenessBand::Aware,
                ..DispositionProfile::neutral()
            },
            base_rate_per_million,
            TransitionWeights {
                influence: 1_000_000,
                ..TransitionWeights::default()
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectPersistence {
    Pulse,
    Level,
    Commitment,
    Evidence,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CulturalEffectBinding {
    pub id: String,
    pub target_id: String,
    pub signal_kind: String,
    pub scope: BTreeSet<String>,
    pub cadence_boundaries: u32,
    pub persistence: EffectPersistence,
    pub requires_evidence: bool,
}

impl CulturalEffectBinding {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        target_id: impl Into<String>,
        signal_kind: impl Into<String>,
        persistence: EffectPersistence,
    ) -> Self {
        Self {
            id: id.into(),
            target_id: target_id.into(),
            signal_kind: signal_kind.into(),
            scope: BTreeSet::new(),
            cadence_boundaries: 1,
            persistence,
            requires_evidence: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstitutionBinding {
    pub id: String,
    pub target_id: String,
    pub institution: EntityRef,
    #[serde(default)]
    pub affected_cohorts: BTreeSet<String>,
}

impl InstitutionBinding {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        target_id: impl Into<String>,
        institution: EntityRef,
    ) -> Self {
        Self {
            id: id.into(),
            target_id: target_id.into(),
            institution,
            affected_cohorts: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CultureDefinition {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub targets: Vec<CultureTargetDefinition>,
    #[serde(default)]
    pub cohorts: Vec<CultureCohortDefinition>,
    #[serde(default)]
    pub channels: Vec<ChannelSpec>,
    #[serde(default)]
    pub transitions: Vec<TransitionSpec>,
    #[serde(default)]
    pub effects: Vec<CulturalEffectBinding>,
    #[serde(default)]
    pub institutions: Vec<InstitutionBinding>,
    #[serde(default)]
    pub budgets: CultureBudgets,
    #[serde(default = "default_retirement_policy")]
    pub retirement: RetirementPolicy,
}

fn default_retirement_policy() -> RetirementPolicy {
    RetirementPolicy::after_quiet_boundaries(30)
}

impl CultureDefinition {
    #[must_use]
    pub fn builder(id: impl Into<String>) -> CultureDefinitionBuilder {
        CultureDefinitionBuilder {
            definition: Self {
                schema_version: CULTURE_SCHEMA_VERSION,
                id: id.into(),
                targets: Vec::new(),
                cohorts: Vec::new(),
                channels: Vec::new(),
                transitions: Vec::new(),
                effects: Vec::new(),
                institutions: Vec::new(),
                budgets: CultureBudgets::default(),
                retirement: default_retirement_policy(),
            },
        }
    }
}

pub struct CultureDefinitionBuilder {
    definition: CultureDefinition,
}

impl CultureDefinitionBuilder {
    #[must_use]
    pub fn target(mut self, id: impl Into<String>) -> Self {
        self.definition
            .targets
            .push(CultureTargetDefinition::new(id));
        self
    }

    #[must_use]
    pub fn target_definition(mut self, target: CultureTargetDefinition) -> Self {
        self.definition.targets.push(target);
        self
    }

    #[must_use]
    pub fn cohort(mut self, cohort: CultureCohortDefinition) -> Self {
        self.definition.cohorts.push(cohort);
        self
    }

    #[must_use]
    pub fn channel(mut self, channel: ChannelSpec) -> Self {
        self.definition.channels.push(channel);
        self
    }

    #[must_use]
    pub fn transition(mut self, transition: TransitionSpec) -> Self {
        self.definition.transitions.push(transition);
        self
    }

    #[must_use]
    pub fn effect(mut self, effect: CulturalEffectBinding) -> Self {
        self.definition.effects.push(effect);
        self
    }

    #[must_use]
    pub fn institution(mut self, institution: InstitutionBinding) -> Self {
        self.definition.institutions.push(institution);
        self
    }

    #[must_use]
    pub fn budgets(mut self, budgets: CultureBudgets) -> Self {
        self.definition.budgets = budgets;
        self
    }

    #[must_use]
    pub fn retirement(mut self, retirement: RetirementPolicy) -> Self {
        self.definition.retirement = retirement;
        self
    }

    pub fn build(self) -> Result<CultureDefinition, CanwuError> {
        validate_definition(&self.definition)?;
        Ok(self.definition)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledTarget {
    pub(crate) key: TargetKey,
    pub(crate) source_id: String,
    pub(crate) parent: Option<TargetKey>,
    pub(crate) neutral_profile: DispositionProfile,
    pub(crate) metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledCohort {
    pub(crate) key: CohortKey,
    pub(crate) source_id: String,
    pub(crate) territory: TerritoryId,
    pub(crate) headcount: u64,
    pub(crate) classification: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledChannel {
    pub(crate) key: ChannelKey,
    pub(crate) source_id: String,
    pub(crate) source_cohort: Option<CohortKey>,
    pub(crate) target_cohort: CohortKey,
    pub(crate) target: TargetKey,
    pub(crate) reach_per_mille: u16,
    pub(crate) trust_per_mille: u16,
    pub(crate) interpretation_fidelity_per_mille: u16,
    pub(crate) delay_boundaries: u32,
    pub(crate) capacity: u64,
    pub(crate) active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledTransition {
    pub(crate) key: TransitionKey,
    pub(crate) source_id: String,
    pub(crate) target: TargetKey,
    pub(crate) affected_cohorts: Vec<CohortKey>,
    pub(crate) from: DispositionProfile,
    pub(crate) to: DispositionProfile,
    pub(crate) base_rate_per_million: u32,
    pub(crate) weights: TransitionWeights,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledEffect {
    pub(crate) key: EffectKey,
    pub(crate) source_id: String,
    pub(crate) target: TargetKey,
    pub(crate) signal_kind: String,
    pub(crate) scope: Vec<String>,
    pub(crate) cadence_boundaries: u32,
    pub(crate) persistence: EffectPersistence,
    pub(crate) requires_evidence: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledInstitutionBinding {
    pub(crate) key: InstitutionKey,
    pub(crate) source_id: String,
    pub(crate) target: TargetKey,
    pub(crate) institution: EntityRef,
    pub(crate) affected_cohorts: Vec<CohortKey>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompiledCulturePlan {
    pub(crate) plan_version: u32,
    pub(crate) definition_id: String,
    pub(crate) content_hash: String,
    pub(crate) budgets: CultureBudgets,
    pub(crate) retirement: RetirementPolicy,
    pub(crate) targets: Vec<CompiledTarget>,
    pub(crate) cohorts: Vec<CompiledCohort>,
    pub(crate) channels: Vec<CompiledChannel>,
    pub(crate) transitions: Vec<CompiledTransition>,
    pub(crate) effects: Vec<CompiledEffect>,
    pub(crate) institutions: Vec<CompiledInstitutionBinding>,
    pub(crate) target_by_id: BTreeMap<String, TargetKey>,
    pub(crate) cohort_by_id: BTreeMap<String, CohortKey>,
    pub(crate) transitions_by_target: BTreeMap<TargetKey, Vec<TransitionKey>>,
    pub(crate) effects_by_target: BTreeMap<TargetKey, Vec<EffectKey>>,
    pub(crate) institutions_by_target: BTreeMap<TargetKey, Vec<InstitutionKey>>,
}

impl CompiledCulturePlan {
    #[must_use]
    pub fn definition_id(&self) -> &str {
        &self.definition_id
    }

    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    #[must_use]
    pub const fn budgets(&self) -> CultureBudgets {
        self.budgets
    }

    #[must_use]
    pub fn target_key(&self, id: &str) -> Option<TargetKey> {
        self.target_by_id.get(id).copied()
    }

    #[must_use]
    pub fn effect_key(&self, id: &str) -> Option<EffectKey> {
        self.effects
            .iter()
            .find(|effect| effect.source_id == id)
            .map(|effect| effect.key)
    }

    #[must_use]
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    #[must_use]
    pub fn cohort_count(&self) -> usize {
        self.cohorts.len()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DirtyPair {
    pub cohort: CohortKey,
    pub target: TargetKey,
}

/// Ordered dirty-set index used by incremental society settlement.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DirtySet {
    pairs: BTreeSet<DirtyPair>,
}

impl DirtySet {
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    pub fn mark(&mut self, pair: DirtyPair) -> bool {
        self.pairs.insert(pair)
    }

    #[must_use]
    pub(crate) fn contains(&self, pair: DirtyPair) -> bool {
        self.pairs.contains(&pair)
    }

    pub(crate) fn mark_pairs(&mut self, pairs: impl IntoIterator<Item = DirtyPair>) {
        self.pairs.extend(pairs);
    }

    /// Marks all cohort relationships affected by a target's compiled rules.
    /// A rule with no cohort scope intentionally means every compiled cohort,
    /// while still materializing only when the caller explicitly marks it.
    pub fn mark_target(
        &mut self,
        plan: &CompiledCulturePlan,
        target_id: &str,
    ) -> Result<usize, CanwuError> {
        let pairs = Self::pairs_for_target(plan, target_id)?;
        let before = self.len();
        self.mark_pairs(pairs);
        Ok(self.len().saturating_sub(before))
    }

    pub(crate) fn pairs_for_target(
        plan: &CompiledCulturePlan,
        target_id: &str,
    ) -> Result<Vec<DirtyPair>, CanwuError> {
        let target = plan
            .target_by_id
            .get(target_id)
            .ok_or_else(|| invalid(format!("unknown culture target {target_id}")))?;
        let mut cohorts = BTreeSet::new();
        for transition_key in plan.transitions_by_target.get(target).into_iter().flatten() {
            let transition = &plan.transitions[transition_key.get() as usize];
            if transition.affected_cohorts.is_empty() {
                cohorts.extend(plan.cohorts.iter().map(|cohort| cohort.key));
            } else {
                cohorts.extend(transition.affected_cohorts.iter().copied());
            }
        }
        if cohorts.len() > plan.budgets.max_fan_out {
            return Err(invalid(format!(
                "culture target {target_id} expands to {} dirty cohorts, above fan-out budget {}",
                cohorts.len(),
                plan.budgets.max_fan_out
            )));
        }
        Ok(cohorts
            .into_iter()
            .map(|cohort| DirtyPair {
                cohort,
                target: *target,
            })
            .collect())
    }

    /// Drains pairs in canonical cohort/target order for one boundary.
    pub fn drain_sorted(&mut self) -> Vec<DirtyPair> {
        std::mem::take(&mut self.pairs).into_iter().collect()
    }

    pub(crate) fn pairs(&self) -> impl Iterator<Item = DirtyPair> + '_ {
        self.pairs.iter().copied()
    }

    pub(crate) fn remove_target(&mut self, target: TargetKey) {
        self.pairs.retain(|pair| pair.target != target);
    }

    pub(crate) fn count_for_targets(&self, targets: &BTreeSet<TargetKey>) -> usize {
        self.pairs
            .iter()
            .filter(|pair| targets.contains(&pair.target))
            .count()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CultureLifecycle {
    Active,
    Dormant,
    Retired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetiredTargetTombstone {
    pub target_id: String,
    pub generation: u64,
    pub last_active_at: SimTime,
    pub retired_at: SimTime,
    pub reason: String,
    pub policy_hash: String,
    pub successor: Option<String>,
    pub evidence: Vec<CauseRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetLifecycle {
    pub target_id: String,
    pub generation: u64,
    pub state: CultureLifecycle,
    pub engaged_headcount: u64,
    pub quiet_boundaries: u32,
    pub dormant_since_boundary: Option<u64>,
    pub last_active_at: SimTime,
    pub last_work_at: Option<SimTime>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleTransitionKind {
    BecameDormant,
    Reactivated,
    Retired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecycleTransition {
    pub target_id: String,
    pub generation: u64,
    pub kind: LifecycleTransitionKind,
    pub at: SimTime,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CulturalSignal {
    pub effect_id: String,
    pub target_id: String,
    pub generation: u64,
    pub signal_kind: String,
    pub persistence: EffectPersistence,
    pub scope: Vec<String>,
    pub strength_per_mille: u16,
    pub emitted_at: SimTime,
    pub evidence: Vec<CauseRef>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CulturalSignalBatch {
    pub id: String,
    pub plan_hash: String,
    pub emitted_at: SimTime,
    pub earliest_eligible_at: SimTime,
    pub signals: Vec<CulturalSignal>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectEmissionCursor {
    pub effect_id: String,
    pub target_generation: u64,
    pub boundary_index: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CultureState {
    pub(crate) schema_version: u32,
    pub(crate) plan_hash: String,
    pub(crate) boundary_index: u64,
    pub(crate) last_boundary_at: SimTime,
    pub(crate) latest_activity_at: SimTime,
    pub(crate) targets: BTreeMap<String, TargetLifecycle>,
    pub(crate) hot_targets: BTreeSet<String>,
    pub(crate) dormant_due: BTreeMap<u64, BTreeSet<String>>,
    pub(crate) dirty_pairs: DirtySet,
    pub(crate) effect_emissions: BTreeMap<String, EffectEmissionCursor>,
    pub(crate) tombstones: Vec<RetiredTargetTombstone>,
}

impl CultureState {
    #[must_use]
    pub(crate) fn from_plan_at(plan: &CompiledCulturePlan, initial_time: SimTime) -> Self {
        let mut targets = BTreeMap::new();
        for target in &plan.targets {
            targets.insert(
                target.source_id.clone(),
                TargetLifecycle {
                    target_id: target.source_id.clone(),
                    generation: 1,
                    state: CultureLifecycle::Active,
                    engaged_headcount: 0,
                    quiet_boundaries: 0,
                    dormant_since_boundary: None,
                    last_active_at: initial_time,
                    last_work_at: None,
                },
            );
        }
        Self {
            schema_version: CULTURE_SCHEMA_VERSION,
            plan_hash: plan.content_hash.clone(),
            boundary_index: 0,
            last_boundary_at: initial_time,
            latest_activity_at: initial_time,
            hot_targets: targets.keys().cloned().collect(),
            targets,
            dormant_due: BTreeMap::new(),
            dirty_pairs: DirtySet::default(),
            effect_emissions: BTreeMap::new(),
            tombstones: Vec::new(),
        }
    }

    #[must_use]
    pub fn plan_hash(&self) -> &str {
        &self.plan_hash
    }

    #[must_use]
    pub const fn boundary_index(&self) -> u64 {
        self.boundary_index
    }

    #[must_use]
    pub fn targets(&self) -> &BTreeMap<String, TargetLifecycle> {
        &self.targets
    }

    #[must_use]
    pub fn tombstones(&self) -> &[RetiredTargetTombstone] {
        &self.tombstones
    }

    /// Canonicalizes tombstone order and validates the persisted lifecycle
    /// index against the compiled plan hash.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn validate(&self, expected_plan_hash: &str) -> Result<(), CanwuError> {
        if self.schema_version != CULTURE_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported culture state schema version {}",
                self.schema_version
            )));
        }
        if self.plan_hash != expected_plan_hash || !is_hash(&self.plan_hash) {
            return Err(invalid(
                "culture state plan hash is not bound to the compiled plan",
            ));
        }
        let mut expected_latest_activity = self.last_boundary_at;
        for (key, target) in &self.targets {
            if key != &target.target_id || target.generation == 0 {
                return Err(invalid(format!(
                    "culture lifecycle entry {key} has an invalid identity or generation"
                )));
            }
            if (target.state == CultureLifecycle::Dormant)
                != target.dormant_since_boundary.is_some()
            {
                return Err(invalid(format!(
                    "culture lifecycle entry {key} has an invalid dormancy origin"
                )));
            }
            expected_latest_activity = expected_latest_activity.max(target.last_active_at);
            if let Some(last_work_at) = target.last_work_at {
                expected_latest_activity = expected_latest_activity.max(last_work_at);
            }
        }
        if self.latest_activity_at != expected_latest_activity {
            return Err(invalid(
                "culture latest-activity index is not derived from target activity",
            ));
        }
        let expected_hot = self
            .targets
            .values()
            .filter(|target| target.state == CultureLifecycle::Active)
            .map(|target| target.target_id.clone())
            .collect::<BTreeSet<_>>();
        if self.hot_targets != expected_hot {
            return Err(invalid(
                "culture hot-target index does not match active lifecycle entries",
            ));
        }
        let mut scheduled_targets = BTreeSet::new();
        for (due_at, targets) in &self.dormant_due {
            if *due_at <= self.boundary_index || targets.is_empty() {
                return Err(invalid("culture dormant schedule is stale or empty"));
            }
            for target_id in targets {
                if !scheduled_targets.insert(target_id.clone())
                    || self
                        .targets
                        .get(target_id)
                        .is_none_or(|target| target.state != CultureLifecycle::Dormant)
                {
                    return Err(invalid(
                        "culture dormant schedule does not match dormant lifecycle entries",
                    ));
                }
            }
        }
        let dormant_targets = self
            .targets
            .values()
            .filter(|target| target.state == CultureLifecycle::Dormant)
            .map(|target| target.target_id.clone())
            .collect::<BTreeSet<_>>();
        if scheduled_targets != dormant_targets {
            return Err(invalid(
                "every dormant culture target must have exactly one retirement schedule",
            ));
        }
        if self.tombstones.windows(2).any(|pair| {
            (pair[0].target_id.as_str(), pair[0].generation)
                >= (pair[1].target_id.as_str(), pair[1].generation)
        }) {
            return Err(invalid("culture tombstones must be sorted and unique"));
        }
        for tombstone in &self.tombstones {
            let Some(target) = self.targets.get(&tombstone.target_id) else {
                return Err(invalid(format!(
                    "culture tombstone references unknown target {}",
                    tombstone.target_id
                )));
            };
            if target.generation < tombstone.generation
                || tombstone.generation == 0
                || tombstone.policy_hash != expected_plan_hash
                || tombstone.retired_at < tombstone.last_active_at
                || tombstone.retired_at > self.last_boundary_at
            {
                return Err(invalid(format!(
                    "culture tombstone {}@{} is not generation-bound",
                    tombstone.target_id, tombstone.generation
                )));
            }
        }
        for (effect_id, cursor) in &self.effect_emissions {
            if effect_id != &cursor.effect_id
                || cursor.target_generation == 0
                || cursor.boundary_index > self.boundary_index
            {
                return Err(invalid(format!(
                    "culture effect emission cursor {effect_id} is invalid"
                )));
            }
        }
        Ok(())
    }

    /// Validates both the persisted lifecycle index and its target catalog
    /// against the exact compiled plan used by the host.
    #[allow(clippy::too_many_lines)]
    pub fn validate_against_plan(&self, plan: &CompiledCulturePlan) -> Result<(), CanwuError> {
        if self.tombstones.len() > plan.budgets.max_tombstones {
            return Err(invalid(format!(
                "culture state exceeds tombstone budget: {} > {}",
                self.tombstones.len(),
                plan.budgets.max_tombstones
            )));
        }
        if self
            .tombstones
            .iter()
            .any(|tombstone| tombstone.evidence.len() > plan.budgets.max_tombstone_evidence)
        {
            return Err(invalid(
                "culture tombstone exceeds its evidence-count budget",
            ));
        }
        let total_evidence = self
            .tombstones
            .iter()
            .try_fold(0_usize, |total, tombstone| {
                total
                    .checked_add(tombstone.evidence.len())
                    .ok_or_else(|| invalid("culture tombstone evidence count overflowed"))
            })?;
        let minimum_evidence_bytes = total_evidence
            .checked_mul(64)
            .ok_or_else(|| invalid("culture tombstone evidence size overflowed"))?;
        if minimum_evidence_bytes > plan.budgets.max_state_bytes {
            return Err(invalid(
                "culture state exceeds its byte budget before evidence validation",
            ));
        }
        let estimated_state_bytes = self.estimated_bytes()?;
        if estimated_state_bytes > plan.budgets.max_state_bytes {
            return Err(invalid(format!(
                "culture state exceeds byte budget: {estimated_state_bytes} > {}",
                plan.budgets.max_state_bytes
            )));
        }
        self.validate(&plan.content_hash)?;
        let expected_targets = plan
            .targets
            .iter()
            .map(|target| target.source_id.as_str())
            .collect::<BTreeSet<_>>();
        let actual_targets = self
            .targets
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual_targets != expected_targets {
            return Err(invalid(
                "culture lifecycle target catalog does not match the compiled plan",
            ));
        }
        for pair in self.dirty_pairs.pairs() {
            let Some(target) = plan.targets.get(pair.target.get() as usize) else {
                return Err(invalid("culture dirty set contains an unknown target key"));
            };
            let cohort_exists = plan.cohorts.get(pair.cohort.get() as usize).is_some();
            let affected_by_rule = plan
                .transitions_by_target
                .get(&pair.target)
                .into_iter()
                .flatten()
                .filter_map(|key| plan.transitions.get(key.get() as usize))
                .any(|transition| {
                    transition.affected_cohorts.is_empty()
                        || transition.affected_cohorts.contains(&pair.cohort)
                });
            if !cohort_exists
                || !affected_by_rule
                || self
                    .targets
                    .get(&target.source_id)
                    .is_none_or(|lifecycle| lifecycle.state != CultureLifecycle::Active)
            {
                return Err(invalid(
                    "culture dirty set contains an unknown cohort or inactive target",
                ));
            }
        }
        for target in self.targets.values() {
            match target.state {
                CultureLifecycle::Active
                    if target.quiet_boundaries >= plan.retirement.dormant_after_boundaries =>
                {
                    return Err(invalid(format!(
                        "active culture target {} exceeds its dormant quiet window",
                        target.target_id
                    )));
                }
                CultureLifecycle::Dormant
                    if target.quiet_boundaries < plan.retirement.dormant_after_boundaries
                        || target.quiet_boundaries >= plan.retirement.retired_after_boundaries =>
                {
                    return Err(invalid(format!(
                        "dormant culture target {} has an invalid quiet-boundary count",
                        target.target_id
                    )));
                }
                CultureLifecycle::Retired
                    if target.quiet_boundaries < plan.retirement.retired_after_boundaries =>
                {
                    return Err(invalid(format!(
                        "retired culture target {} has not reached its retirement window",
                        target.target_id
                    )));
                }
                _ => {}
            }
            if target.state == CultureLifecycle::Dormant {
                let dormant_since = target
                    .dormant_since_boundary
                    .ok_or_else(|| invalid("dormant culture target has no origin boundary"))?;
                if dormant_since > self.boundary_index {
                    return Err(invalid("culture dormancy origin is in the future"));
                }
                let expected_due = dormant_since
                    .checked_add(u64::from(
                        plan.retirement
                            .retired_after_boundaries
                            .saturating_sub(plan.retirement.dormant_after_boundaries),
                    ))
                    .ok_or_else(|| invalid("culture dormant schedule exceeds boundary range"))?;
                if self
                    .dormant_due
                    .get(&expected_due)
                    .is_none_or(|targets| !targets.contains(&target.target_id))
                {
                    return Err(invalid(format!(
                        "culture target {} has a non-canonical dormant schedule",
                        target.target_id
                    )));
                }
            }
        }
        for (effect_id, cursor) in &self.effect_emissions {
            let Some(effect) = plan
                .effects
                .iter()
                .find(|effect| effect.source_id == *effect_id)
            else {
                return Err(invalid(format!(
                    "culture emission cursor references unknown effect {effect_id}"
                )));
            };
            let target = plan
                .targets
                .get(effect.target.get() as usize)
                .ok_or_else(|| invalid("compiled culture effect has an invalid target key"))?;
            if self
                .targets
                .get(&target.source_id)
                .is_none_or(|lifecycle| cursor.target_generation > lifecycle.generation)
            {
                return Err(invalid(format!(
                    "culture emission cursor {effect_id} has an invalid generation"
                )));
            }
        }
        let mut tombstone_index = 0_usize;
        for target in self.targets.values() {
            let expected_end = if target.state == CultureLifecycle::Retired {
                target.generation
            } else {
                target.generation.saturating_sub(1)
            };
            let first_tombstone = tombstone_index;
            while let Some(tombstone) = self.tombstones.get(tombstone_index)
                && tombstone.target_id == target.target_id
            {
                let generation = u64::try_from(tombstone_index - first_tombstone)
                    .ok()
                    .and_then(|offset| offset.checked_add(1))
                    .ok_or_else(|| invalid("culture tombstone generation count overflowed"))?;
                if tombstone.generation != generation {
                    return Err(invalid(format!(
                        "culture target {} tombstone history is non-contiguous",
                        target.target_id
                    )));
                }
                tombstone_index += 1;
            }
            let actual_end = u64::try_from(tombstone_index - first_tombstone)
                .map_err(|_| invalid("culture tombstone generation count overflowed"))?;
            if actual_end != expected_end {
                return Err(invalid(format!(
                    "culture target {} lifecycle is not backed by a complete tombstone history",
                    target.target_id
                )));
            }
        }
        if tombstone_index != self.tombstones.len() {
            return Err(invalid("culture state contains an unexpected tombstone"));
        }
        for tombstone in &self.tombstones {
            if tombstone.evidence.len() > plan.budgets.max_tombstone_evidence {
                return Err(invalid(format!(
                    "culture tombstone {}@{} exceeds its evidence budget",
                    tombstone.target_id, tombstone.generation
                )));
            }
            validate_text(
                &tombstone.reason,
                plan.budgets.max_text_bytes,
                "culture tombstone reason",
            )?;
            for evidence in &tombstone.evidence {
                if let CauseRef::System(system) = evidence {
                    validate_text(
                        system,
                        plan.budgets.max_text_bytes,
                        "culture tombstone system evidence",
                    )?;
                }
            }
            if tombstone
                .successor
                .as_ref()
                .is_some_and(|successor| !self.targets.contains_key(successor))
            {
                return Err(invalid(format!(
                    "culture tombstone {}@{} references an unknown successor",
                    tombstone.target_id, tombstone.generation
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn estimated_bytes(&self) -> Result<usize, CanwuError> {
        let mut total = 512_usize;
        for (target_id, target) in &self.targets {
            total = checked_state_add(total, 256, "target")?;
            total = checked_state_add(total, target_id.len(), "target ID")?;
            total = checked_state_add(total, target.target_id.len(), "target identity")?;
        }
        for target_id in &self.hot_targets {
            total = checked_state_add(total, 64, "hot target")?;
            total = checked_state_add(total, target_id.len(), "hot target ID")?;
        }
        for targets in self.dormant_due.values() {
            total = checked_state_add(total, 64, "dormant schedule")?;
            for target_id in targets {
                total = checked_state_add(total, 64, "scheduled target")?;
                total = checked_state_add(total, target_id.len(), "scheduled target ID")?;
            }
        }
        total = checked_state_add(
            total,
            self.dirty_pairs
                .len()
                .checked_mul(48)
                .ok_or_else(|| invalid("culture dirty-set size overflowed"))?,
            "dirty set",
        )?;
        for (effect_id, cursor) in &self.effect_emissions {
            total = checked_state_add(total, 160, "effect cursor")?;
            total = checked_state_add(total, effect_id.len(), "effect cursor ID")?;
            total = checked_state_add(total, cursor.effect_id.len(), "effect identity")?;
        }
        for tombstone in &self.tombstones {
            total = checked_state_add(total, tombstone_estimated_bytes(tombstone)?, "tombstone")?;
        }
        Ok(total)
    }

    pub(crate) fn contains_tombstone(&self, target_id: &str, generation: u64) -> bool {
        self.tombstones
            .binary_search_by(|candidate| {
                candidate
                    .target_id
                    .as_str()
                    .cmp(target_id)
                    .then(candidate.generation.cmp(&generation))
            })
            .is_ok()
    }

    pub(crate) fn insert_tombstone(&mut self, tombstone: RetiredTargetTombstone) {
        let search = self.tombstones.binary_search_by(|candidate| {
            candidate
                .target_id
                .cmp(&tombstone.target_id)
                .then(candidate.generation.cmp(&tombstone.generation))
        });
        debug_assert!(search.is_err(), "tombstone insertion was prevalidated");
        let position = search.unwrap_or_else(|position| position);
        self.tombstones.insert(position, tombstone);
    }

    /// Encodes the lifecycle index as the plugin-owned root domain record.
    pub(crate) fn into_record(self) -> Result<DomainRecord, CanwuError> {
        self.validate(&self.plan_hash)?;
        let draft = DomainRecordDraft::from_typed(culture_state_reference(), &self)?;
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
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[allow(clippy::collapsible_if, clippy::too_many_lines)]
pub(crate) fn validate_definition(definition: &CultureDefinition) -> Result<(), CanwuError> {
    if definition.schema_version != CULTURE_SCHEMA_VERSION {
        return Err(invalid(format!(
            "unsupported culture schema version {}",
            definition.schema_version
        )));
    }
    validate_identifier(&definition.id, "culture definition")?;
    definition.retirement.validate()?;
    let budgets = definition.budgets;
    if budgets.max_fan_out == 0
        || budgets.max_signals_per_batch == 0
        || budgets.max_evidence_per_signal == 0
        || budgets.max_tombstone_evidence == 0
        || budgets.max_tombstones == 0
        || budgets.max_text_bytes == 0
        || budgets.max_state_bytes == 0
        || budgets.max_memory_bytes == 0
    {
        return Err(invalid("culture budgets must be greater than zero"));
    }
    for (actual, limit, label) in [
        (definition.targets.len(), budgets.max_targets, "targets"),
        (definition.cohorts.len(), budgets.max_cohorts, "cohorts"),
        (definition.channels.len(), budgets.max_channels, "channels"),
        (
            definition.transitions.len(),
            budgets.max_transitions,
            "transitions",
        ),
        (definition.effects.len(), budgets.max_effects, "effects"),
        (
            definition.institutions.len(),
            budgets.max_institutions,
            "institutions",
        ),
    ] {
        if actual > limit {
            return Err(invalid(format!(
                "culture definition exceeds {label} budget: {actual} > {limit}"
            )));
        }
    }

    let target_ids = unique_ids(
        definition.targets.iter().map(|target| target.id.as_str()),
        "target",
    )?;
    let cohort_ids = unique_ids(
        definition.cohorts.iter().map(|cohort| cohort.id.as_str()),
        "cohort",
    )?;
    let _channel_ids = unique_ids(
        definition
            .channels
            .iter()
            .map(|channel| channel.id.as_str()),
        "channel",
    )?;
    let _transition_ids = unique_ids(
        definition
            .transitions
            .iter()
            .map(|transition| transition.id.as_str()),
        "transition",
    )?;
    let _effect_ids = unique_ids(
        definition.effects.iter().map(|effect| effect.id.as_str()),
        "effect",
    )?;
    let _institution_ids = unique_ids(
        definition
            .institutions
            .iter()
            .map(|institution| institution.id.as_str()),
        "institution binding",
    )?;

    for target in &definition.targets {
        if let Some(parent) = &target.parent
            && (parent == &target.id || !target_ids.contains(parent))
        {
            return Err(invalid(format!(
                "target {} references an invalid parent {parent}",
                target.id
            )));
        }
    }
    for target in &definition.targets {
        let mut seen = BTreeSet::new();
        let mut current = Some(target.id.as_str());
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err(invalid(format!(
                    "target {} participates in an ancestry cycle",
                    target.id
                )));
            }
            current = definition
                .targets
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.parent.as_deref());
        }
    }
    for cohort in &definition.cohorts {
        validate_identifier(&cohort.id, "cohort")?;
        if cohort.headcount == 0 {
            return Err(invalid(format!("cohort {} has zero headcount", cohort.id)));
        }
        validate_authored_map(
            &cohort.classification,
            budgets,
            &format!("cohort {} classification", cohort.id),
        )?;
    }
    for target in &definition.targets {
        validate_identifier(&target.id, "target")?;
        validate_authored_map(
            &target.metadata,
            budgets,
            &format!("target {} metadata", target.id),
        )?;
    }
    for channel in &definition.channels {
        validate_identifier(&channel.id, "channel")?;
        if !cohort_ids.contains(&channel.target_cohort_id)
            || !target_ids.contains(&channel.target_id)
            || channel
                .source_cohort_id
                .as_ref()
                .is_some_and(|id| !cohort_ids.contains(id))
        {
            return Err(invalid(format!(
                "channel {} references an unknown cohort or target",
                channel.id
            )));
        }
        validate_per_mille(channel.reach_per_mille, "channel reach")?;
        validate_per_mille(channel.trust_per_mille, "channel trust")?;
        validate_per_mille(
            channel.interpretation_fidelity_per_mille,
            "channel interpretation fidelity",
        )?;
    }
    for transition in &definition.transitions {
        validate_identifier(&transition.id, "transition")?;
        if !target_ids.contains(&transition.target_id)
            || transition
                .affected_cohorts
                .iter()
                .any(|id| !cohort_ids.contains(id))
            || transition.from == transition.to
            || transition.base_rate_per_million > 1_000_000
        {
            return Err(invalid(format!(
                "transition {} references invalid state or rate",
                transition.id
            )));
        }
        let expanded_fan_out = if transition.affected_cohorts.is_empty() {
            cohort_ids.len()
        } else {
            transition.affected_cohorts.len()
        };
        if expanded_fan_out > budgets.max_fan_out {
            return Err(invalid(format!(
                "transition {} expands to {expanded_fan_out} cohorts, above fan-out budget {}",
                transition.id, budgets.max_fan_out
            )));
        }
        for weight in [
            transition.weights.influence,
            transition.weights.institutional_support,
            transition.weights.institutional_enforcement,
            transition.weights.policy_support,
            transition.weights.policy_coercion,
            transition.weights.policy_disruption,
        ] {
            if !(-1_000_000..=1_000_000).contains(&weight) {
                return Err(invalid(format!(
                    "transition {} contains an out-of-range weight",
                    transition.id
                )));
            }
        }
    }
    for target_id in &target_ids {
        let mut expanded_cohorts = BTreeSet::new();
        for transition in definition
            .transitions
            .iter()
            .filter(|transition| transition.target_id == *target_id)
        {
            if transition.affected_cohorts.is_empty() {
                expanded_cohorts.extend(cohort_ids.iter().cloned());
            } else {
                expanded_cohorts.extend(transition.affected_cohorts.iter().cloned());
            }
        }
        if expanded_cohorts.len() > budgets.max_fan_out {
            return Err(invalid(format!(
                "culture target {target_id} expands to {} transition cohorts, above fan-out budget {}",
                expanded_cohorts.len(),
                budgets.max_fan_out
            )));
        }
    }
    for effect in &definition.effects {
        validate_identifier(&effect.id, "effect")?;
        validate_identifier(&effect.signal_kind, "effect signal kind")?;
        if !target_ids.contains(&effect.target_id) || effect.cadence_boundaries == 0 {
            return Err(invalid(format!(
                "effect {} references an unknown target or zero cadence",
                effect.id
            )));
        }
        if effect.scope.len() > budgets.max_fan_out {
            return Err(invalid(format!(
                "effect {} exceeds fan-out budget",
                effect.id
            )));
        }
        for scope in &effect.scope {
            validate_text(scope, budgets.max_text_bytes, "effect scope")?;
        }
    }
    for institution in &definition.institutions {
        validate_identifier(&institution.id, "institution binding")?;
        if !target_ids.contains(&institution.target_id)
            || institution
                .affected_cohorts
                .iter()
                .any(|id| !cohort_ids.contains(id))
        {
            return Err(invalid(format!(
                "institution binding {} references an unknown target or cohort",
                institution.id
            )));
        }
        let expanded_fan_out = if institution.affected_cohorts.is_empty() {
            cohort_ids.len()
        } else {
            institution.affected_cohorts.len()
        };
        if expanded_fan_out > budgets.max_fan_out {
            return Err(invalid(format!(
                "institution binding {} expands to {expanded_fan_out} cohorts, above fan-out budget {}",
                institution.id, budgets.max_fan_out
            )));
        }
    }
    Ok(())
}

fn validate_authored_map(
    values: &BTreeMap<String, String>,
    budgets: CultureBudgets,
    label: &str,
) -> Result<(), CanwuError> {
    if values.len() > budgets.max_fan_out {
        return Err(invalid(format!(
            "{label} exceeds entry budget: {} > {}",
            values.len(),
            budgets.max_fan_out
        )));
    }
    for (key, value) in values {
        validate_text(key, budgets.max_text_bytes, label)?;
        validate_text(value, budgets.max_text_bytes, label)?;
    }
    Ok(())
}

fn validate_text(value: &str, max_bytes: usize, label: &str) -> Result<(), CanwuError> {
    if value.len() > max_bytes {
        return Err(invalid(format!(
            "{label} text exceeds byte budget: {} > {max_bytes}",
            value.len()
        )));
    }
    Ok(())
}

fn checked_state_add(total: usize, value: usize, label: &str) -> Result<usize, CanwuError> {
    total
        .checked_add(value)
        .ok_or_else(|| invalid(format!("culture {label} byte estimate overflowed")))
}

pub(crate) fn cause_ref_estimated_bytes(cause: &CauseRef) -> usize {
    match cause {
        CauseRef::System(system) => 64_usize.saturating_add(system.len()),
        CauseRef::Boundary(_) | CauseRef::Command(_) | CauseRef::Event(_) => 64,
    }
}

pub(crate) fn tombstone_estimated_bytes(
    tombstone: &RetiredTargetTombstone,
) -> Result<usize, CanwuError> {
    let mut total = 384_usize;
    for text in [
        tombstone.target_id.as_str(),
        tombstone.reason.as_str(),
        tombstone.policy_hash.as_str(),
    ] {
        total = checked_state_add(total, text.len(), "tombstone text")?;
    }
    if let Some(successor) = &tombstone.successor {
        total = checked_state_add(total, successor.len(), "tombstone successor")?;
    }
    for evidence in &tombstone.evidence {
        total = checked_state_add(
            total,
            cause_ref_estimated_bytes(evidence),
            "tombstone evidence",
        )?;
    }
    Ok(total)
}

fn unique_ids<'a>(
    values: impl IntoIterator<Item = &'a str>,
    label: &str,
) -> Result<BTreeSet<String>, CanwuError> {
    let mut ids = BTreeSet::new();
    for id in values {
        validate_identifier(id, label)?;
        if !ids.insert(id.to_owned()) {
            return Err(invalid(format!("duplicate {label} ID {id}")));
        }
    }
    Ok(ids)
}

fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_whitespace) {
        return Err(invalid(format!(
            "{label} identifier must be 1..=128 bytes and contain no whitespace"
        )));
    }
    Ok(())
}

fn validate_per_mille(value: u16, label: &str) -> Result<(), CanwuError> {
    if value > 1_000 {
        return Err(invalid(format!("{label} exceeds 1000 per mille")));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
