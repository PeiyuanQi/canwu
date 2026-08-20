use crate::model::{
    InstitutionalAlignment, PolicyDecision, SocietyState, SocietyStateRecord,
    core_reference_schemas, invalid, society_state_reference,
};
use crate::settle_transitions;
use crate::solver::{compute_aggregates, compute_mobilization_candidates, compute_projections};
use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    CanwuError, DecisionOrigin, DomainRecord, DomainRecordMutation, DomainRecordSchema,
    DomainRecordType, ErrorCode, Issuer, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginActionDescriptor, PluginRegistrar, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, SystemDirective,
};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PLUGIN_NAME: &str = "canwu-society";

#[derive(Clone, Copy, Debug, Default)]
pub struct SocietyPlugin;

impl SimulationPlugin for SocietyPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        "0.1.0-experimental"
    }

    fn semantic_hash(&self) -> &'static str {
        "a4e005ac53d979c74d6fa1d01302df1116fc5322c6461a60edfb1d83c6dddfd1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<SocietyStateRecord>();
        schema.payload_schema = society_payload_schema();
        schema.references = core_reference_schemas();
        registrar.register_record_schema(schema)?;

        registrar.register_command(
            PluginActionDescriptor {
                name: "set_institutional_policy".to_owned(),
                description: "Record a versioned institutional alignment decision".to_owned(),
                payload_schema: policy_payload_schema(),
                reads: vec![society_state_key(), policy_decision_key()],
                writes: vec![policy_decision_key()],
            },
            set_institutional_policy,
        )?;

        let mut transition = BoundarySystemContract::new(
            "settle-social-transitions",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        transition.reads = vec![society_state_key(), policy_decision_key()];
        transition.writes = vec![society_state_key()];
        transition.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(transition, settle_social_transitions)?;

        let mut mobilization = BoundarySystemContract::new(
            "evaluate-mobilization-candidates",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::Daily,
        );
        mobilization.reads = vec![society_state_key()];
        mobilization.writes = vec![society_state_key()];
        mobilization.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(mobilization, evaluate_mobilization)?;

        let mut aggregate = BoundarySystemContract::new(
            "aggregate-social-state",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::Daily,
        );
        aggregate.reads = vec![society_state_key()];
        aggregate.writes = vec![society_state_key()];
        aggregate.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(aggregate, aggregate_social_state)?;

        let mut project = BoundarySystemContract::new(
            "materialize-society-projections",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        project.reads = vec![society_state_key()];
        project.writes = vec![society_state_key()];
        project.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(project, materialize_projections)
    }
}

fn set_institutional_policy(
    view: &SimulationView<'_>,
    context: &canwu_api::CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let decision: PolicyDecision = serde_json::from_value(payload.clone()).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("institutional policy payload could not be decoded: {error}"),
        )
    })?;
    validate_policy_decision(&decision)?;
    let Some((_, state)) = load_state(view)? else {
        return Err(CanwuError::new(
            ErrorCode::DomainRecordNotFound,
            "the society state record is not configured",
        ));
    };
    let alignment = state
        .institutional_alignments
        .get(&decision.alignment_id)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidDecision,
                format!("unknown institutional alignment {}", decision.alignment_id),
            )
        })?;
    validate_policy_authority(context, alignment)?;
    if decision.decision_version <= alignment.last_decision_version {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            format!(
                "institutional decision version {} is not newer than applied version {}",
                decision.decision_version, alignment.last_decision_version
            ),
        ));
    }
    if let Some(pending) = view.component(
        &policy_decision_key(),
        &alignment.institution,
        &alignment.id,
    )? {
        let pending: PolicyDecision = serde_json::from_value(pending.clone()).map_err(|error| {
            invalid(format!(
                "stored institutional decision is malformed: {error}"
            ))
        })?;
        if decision.decision_version <= pending.decision_version {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                format!(
                    "institutional decision version {} is not newer than pending version {}",
                    decision.decision_version, pending.decision_version
                ),
            ));
        }
    }

    Ok(vec![SystemDirective::SetComponent {
        state: policy_decision_key(),
        entity: alignment.institution.clone(),
        component: alignment.id.clone(),
        value: serde_json::to_value(&decision).map_err(|error| {
            CanwuError::new(
                ErrorCode::InvalidPayload,
                format!("institutional decision could not be encoded: {error}"),
            )
        })?,
        summary: format!(
            "Institutional alignment {} received a new policy",
            alignment.id
        ),
    }])
}

fn validate_policy_authority(
    context: &canwu_api::CommandContext,
    alignment: &InstitutionalAlignment,
) -> Result<(), CanwuError> {
    let Some(controller_id) = context.decision_controller_id.as_deref() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "institutional policy changes require a validated DecisionTicket controller",
        ));
    };
    if !matches!(
        &context.issuer,
        Issuer::Ai(issuer) | Issuer::Human(issuer) if issuer == controller_id
    ) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "institutional policy issuer does not match its validated DecisionTicket controller",
        ));
    }
    if context.authority.command_subject.as_ref() != Some(&alignment.institution) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!(
                "command subject {:?} cannot change institutional alignment {} owned by {:?}",
                context.authority.command_subject, alignment.id, alignment.institution
            ),
        ));
    }
    match (
        &context.authority.decision_origin,
        alignment.authorized_actor,
    ) {
        (DecisionOrigin::Actor { actor }, Some(authorized)) if *actor == authorized => {}
        (
            DecisionOrigin::Institution {
                institution,
                responsible_actor: Some(actor),
            },
            Some(authorized),
        ) if institution == &alignment.institution && *actor == authorized => {}
        (
            DecisionOrigin::Institution {
                institution,
                responsible_actor: _,
            },
            None,
        ) if institution == &alignment.institution => {}
        _ => {
            return Err(CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!(
                    "decision origin {:?} cannot change institutional alignment {}",
                    context.authority.decision_origin, alignment.id
                ),
            ));
        }
    }
    Ok(())
}

fn settle_social_transitions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let before = state.clone();
    apply_pending_policies(view, &mut state)?;
    settle_transitions(&mut state, context.at)?;
    if state == before {
        return Ok(BoundaryProposal::default());
    }
    update_state(&record, state, "Settled aggregate social transitions")
}

fn evaluate_mobilization(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let candidates = compute_mobilization_candidates(&state, context.at);
    if state.mobilization_candidates == candidates && state.last_mobilization_at == Some(context.at)
    {
        return Ok(BoundaryProposal::default());
    }
    state.mobilization_candidates = candidates;
    state.last_mobilization_at = Some(context.at);
    update_state(&record, state, "Evaluated social mobilization candidates")
}

fn aggregate_social_state(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let aggregates = compute_aggregates(&state);
    if state.aggregates == aggregates && state.last_aggregation_at == Some(context.at) {
        return Ok(BoundaryProposal::default());
    }
    state.aggregates = aggregates;
    state.last_aggregation_at = Some(context.at);
    update_state(&record, state, "Aggregated social state")
}

fn materialize_projections(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some((record, mut state)) = load_state(view)? else {
        return Ok(BoundaryProposal::default());
    };
    let projections = compute_projections(&state, context.at);
    if state.projections == projections && state.last_projection_at == Some(context.at) {
        return Ok(BoundaryProposal::default());
    }
    state.projections = projections;
    state.last_projection_at = Some(context.at);
    update_state(
        &record,
        state,
        "Materialized actor-relative society projections",
    )
}

fn apply_pending_policies(
    view: &SimulationView<'_>,
    state: &mut SocietyState,
) -> Result<(), CanwuError> {
    for alignment in state.institutional_alignments.values_mut() {
        let Some(value) = view.component(
            &policy_decision_key(),
            &alignment.institution,
            &alignment.id,
        )?
        else {
            continue;
        };
        let decision: PolicyDecision = serde_json::from_value(value.clone()).map_err(|error| {
            invalid(format!(
                "stored institutional decision is malformed: {error}"
            ))
        })?;
        validate_policy_decision(&decision)?;
        if decision.alignment_id != alignment.id {
            return Err(invalid(format!(
                "institutional decision {} was stored on alignment {}",
                decision.alignment_id, alignment.id
            )));
        }
        if decision.decision_version <= alignment.last_decision_version {
            continue;
        }
        alignment.support_per_mille = decision.support_per_mille;
        alignment.enforcement_per_mille = decision.enforcement_per_mille;
        alignment.access_grant_per_mille = decision.access_grant_per_mille;
        alignment.last_decision_version = decision.decision_version;
    }
    Ok(())
}

fn update_state(
    record: &DomainRecord,
    mut state: SocietyState,
    summary: &str,
) -> Result<BoundaryProposal, CanwuError> {
    state.canonicalize()?;
    state.validate()?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: state.record_draft()?,
                expected_version: record.version,
            },
            summary: summary.to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn load_state(
    view: &SimulationView<'_>,
) -> Result<Option<(DomainRecord, SocietyState)>, CanwuError> {
    let Some(record) = view.typed_domain_record(&society_state_reference())? else {
        return Ok(None);
    };
    let mut state = record.decode_payload::<SocietyStateRecord>()?;
    state.canonicalize()?;
    state.validate()?;
    state.validate_at(view.time())?;
    state.validate_record_binding(record)?;
    Ok(Some((record.clone(), state)))
}

pub(crate) fn validate_policy_decision(decision: &PolicyDecision) -> Result<(), CanwuError> {
    if decision.decision_version == 0
        || decision.support_per_mille > 1_000
        || decision.enforcement_per_mille > 1_000
        || decision.access_grant_per_mille > 1_000
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "institutional policy fields must be versioned and within permille bounds",
        ));
    }
    Ok(())
}

fn society_state_key() -> StateKey {
    StateKey::new(SocietyStateRecord::NAMESPACE, SocietyStateRecord::NAME)
}

pub(crate) fn policy_decision_key() -> StateKey {
    StateKey::new("canwu.society", "policy-decisions")
}

fn policy_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            (
                "access_grant_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "alignment_id".to_owned(),
                required(PayloadValueType::String),
            ),
            (
                "decision_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "enforcement_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "support_per_mille".to_owned(),
                required(PayloadValueType::Integer),
            ),
        ]),
        allow_additional: false,
    }
}

fn society_payload_schema() -> PayloadSchema {
    PayloadSchema::Object {
        properties: BTreeMap::from([
            ("aggregates".to_owned(), required(PayloadValueType::Object)),
            ("cohorts".to_owned(), required(PayloadValueType::Object)),
            (
                "distributions".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "influence_edges".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "institutional_alignments".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "last_aggregation_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_mobilization_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_projection_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "last_transition_at".to_owned(),
                optional(PayloadValueType::Integer),
            ),
            (
                "mobilization_candidates".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "observer_profiles".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "organization_relations".to_owned(),
                required(PayloadValueType::Object),
            ),
            (
                "organizations".to_owned(),
                required(PayloadValueType::Object),
            ),
            ("policies".to_owned(), required(PayloadValueType::Object)),
            ("projections".to_owned(), required(PayloadValueType::Object)),
            ("remainders".to_owned(), required(PayloadValueType::Object)),
            (
                "schema_version".to_owned(),
                required(PayloadValueType::Integer),
            ),
            ("targets".to_owned(), required(PayloadValueType::Object)),
            (
                "topology_passes".to_owned(),
                required(PayloadValueType::Integer),
            ),
            (
                "transition_rules".to_owned(),
                required(PayloadValueType::Object),
            ),
        ]),
        allow_additional: false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use canwu_api::{
        Canwu, CommandAuthority, CommandContext, CommandId, CommandIngress, CommandPolicyContext,
        EntityRef, Issuer, SimTime,
    };

    #[test]
    fn decision_policy_authority_binds_controller_origin_and_subject() {
        let ids = Canwu::demo_ids();
        let alignment = InstitutionalAlignment {
            id: "alignment".to_owned(),
            institution: EntityRef::Government(ids.government),
            target_id: "target".to_owned(),
            affected_cohorts: std::collections::BTreeSet::default(),
            support_per_mille: 0,
            enforcement_per_mille: 0,
            access_grant_per_mille: 0,
            authorized_actor: Some(ids.commander),
            last_decision_version: 0,
        };
        let context = |subject| CommandContext {
            issuer: Issuer::Ai("controller".to_owned()),
            authority: CommandAuthority {
                decision_origin: DecisionOrigin::Actor {
                    actor: ids.commander,
                },
                seat_id: None,
                permission_profile_id: None,
                command_subject: subject,
            },
            decision_controller_id: Some("controller".to_owned()),
            run_policy: CommandPolicyContext::LegacyUnspecified,
            ingress: CommandIngress::LiveRequest,
            attempt_id: None,
            command_id: CommandId::new(1),
            request_id: None,
            revision: 0,
            simulation_time: SimTime::EPOCH,
            expected_revision: None,
            expected_time: None,
        };

        assert!(validate_policy_authority(&context(None), &alignment).is_err());
        assert!(
            validate_policy_authority(&context(Some(EntityRef::Army(ids.army))), &alignment)
                .is_err()
        );
        assert!(
            validate_policy_authority(
                &context(Some(EntityRef::Government(ids.government))),
                &alignment,
            )
            .is_ok()
        );
    }
}
