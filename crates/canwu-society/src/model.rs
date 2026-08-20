use crate::PLUGIN_NAME;
use canwu_api::{
    CanwuError, CoreEntityKind, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordLifecycle, DomainRecordType, DomainReference, DomainReferenceTarget,
    DomainValueKindClass, EntityRef, ErrorCode, PersonId, SimTime, TerritoryId,
    TypedDomainRecordRef,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SOCIETY_SCHEMA_VERSION: u32 = 2;
const ROOT_ID: &str = "root";

pub struct SocietyStateRecord;

impl DomainRecordType for SocietyStateRecord {
    type Payload = SocietyState;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "canwu.society";
    const NAME: &'static str = "state";
}

#[must_use]
pub fn society_state_reference() -> TypedDomainRecordRef<SocietyStateRecord> {
    TypedDomainRecordRef::new(ROOT_ID)
}

#[must_use]
pub fn distribution_id(cohort_id: &str, target_id: &str) -> String {
    format!("{cohort_id}::{target_id}")
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AwarenessBand {
    #[default]
    Unaware,
    Aware,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssentBand {
    Opposed,
    #[default]
    Neutral,
    Sympathetic,
    Committed,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PracticeBand {
    #[default]
    None,
    Occasional,
    Regular,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicAlignmentBand {
    Opposed,
    #[default]
    Unaligned,
    Conforming,
    Advocating,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationalTieBand {
    #[default]
    None,
    Peripheral,
    Member,
    Leader,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MobilizationBand {
    #[default]
    None,
    Latent,
    Active,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityBand {
    Hidden,
    #[default]
    Private,
    Public,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DispositionProfile {
    pub awareness: AwarenessBand,
    pub assent: AssentBand,
    pub practice: PracticeBand,
    pub public_alignment: PublicAlignmentBand,
    pub organizational_tie: OrganizationalTieBand,
    pub mobilization: MobilizationBand,
    pub visibility: VisibilityBand,
}

impl DispositionProfile {
    #[must_use]
    pub const fn neutral() -> Self {
        Self {
            awareness: AwarenessBand::Unaware,
            assent: AssentBand::Neutral,
            practice: PracticeBand::None,
            public_alignment: PublicAlignmentBand::Unaligned,
            organizational_tie: OrganizationalTieBand::None,
            mobilization: MobilizationBand::None,
            visibility: VisibilityBand::Private,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SocietyCohort {
    pub id: String,
    pub territory: TerritoryId,
    pub headcount: u64,
    #[serde(default)]
    pub classification: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AffiliationTarget {
    pub id: String,
    pub parent: Option<String>,
    pub neutral_profile: DispositionProfile,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DispositionBucket {
    pub profile: DispositionProfile,
    pub headcount: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DispositionDistribution {
    pub id: String,
    pub cohort_id: String,
    pub target_id: String,
    pub buckets: Vec<DispositionBucket>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum InfluenceSource {
    Cohort(String),
    Organization(String),
    Institution(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SocialInfluenceEdge {
    pub id: String,
    pub source: InfluenceSource,
    pub target_cohort_id: String,
    pub target_id: String,
    pub channel: String,
    pub reach_per_mille: u16,
    pub trust_per_mille: u16,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrganizationNode {
    pub id: String,
    pub target_id: String,
    pub base_reach_per_mille: u16,
    pub concealment_per_mille: u16,
    pub active: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrganizationRelation {
    pub id: String,
    pub source_organization_id: String,
    pub target_organization_id: String,
    pub relation: String,
    pub strength_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstitutionalAlignment {
    pub id: String,
    pub institution: EntityRef,
    pub target_id: String,
    #[serde(default)]
    pub affected_cohorts: BTreeSet<String>,
    pub support_per_mille: u16,
    pub enforcement_per_mille: u16,
    pub access_grant_per_mille: u16,
    pub authorized_actor: Option<PersonId>,
    pub last_decision_version: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyPressure {
    pub id: String,
    pub target_id: String,
    #[serde(default)]
    pub affected_cohorts: BTreeSet<String>,
    pub support_per_mille: u16,
    pub legal_access_per_mille: u16,
    pub surveillance_per_mille: u16,
    pub censorship_per_mille: u16,
    pub coercion_per_mille: u16,
    pub material_penalty_per_mille: u16,
    pub disruption_per_mille: u16,
    pub migration_pressure_per_mille: u16,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionWeights {
    pub influence: i32,
    pub institutional_support: i32,
    pub institutional_enforcement: i32,
    pub policy_support: i32,
    pub policy_coercion: i32,
    pub policy_disruption: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionRule {
    pub id: String,
    pub target_id: String,
    #[serde(default)]
    pub affected_cohorts: BTreeSet<String>,
    pub from: DispositionProfile,
    pub to: DispositionProfile,
    pub base_rate_per_million: u32,
    pub weights: TransitionWeights,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TransitionRemainder {
    pub id: String,
    pub rule_id: String,
    pub cohort_id: String,
    pub remainder: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObserverProfile {
    pub actor: PersonId,
    #[serde(default)]
    pub cohorts: BTreeSet<String>,
    #[serde(default)]
    pub targets: BTreeSet<String>,
    pub public_detection_per_mille: u16,
    pub private_detection_per_mille: u16,
    pub false_positive_per_mille: u16,
    pub confidence_per_mille: u16,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SocietyAggregate {
    pub cohort_id: String,
    pub target_id: String,
    pub headcount: u64,
    pub aware: u64,
    pub assenting: u64,
    pub practicing: u64,
    pub publicly_aligned: u64,
    pub organizationally_tied: u64,
    pub mobilized: u64,
    pub hidden: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MobilizationCandidate {
    pub id: String,
    pub cohort_id: String,
    pub target_id: String,
    pub mobilized_headcount: u64,
    pub organization_capacity_per_mille: u16,
    pub coercion_per_mille: u16,
    pub observed_at: SimTime,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectionEntry {
    pub cohort_id: String,
    pub target_id: String,
    pub estimated_publicly_aligned: u64,
    pub estimated_organizationally_tied: u64,
    pub estimated_mobilized: u64,
    pub confidence_per_mille: u16,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SocietyProjection {
    pub actor: PersonId,
    pub observed_at: SimTime,
    pub entries: BTreeMap<String, ProjectionEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyDecision {
    pub alignment_id: String,
    pub decision_version: u64,
    pub support_per_mille: u16,
    pub enforcement_per_mille: u16,
    pub access_grant_per_mille: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SocietyState {
    pub schema_version: u32,
    pub topology_passes: u8,
    pub cohorts: BTreeMap<String, SocietyCohort>,
    pub targets: BTreeMap<String, AffiliationTarget>,
    pub distributions: BTreeMap<String, DispositionDistribution>,
    pub influence_edges: BTreeMap<String, SocialInfluenceEdge>,
    pub organizations: BTreeMap<String, OrganizationNode>,
    pub organization_relations: BTreeMap<String, OrganizationRelation>,
    pub institutional_alignments: BTreeMap<String, InstitutionalAlignment>,
    pub policies: BTreeMap<String, PolicyPressure>,
    pub transition_rules: BTreeMap<String, TransitionRule>,
    pub remainders: BTreeMap<String, TransitionRemainder>,
    pub observer_profiles: BTreeMap<String, ObserverProfile>,
    pub aggregates: BTreeMap<String, SocietyAggregate>,
    pub mobilization_candidates: BTreeMap<String, MobilizationCandidate>,
    pub projections: BTreeMap<String, SocietyProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_mobilization_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_aggregation_at: Option<SimTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_projection_at: Option<SimTime>,
}

impl Default for SocietyState {
    fn default() -> Self {
        Self {
            schema_version: SOCIETY_SCHEMA_VERSION,
            topology_passes: 8,
            cohorts: BTreeMap::new(),
            targets: BTreeMap::new(),
            distributions: BTreeMap::new(),
            influence_edges: BTreeMap::new(),
            organizations: BTreeMap::new(),
            organization_relations: BTreeMap::new(),
            institutional_alignments: BTreeMap::new(),
            policies: BTreeMap::new(),
            transition_rules: BTreeMap::new(),
            remainders: BTreeMap::new(),
            observer_profiles: BTreeMap::new(),
            aggregates: BTreeMap::new(),
            mobilization_candidates: BTreeMap::new(),
            projections: BTreeMap::new(),
            last_transition_at: None,
            last_mobilization_at: None,
            last_aggregation_at: None,
            last_projection_at: None,
        }
    }
}

impl SocietyState {
    /// Merges duplicate disposition buckets and removes empty buckets.
    ///
    /// # Errors
    ///
    /// Returns an error when merging bucket headcounts would overflow `u64`.
    pub fn canonicalize(&mut self) -> Result<(), CanwuError> {
        for distribution in self.distributions.values_mut() {
            let mut combined = BTreeMap::<DispositionProfile, u64>::new();
            for bucket in &distribution.buckets {
                let entry = combined.entry(bucket.profile).or_default();
                *entry = entry.checked_add(bucket.headcount).ok_or_else(|| {
                    invalid("disposition bucket headcount overflowed while canonicalizing")
                })?;
            }
            distribution.buckets = combined
                .into_iter()
                .filter(|(_, headcount)| *headcount > 0)
                .map(|(profile, headcount)| DispositionBucket { profile, headcount })
                .collect();
        }
        Ok(())
    }

    /// Validates identifiers, references, bounds, and population conservation.
    ///
    /// # Errors
    ///
    /// Returns an `InvalidDomainRecord` error when any society invariant is
    /// violated.
    #[allow(clippy::too_many_lines)]
    pub fn validate(&self) -> Result<(), CanwuError> {
        if self.schema_version != SOCIETY_SCHEMA_VERSION {
            return Err(invalid(format!(
                "unsupported society schema version {}",
                self.schema_version
            )));
        }
        if !(1..=32).contains(&self.topology_passes) {
            return Err(invalid("topology_passes must be between 1 and 32"));
        }

        validate_keyed(&self.cohorts, |value| &value.id, "cohort")?;
        validate_keyed(&self.targets, |value| &value.id, "affiliation target")?;
        validate_compound_keyed(&self.distributions, |value| &value.id, "distribution")?;
        validate_keyed(&self.influence_edges, |value| &value.id, "influence edge")?;
        validate_keyed(&self.organizations, |value| &value.id, "organization")?;
        validate_keyed(
            &self.organization_relations,
            |value| &value.id,
            "organization relation",
        )?;
        validate_keyed(
            &self.institutional_alignments,
            |value| &value.id,
            "institutional alignment",
        )?;
        validate_keyed(&self.policies, |value| &value.id, "policy")?;
        validate_keyed(&self.transition_rules, |value| &value.id, "transition rule")?;
        validate_compound_keyed(&self.remainders, |value| &value.id, "transition remainder")?;

        for cohort in self.cohorts.values() {
            if cohort.headcount == 0 {
                return Err(invalid(format!("cohort {} has zero population", cohort.id)));
            }
        }
        for target in self.targets.values() {
            if let Some(parent) = &target.parent
                && (!self.targets.contains_key(parent) || parent == &target.id)
            {
                return Err(invalid(format!(
                    "target {} has an invalid parent {parent}",
                    target.id
                )));
            }
            let mut visited = BTreeSet::new();
            let mut current = Some(target.id.as_str());
            while let Some(id) = current {
                if !visited.insert(id) {
                    return Err(invalid(format!(
                        "target {} participates in an ancestry cycle",
                        target.id
                    )));
                }
                current = self
                    .targets
                    .get(id)
                    .and_then(|candidate| candidate.parent.as_deref());
            }
        }
        for distribution in self.distributions.values() {
            let expected_id = distribution_id(&distribution.cohort_id, &distribution.target_id);
            if distribution.id != expected_id {
                return Err(invalid(format!(
                    "distribution {} must use canonical identity {expected_id}",
                    distribution.id
                )));
            }
            let cohort = self.cohorts.get(&distribution.cohort_id).ok_or_else(|| {
                invalid(format!(
                    "distribution {} references unknown cohort {}",
                    distribution.id, distribution.cohort_id
                ))
            })?;
            if !self.targets.contains_key(&distribution.target_id) {
                return Err(invalid(format!(
                    "distribution {} references unknown target {}",
                    distribution.id, distribution.target_id
                )));
            }
            if distribution.buckets.is_empty() {
                return Err(invalid(format!(
                    "distribution {} contains no non-empty buckets",
                    distribution.id
                )));
            }
            if distribution
                .buckets
                .windows(2)
                .any(|pair| pair[0].profile >= pair[1].profile)
            {
                return Err(invalid(format!(
                    "distribution {} buckets are not canonical and unique",
                    distribution.id
                )));
            }
            let total = distribution
                .buckets
                .iter()
                .try_fold(0_u64, |total, bucket| {
                    if bucket.headcount == 0 {
                        return Err(invalid(format!(
                            "distribution {} contains an empty bucket",
                            distribution.id
                        )));
                    }
                    total.checked_add(bucket.headcount).ok_or_else(|| {
                        invalid(format!(
                            "distribution {} headcount overflowed",
                            distribution.id
                        ))
                    })
                })?;
            if total != cohort.headcount {
                return Err(invalid(format!(
                    "distribution {} totals {total}, expected cohort headcount {}",
                    distribution.id, cohort.headcount
                )));
            }
        }

        for edge in self.influence_edges.values() {
            validate_per_mille(edge.reach_per_mille, "edge reach")?;
            validate_per_mille(edge.trust_per_mille, "edge trust")?;
            require_cohort_target(self, &edge.target_cohort_id, &edge.target_id, &edge.id)?;
            match &edge.source {
                InfluenceSource::Cohort(id) if !self.cohorts.contains_key(id) => {
                    return Err(invalid(format!(
                        "edge {} has unknown source cohort {id}",
                        edge.id
                    )));
                }
                InfluenceSource::Organization(id) if !self.organizations.contains_key(id) => {
                    return Err(invalid(format!(
                        "edge {} has unknown source organization {id}",
                        edge.id
                    )));
                }
                InfluenceSource::Institution(id)
                    if !self.institutional_alignments.contains_key(id) =>
                {
                    return Err(invalid(format!(
                        "edge {} has unknown source institution {id}",
                        edge.id
                    )));
                }
                InfluenceSource::Cohort(_)
                | InfluenceSource::Organization(_)
                | InfluenceSource::Institution(_) => {}
            }
        }

        for organization in self.organizations.values() {
            if !self.targets.contains_key(&organization.target_id) {
                return Err(invalid(format!(
                    "organization {} references unknown target {}",
                    organization.id, organization.target_id
                )));
            }
            validate_per_mille(organization.base_reach_per_mille, "organization reach")?;
            validate_per_mille(
                organization.concealment_per_mille,
                "organization concealment",
            )?;
        }
        for relation in self.organization_relations.values() {
            if !self
                .organizations
                .contains_key(&relation.source_organization_id)
                || !self
                    .organizations
                    .contains_key(&relation.target_organization_id)
            {
                return Err(invalid(format!(
                    "organization relation {} has an unknown endpoint",
                    relation.id
                )));
            }
            validate_identifier(&relation.relation, "organization relation kind")?;
            validate_per_mille(
                relation.strength_per_mille,
                "organization relation strength",
            )?;
        }

        for alignment in self.institutional_alignments.values() {
            if !matches!(
                alignment.institution,
                EntityRef::Government(_) | EntityRef::Organization(_)
            ) {
                return Err(invalid(format!(
                    "alignment {} institution must be a government or organization",
                    alignment.id
                )));
            }
            validate_target_and_cohorts(
                self,
                &alignment.target_id,
                &alignment.affected_cohorts,
                &alignment.id,
            )?;
            validate_per_mille(alignment.support_per_mille, "institutional support")?;
            validate_per_mille(alignment.enforcement_per_mille, "institutional enforcement")?;
            validate_per_mille(alignment.access_grant_per_mille, "institutional access")?;
        }
        for policy in self.policies.values() {
            validate_target_and_cohorts(
                self,
                &policy.target_id,
                &policy.affected_cohorts,
                &policy.id,
            )?;
            for (value, name) in [
                (policy.support_per_mille, "policy support"),
                (policy.legal_access_per_mille, "policy access"),
                (policy.surveillance_per_mille, "policy surveillance"),
                (policy.censorship_per_mille, "policy censorship"),
                (policy.coercion_per_mille, "policy coercion"),
                (policy.material_penalty_per_mille, "policy material penalty"),
                (policy.disruption_per_mille, "policy disruption"),
                (
                    policy.migration_pressure_per_mille,
                    "policy migration pressure",
                ),
            ] {
                validate_per_mille(value, name)?;
            }
        }
        for rule in self.transition_rules.values() {
            validate_target_and_cohorts(self, &rule.target_id, &rule.affected_cohorts, &rule.id)?;
            if rule.from == rule.to {
                return Err(invalid(format!(
                    "rule {} must change at least one disposition dimension",
                    rule.id
                )));
            }
            if rule.base_rate_per_million > 1_000_000 {
                return Err(invalid(format!(
                    "rule {} base rate exceeds one million",
                    rule.id
                )));
            }
            for weight in [
                rule.weights.influence,
                rule.weights.institutional_support,
                rule.weights.institutional_enforcement,
                rule.weights.policy_support,
                rule.weights.policy_coercion,
                rule.weights.policy_disruption,
            ] {
                if !(-1_000_000..=1_000_000).contains(&weight) {
                    return Err(invalid(format!(
                        "rule {} contains an out-of-range transition weight",
                        rule.id
                    )));
                }
            }
        }
        for remainder in self.remainders.values() {
            if remainder.id != remainder_id(&remainder.rule_id, &remainder.cohort_id)
                || !self.transition_rules.contains_key(&remainder.rule_id)
                || !self.cohorts.contains_key(&remainder.cohort_id)
                || remainder.remainder >= 1_000_000
            {
                return Err(invalid(format!(
                    "transition remainder {} is malformed",
                    remainder.id
                )));
            }
        }
        for (key, observer) in &self.observer_profiles {
            if key != &observer.actor.get().to_string() {
                return Err(invalid(format!(
                    "observer profile key {key} does not match actor {}",
                    observer.actor
                )));
            }
            validate_cohort_set(self, &observer.cohorts, key)?;
            for target in &observer.targets {
                if !self.targets.contains_key(target) {
                    return Err(invalid(format!(
                        "observer {key} references unknown target {target}"
                    )));
                }
            }
            validate_per_mille(observer.public_detection_per_mille, "public detection")?;
            validate_per_mille(observer.private_detection_per_mille, "private detection")?;
            validate_per_mille(observer.false_positive_per_mille, "false positive rate")?;
            validate_per_mille(observer.confidence_per_mille, "observer confidence")?;
        }
        validate_derived_state(self)?;
        Ok(())
    }

    /// Encodes validated society state as the plugin-owned root domain record.
    ///
    /// # Errors
    ///
    /// Returns an error when the state is invalid or cannot be serialized.
    pub fn into_record(mut self) -> Result<DomainRecord, CanwuError> {
        self.canonicalize()?;
        self.validate()?;
        let draft = self.record_draft()?;
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

    pub(crate) fn record_draft(&self) -> Result<DomainRecordDraft, CanwuError> {
        let mut draft = DomainRecordDraft::from_typed(society_state_reference(), self)?;
        draft.references = self.core_references();
        Ok(draft)
    }

    pub(crate) fn validate_record_binding(&self, record: &DomainRecord) -> Result<(), CanwuError> {
        let expected = self.core_references();
        if record.references != expected {
            return Err(invalid(
                "society root references do not match the core entities named by its payload",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_at(&self, current_time: SimTime) -> Result<(), CanwuError> {
        if [
            self.last_transition_at,
            self.last_mobilization_at,
            self.last_aggregation_at,
            self.last_projection_at,
        ]
        .into_iter()
        .flatten()
        .any(|at| at > current_time)
        {
            return Err(invalid(
                "society materialization timestamps cannot be later than simulation time",
            ));
        }
        Ok(())
    }

    pub(crate) fn core_references(&self) -> Vec<DomainReference> {
        let mut references = BTreeSet::new();
        for cohort in self.cohorts.values() {
            references.insert(DomainReference {
                role: "territory".to_owned(),
                target: DomainReferenceTarget::Core(EntityRef::Territory(cohort.territory)),
            });
        }
        for alignment in self.institutional_alignments.values() {
            references.insert(DomainReference {
                role: "institution".to_owned(),
                target: DomainReferenceTarget::Core(alignment.institution.clone()),
            });
            if let Some(actor) = alignment.authorized_actor {
                references.insert(DomainReference {
                    role: "actor".to_owned(),
                    target: DomainReferenceTarget::Core(EntityRef::Person(actor)),
                });
            }
        }
        for observer in self.observer_profiles.values() {
            references.insert(DomainReference {
                role: "actor".to_owned(),
                target: DomainReferenceTarget::Core(EntityRef::Person(observer.actor)),
            });
        }
        references.into_iter().collect()
    }
}

#[must_use]
pub fn remainder_id(rule_id: &str, cohort_id: &str) -> String {
    format!("{rule_id}::{cohort_id}")
}

fn validate_keyed<T>(
    values: &BTreeMap<String, T>,
    id: impl Fn(&T) -> &String,
    label: &str,
) -> Result<(), CanwuError> {
    for (key, value) in values {
        validate_identifier(key, label)?;
        if key != id(value) {
            return Err(invalid(format!(
                "{label} map key {key} does not match its stored identity {}",
                id(value)
            )));
        }
    }
    Ok(())
}

fn validate_compound_keyed<T>(
    values: &BTreeMap<String, T>,
    id: impl Fn(&T) -> &String,
    label: &str,
) -> Result<(), CanwuError> {
    for (key, value) in values {
        let parts: Vec<_> = key.split("::").collect();
        if parts.len() != 2 {
            return Err(invalid(format!(
                "{label} identity {key:?} is not a two-part canonical identity"
            )));
        }
        for part in parts {
            validate_identifier(part, label)?;
        }
        if key != id(value) {
            return Err(invalid(format!(
                "{label} map key {key} does not match its stored identity {}",
                id(value)
            )));
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str) -> Result<(), CanwuError> {
    if value.is_empty()
        || value.contains("::")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(invalid(format!(
            "{label} identity {value:?} is not canonical"
        )));
    }
    Ok(())
}

fn validate_per_mille(value: u16, label: &str) -> Result<(), CanwuError> {
    if value > 1_000 {
        return Err(invalid(format!("{label} {value} exceeds 1000 permille")));
    }
    Ok(())
}

fn require_cohort_target(
    state: &SocietyState,
    cohort_id: &str,
    target_id: &str,
    owner: &str,
) -> Result<(), CanwuError> {
    if !state.cohorts.contains_key(cohort_id) || !state.targets.contains_key(target_id) {
        return Err(invalid(format!(
            "{owner} references unknown cohort {cohort_id} or target {target_id}"
        )));
    }
    Ok(())
}

fn validate_target_and_cohorts(
    state: &SocietyState,
    target_id: &str,
    cohorts: &BTreeSet<String>,
    owner: &str,
) -> Result<(), CanwuError> {
    if !state.targets.contains_key(target_id) {
        return Err(invalid(format!(
            "{owner} references unknown target {target_id}"
        )));
    }
    validate_cohort_set(state, cohorts, owner)
}

fn validate_cohort_set(
    state: &SocietyState,
    cohorts: &BTreeSet<String>,
    owner: &str,
) -> Result<(), CanwuError> {
    for cohort in cohorts {
        if !state.cohorts.contains_key(cohort) {
            return Err(invalid(format!(
                "{owner} references unknown cohort {cohort}"
            )));
        }
    }
    Ok(())
}

fn validate_derived_state(state: &SocietyState) -> Result<(), CanwuError> {
    for (key, aggregate) in &state.aggregates {
        validate_compound_identity(key, "aggregate")?;
        let expected = distribution_id(&aggregate.cohort_id, &aggregate.target_id);
        if key != &expected {
            return Err(invalid(format!(
                "aggregate key {key} must use canonical identity {expected}"
            )));
        }
        let cohort = state.cohorts.get(&aggregate.cohort_id).ok_or_else(|| {
            invalid(format!(
                "aggregate {key} references unknown cohort {}",
                aggregate.cohort_id
            ))
        })?;
        if !state.targets.contains_key(&aggregate.target_id)
            || !state.distributions.contains_key(key)
            || aggregate.headcount != cohort.headcount
            || [
                aggregate.aware,
                aggregate.assenting,
                aggregate.practicing,
                aggregate.publicly_aligned,
                aggregate.organizationally_tied,
                aggregate.mobilized,
                aggregate.hidden,
            ]
            .into_iter()
            .any(|value| value > aggregate.headcount)
        {
            return Err(invalid(format!("aggregate {key} is malformed")));
        }
    }
    for (key, candidate) in &state.mobilization_candidates {
        validate_compound_identity(key, "mobilization candidate")?;
        let expected = distribution_id(&candidate.cohort_id, &candidate.target_id);
        let cohort = state.cohorts.get(&candidate.cohort_id).ok_or_else(|| {
            invalid(format!(
                "mobilization candidate {key} references an unknown cohort"
            ))
        })?;
        if key != &candidate.id
            || key != &expected
            || !state.targets.contains_key(&candidate.target_id)
            || !state.distributions.contains_key(key)
            || candidate.mobilized_headcount > cohort.headcount
            || candidate.organization_capacity_per_mille > 1_000
            || candidate.coercion_per_mille > 1_000
            || state
                .last_mobilization_at
                .is_none_or(|at| candidate.observed_at > at)
        {
            return Err(invalid(format!(
                "mobilization candidate {key} is malformed"
            )));
        }
    }
    for (key, projection) in &state.projections {
        let observer = state
            .observer_profiles
            .get(key)
            .ok_or_else(|| invalid(format!("society projection {key} has no observer profile")))?;
        if key != &projection.actor.get().to_string()
            || state
                .last_projection_at
                .is_none_or(|at| projection.observed_at > at)
        {
            return Err(invalid(format!("society projection {key} is malformed")));
        }
        for (entry_key, entry) in &projection.entries {
            validate_compound_identity(entry_key, "projection entry")?;
            let expected = distribution_id(&entry.cohort_id, &entry.target_id);
            let cohort = state.cohorts.get(&entry.cohort_id).ok_or_else(|| {
                invalid(format!(
                    "projection entry {entry_key} references an unknown cohort"
                ))
            })?;
            if entry_key != &expected
                || !state.targets.contains_key(&entry.target_id)
                || !state.distributions.contains_key(entry_key)
                || (!observer.cohorts.is_empty() && !observer.cohorts.contains(&entry.cohort_id))
                || (!observer.targets.is_empty() && !observer.targets.contains(&entry.target_id))
                || entry.estimated_publicly_aligned > cohort.headcount
                || entry.estimated_organizationally_tied > cohort.headcount
                || entry.estimated_mobilized > cohort.headcount
                || entry.confidence_per_mille > 1_000
            {
                return Err(invalid(format!(
                    "projection entry {entry_key} is malformed"
                )));
            }
        }
    }
    validate_exact_derived_state(state)
}

fn validate_exact_derived_state(state: &SocietyState) -> Result<(), CanwuError> {
    let expected_aggregates = state
        .last_aggregation_at
        .map_or_else(BTreeMap::new, |_| crate::solver::compute_aggregates(state));
    if state.aggregates != expected_aggregates {
        return Err(invalid(
            "materialized society aggregates do not match authoritative distributions",
        ));
    }
    let expected_candidates = state.last_mobilization_at.map_or_else(BTreeMap::new, |at| {
        crate::solver::compute_mobilization_candidates(state, at)
    });
    if state.mobilization_candidates != expected_candidates {
        return Err(invalid(
            "materialized mobilization candidates do not match authoritative state",
        ));
    }
    let expected_projections = state.last_projection_at.map_or_else(BTreeMap::new, |at| {
        crate::solver::compute_projections(state, at)
    });
    if state.projections != expected_projections {
        return Err(invalid(
            "materialized society projections do not match authoritative state",
        ));
    }
    Ok(())
}

fn validate_compound_identity(value: &str, label: &str) -> Result<(), CanwuError> {
    let parts: Vec<_> = value.split("::").collect();
    if parts.len() != 2 {
        return Err(invalid(format!(
            "{label} identity {value:?} is not a two-part canonical identity"
        )));
    }
    for part in parts {
        validate_identifier(part, label)?;
    }
    Ok(())
}

pub(crate) fn core_reference_schemas() -> Vec<canwu_api::DomainReferenceSchema> {
    vec![
        canwu_api::DomainReferenceSchema {
            role: "actor".to_owned(),
            targets: vec![canwu_api::DomainReferenceTargetKind::Core(
                CoreEntityKind::Person,
            )],
            required: false,
            multiple: true,
            allow_retired: false,
        },
        canwu_api::DomainReferenceSchema {
            role: "institution".to_owned(),
            targets: vec![
                canwu_api::DomainReferenceTargetKind::Core(CoreEntityKind::Government),
                canwu_api::DomainReferenceTargetKind::Core(CoreEntityKind::Organization),
            ],
            required: false,
            multiple: true,
            allow_retired: false,
        },
        canwu_api::DomainReferenceSchema {
            role: "territory".to_owned(),
            targets: vec![canwu_api::DomainReferenceTargetKind::Core(
                CoreEntityKind::Territory,
            )],
            required: false,
            multiple: true,
            allow_retired: false,
        },
    ]
}

pub(crate) fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}
