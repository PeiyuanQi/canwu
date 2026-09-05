use crate::{PLUGIN_NAME, PLUGIN_NAMESPACE};
use blake3::Hasher;
use canwu_api::{
    CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordType, DomainValueKindClass, EntityRef, PersonId, SimTime, TypedDomainRecordRef,
    canonical_hash,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_RECORDS: usize = 4_096;
pub const MAX_SUBUNITS: usize = 64;
pub const MAX_COMPOSITION_ENTRIES: usize = 64;
pub const MAX_OPERATION_PARTICIPANTS: usize = 128;

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(canwu_api::ErrorCode::InvalidDomainRecord, message.into())
}

fn validate_id(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.len() > 192
        || !value.contains(':')
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
    {
        return Err(invalid(format!("{label} is not a namespaced identifier")));
    }
    Ok(())
}

macro_rules! id_type {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, CanwuError> {
                let value = value.into();
                validate_id(&value, $label)?;
                Ok(Self(value))
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

id_type!(ForceId, "force");
id_type!(SubunitId, "subunit");
id_type!(OperationId, "operation");
id_type!(CombatId, "combat");
id_type!(OccupationId, "occupation");
id_type!(MilitaryNodeId, "military node");
id_type!(CommanderProfileId, "commander profile");
id_type!(RulesetId, "ruleset");
id_type!(MilitaryOperationKey, "military operation key");
id_type!(ProviderOutcomeId, "provider outcome");

macro_rules! record_type {
    ($name:ident, $payload:ty, $record_name:literal) => {
        pub struct $name;
        impl DomainRecordType for $name {
            type Payload = $payload;
            type Class = DomainValueKindClass;
            const NAMESPACE: &'static str = PLUGIN_NAMESPACE;
            const NAME: &'static str = $record_name;
        }
    };
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryRecordMeta {
    pub schema_version: u32,
    pub revision: u64,
    pub semantic_digest: String,
    pub established_at: SimTime,
}

impl MilitaryRecordMeta {
    pub fn new<T: Serialize>(
        revision: u64,
        established_at: SimTime,
        payload: &T,
    ) -> Result<Self, CanwuError> {
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            revision,
            semantic_digest: canonical_hash("canwu.military.payload.v1", payload)?,
            established_at,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryRulesetV1 {
    pub id: RulesetId,
    pub schema_version: u32,
    pub profile: String,
    pub semantic_hash: String,
    pub source_kind: String,
    pub source_note: String,
    pub branch_profiles: BTreeMap<String, BranchProfileV1>,
    pub tactics: BTreeMap<String, TacticProfileV1>,
    pub terrain_modifiers: BTreeMap<String, TerrainModifierV1>,
    pub recruitment: RecruitmentProfileV1,
    pub combat: CombatProfileV1,
    pub occupation: OccupationProfileV1,
}

impl MilitaryRulesetV1 {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != SCHEMA_VERSION
            || self.branch_profiles.is_empty()
            || self.tactics.is_empty()
            || self.terrain_modifiers.is_empty()
            || self.combat.max_rounds == 0
            || self.occupation.max_resistance_per_mille > 1_000
        {
            return Err(invalid("military ruleset is incomplete or invalid"));
        }
        validate_id(self.id.as_str(), "ruleset")?;
        let expected = canonical_hash(
            "canwu.military.ruleset.v1",
            &RulesetDigestView {
                id: &self.id,
                profile: &self.profile,
                branch_count: self.branch_profiles.len(),
                tactic_count: self.tactics.len(),
                terrain_count: self.terrain_modifiers.len(),
            },
        )?;
        if self.semantic_hash != expected {
            return Err(invalid("military ruleset semantic hash mismatch"));
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct RulesetDigestView<'a> {
    id: &'a RulesetId,
    profile: &'a str,
    branch_count: usize,
    tactic_count: usize,
    terrain_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BranchProfileV1 {
    pub branch: String,
    pub training_per_day: u16,
    pub equipment_per_mille: u16,
    pub supply_per_day: u64,
    pub movement_minutes: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TacticProfileV1 {
    pub id: String,
    pub attack_per_mille: i16,
    pub defense_per_mille: i16,
    pub concealment_per_mille: u16,
    pub withdrawal_threshold_per_mille: u16,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerrainModifierV1 {
    pub id: String,
    pub movement_per_mille: u16,
    pub attack_per_mille: i16,
    pub defense_per_mille: i16,
    pub concealment_per_mille: u16,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecruitmentProfileV1 {
    pub training_days: u16,
    pub minimum_age: u8,
    pub maximum_age: u8,
    pub replacement_delay_days: u16,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CombatProfileV1 {
    pub max_rounds: u16,
    pub casualty_per_mille: u16,
    pub prisoner_per_mille: u16,
    pub fatigue_per_round: u16,
    pub morale_break_per_mille: u16,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OccupationProfileV1 {
    pub garrison_per_node: u32,
    pub security_per_day: u16,
    pub integration_per_day: u16,
    pub max_resistance_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryCatalog {
    pub meta: MilitaryRecordMeta,
    pub ruleset: MilitaryRulesetV1,
    pub nodes: BTreeMap<MilitaryNodeId, MilitaryNodeProfile>,
    pub commanders: BTreeMap<PersonId, CommanderProfile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryNodeProfile {
    pub id: MilitaryNodeId,
    pub territory: String,
    pub terrain: String,
    pub administrative: bool,
    pub supply_capacity: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommanderProfile {
    pub id: PersonId,
    pub profile: CommanderProfileId,
    pub organization: u16,
    pub reconnaissance: u16,
    pub logistics: u16,
    pub tactics: u16,
    pub political: u16,
    pub obedience: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForceState {
    pub meta: MilitaryRecordMeta,
    pub id: ForceId,
    pub owner: EntityRef,
    pub formation_parent: Option<ForceId>,
    pub location: MilitaryNodeId,
    pub commander: Option<PersonId>,
    pub subunits: BTreeMap<SubunitId, SubunitState>,
    pub authorized_strength: u32,
    pub actual_strength: u32,
    pub training_per_mille: u16,
    pub equipment_per_mille: u16,
    pub fatigue_per_mille: u16,
    pub supply_per_mille: u16,
    pub morale_per_mille: u16,
    pub discipline_per_mille: u16,
    pub cohesion_per_mille: u16,
    pub loyalty_per_mille: u16,
    pub casualties: u32,
    pub missing: u32,
    pub prisoners: u32,
    pub deserters: u32,
    pub replacements_pending: u32,
    pub transport_capacity: u32,
    pub active_operation: Option<OperationId>,
    pub active_order: Option<MilitaryOperationKey>,
    pub status: ForceStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubunitState {
    pub id: SubunitId,
    pub branch: String,
    pub strength: u32,
    pub training_per_mille: u16,
    pub equipment_per_mille: u16,
    pub fatigue_per_mille: u16,
    pub status: SubunitStatus,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceStatus {
    #[default]
    Forming,
    Ready,
    Moving,
    Engaged,
    Routing,
    Garrison,
    Demobilized,
    Retired,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubunitStatus {
    #[default]
    Active,
    Wounded,
    Captured,
    Dispersed,
    Retired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationState {
    pub meta: MilitaryRecordMeta,
    pub id: OperationId,
    pub key: MilitaryOperationKey,
    pub owner: EntityRef,
    pub objective: String,
    pub forces: Vec<ForceId>,
    pub phase: OperationPhase,
    pub from: MilitaryNodeId,
    pub destination: MilitaryNodeId,
    pub route_digest: String,
    pub terrain: String,
    pub weather: String,
    pub started_at: SimTime,
    pub due_at: SimTime,
    pub command_delay_minutes: u64,
    pub supply_line: Option<MilitaryNodeId>,
    pub exit_condition: String,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationPhase {
    #[default]
    Planned,
    Moving,
    Contact,
    Engaged,
    Withdrawing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CombatState {
    pub meta: MilitaryRecordMeta,
    pub id: CombatId,
    pub operation: OperationId,
    pub location: MilitaryNodeId,
    pub attacker: ForceId,
    pub defender: ForceId,
    pub stage: CombatStage,
    pub round: u16,
    pub attacker_tactic: String,
    pub defender_tactic: String,
    pub attacker_preparation_per_mille: u16,
    pub defender_preparation_per_mille: u16,
    pub attacker_visible_strength: u32,
    pub defender_visible_strength: u32,
    pub attacker_casualties: u32,
    pub defender_casualties: u32,
    pub attacker_prisoners: u32,
    pub defender_prisoners: u32,
    pub result: Option<CombatResult>,
    pub random_envelopes: Vec<RandomEnvelope>,
    pub causal_notes: Vec<String>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatStage {
    #[default]
    Proposed,
    Contact,
    RoundResolved,
    Aftermath,
    Closed,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatResult {
    AttackerVictory,
    DefenderVictory,
    MutualDisengagement,
    AttackerRouted,
    DefenderRouted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RandomEnvelope {
    pub purpose: String,
    pub input_digest: String,
    pub ruleset_hash: String,
    pub boundary_id: String,
    pub draw_slot: u32,
    pub native_value: u64,
    pub upper_exclusive: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OccupationState {
    pub meta: MilitaryRecordMeta,
    pub id: OccupationId,
    pub node: MilitaryNodeId,
    pub occupying_force: ForceId,
    pub military_control_per_mille: u16,
    pub garrison_strength: u32,
    pub administrative_reach_per_mille: u16,
    pub security_per_mille: u16,
    pub fiscal_capacity_per_mille: u16,
    pub legitimacy_per_mille: u16,
    pub collaboration_per_mille: u16,
    pub resistance_per_mille: u16,
    pub extraction_burden_per_mille: u16,
    pub integration: IntegrationStage,
    pub policy_revision: u64,
    pub pending_provider_outcomes: BTreeSet<ProviderOutcomeId>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStage {
    #[default]
    Unadministered,
    MilitaryControl,
    AdministrativeTakeover,
    LegalRecognition,
    FiscalIntegration,
    SocialIntegration,
    CulturalPractice,
    Intergenerational,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryKnowledge {
    pub meta: MilitaryRecordMeta,
    pub holder: String,
    pub subject: String,
    pub source: String,
    pub observed_at: SimTime,
    pub acquired_at: SimTime,
    pub expires_at: Option<SimTime>,
    pub confidence_low: u32,
    pub confidence_high: u32,
    pub fact: KnowledgeFact,
    pub contradicted_by: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "fact", content = "value", rename_all = "snake_case")]
pub enum KnowledgeFact {
    ForceEstimate {
        location: String,
        strength_low: u32,
        strength_high: u32,
        supply_low: u16,
        supply_high: u16,
    },
    ContactEstimate {
        location: String,
        certainty_per_mille: u16,
    },
    OperationIntent {
        objective: String,
        destination: String,
    },
    OccupationReport {
        security_per_mille: u16,
        resistance_per_mille: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderOutcome {
    pub meta: MilitaryRecordMeta,
    pub id: ProviderOutcomeId,
    pub operation: MilitaryOperationKey,
    pub provider_plugin: String,
    pub provider_record: String,
    pub provider_version: u64,
    pub disposition: ProviderDisposition,
    pub quantity: u64,
    pub digest: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderDisposition {
    Accepted,
    Committed,
    Rejected,
    Compensating,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryLedger {
    pub meta: MilitaryRecordMeta,
    pub outcomes: BTreeMap<MilitaryOperationKey, MilitaryOutcome>,
    pub pending: BTreeMap<MilitaryOperationKey, PendingMilitaryEffect>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryOutcome {
    pub operation: MilitaryOperationKey,
    pub disposition: OutcomeDisposition,
    pub record: String,
    pub message: String,
    pub at: SimTime,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeDisposition {
    Accepted,
    Rejected,
    NoOp,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingMilitaryEffect {
    pub operation: MilitaryOperationKey,
    pub provider_plugin: String,
    pub kind: String,
    pub expected_source_version: u64,
    pub state: PendingEffectState,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingEffectState {
    Pending,
    Accepted,
    Committed,
    Rejected,
    Compensating,
}

record_type!(MilitaryCatalogRecord, MilitaryCatalog, "catalog");
record_type!(ForceStateRecord, ForceState, "force");
record_type!(OperationStateRecord, OperationState, "operation");
record_type!(CombatStateRecord, CombatState, "combat");
record_type!(OccupationStateRecord, OccupationState, "occupation");
record_type!(MilitaryKnowledgeRecord, MilitaryKnowledge, "knowledge");
record_type!(ProviderOutcomeRecord, ProviderOutcome, "provider_outcome");
record_type!(MilitaryLedgerRecord, MilitaryLedger, "ledger");

pub fn catalog_reference() -> TypedDomainRecordRef<MilitaryCatalogRecord> {
    TypedDomainRecordRef::new("root")
}
pub fn force_reference(id: &ForceId) -> TypedDomainRecordRef<ForceStateRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
pub fn operation_reference(id: &OperationId) -> TypedDomainRecordRef<OperationStateRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
pub fn combat_reference(id: &CombatId) -> TypedDomainRecordRef<CombatStateRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
pub fn occupation_reference(id: &OccupationId) -> TypedDomainRecordRef<OccupationStateRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
pub fn knowledge_reference(id: &str) -> TypedDomainRecordRef<MilitaryKnowledgeRecord> {
    TypedDomainRecordRef::new(id)
}
pub fn provider_outcome_reference(
    id: &ProviderOutcomeId,
) -> TypedDomainRecordRef<ProviderOutcomeRecord> {
    TypedDomainRecordRef::new(id.as_str())
}
pub fn ledger_reference() -> TypedDomainRecordRef<MilitaryLedgerRecord> {
    TypedDomainRecordRef::new("root")
}

pub fn record_from<T: DomainRecordType>(
    reference: TypedDomainRecordRef<T>,
    payload: &T::Payload,
    _at: SimTime,
) -> Result<DomainRecord, CanwuError>
where
    T::Payload: Serialize,
{
    let draft = DomainRecordDraft::from_typed(reference, payload)?;
    Ok(DomainRecord {
        reference: draft.reference,
        owner: PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    })
}

pub fn digest<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    canonical_hash("canwu.military.value.v1", value)
}

pub fn validate_per_mille(value: u16, label: &str) -> Result<(), CanwuError> {
    if value > 1_000 {
        Err(invalid(format!("{label} exceeds 1000 per mille")))
    } else {
        Ok(())
    }
}

impl ForceState {
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.meta.schema_version != SCHEMA_VERSION
            || self.subunits.len() > MAX_SUBUNITS
            || self.subunits.values().any(|unit| unit.strength == 0)
        {
            return Err(invalid("force state is invalid"));
        }
        for value in [
            self.training_per_mille,
            self.equipment_per_mille,
            self.fatigue_per_mille,
            self.supply_per_mille,
            self.morale_per_mille,
            self.discipline_per_mille,
            self.cohesion_per_mille,
            self.loyalty_per_mille,
        ] {
            validate_per_mille(value, "force metric")?;
        }
        if self.actual_strength > self.authorized_strength
            || self
                .casualties
                .saturating_add(self.missing)
                .saturating_add(self.prisoners)
                .saturating_add(self.deserters)
                > self.authorized_strength
        {
            return Err(invalid("force strength accounting is inconsistent"));
        }
        Ok(())
    }
}

impl OccupationState {
    pub fn validate(&self) -> Result<(), CanwuError> {
        for value in [
            self.military_control_per_mille,
            self.administrative_reach_per_mille,
            self.security_per_mille,
            self.fiscal_capacity_per_mille,
            self.legitimacy_per_mille,
            self.collaboration_per_mille,
            self.resistance_per_mille,
            self.extraction_burden_per_mille,
        ] {
            validate_per_mille(value, "occupation metric")?;
        }
        Ok(())
    }
}

pub fn input_digest<T: Serialize>(value: &T) -> Result<String, CanwuError> {
    let mut hasher = Hasher::new();
    hasher.update(&serde_json::to_vec(value).map_err(|e| invalid(e.to_string()))?);
    Ok(hasher.finalize().to_hex().to_string())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MilitaryCommand {
    CreateForce {
        operation: MilitaryOperationKey,
        force: ForceId,
        owner: EntityRef,
        location: MilitaryNodeId,
        authorized_strength: u32,
        branch: String,
        commander: Option<PersonId>,
    },
    AssignCommander {
        operation: MilitaryOperationKey,
        force: ForceId,
        commander: PersonId,
        expected_force_revision: u64,
    },
    Recruit {
        operation: MilitaryOperationKey,
        force: ForceId,
        subunit: SubunitId,
        branch: String,
        quantity: u32,
        expected_force_revision: u64,
        society_operation: Option<String>,
    },
    TrainAndEquip {
        operation: MilitaryOperationKey,
        force: ForceId,
        expected_force_revision: u64,
        training_delta: u16,
        equipment_delta: u16,
    },
    OrderMarch {
        operation: MilitaryOperationKey,
        force: ForceId,
        operation_id: OperationId,
        destination: MilitaryNodeId,
        objective: String,
        tactic: String,
        expected_force_revision: u64,
    },
    PlanOperation {
        operation: MilitaryOperationKey,
        operation_id: OperationId,
        owner: EntityRef,
        objective: String,
        force: ForceId,
        from: MilitaryNodeId,
        destination: MilitaryNodeId,
        tactic: String,
    },
    Recon {
        operation: MilitaryOperationKey,
        force: ForceId,
        target_force: Option<ForceId>,
        node: MilitaryNodeId,
        expected_force_revision: u64,
    },
    PrepareAmbush {
        operation: MilitaryOperationKey,
        force: ForceId,
        node: MilitaryNodeId,
        tactic: String,
        expected_force_revision: u64,
    },
    ExecuteSpecialOperation {
        operation: MilitaryOperationKey,
        operation_id: OperationId,
        force: ForceId,
        objective: String,
        target: MilitaryNodeId,
    },
    EstablishOccupation {
        operation: MilitaryOperationKey,
        occupation: OccupationId,
        force: ForceId,
        node: MilitaryNodeId,
        expected_force_revision: u64,
    },
    SetOccupationPolicy {
        operation: MilitaryOperationKey,
        occupation: OccupationId,
        policy_revision: u64,
        security_per_mille: u16,
        collaboration_per_mille: u16,
        extraction_burden_per_mille: u16,
    },
    MilitaryAdministrationAction {
        operation: MilitaryOperationKey,
        occupation: OccupationId,
        action: String,
        provider_plugin: String,
        expected_provider_version: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MilitaryCommandEnvelope {
    pub command: MilitaryCommand,
    pub input_digest: String,
}
