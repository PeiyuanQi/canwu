//! Commands: the host's verbs, their admission, and their application in
//! phase 7 against the staged state.

use crate::crossing::{CrossingStateV1, CrossingStatus, InterceptorInputV1};
use crate::engagement::{
    ConditionInputV1, EngagementOutcomeKind, EngagementStatus, EnvironmentInputV1, Side,
};
use crate::force::{
    BattleStateV1, ComponentStateV1, ComponentValuesV1, ForceStateV1, ForceStatus, LocationV1,
    LossTallyV1, SideTermsV1, SiegeRoleV1,
};
use crate::ids::{
    ComponentId, CrossingId, EngagementId, ForceId, MAX_PARENT_ID_BYTES, ProfileId, RegistryRecord,
    RulebookRecord, SCHEMA_VERSION, SiegeId, crossing_reference, engagement_reference,
    force_reference, registry_reference, rulebook_reference, siege_reference,
};
use crate::plugin::{CombatRegistryV1, MILITARY_APPLY_INGRESS, MILITARY_STEP_INGRESS};
use crate::rulebook::{
    CombatRulebookV1, EngagementKind, MAX_COMPONENT_MEN, MAX_INPUT_BP, MAX_VALUE, bounded,
};
use crate::settle::{
    CloseInfo, OpenSpec, Run, close_engagement, end_siege, finish_crossing, open_engagement,
};
use crate::siege::{
    SiegeEndCause, SiegeStateV1, SiegeStatus, SiegeSupplyInputV1, SiteProfileV1, StormRule,
};
use canwu_api::{
    CanwuError, CommandContext, CommandIngress, EntityRef, ErrorCode, Issuer, KnowledgeHolderRef,
    SimDuration, SimTime, SimulationView, SystemDirective,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The target of a `SupplyInput`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum InputTargetV1 {
    Engagement(EngagementId),
    Siege(SiegeId),
    Crossing(CrossingId),
}

/// An order to one side of an engagement.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SideAction {
    /// Stands down a `Withdraw` order.
    Hold,
    /// The side withdraws after the next step.
    Withdraw,
    /// Closes the engagement `Abandoned` in the applying boundary; a
    /// defender's abandon is a `Withdraw`.
    Abandon,
}

/// A change to a siege's naval input or a crossing's interceptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "input", rename_all = "snake_case")]
pub enum InterceptorRevisionV1 {
    Set(InterceptorInputV1),
    Withdraw,
}

/// One component of a `ReviseForce` composition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ComponentRevisionV1 {
    pub present: u64,
    pub values: ComponentValuesV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OpenEngagementV1 {
    pub id: EngagementId,
    pub kind: EngagementKind,
    /// `None` takes the rulebook's default profile.
    pub profile: Option<ProfileId>,
    pub site: EntityRef,
    pub fort: Option<(u8, bool)>,
    pub attackers: Vec<ForceId>,
    pub defenders: Vec<ForceId>,
    pub holders: [Vec<KnowledgeHolderRef>; 2],
    pub inputs: [ConditionInputV1; 2],
    pub environment: EnvironmentInputV1,
    /// How long the fight may run before it closes `Stalled`: `None` takes
    /// the kind rule's `max_minutes`; a value must be from 1 to the kind
    /// rule's (a one-day fight of a kind whose rule runs several days).
    pub max_minutes: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InvestSiteV1 {
    pub id: SiegeId,
    pub site: EntityRef,
    pub profile: SiteProfileV1,
    pub besiegers: Vec<ForceId>,
    pub defenders: Vec<ForceId>,
    pub holders: [Vec<KnowledgeHolderRef>; 2],
    pub inputs: [ConditionInputV1; 2],
    pub environment: EnvironmentInputV1,
    pub work_bp: u32,
    pub supply: SiegeSupplyInputV1,
    pub naval: Option<InterceptorInputV1>,
    pub storm_on: StormRule,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviseSiegeV1 {
    pub siege: SiegeId,
    pub expected_host_revision: u64,
    pub supply: Option<SiegeSupplyInputV1>,
    pub profile: Option<SiteProfileV1>,
    pub work_bp: Option<u32>,
    pub naval: Option<InterceptorRevisionV1>,
    /// Opens a gate for this many steps of the default profile (the host's
    /// betrayal or incident rule).
    pub gate_open_steps: Option<u32>,
    /// Orders (`true`) or stands down (`false`) the storm.
    pub assault: Option<bool>,
    /// Orders one sortie once the defenders' cooldown has passed.
    pub sortie: bool,
    pub storm_on: Option<StormRule>,
    /// Releases these forces from the siege, from either side: each leaves
    /// its side's list and its siege role, free to be placed. A force in a
    /// fight is refused, and so is a release that would leave no besieger
    /// (`LiftSiege` ends a siege). A siege left with no defender at the site
    /// falls `Abandoned` at its next siege day.
    pub release: Vec<ForceId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrderCrossingV1 {
    pub id: CrossingId,
    pub force: ForceId,
    pub from: EntityRef,
    pub to: EntityRef,
    pub night: bool,
    pub defenders: Vec<ForceId>,
    pub holders: [Vec<KnowledgeHolderRef>; 2],
    pub inputs: [ConditionInputV1; 2],
    pub environment: EnvironmentInputV1,
    pub interceptor: Option<InterceptorInputV1>,
}

/// Every command of the engagement engine (action `military_command_v1`).
///
/// Every command that changes a record names that record's
/// `expected_host_revision`, which only host commands advance; a mismatch
/// is a recorded `DomainRecordVersionConflict` rejection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum MilitaryCommandV1 {
    /// From a `System` issuer; refused once a rulebook is installed.
    InstallRulebook {
        rulebook: Box<CombatRulebookV1>,
    },
    /// An active force with no battle state and host revision 0.
    RegisterForce {
        force: Box<ForceStateV1>,
    },
    ReviseForce {
        force: ForceId,
        expected_host_revision: u64,
        components: Option<BTreeMap<ComponentId, ComponentRevisionV1>>,
        terms: Option<SideTermsV1>,
        controllers: Option<Vec<Issuer>>,
        clear_morale_shock: bool,
    },
    PlaceForce {
        force: ForceId,
        expected_host_revision: u64,
        location: LocationV1,
    },
    RetireForce {
        force: ForceId,
        expected_host_revision: u64,
    },
    OpenEngagement(Box<OpenEngagementV1>),
    JoinEngagement {
        engagement: EngagementId,
        expected_host_revision: u64,
        side: Side,
        forces: Vec<ForceId>,
    },
    /// Adds free forces at the site to an open siege: the besiegers
    /// (`Attacker`) or the defenders (`Defender`). While the siege's assault
    /// or sortie is in progress the forces also enter it on their side (the
    /// besiegers are an assault's attackers and a sortie's defenders), as
    /// `JoinEngagement` would, and its host revision advances.
    JoinSiege {
        siege: SiegeId,
        expected_host_revision: u64,
        side: Side,
        forces: Vec<ForceId>,
    },
    SupplyInput {
        target: InputTargetV1,
        expected_host_revision: u64,
        side: Option<Side>,
        condition: Option<ConditionInputV1>,
        environment: Option<EnvironmentInputV1>,
    },
    OrderAction {
        engagement: EngagementId,
        expected_host_revision: u64,
        side: Side,
        action: SideAction,
    },
    ReviseStepProfile {
        engagement: EngagementId,
        expected_host_revision: u64,
        profile: ProfileId,
    },
    InvestSite(Box<InvestSiteV1>),
    ReviseSiege(Box<ReviseSiegeV1>),
    LiftSiege {
        siege: SiegeId,
        expected_host_revision: u64,
    },
    OrderCrossing(Box<OrderCrossingV1>),
    ReviseCrossing {
        crossing: CrossingId,
        expected_host_revision: u64,
        interceptor: Option<InterceptorRevisionV1>,
        night: Option<bool>,
        /// The forces that resist the next landings, checked as
        /// `OrderCrossing` checks its own: each at the far bank, not the
        /// crossing force, within the side limit. A fight in progress keeps
        /// its sides.
        defenders: Option<Vec<ForceId>>,
    },
    CancelCrossing {
        crossing: CrossingId,
        expected_host_revision: u64,
    },
    /// From a trusted issuer: steps the work due by `up_to` in the next
    /// boundary at once, after the apply packets admitted with it. A host
    /// that steps at a boundary it already settles queues a timed step
    /// instead ([`crate::timed_step_ingress`]), which adds no boundary.
    Step {
        up_to: SimTime,
    },
}

/// The payload of an apply packet.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct AppliedCommandV1 {
    pub issuer: Issuer,
    pub command: MilitaryCommandV1,
}

/// The payload of a step packet.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct StepPacketV1 {
    pub up_to: SimTime,
}

fn reject(code: ErrorCode, message: impl Into<String>) -> CanwuError {
    CanwuError::new(code, message.into())
}

fn payload(message: impl Into<String>) -> CanwuError {
    reject(ErrorCode::InvalidPayload, message)
}

fn authority(message: impl Into<String>) -> CanwuError {
    reject(ErrorCode::InvalidAuthority, message)
}

fn over(message: impl Into<String>) -> CanwuError {
    reject(ErrorCode::ValueOutOfRange, message)
}

fn missing(message: impl Into<String>) -> CanwuError {
    reject(ErrorCode::EntityNotFound, message)
}

fn stale(message: impl Into<String>) -> CanwuError {
    reject(ErrorCode::DomainRecordVersionConflict, message)
}

/// Every error admission returns uses a code the engine records as a
/// rejected command attempt.
fn admission_error(error: CanwuError) -> CanwuError {
    match error.code {
        ErrorCode::InvalidPayload
        | ErrorCode::InvalidAuthority
        | ErrorCode::ValueOutOfRange
        | ErrorCode::EntityNotFound
        | ErrorCode::MixedCommandIngress => error,
        _ => payload(error.message),
    }
}

/// Admits one command: static checks, holder acceptance, and the `Step`
/// and `InstallRulebook` authority; then queues its apply packet (or, for
/// `Step`, its step packet) for the next cut.
pub(crate) fn admit_command(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload_value: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    if context.ingress == CommandIngress::LegacyDirect {
        return Err(reject(
            ErrorCode::MixedCommandIngress,
            "military commands require canonical command ingress",
        ));
    }
    let command: MilitaryCommandV1 = serde_json::from_value(payload_value.clone())
        .map_err(|error| payload(format!("military command is invalid: {error}")))?;
    admit(view, context, command).map_err(admission_error)
}

fn admit(
    view: &SimulationView<'_>,
    context: &CommandContext,
    command: MilitaryCommandV1,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let rulebook = committed_rulebook(view)?;
    static_checks(&command)?;
    range_checks(&command)?;
    for holder in command_holders(&command) {
        if !view.knowledge_holder_accepts_publication(holder)? {
            return Err(payload(format!(
                "report holder {holder:?} would not be accepted"
            )));
        }
    }
    match &command {
        MilitaryCommandV1::InstallRulebook {
            rulebook: candidate,
        } => {
            if !matches!(context.issuer, Issuer::System(_)) {
                return Err(authority("a rulebook is installed by a System issuer"));
            }
            candidate
                .validate()
                .map_err(|error| payload(error.message))?;
        }
        MilitaryCommandV1::Step { up_to } => {
            let Some(rulebook) = rulebook else {
                return Err(payload("no combat rulebook is installed"));
            };
            if !rulebook.trusted_issuers.contains(&context.issuer) {
                return Err(authority("Step is a trusted issuer's command"));
            }
            if *up_to < context.simulation_time {
                return Err(payload("Step names a time before its boundary"));
            }
            return Ok(vec![SystemDirective::EnqueuePluginIngress {
                after: SimDuration::ZERO,
                packet_type: MILITARY_STEP_INGRESS.to_owned(),
                priority: 0,
                payload: serde_json::to_value(StepPacketV1 { up_to: *up_to })
                    .map_err(|error| payload(error.to_string()))?,
                affected: Vec::new(),
            }]);
        }
        _ => {}
    }
    let affected = host_entities(&command);
    Ok(vec![SystemDirective::EnqueuePluginIngress {
        after: SimDuration::ZERO,
        packet_type: MILITARY_APPLY_INGRESS.to_owned(),
        priority: 0,
        payload: serde_json::to_value(AppliedCommandV1 {
            issuer: context.issuer.clone(),
            command,
        })
        .map_err(|error| payload(error.to_string()))?,
        affected,
    }])
}

fn committed_rulebook(view: &SimulationView<'_>) -> Result<Option<CombatRulebookV1>, CanwuError> {
    let Some(registry) = view.typed_domain_record(&registry_reference())? else {
        return Ok(None);
    };
    let registry: CombatRegistryV1 = registry.decode_payload::<RegistryRecord>()?;
    let Some(id) = registry.rulebook else {
        return Ok(None);
    };
    view.typed_domain_record(&rulebook_reference(&id))?
        .map(canwu_api::DomainRecord::decode_payload::<RulebookRecord>)
        .transpose()
}

/// The host entities a command names; the engine refuses the command when
/// one does not exist.
fn host_entities(command: &MilitaryCommandV1) -> Vec<EntityRef> {
    let location = |location: &LocationV1| match location {
        LocationV1::At { site } => vec![site.clone()],
        LocationV1::OnRoute {
            route, from, to, ..
        } => vec![route.clone(), from.clone(), to.clone()],
    };
    let mut entities = match command {
        MilitaryCommandV1::RegisterForce { force } => {
            let mut entities = location(&force.location);
            entities.push(force.owner.clone());
            entities
        }
        MilitaryCommandV1::PlaceForce { location: at, .. } => location(at),
        MilitaryCommandV1::OpenEngagement(open) => vec![open.site.clone()],
        MilitaryCommandV1::InvestSite(invest) => vec![invest.site.clone()],
        MilitaryCommandV1::OrderCrossing(order) => vec![order.from.clone(), order.to.clone()],
        _ => Vec::new(),
    };
    entities.sort();
    entities.dedup();
    entities
}

fn command_holders(command: &MilitaryCommandV1) -> Vec<&KnowledgeHolderRef> {
    let holders = match command {
        MilitaryCommandV1::OpenEngagement(open) => &open.holders,
        MilitaryCommandV1::InvestSite(invest) => &invest.holders,
        MilitaryCommandV1::OrderCrossing(order) => &order.holders,
        _ => return Vec::new(),
    };
    holders.iter().flatten().collect()
}

fn disjoint_sides(left: &[ForceId], right: &[ForceId]) -> Result<(), CanwuError> {
    let mut seen = BTreeSet::new();
    for force in left.iter().chain(right) {
        if !seen.insert(force) {
            return Err(payload(format!("force {force} is named twice")));
        }
    }
    Ok(())
}

fn distinct_holders(holders: &[Vec<KnowledgeHolderRef>; 2]) -> Result<(), CanwuError> {
    for (index, side) in holders.iter().enumerate() {
        let unique: BTreeSet<_> = side.iter().collect();
        if unique.len() != side.len() {
            return Err(payload("a report holder is named twice on one side"));
        }
        if index == 0 && side.iter().any(|holder| holders[1].contains(holder)) {
            return Err(payload("a report holder is named on both sides"));
        }
    }
    Ok(())
}

/// Every host integer the arithmetic reads is bounded at admission
/// (`ValueOutOfRange`, a recorded rejected attempt), and again where phase 7
/// applies it.
fn range_checks(command: &MilitaryCommandV1) -> Result<(), CanwuError> {
    let inputs = |inputs: &[ConditionInputV1; 2], environment: &EnvironmentInputV1| {
        inputs
            .iter()
            .try_for_each(ConditionInputV1::check_ranges)
            .and_then(|()| environment.check_ranges())
    };
    match command {
        MilitaryCommandV1::RegisterForce { force } => force.validate(),
        MilitaryCommandV1::ReviseForce {
            components, terms, ..
        } => {
            for revision in components.iter().flat_map(BTreeMap::values) {
                bounded(revision.present, MAX_COMPONENT_MEN, "present")?;
                revision.values.check_ranges()?;
            }
            if let Some(terms) = terms {
                for value in [
                    terms.morale_sensitivity,
                    terms.battle_command,
                    terms.assessment,
                    terms.credibility,
                ] {
                    bounded(value, MAX_VALUE, "a side term")?;
                }
            }
            Ok(())
        }
        MilitaryCommandV1::OpenEngagement(open) => inputs(&open.inputs, &open.environment),
        MilitaryCommandV1::SupplyInput {
            condition,
            environment,
            ..
        } => {
            if let Some(condition) = condition {
                condition.check_ranges()?;
            }
            environment
                .as_ref()
                .map_or(Ok(()), EnvironmentInputV1::check_ranges)
        }
        MilitaryCommandV1::InvestSite(invest) => {
            inputs(&invest.inputs, &invest.environment)?;
            invest.profile.check_ranges()?;
            bounded(invest.work_bp, MAX_INPUT_BP, "work_bp")?;
            invest
                .naval
                .as_ref()
                .map_or(Ok(()), InterceptorInputV1::check_ranges)
        }
        MilitaryCommandV1::ReviseSiege(revise) => {
            if let Some(profile) = &revise.profile {
                profile.check_ranges()?;
            }
            if let Some(work_bp) = revise.work_bp {
                bounded(work_bp, MAX_INPUT_BP, "work_bp")?;
            }
            if let Some(steps) = revise.gate_open_steps {
                bounded(steps, 1_000_000, "gate_open_steps")?;
            }
            match &revise.naval {
                Some(InterceptorRevisionV1::Set(input)) => input.check_ranges(),
                _ => Ok(()),
            }
        }
        MilitaryCommandV1::OrderCrossing(order) => {
            inputs(&order.inputs, &order.environment)?;
            order
                .interceptor
                .as_ref()
                .map_or(Ok(()), InterceptorInputV1::check_ranges)
        }
        MilitaryCommandV1::ReviseCrossing {
            interceptor: Some(InterceptorRevisionV1::Set(input)),
            ..
        } => input.check_ranges(),
        _ => Ok(()),
    }
}

/// The checks that need no state.
fn static_checks(command: &MilitaryCommandV1) -> Result<(), CanwuError> {
    match command {
        MilitaryCommandV1::OpenEngagement(open) => {
            if open.attackers.is_empty() || open.defenders.is_empty() {
                return Err(payload("an engagement needs forces on both sides"));
            }
            disjoint_sides(&open.attackers, &open.defenders)?;
            distinct_holders(&open.holders)?;
        }
        MilitaryCommandV1::JoinEngagement { forces, .. }
        | MilitaryCommandV1::JoinSiege { forces, .. } => {
            if forces.is_empty() {
                return Err(payload("a join names no forces"));
            }
            disjoint_sides(forces, &[])?;
        }
        MilitaryCommandV1::ReviseSiege(revise) => disjoint_sides(&revise.release, &[])?,
        MilitaryCommandV1::ReviseCrossing {
            defenders: Some(defenders),
            ..
        } => disjoint_sides(defenders, &[])?,
        MilitaryCommandV1::InvestSite(invest) => {
            if invest.besiegers.is_empty() {
                return Err(payload("a siege needs besiegers"));
            }
            if invest.id.as_str().len() > MAX_PARENT_ID_BYTES {
                return Err(payload("a siege id is too long for the fights it opens"));
            }
            disjoint_sides(&invest.besiegers, &invest.defenders)?;
            distinct_holders(&invest.holders)?;
        }
        MilitaryCommandV1::OrderCrossing(order) => {
            if order.id.as_str().len() > MAX_PARENT_ID_BYTES {
                return Err(payload("a crossing id is too long for the fights it opens"));
            }
            disjoint_sides(std::slice::from_ref(&order.force), &order.defenders)?;
            distinct_holders(&order.holders)?;
        }
        MilitaryCommandV1::SupplyInput {
            condition,
            environment,
            side,
            ..
        } => {
            if condition.is_none() && environment.is_none() {
                return Err(payload("a SupplyInput carries no input"));
            }
            if condition.is_some() && side.is_none() {
                return Err(payload("a condition input names no side"));
            }
        }
        _ => {}
    }
    Ok(())
}

// ------------------------------------------------------------- application

fn require_rulebook(run: &mut Run<'_, '_>) -> Result<std::rc::Rc<CombatRulebookV1>, CanwuError> {
    run.rulebook()?
        .ok_or_else(|| payload("no combat rulebook is installed"))
}

fn trusted(rulebook: &CombatRulebookV1, issuer: &Issuer) -> bool {
    rulebook.trusted_issuers.contains(issuer)
}

fn controls(rulebook: &CombatRulebookV1, issuer: &Issuer, force: &ForceStateV1) -> bool {
    trusted(rulebook, issuer) || force.controllers.contains(issuer)
}

fn check_revision(actual: u64, expected: u64, label: &str) -> Result<(), CanwuError> {
    if actual == expected {
        Ok(())
    } else {
        Err(stale(format!(
            "{label} is at host revision {actual}, the command names {expected}"
        )))
    }
}

fn load_force(run: &Run<'_, '_>, id: &ForceId) -> Result<ForceStateV1, CanwuError> {
    let force = run
        .get(&force_reference(id))?
        .ok_or_else(|| missing(format!("force {id} is not registered")))?;
    if force.status != ForceStatus::Active {
        return Err(missing(format!("force {id} is retired")));
    }
    Ok(force)
}

pub(crate) fn check_condition(
    rulebook: &CombatRulebookV1,
    input: &ConditionInputV1,
) -> Result<(), CanwuError> {
    input.check_ranges()?;
    if input.channels.len() != rulebook.condition_channels.len() {
        return Err(payload(format!(
            "a condition input gives {} channels, the rulebook declares {}",
            input.channels.len(),
            rulebook.condition_channels.len()
        )));
    }
    if !rulebook.knows_posture(&input.posture) {
        return Err(payload(format!("posture {} is unknown", input.posture)));
    }
    if input.evidence.len() > rulebook.limits.evidence_refs_per_input as usize {
        return Err(over("a condition input cites too much evidence"));
    }
    Ok(())
}

pub(crate) fn check_environment(
    rulebook: &CombatRulebookV1,
    input: &EnvironmentInputV1,
) -> Result<(), CanwuError> {
    input.check_ranges()?;
    if input
        .terrain
        .as_ref()
        .is_some_and(|terrain| !rulebook.terrain.contains_key(terrain))
    {
        return Err(payload("the environment names an unknown terrain"));
    }
    if input.evidence.len() > rulebook.limits.evidence_refs_per_input as usize {
        return Err(over("an environment input cites too much evidence"));
    }
    Ok(())
}

fn check_side_size(rulebook: &CombatRulebookV1, forces: &[ForceId]) -> Result<(), CanwuError> {
    if forces.len() > rulebook.limits.forces_per_side as usize {
        return Err(over("more forces on a side than the limit"));
    }
    Ok(())
}

fn check_holders(
    rulebook: &CombatRulebookV1,
    holders: &[Vec<KnowledgeHolderRef>; 2],
) -> Result<(), CanwuError> {
    if holders
        .iter()
        .any(|side| side.len() > rulebook.limits.holders_per_side as usize)
    {
        return Err(over("more report holders on a side than the limit"));
    }
    Ok(())
}

fn single_owner(forces: &[ForceStateV1]) -> Result<(), CanwuError> {
    if forces.windows(2).any(|pair| pair[0].owner != pair[1].owner) {
        return Err(payload("a side has forces of more than one owner"));
    }
    Ok(())
}

/// A crossing's defenders, as an order or a revision names them: within the
/// side limit, not the crossing force, each registered and at the far bank.
fn check_defenders(
    run: &Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    crossing_force: &ForceId,
    to: &EntityRef,
    defenders: &[ForceId],
) -> Result<(), CanwuError> {
    check_side_size(rulebook, defenders)?;
    if defenders.contains(crossing_force) {
        return Err(payload("a force cannot defend against its own crossing"));
    }
    for id in defenders {
        if !load_force(run, id)?.location.is_at(to) {
            return Err(payload(format!("defender {id} is not at the far bank")));
        }
    }
    Ok(())
}

fn validate_interceptor(
    run: &Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    interceptor: &InterceptorInputV1,
) -> Result<(), CanwuError> {
    interceptor.check_ranges()?;
    load_force(run, &interceptor.force)?;
    if interceptor.evidence.len() > rulebook.limits.evidence_refs_per_input as usize {
        return Err(over("an interceptor input cites too much evidence"));
    }
    Ok(())
}

/// Applies one admitted command against the staged state. Every refusal is
/// an error the caller records as a rejection after restoring the staging.
pub(crate) fn apply(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    command: &MilitaryCommandV1,
) -> Result<(), CanwuError> {
    match command {
        MilitaryCommandV1::InstallRulebook { rulebook } => install_rulebook(run, rulebook),
        MilitaryCommandV1::RegisterForce { force } => register_force(run, issuer, force),
        MilitaryCommandV1::ReviseForce {
            force,
            expected_host_revision,
            components,
            terms,
            controllers,
            clear_morale_shock,
        } => {
            let rulebook = require_rulebook(run)?;
            let mut state = load_force(run, force)?;
            check_revision(state.host_revision, *expected_host_revision, "the force")?;
            if !controls(&rulebook, issuer, &state) {
                return Err(authority("the issuer does not control the force"));
            }
            if let Some(components) = components {
                let mut next = BTreeMap::new();
                for (id, revision) in components {
                    let losses = state
                        .components
                        .get(id)
                        .map_or_else(LossTallyV1::default, |component| component.losses);
                    next.insert(
                        id.clone(),
                        ComponentStateV1 {
                            present: revision.present,
                            values: revision.values.clone(),
                            losses,
                        },
                    );
                }
                // A component left out keeps its losses with no men present.
                for (id, component) in &state.components {
                    if !next.contains_key(id) && !component.losses.is_zero() {
                        next.insert(
                            id.clone(),
                            ComponentStateV1 {
                                present: 0,
                                values: component.values.clone(),
                                losses: component.losses,
                            },
                        );
                    }
                }
                state.components = next;
            }
            if let Some(terms) = terms {
                state.terms = terms.clone();
            }
            if let Some(controllers) = controllers {
                state.controllers.clone_from(controllers);
            }
            if *clear_morale_shock {
                state.battle.morale_shock = 0;
            }
            if let Some(cohesion) = state.battle.cohesion {
                state.battle.cohesion = Some(cohesion.min(state.terms.cohesion_cap));
            }
            state.host_revision += 1;
            state.validate()?;
            state.validate_limits(&rulebook)?;
            run.put(&force_reference(force), &state)
        }
        MilitaryCommandV1::PlaceForce {
            force,
            expected_host_revision,
            location,
        } => {
            let rulebook = require_rulebook(run)?;
            let mut state = load_force(run, force)?;
            check_revision(state.host_revision, *expected_host_revision, "the force")?;
            if !controls(&rulebook, issuer, &state) {
                return Err(authority("the issuer does not control the force"));
            }
            if !state.is_free() {
                return Err(payload(
                    "a force engaged, in a siege, or crossing cannot be placed",
                ));
            }
            state.location = location.clone();
            state.host_revision += 1;
            state.validate()?;
            run.put(&force_reference(force), &state)
        }
        MilitaryCommandV1::RetireForce {
            force,
            expected_host_revision,
        } => {
            let rulebook = require_rulebook(run)?;
            let mut state = load_force(run, force)?;
            check_revision(state.host_revision, *expected_host_revision, "the force")?;
            if !controls(&rulebook, issuer, &state) {
                return Err(authority("the issuer does not control the force"));
            }
            if !state.is_free() {
                return Err(payload(
                    "a force engaged, in a siege, or crossing cannot be retired",
                ));
            }
            state.status = ForceStatus::Retired;
            state.host_revision += 1;
            run.put(&force_reference(force), &state)?;
            let at = run.at;
            let registry = run.registry_mut()?;
            registry.force_count = registry.force_count.saturating_sub(1);
            crate::settle::queue_retire(registry, at, force_reference(force).into_untyped());
            Ok(())
        }
        MilitaryCommandV1::OpenEngagement(open) => open_command(run, issuer, open),
        MilitaryCommandV1::JoinEngagement {
            engagement,
            expected_host_revision,
            side,
            forces,
        } => join_engagement(
            run,
            issuer,
            engagement,
            *expected_host_revision,
            *side,
            forces,
        ),
        MilitaryCommandV1::JoinSiege {
            siege,
            expected_host_revision,
            side,
            forces,
        } => join_siege(run, issuer, siege, *expected_host_revision, *side, forces),
        MilitaryCommandV1::SupplyInput {
            target,
            expected_host_revision,
            side,
            condition,
            environment,
        } => supply_input(
            run,
            issuer,
            target,
            *expected_host_revision,
            *side,
            condition.as_ref(),
            environment.as_ref(),
        ),
        MilitaryCommandV1::OrderAction {
            engagement,
            expected_host_revision,
            side,
            action,
        } => order_action(
            run,
            issuer,
            engagement,
            *expected_host_revision,
            *side,
            *action,
        ),
        MilitaryCommandV1::ReviseStepProfile {
            engagement,
            expected_host_revision,
            profile,
        } => {
            let rulebook = require_rulebook(run)?;
            let reference = engagement_reference(engagement);
            let mut state = run
                .get(&reference)?
                .ok_or_else(|| missing(format!("engagement {engagement} does not exist")))?;
            if state.is_closed() {
                return Err(payload("the engagement is closed"));
            }
            check_revision(
                state.host_revision,
                *expected_host_revision,
                "the engagement",
            )?;
            if !trusted(&rulebook, issuer) && !controls_any(run, &rulebook, issuer, state.forces())?
            {
                return Err(authority("the issuer controls no force of the engagement"));
            }
            let step = rulebook
                .step_profiles
                .get(profile)
                .ok_or_else(|| payload(format!("step profile {profile} is unknown")))?;
            state.profile = profile.clone();
            if matches!(state.status, EngagementStatus::Held { .. }) {
                resume(&mut state, run.at, step.step_minutes);
            } else {
                state.next_step_at = add_minutes(state.last_step_at, step.step_minutes)?;
            }
            state.host_revision += 1;
            run.put(&reference, &state)
        }
        MilitaryCommandV1::InvestSite(invest) => invest_site(run, issuer, invest),
        MilitaryCommandV1::ReviseSiege(revise) => revise_siege(run, issuer, revise),
        MilitaryCommandV1::LiftSiege {
            siege,
            expected_host_revision,
        } => {
            let rulebook = require_rulebook(run)?;
            let mut state = open_siege(run, siege)?;
            check_revision(state.host_revision, *expected_host_revision, "the siege")?;
            if !trusted(&rulebook, issuer)
                && !controls_any(run, &rulebook, issuer, state.besiegers.iter())?
            {
                return Err(authority("the issuer controls no besieger"));
            }
            if state.engagement.is_some() {
                return Err(payload("the siege has an assault or sortie in progress"));
            }
            state.host_revision += 1;
            let at = run.at;
            end_siege(
                run,
                &rulebook,
                &mut state,
                SiegeStatus::Lifted,
                SiegeEndCause::Withdrawn,
                at,
            )
        }
        MilitaryCommandV1::OrderCrossing(order) => order_crossing(run, issuer, order),
        MilitaryCommandV1::ReviseCrossing {
            crossing,
            expected_host_revision,
            interceptor,
            night,
            defenders,
        } => {
            let rulebook = require_rulebook(run)?;
            let mut state = open_crossing(run, crossing)?;
            check_revision(state.host_revision, *expected_host_revision, "the crossing")?;
            if let Some(defenders) = defenders {
                check_defenders(run, &rulebook, &state.force, &state.to, defenders)?;
                // The order named the defenders, so the crossing force's
                // controller may name them again; a defender's controller
                // speaks for its own bank (as for an interceptor) and drops
                // only the forces it controls.
                let ordered = trusted(&rulebook, issuer)
                    || controls(&rulebook, issuer, &load_force(run, &state.force)?);
                if !ordered {
                    if !controls_any(
                        run,
                        &rulebook,
                        issuer,
                        state.defenders.iter().chain(defenders),
                    )? {
                        return Err(authority(
                            "the issuer controls neither the crossing force nor a defender",
                        ));
                    }
                    // A force no longer registered may be dropped by anyone
                    // who speaks for the bank.
                    for removed in state.defenders.iter().filter(|id| !defenders.contains(id)) {
                        if let Some(force) = run.get(&force_reference(removed))?
                            && !controls(&rulebook, issuer, &force)
                        {
                            return Err(authority(format!(
                                "the issuer does not control {removed}, which it would remove"
                            )));
                        }
                    }
                }
                state.defenders.clone_from(defenders);
            }
            if let Some(revision) = interceptor {
                let mut controllers: Vec<&ForceId> =
                    state.interceptor.iter().map(|input| &input.force).collect();
                if let InterceptorRevisionV1::Set(input) = revision {
                    validate_interceptor(run, &rulebook, input)?;
                    controllers.push(&input.force);
                }
                if !trusted(&rulebook, issuer)
                    && !controls_any(run, &rulebook, issuer, controllers.into_iter())?
                {
                    return Err(authority("the issuer controls no interceptor"));
                }
                state.interceptor = match revision {
                    InterceptorRevisionV1::Set(input) => Some(input.clone()),
                    InterceptorRevisionV1::Withdraw => None,
                };
            }
            if let Some(night) = night {
                if !controls(&rulebook, issuer, &load_force(run, &state.force)?) {
                    return Err(authority("the issuer does not control the crossing force"));
                }
                state.night = *night;
            }
            state.host_revision += 1;
            run.put(&crossing_reference(crossing), &state)
        }
        MilitaryCommandV1::CancelCrossing {
            crossing,
            expected_host_revision,
        } => {
            let rulebook = require_rulebook(run)?;
            let mut state = open_crossing(run, crossing)?;
            check_revision(state.host_revision, *expected_host_revision, "the crossing")?;
            if !controls(&rulebook, issuer, &load_force(run, &state.force)?) {
                return Err(authority("the issuer does not control the crossing force"));
            }
            if state.engagement.is_some() {
                return Err(payload("the crossing has a fight in progress"));
            }
            state.host_revision += 1;
            let at = run.at;
            finish_crossing(run, &rulebook, &mut state, CrossingStatus::Cancelled, at)
        }
        MilitaryCommandV1::Step { .. } => Err(payload("Step is never applied as a command")),
    }
}

fn controls_any<'f>(
    run: &Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    issuer: &Issuer,
    forces: impl Iterator<Item = &'f ForceId>,
) -> Result<bool, CanwuError> {
    for force in forces {
        if let Some(state) = run.get(&force_reference(force))?
            && controls(rulebook, issuer, &state)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn add_minutes(at: SimTime, minutes: u32) -> Result<SimTime, CanwuError> {
    at.checked_add(SimDuration::minutes(i64::from(minutes)))
        .ok_or_else(|| reject(ErrorCode::InvalidDuration, "military time overflow"))
}

/// Puts a held engagement back to stepping from now.
fn resume(state: &mut crate::EngagementStateV1, at: SimTime, step_minutes: u32) {
    state.status = EngagementStatus::Open;
    state.last_step_at = at;
    state.next_step_at = at
        .checked_add(SimDuration::minutes(i64::from(step_minutes)))
        .unwrap_or(at);
}

fn install_rulebook(run: &mut Run<'_, '_>, rulebook: &CombatRulebookV1) -> Result<(), CanwuError> {
    if run
        .registry()
        .is_some_and(|registry| registry.rulebook.is_some())
    {
        return Err(payload("a combat rulebook is installed already"));
    }
    rulebook
        .validate()
        .map_err(|error| payload(error.message))?;
    if !run.resolver.matches(&rulebook.resolver) {
        return Err(payload(format!(
            "the rulebook names resolver {}@{}, the plugin runs {}@{}",
            rulebook.resolver.name,
            rulebook.resolver.version,
            run.resolver.name,
            run.resolver.version
        )));
    }
    run.create(&rulebook_reference(&rulebook.id), rulebook)?;
    match run.registry_slot() {
        Some(registry) => registry.rulebook = Some(rulebook.id.clone()),
        slot @ None => *slot = Some(CombatRegistryV1::new(Some(rulebook.id.clone()))),
    }
    run.forget_rulebook();
    Ok(())
}

fn register_force(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    force: &ForceStateV1,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    if force.status != ForceStatus::Active
        || force.battle != BattleStateV1::default()
        || force.host_revision != 0
        || force.schema_version != SCHEMA_VERSION
    {
        return Err(payload(
            "a registered force is active, at host revision 0, with no battle state",
        ));
    }
    force.validate()?;
    force.validate_limits(&rulebook)?;
    if !controls(&rulebook, issuer, force) {
        return Err(authority("the issuer is neither trusted nor a controller"));
    }
    if run.exists_any(force_reference(&force.id).as_untyped())? {
        return Err(payload(format!("force {} exists already", force.id)));
    }
    let limit = rulebook.limits.forces;
    let registry = run.registry_mut()?;
    if registry.force_count >= limit {
        return Err(over("the rulebook's force limit is reached"));
    }
    registry.force_count += 1;
    run.create(&force_reference(&force.id), force)
}

fn open_command(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    open: &OpenEngagementV1,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    let open_count = run.registry_mut()?.open_engagements.len();
    if open_count >= rulebook.limits.open_engagements as usize {
        return Err(over("the open engagement limit is reached"));
    }
    if run.exists_any(engagement_reference(&open.id).as_untyped())? {
        return Err(payload(format!("engagement {} exists already", open.id)));
    }
    let kind_rule = rulebook
        .kinds
        .get(&open.kind)
        .ok_or_else(|| payload("the engagement kind has no rule"))?;
    if let Some(minutes) = open.max_minutes
        && (minutes == 0 || minutes > kind_rule.max_minutes)
    {
        return Err(over(format!(
            "a fight of {minutes} minutes is outside 1 to the kind rule's {}",
            kind_rule.max_minutes
        )));
    }
    let profile = open
        .profile
        .clone()
        .unwrap_or_else(|| rulebook.default_profile.clone());
    if !rulebook.step_profiles.contains_key(&profile) {
        return Err(payload(format!("step profile {profile} is unknown")));
    }
    check_side_size(&rulebook, &open.attackers)?;
    check_side_size(&rulebook, &open.defenders)?;
    check_holders(&rulebook, &open.holders)?;
    for input in &open.inputs {
        check_condition(&rulebook, input)?;
    }
    check_environment(&rulebook, &open.environment)?;
    if open.environment.terrain.is_none() && kind_rule.terrain.is_none() {
        return Err(payload(
            "neither the environment nor the kind names a terrain",
        ));
    }
    if let Some((grade, _)) = open.fort
        && rulebook.fort_grade(grade).is_none()
    {
        return Err(payload(format!("fortification grade {grade} is unknown")));
    }
    let attackers = open
        .attackers
        .iter()
        .map(|id| load_force(run, id))
        .collect::<Result<Vec<_>, _>>()?;
    let defenders = open
        .defenders
        .iter()
        .map(|id| load_force(run, id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut siege = None;
    for force in attackers.iter().chain(&defenders) {
        if !force.location.is_at(&open.site) {
            return Err(payload(format!("force {} is not at the site", force.id)));
        }
    }
    for force in &attackers {
        if !force.is_free() {
            return Err(payload(format!(
                "force {} is engaged, in a siege, or crossing",
                force.id
            )));
        }
        if !controls(&rulebook, issuer, force) {
            return Err(authority(format!(
                "the issuer does not control {}",
                force.id
            )));
        }
    }
    for force in &defenders {
        if open.kind == EngagementKind::Relief {
            let Some(SiegeRoleV1::Besieging { siege: besieged }) = &force.battle.siege_role else {
                return Err(payload("a relief is fought against besiegers"));
            };
            if force.battle.engaged_in.is_some() || force.battle.crossing.is_some() {
                return Err(payload(format!("force {} is engaged", force.id)));
            }
            if siege.get_or_insert_with(|| besieged.clone()) != besieged {
                return Err(payload(
                    "a relief is fought against the besiegers of one siege",
                ));
            }
        } else if !force.is_free() {
            return Err(payload(format!(
                "force {} is engaged, in a siege, or crossing",
                force.id
            )));
        }
    }
    single_owner(&attackers)?;
    single_owner(&defenders)?;
    let at = run.at;
    open_engagement(
        run,
        &rulebook,
        OpenSpec {
            id: open.id.clone(),
            kind: open.kind,
            profile,
            site: open.site.clone(),
            fort: open.fort,
            terrain: None,
            siege,
            crossing: None,
            attackers: open.attackers.clone(),
            defenders: open.defenders.clone(),
            holders: open.holders.clone(),
            inputs: [
                Some((open.inputs[0].clone(), at)),
                Some((open.inputs[1].clone(), at)),
            ],
            environment: Some((open.environment.clone(), at)),
            men_caps: [None, None],
            max_minutes: open.max_minutes,
            opened_at: at,
        },
    )
}

fn join_engagement(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    engagement: &EngagementId,
    expected: u64,
    side: Side,
    forces: &[ForceId],
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    let reference = engagement_reference(engagement);
    let mut state = run
        .get(&reference)?
        .ok_or_else(|| missing(format!("engagement {engagement} does not exist")))?;
    if state.is_closed() {
        return Err(payload("the engagement is closed"));
    }
    check_revision(state.host_revision, expected, "the engagement")?;
    let mut all = state.side(side).forces.clone();
    all.extend(forces.iter().cloned());
    check_side_size(&rulebook, &all)?;
    let owner = run
        .get(&force_reference(&state.side(side).forces[0]))?
        .map(|force| force.owner);
    let mut joining = Vec::new();
    for id in forces {
        if state.forces().any(|existing| existing == id) {
            return Err(payload(format!("force {id} is in the engagement already")));
        }
        let force = load_force(run, id)?;
        if !force.location.is_at(&state.site) || !force.is_free() {
            return Err(payload(format!("force {id} is not free at the site")));
        }
        if owner.as_ref().is_some_and(|owner| owner != &force.owner) {
            return Err(payload("a side has forces of more than one owner"));
        }
        if !controls(&rulebook, issuer, &force) {
            return Err(authority(format!("the issuer does not control {id}")));
        }
        joining.push(force);
    }
    crate::settle::join_side(run, &rulebook, &mut state, side, joining)?;
    state.host_revision += 1;
    run.put(&reference, &state)
}

/// The owner of a side: that of its first force still registered.
fn side_owner(run: &Run<'_, '_>, forces: &[ForceId]) -> Result<Option<EntityRef>, CanwuError> {
    for id in forces {
        if let Some(force) = run.get(&force_reference(id))? {
            return Ok(Some(force.owner));
        }
    }
    Ok(None)
}

/// Adds free forces at the site to one side of an open siege; the siege
/// reads them from its next siege day that runs. When the siege's assault or
/// sortie is in progress the forces also enter it on their side, as
/// `JoinEngagement` would, within both side limits.
fn join_siege(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    siege: &SiegeId,
    expected: u64,
    side: Side,
    forces: &[ForceId],
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    let mut state = open_siege(run, siege)?;
    check_revision(state.host_revision, expected, "the siege")?;
    let (listed, opposite) = match side {
        Side::Attacker => (state.besiegers.clone(), state.defenders.clone()),
        Side::Defender => (state.defenders.clone(), state.besiegers.clone()),
    };
    let mut all = listed.clone();
    all.extend(forces.iter().cloned());
    check_side_size(&rulebook, &all)?;
    // The siege's fight in progress: the besiegers are an assault's
    // attackers and a sortie's defenders.
    let fight = match &state.engagement {
        Some(id) => run
            .get(&engagement_reference(id))?
            .filter(|engagement| !engagement.is_closed()),
        None => None,
    };
    // Only `open_siege_fight` sets `siege.engagement`: an assault or a sortie.
    debug_assert!(fight.as_ref().is_none_or(|engagement| matches!(
        engagement.kind,
        EngagementKind::Assault | EngagementKind::Sortie
    )));
    let fight_side = match &fight {
        Some(engagement) if engagement.kind == EngagementKind::Sortie => side.other(),
        _ => side,
    };
    if let Some(engagement) = &fight {
        let mut all = engagement.side(fight_side).forces.clone();
        all.extend(forces.iter().cloned());
        check_side_size(&rulebook, &all)?;
    }
    let owner = side_owner(run, &listed)?;
    let opposing = side_owner(run, &opposite)?;
    let mut joining = Vec::new();
    for id in forces {
        if state.besiegers.contains(id) || state.defenders.contains(id) {
            return Err(payload(format!("force {id} is in the siege already")));
        }
        let force = load_force(run, id)?;
        if !force.location.is_at(&state.site) || !force.is_free() {
            return Err(payload(format!("force {id} is not free at the site")));
        }
        if !controls(&rulebook, issuer, &force) {
            return Err(authority(format!("the issuer does not control {id}")));
        }
        if owner.as_ref().is_some_and(|owner| owner != &force.owner) {
            return Err(payload("a side has forces of more than one owner"));
        }
        if opposing.as_ref() == Some(&force.owner) {
            return Err(payload(format!(
                "force {id} would face forces of its own owner"
            )));
        }
        joining.push(force);
    }
    single_owner(&joining)?;
    for force in &mut joining {
        force.battle.siege_role = Some(match side {
            Side::Attacker => SiegeRoleV1::Besieging {
                siege: siege.clone(),
            },
            Side::Defender => SiegeRoleV1::Defending {
                siege: siege.clone(),
            },
        });
    }
    match fight {
        Some(mut engagement) => {
            crate::settle::join_side(run, &rulebook, &mut engagement, fight_side, joining)?;
            engagement.host_revision += 1;
            run.put(&engagement_reference(&engagement.id.clone()), &engagement)?;
        }
        None => {
            for force in joining {
                run.put(&force_reference(&force.id.clone()), &force)?;
            }
        }
    }
    match side {
        Side::Attacker => state.besiegers.extend(forces.iter().cloned()),
        Side::Defender => state.defenders.extend(forces.iter().cloned()),
    }
    state.host_revision += 1;
    run.put(&siege_reference(siege), &state)
}

/// Releases forces from a siege: each leaves its side and its siege role.
fn release_from_siege(
    run: &mut Run<'_, '_>,
    rulebook: &CombatRulebookV1,
    issuer: &Issuer,
    state: &mut SiegeStateV1,
    release: &[ForceId],
) -> Result<(), CanwuError> {
    if release.is_empty() {
        return Ok(());
    }
    if state.besiegers.iter().all(|id| release.contains(id)) {
        return Err(payload(
            "a release would leave the siege without a besieger; LiftSiege ends it",
        ));
    }
    for id in release {
        if !state.besiegers.contains(id) && !state.defenders.contains(id) {
            return Err(payload(format!("force {id} is not in the siege")));
        }
        let mut force = load_force(run, id)?;
        if !controls(rulebook, issuer, &force) {
            return Err(authority(format!("the issuer does not control {id}")));
        }
        if force.battle.engaged_in.is_some() {
            return Err(payload(format!(
                "force {id} is in a fight and cannot leave the siege"
            )));
        }
        if force
            .battle
            .siege_role
            .as_ref()
            .is_some_and(|role| role.siege() == &state.id)
        {
            force.battle.siege_role = None;
        }
        run.put(&force_reference(id), &force)?;
    }
    state.besiegers.retain(|id| !release.contains(id));
    state.defenders.retain(|id| !release.contains(id));
    Ok(())
}

fn supply_input(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    target: &InputTargetV1,
    expected: u64,
    side: Option<Side>,
    condition: Option<&ConditionInputV1>,
    environment: Option<&EnvironmentInputV1>,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    if let Some(condition) = condition {
        check_condition(&rulebook, condition)?;
    }
    if let Some(environment) = environment {
        check_environment(&rulebook, environment)?;
    }
    let at = run.at;
    match target {
        InputTargetV1::Engagement(id) => {
            let reference = engagement_reference(id);
            let mut state = run
                .get(&reference)?
                .ok_or_else(|| missing(format!("engagement {id} does not exist")))?;
            if state.is_closed() {
                return Err(payload("the engagement is closed"));
            }
            check_revision(state.host_revision, expected, "the engagement")?;
            if let (Some(condition), Some(side)) = (condition, side) {
                if !controls_any(run, &rulebook, issuer, state.side(side).forces.iter())? {
                    return Err(authority("the issuer controls no force on that side"));
                }
                state.side_mut(side).input = Some((condition.clone(), at));
            }
            if let Some(environment) = environment {
                if !controls_any(run, &rulebook, issuer, state.forces())? {
                    return Err(authority("the issuer controls no force of the engagement"));
                }
                let kind_terrain = rulebook
                    .kinds
                    .get(&state.kind)
                    .and_then(|rule| rule.terrain.clone());
                if state.terrain.is_none()
                    && environment.terrain.is_none()
                    && kind_terrain.is_none()
                {
                    return Err(payload(
                        "neither the environment nor the kind names a terrain",
                    ));
                }
                state.environment = Some((environment.clone(), at));
            }
            if matches!(state.status, EngagementStatus::Held { .. }) {
                let step = rulebook
                    .step_profiles
                    .get(&state.profile)
                    .map_or(1, |profile| profile.step_minutes);
                resume(&mut state, at, step);
            }
            state.host_revision += 1;
            run.put(&reference, &state)
        }
        InputTargetV1::Siege(id) => {
            let mut state = open_siege(run, id)?;
            check_revision(state.host_revision, expected, "the siege")?;
            if let (Some(condition), Some(side)) = (condition, side) {
                let forces = match side {
                    Side::Attacker => &state.besiegers,
                    Side::Defender => &state.defenders,
                };
                if !controls_any(run, &rulebook, issuer, forces.iter())? {
                    return Err(authority("the issuer controls no force on that side"));
                }
                state.inputs[side.index()] = Some((condition.clone(), at));
            }
            if let Some(environment) = environment {
                if !controls_any(
                    run,
                    &rulebook,
                    issuer,
                    state.besiegers.iter().chain(&state.defenders),
                )? {
                    return Err(authority("the issuer controls no force of the siege"));
                }
                state.environment = Some((environment.clone(), at));
            }
            state.host_revision += 1;
            run.put(&siege_reference(id), &state)
        }
        InputTargetV1::Crossing(id) => {
            let mut state = open_crossing(run, id)?;
            check_revision(state.host_revision, expected, "the crossing")?;
            let defending: Vec<ForceId> = state
                .defenders
                .iter()
                .chain(state.interceptor.iter().map(|input| &input.force))
                .cloned()
                .collect();
            if let (Some(condition), Some(side)) = (condition, side) {
                let allowed = match side {
                    Side::Attacker => {
                        controls_any(run, &rulebook, issuer, std::iter::once(&state.force))?
                    }
                    Side::Defender => controls_any(run, &rulebook, issuer, defending.iter())?,
                };
                if !allowed {
                    return Err(authority("the issuer controls no force on that side"));
                }
                state.inputs[side.index()] = Some((condition.clone(), at));
            }
            if let Some(environment) = environment {
                if !controls_any(
                    run,
                    &rulebook,
                    issuer,
                    std::iter::once(&state.force).chain(&defending),
                )? {
                    return Err(authority("the issuer controls no force of the crossing"));
                }
                state.environment = Some((environment.clone(), at));
                // A wave that waited for the environment is due from now.
                state.next_wave_at = state.next_wave_at.max(at);
            }
            state.host_revision += 1;
            run.put(&crossing_reference(id), &state)
        }
    }
}

fn order_action(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    engagement: &EngagementId,
    expected: u64,
    side: Side,
    action: SideAction,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    let reference = engagement_reference(engagement);
    let mut state = run
        .get(&reference)?
        .ok_or_else(|| missing(format!("engagement {engagement} does not exist")))?;
    if state.is_closed() {
        return Err(payload("the engagement is closed"));
    }
    check_revision(state.host_revision, expected, "the engagement")?;
    if !controls_any(run, &rulebook, issuer, state.side(side).forces.iter())? {
        return Err(authority("the issuer controls no force on that side"));
    }
    state.host_revision += 1;
    match action {
        SideAction::Hold => state.side_mut(side).withdraw_ordered = false,
        SideAction::Withdraw => state.side_mut(side).withdraw_ordered = true,
        SideAction::Abandon if side == Side::Defender => {
            state.side_mut(side).withdraw_ordered = true;
        }
        SideAction::Abandon => {
            let at = run.at;
            return close_engagement(
                run,
                &rulebook,
                state,
                CloseInfo {
                    kind: EngagementOutcomeKind::Abandoned,
                    routed: [Vec::new(), Vec::new()],
                    site_fell: false,
                    at,
                },
                None,
            );
        }
    }
    run.put(&reference, &state)
}

fn open_siege(run: &Run<'_, '_>, id: &SiegeId) -> Result<SiegeStateV1, CanwuError> {
    let state = run
        .get(&siege_reference(id))?
        .ok_or_else(|| missing(format!("siege {id} does not exist")))?;
    if !state.is_open() {
        return Err(payload("the siege has ended"));
    }
    Ok(state)
}

fn open_crossing(run: &Run<'_, '_>, id: &CrossingId) -> Result<CrossingStateV1, CanwuError> {
    let state = run
        .get(&crossing_reference(id))?
        .ok_or_else(|| missing(format!("crossing {id} does not exist")))?;
    if !state.is_open() {
        return Err(payload("the crossing has ended"));
    }
    Ok(state)
}

fn invest_site(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    invest: &InvestSiteV1,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    if run.registry_mut()?.open_sieges.len() >= rulebook.limits.open_sieges as usize {
        return Err(over("the open siege limit is reached"));
    }
    if run.exists_any(siege_reference(&invest.id).as_untyped())? {
        return Err(payload(format!("siege {} exists already", invest.id)));
    }
    check_side_size(&rulebook, &invest.besiegers)?;
    check_side_size(&rulebook, &invest.defenders)?;
    check_holders(&rulebook, &invest.holders)?;
    for input in &invest.inputs {
        check_condition(&rulebook, input)?;
    }
    check_environment(&rulebook, &invest.environment)?;
    invest.profile.check_ranges()?;
    bounded(invest.work_bp, MAX_INPUT_BP, "work_bp")?;
    let grade = rulebook.fort_grade(invest.profile.grade).ok_or_else(|| {
        payload(format!(
            "fortification grade {} is unknown",
            invest.profile.grade
        ))
    })?;
    if invest.supply.evidence.len() > rulebook.limits.evidence_refs_per_input as usize {
        return Err(over("the supply input cites too much evidence"));
    }
    if let Some(naval) = &invest.naval {
        validate_interceptor(run, &rulebook, naval)?;
    }
    let mut besiegers = Vec::new();
    for id in &invest.besiegers {
        let force = load_force(run, id)?;
        if !force.location.is_at(&invest.site) || !force.is_free() {
            return Err(payload(format!("force {id} is not free at the site")));
        }
        if !controls(&rulebook, issuer, &force) {
            return Err(authority(format!("the issuer does not control {id}")));
        }
        besiegers.push(force);
    }
    let mut defenders = Vec::new();
    for id in &invest.defenders {
        let force = load_force(run, id)?;
        if !force.location.is_at(&invest.site) || !force.is_free() {
            return Err(payload(format!("force {id} is not free at the site")));
        }
        defenders.push(force);
    }
    single_owner(&besiegers)?;
    single_owner(&defenders)?;
    if let (Some(besieger), Some(defender)) = (besiegers.first(), defenders.first())
        && besieger.owner == defender.owner
    {
        return Err(payload("besiegers and defenders have one owner"));
    }
    let at = run.at;
    let state = SiegeStateV1 {
        schema_version: SCHEMA_VERSION,
        id: invest.id.clone(),
        host_revision: 0,
        site: invest.site.clone(),
        profile: invest.profile.clone(),
        besiegers: invest.besiegers.clone(),
        defenders: invest.defenders.clone(),
        holders: invest.holders.clone(),
        inputs: [
            Some((invest.inputs[0].clone(), at)),
            Some((invest.inputs[1].clone(), at)),
        ],
        environment: Some((invest.environment.clone(), at)),
        naval: invest.naval.clone(),
        investment_bp: 0,
        pieces: 0,
        emplacement_from: None,
        bombardment_from: None,
        work_bp: invest.work_bp,
        breach_progress: 0,
        breach_threshold: grade.breach_threshold,
        breach_open: false,
        gate_open_until: None,
        supply: invest.supply.clone(),
        storm_on: invest.storm_on,
        assault_ordered: false,
        sortie_ordered: false,
        engagement: None,
        started_at: at,
        next_day_at: add_minutes(at, rulebook.siege.day_minutes)?,
        status: SiegeStatus::Open,
        end_cause: None,
        ended_at: None,
    };
    state.validate()?;
    for (forces, role) in [(besiegers, true), (defenders, false)] {
        for mut force in forces {
            force.battle.siege_role = Some(if role {
                SiegeRoleV1::Besieging {
                    siege: invest.id.clone(),
                }
            } else {
                SiegeRoleV1::Defending {
                    siege: invest.id.clone(),
                }
            });
            run.put(&force_reference(&force.id.clone()), &force)?;
        }
    }
    run.registry_mut()?.open_sieges.insert(invest.id.clone());
    run.create(&siege_reference(&invest.id), &state)
}

fn revise_siege(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    revise: &ReviseSiegeV1,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    let mut state = open_siege(run, &revise.siege)?;
    check_revision(
        state.host_revision,
        revise.expected_host_revision,
        "the siege",
    )?;
    let general = revise.supply.is_some()
        || revise.profile.is_some()
        || revise.work_bp.is_some()
        || revise.naval.is_some()
        || revise.gate_open_steps.is_some()
        || revise.storm_on.is_some();
    let is_trusted = trusted(&rulebook, issuer);
    if general
        && !is_trusted
        && !controls_any(
            run,
            &rulebook,
            issuer,
            state.besiegers.iter().chain(&state.defenders),
        )?
    {
        return Err(authority("the issuer controls no force of the siege"));
    }
    if revise.assault.is_some()
        && !is_trusted
        && !controls_any(run, &rulebook, issuer, state.besiegers.iter())?
    {
        return Err(authority("the storm is the besiegers' order"));
    }
    release_from_siege(run, &rulebook, issuer, &mut state, &revise.release)?;
    let at = run.at;
    if revise.sortie {
        if !is_trusted && !controls_any(run, &rulebook, issuer, state.defenders.iter())? {
            return Err(authority("a sortie is the defenders' order"));
        }
        if state.defenders.is_empty() {
            return Err(payload("the siege has no defenders to sortie"));
        }
        for id in &state.defenders {
            if run
                .get(&force_reference(id))?
                .and_then(|force| force.battle.sortie_ready_at)
                .is_some_and(|ready| ready > at)
            {
                return Err(payload(
                    "a sortie before the defenders' cooldown has passed",
                ));
            }
        }
        state.sortie_ordered = true;
    }
    if let Some(supply) = &revise.supply {
        if supply.evidence.len() > rulebook.limits.evidence_refs_per_input as usize {
            return Err(over("the supply input cites too much evidence"));
        }
        state.supply = supply.clone();
    }
    if let Some(profile) = &revise.profile {
        profile.check_ranges()?;
        let grade = rulebook
            .fort_grade(profile.grade)
            .ok_or_else(|| payload(format!("fortification grade {} is unknown", profile.grade)))?;
        state.breach_threshold = grade.breach_threshold;
        state.profile = profile.clone();
    }
    if let Some(work_bp) = revise.work_bp {
        bounded(work_bp, MAX_INPUT_BP, "work_bp")?;
        state.work_bp = work_bp;
    }
    if let Some(naval) = &revise.naval {
        state.naval = match naval {
            InterceptorRevisionV1::Set(input) => {
                validate_interceptor(run, &rulebook, input)?;
                Some(input.clone())
            }
            InterceptorRevisionV1::Withdraw => None,
        };
    }
    if let Some(steps) = revise.gate_open_steps {
        bounded(steps, 1_000_000, "gate_open_steps")?;
        let step = rulebook
            .step_profiles
            .get(&rulebook.default_profile)
            .map_or(1, |profile| profile.step_minutes);
        state.gate_open_until = Some(add_minutes(at, step.saturating_mul(steps))?);
    }
    if let Some(assault) = revise.assault {
        state.assault_ordered = assault;
    }
    if let Some(storm_on) = revise.storm_on {
        state.storm_on = storm_on;
    }
    state.host_revision += 1;
    run.put(&siege_reference(&revise.siege), &state)
}

fn order_crossing(
    run: &mut Run<'_, '_>,
    issuer: &Issuer,
    order: &OrderCrossingV1,
) -> Result<(), CanwuError> {
    let rulebook = require_rulebook(run)?;
    if run.registry_mut()?.open_crossings.len() >= rulebook.limits.open_crossings as usize {
        return Err(over("the open crossing limit is reached"));
    }
    if run.exists_any(crossing_reference(&order.id).as_untyped())? {
        return Err(payload(format!("crossing {} exists already", order.id)));
    }
    check_holders(&rulebook, &order.holders)?;
    for input in &order.inputs {
        check_condition(&rulebook, input)?;
    }
    check_environment(&rulebook, &order.environment)?;
    let mut force = load_force(run, &order.force)?;
    if !force.location.is_at(&order.from) || !force.is_free() {
        return Err(payload("the crossing force is not free at its bank"));
    }
    if !controls(&rulebook, issuer, &force) {
        return Err(authority("the issuer does not control the crossing force"));
    }
    check_defenders(run, &rulebook, &order.force, &order.to, &order.defenders)?;
    if let Some(interceptor) = &order.interceptor {
        if interceptor.force == order.force {
            return Err(payload("a force cannot intercept its own crossing"));
        }
        validate_interceptor(run, &rulebook, interceptor)?;
    }
    let at = run.at;
    let state = CrossingStateV1 {
        schema_version: SCHEMA_VERSION,
        id: order.id.clone(),
        host_revision: 0,
        force: order.force.clone(),
        from: order.from.clone(),
        to: order.to.clone(),
        night: order.night,
        defenders: order.defenders.clone(),
        holders: order.holders.clone(),
        inputs: [
            Some((order.inputs[0].clone(), at)),
            Some((order.inputs[1].clone(), at)),
        ],
        environment: Some((order.environment.clone(), at)),
        interceptor: order.interceptor.clone(),
        waves_sent: 0,
        landed: 0,
        bridgehead_secure: false,
        next_wave_at: add_minutes(at, rulebook.crossing.wave_minutes)?,
        last_wave: None,
        engagement: None,
        status: CrossingStatus::Open,
        ended_at: None,
    };
    state.validate()?;
    force.battle.crossing = Some(order.id.clone());
    run.put(&force_reference(&order.force), &force)?;
    run.registry_mut()?.open_crossings.insert(order.id.clone());
    run.create(&crossing_reference(&order.id), &state)
}
