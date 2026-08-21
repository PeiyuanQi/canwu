use crate::plugin::{PLUGIN_NAME, policy_decision_key, validate_policy_decision};
use crate::{
    SocietyPlugin, SocietyProjection, SocietyState, SocietyStateRecord, society_state_reference,
};
use canwu_api::{Canwu, CanwuError, ErrorCode, PluginComponentRecord, ViewerContext};

/// Loads and validates the authoritative society record.
///
/// # Errors
///
/// Returns an error when the record is absent, malformed, or violates society
/// invariants.
pub fn load_society_state(canwu: &Canwu) -> Result<SocietyState, CanwuError> {
    let record = canwu
        .typed_domain_record(&society_state_reference())
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::DomainRecordNotFound,
                "the society state record is not configured",
            )
        })?;
    let mut state = record.decode_payload::<SocietyStateRecord>()?;
    state.canonicalize()?;
    state.validate()?;
    state.validate_at(canwu.time())?;
    state.validate_record_binding(record)?;
    validate_policy_components(canwu, &state)?;
    Ok(state)
}

/// Rehydrates a snapshot with the society plugin and revalidates its semantic
/// payload-to-reference binding before returning the simulation.
///
/// # Errors
///
/// Returns an error when the engine snapshot contract or the society record is
/// invalid.
pub fn from_society_snapshot_json(json: &str) -> Result<Canwu, CanwuError> {
    let plugin = SocietyPlugin;
    let canwu = Canwu::from_snapshot_json_with_plugins(json, &[&plugin])?;
    load_society_state(&canwu)?;
    Ok(canwu)
}

fn validate_policy_components(canwu: &Canwu, state: &SocietyState) -> Result<(), CanwuError> {
    validate_policy_component_records(&canwu.snapshot().plugin_components, state)
}

fn validate_policy_component_records(
    components: &[PluginComponentRecord],
    state: &SocietyState,
) -> Result<(), CanwuError> {
    let policy_state = policy_decision_key();
    for component in components
        .iter()
        .filter(|component| component.plugin == PLUGIN_NAME)
    {
        if component.state != policy_state {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                "society plugin owns an unexpected component state",
            ));
        }
        let alignment = state
            .institutional_alignments
            .get(&component.component)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!(
                        "society policy component {} has no institutional alignment",
                        component.component
                    ),
                )
            })?;
        if component.entity != alignment.institution {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "society policy component {} is attached to the wrong institution",
                    component.component
                ),
            ));
        }
        let decision: crate::PolicyDecision = serde_json::from_value(component.value.clone())
            .map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidDomainRecord,
                    format!("society policy component is malformed: {error}"),
                )
            })?;
        validate_policy_decision(&decision)
            .map_err(|error| CanwuError::new(ErrorCode::InvalidDomainRecord, error.message))?;
        if decision.alignment_id != component.component {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "society policy component {} contains decision for {}",
                    component.component, decision.alignment_id
                ),
            ));
        }
        if decision.decision_version < alignment.last_decision_version {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "society policy component {} is older than its applied alignment version",
                    component.component
                ),
            ));
        }
        if decision.decision_version == alignment.last_decision_version
            && (decision.support_per_mille != alignment.support_per_mille
                || decision.enforcement_per_mille != alignment.enforcement_per_mille
                || decision.access_grant_per_mille != alignment.access_grant_per_mille)
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidDomainRecord,
                format!(
                    "society policy component {} disagrees with its applied alignment values",
                    component.component
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::{InstitutionalAlignment, PolicyDecision};
    use canwu_api::{Canwu, EntityRef};

    #[test]
    fn orphan_and_mismatched_policy_components_are_rejected() {
        let ids = Canwu::demo_ids();
        let mut state = SocietyState::default();
        state.institutional_alignments.insert(
            "alignment".to_owned(),
            InstitutionalAlignment {
                id: "alignment".to_owned(),
                institution: EntityRef::Government(ids.government),
                target_id: "target".to_owned(),
                affected_cohorts: std::collections::BTreeSet::default(),
                support_per_mille: 0,
                enforcement_per_mille: 0,
                access_grant_per_mille: 0,
                authorized_actor: Some(ids.commander),
                last_decision_version: 0,
            },
        );
        let component = |name: &str, alignment_id: &str| PluginComponentRecord {
            plugin: PLUGIN_NAME.to_owned(),
            state: policy_decision_key(),
            entity: EntityRef::Government(ids.government),
            component: name.to_owned(),
            value: serde_json::to_value(PolicyDecision {
                alignment_id: alignment_id.to_owned(),
                decision_version: 1,
                support_per_mille: 0,
                enforcement_per_mille: 0,
                access_grant_per_mille: 0,
            })
            .expect("policy decision"),
        };

        assert!(
            validate_policy_component_records(&[component("orphan", "orphan")], &state).is_err()
        );
        assert!(
            validate_policy_component_records(&[component("alignment", "other")], &state).is_err()
        );
        let mut malformed = component("alignment", "alignment");
        malformed.value = serde_json::Value::String("not-a-policy".to_owned());
        assert!(validate_policy_component_records(&[malformed], &state).is_err());
        let mut out_of_bounds = component("alignment", "alignment");
        out_of_bounds.value["support_per_mille"] = serde_json::json!(1_001);
        assert!(validate_policy_component_records(&[out_of_bounds], &state).is_err());
        let mut wrong_entity = component("alignment", "alignment");
        wrong_entity.entity = EntityRef::Army(ids.army);
        assert!(validate_policy_component_records(&[wrong_entity], &state).is_err());
        assert!(
            validate_policy_component_records(&[component("alignment", "alignment")], &state)
                .is_ok()
        );
        let mut applied = state.clone();
        let alignment = applied
            .institutional_alignments
            .get_mut("alignment")
            .expect("alignment");
        alignment.last_decision_version = 2;
        alignment.support_per_mille = 100;
        alignment.enforcement_per_mille = 200;
        alignment.access_grant_per_mille = 300;
        assert!(
            validate_policy_component_records(&[component("alignment", "alignment")], &applied)
                .is_err(),
            "an older persisted component must be rejected"
        );
        let mut mismatched = component("alignment", "alignment");
        mismatched.value["decision_version"] = serde_json::json!(2);
        assert!(
            validate_policy_component_records(&[mismatched], &applied).is_err(),
            "an applied component must match the alignment values"
        );
        let mut exact = component("alignment", "alignment");
        exact.value["decision_version"] = serde_json::json!(2);
        exact.value["support_per_mille"] = serde_json::json!(100);
        exact.value["enforcement_per_mille"] = serde_json::json!(200);
        exact.value["access_grant_per_mille"] = serde_json::json!(300);
        assert!(validate_policy_component_records(&[exact], &applied).is_ok());
        let mut pending = component("alignment", "alignment");
        pending.value["decision_version"] = serde_json::json!(3);
        assert!(validate_policy_component_records(&[pending], &applied).is_ok());
    }
}

/// Returns the previously materialized estimate authorized for one viewer.
///
/// This query never falls back to the authoritative society record.
///
/// # Errors
///
/// Returns an error when the viewer context is stale or forged, the society
/// record is invalid, or no projection has been delivered for the actor.
pub fn projection_for_viewer(
    canwu: &Canwu,
    viewer: &ViewerContext,
) -> Result<SocietyProjection, CanwuError> {
    let actor = viewer.actor().ok_or_else(|| {
        CanwuError::new(
            ErrorCode::InvalidAuthority,
            "society projection requires a person observation principal",
        )
    })?;
    let authorized = canwu.viewer_context(actor)?;
    if authorized != *viewer {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            format!("actor {actor} is not authorized for this society projection"),
        ));
    }
    let state = load_society_state(canwu)?;
    state
        .projections
        .get(&actor.get().to_string())
        .cloned()
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                format!("no delivered society projection exists for actor {actor}"),
            )
        })
}
