use crate::ingress::{
    COHORT_REBASE_INGRESS, CohortHeadcountRebaseV1, SOCIETY_INGRESS_MALFORMED_REJECTION,
    SocietyIngressQueue, SocietyIngressQueueRecord, rebase_admission_rejection,
    society_ingress_queue_reference,
};
use crate::lifecycle::SOCIETY_LIFECYCLE_DELTA_INGRESS;
use crate::plugin::{PLUGIN_NAME, policy_decision_key, validate_policy_decision};
use crate::{
    SocietyPlugin, SocietyProjection, SocietyState, SocietyStateRecord, society_state_reference,
};
use canwu_api::{
    BoundaryId, BoundaryPhase, BoundaryRecord, Canwu, CanwuError, CauseRef, DomainRecordRef,
    DomainRecordVersionRef, DomainRecordVersionSource, ErrorCode, IngressId, IngressPayload,
    IngressRecord, PluginComponentRecord, StateVisibility, ViewerContext,
};
use std::collections::{BTreeMap, BTreeSet};

/// System name the engine records on owner-authorized maintenance record
/// changes, which commit when the maintenance ingress is admitted.
const OWNER_AUTHORIZED_MAINTENANCE_SYSTEM: &str = "owner-authorized-maintenance";

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
    load_ingress_queue(canwu)?;
    Ok(state)
}

/// Loads the owner-side ingress queue and rejects entries an initial
/// scenario seeded, since only the intake may write admitted packets.
fn load_ingress_queue(canwu: &Canwu) -> Result<Option<SocietyIngressQueue>, CanwuError> {
    let Some(record) = canwu.typed_domain_record(&society_ingress_queue_reference()) else {
        return Ok(None);
    };
    let queue = record.decode_payload::<SocietyIngressQueueRecord>()?;
    queue.validate()?;
    if !queue.entries.is_empty()
        && canwu
            .current_domain_record_version(&record.reference)?
            .is_none_or(|version| {
                version.established_by == DomainRecordVersionSource::InitialScenario
            })
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "an initial scenario cannot seed admitted society ingress",
        ));
    }
    Ok(Some(queue))
}

/// Re-derives every queued packet from the journal: the packet must be the
/// exact society ingress admitted at its recorded boundary, a lifecycle delta
/// must have been generated inside the engine, and a rebase's admission
/// verdict must match the stock history at the admission cut (changes
/// committed through phase 11 of the admitting boundary).
fn validate_queued_ingress(canwu: &Canwu) -> Result<(), CanwuError> {
    let Some(queue) = load_ingress_queue(canwu)? else {
        return Ok(());
    };
    if queue.entries.is_empty() {
        return Ok(());
    }
    let ingress = canwu
        .ingress_log()
        .iter()
        .map(|record| (record.id, record))
        .collect::<BTreeMap<IngressId, &IngressRecord>>();
    let boundaries = canwu
        .boundaries()
        .iter()
        .map(|record| (record.id, record))
        .collect::<BTreeMap<BoundaryId, &BoundaryRecord>>();
    let phases = canwu
        .plugin_descriptors()
        .flat_map(|descriptor| {
            descriptor
                .boundary_systems
                .iter()
                .map(|system| ((descriptor.name.clone(), system.name.clone()), system.phase))
        })
        .collect::<BTreeMap<_, _>>();
    let versions = cited_stock_versions(canwu, &queue);
    let history = StockHistory {
        boundaries: &boundaries,
        phases: &phases,
        versions: &versions,
    };
    let queue_error = |message: &str| CanwuError::new(ErrorCode::InvalidAuthority, message);
    for entry in &queue.entries {
        let record = ingress
            .get(&entry.ingress)
            .ok_or_else(|| queue_error("queued society ingress has no retained ingress record"))?;
        let admitted = boundaries
            .get(&entry.admitted_at)
            .is_some_and(|boundary| boundary.admitted_ingress.contains(&entry.ingress));
        let same_packet = matches!(
            &record.payload,
            IngressPayload::Plugin { plugin, packet_type, payload, .. }
                if plugin == PLUGIN_NAME
                    && *packet_type == entry.packet_type
                    && *payload == entry.payload
        );
        if !admitted || !same_packet {
            return Err(queue_error(
                "queued society ingress does not match the packet admitted at its boundary",
            ));
        }
        if entry.packet_type == SOCIETY_LIFECYCLE_DELTA_INGRESS {
            let generated = match &record.cause {
                Some(CauseRef::Boundary(producer)) => {
                    boundaries.get(producer).is_some_and(|boundary| {
                        boundary
                            .generated_ingress
                            .iter()
                            .any(|generation| generation.ingress == entry.ingress)
                    })
                }
                _ => false,
            };
            if !generated {
                return Err(queue_error(
                    "queued lifecycle delta was not generated by a boundary system",
                ));
            }
        } else if entry.packet_type == COHORT_REBASE_INGRESS {
            let expected =
                match serde_json::from_value::<CohortHeadcountRebaseV1>(entry.payload.clone()) {
                    Err(_) => Some(SOCIETY_INGRESS_MALFORMED_REJECTION.to_owned()),
                    Ok(rebase) => {
                        let current = history.current_at_admission(
                            canwu,
                            &rebase.external_stock,
                            entry.admitted_at,
                        );
                        rebase_admission_rejection(&rebase, current)
                    }
                };
            if expected != entry.admission_rejection {
                return Err(queue_error(
                    "queued rebase admission verdict does not match its stock history",
                ));
            }
        }
    }
    Ok(())
}

/// Indexes, once, where each retained version of a stock record cited by a
/// queued rebase was established.
fn cited_stock_versions(
    canwu: &Canwu,
    queue: &SocietyIngressQueue,
) -> BTreeMap<(DomainRecordRef, u64), (BoundaryId, usize)> {
    let cited = queue
        .entries
        .iter()
        .filter(|entry| entry.packet_type == COHORT_REBASE_INGRESS)
        .filter_map(|entry| {
            serde_json::from_value::<CohortHeadcountRebaseV1>(entry.payload.clone()).ok()
        })
        .map(|rebase| rebase.external_stock.record)
        .collect::<BTreeSet<_>>();
    let mut versions = BTreeMap::new();
    for boundary in canwu.boundaries() {
        for (index, change) in boundary.record_changes.iter().enumerate() {
            if cited.contains(&change.current.reference) {
                versions
                    .entry((change.current.reference.clone(), change.current.version))
                    .or_insert((boundary.id, index));
            }
        }
    }
    versions
}

/// Retained record history needed to recompute a rebase admission verdict.
struct StockHistory<'a> {
    boundaries: &'a BTreeMap<BoundaryId, &'a BoundaryRecord>,
    phases: &'a BTreeMap<(String, String), BoundaryPhase>,
    versions: &'a BTreeMap<(DomainRecordRef, u64), (BoundaryId, usize)>,
}

impl StockHistory<'_> {
    /// Whether a record change was visible to the phase-12 intake of
    /// `admitted_at`: it committed in an earlier boundary, it is a
    /// maintenance change (committed at admission, before every phase), or it
    /// is a same-boundary change a phase-7 or phase-10 system proposed with
    /// `SameBoundary` visibility. Unknown writers fail closed as invisible.
    fn visible(&self, admitted_at: BoundaryId, boundary: BoundaryId, index: usize) -> bool {
        boundary < admitted_at
            || (boundary == admitted_at
                && self
                    .boundaries
                    .get(&boundary)
                    .and_then(|record| record.record_changes.get(index))
                    .is_some_and(|change| {
                        change.system == OWNER_AUTHORIZED_MAINTENANCE_SYSTEM
                            || (change.visibility == StateVisibility::SameBoundary
                                && self
                                    .phases
                                    .get(&(change.plugin.clone(), change.system.clone()))
                                    .is_some_and(|phase| {
                                        *phase < BoundaryPhase::StrategicAggregation
                                    }))
                    }))
    }

    /// Whether the exact stock version was current for the phase-12 intake
    /// of boundary `admitted_at`.
    fn current_at_admission(
        &self,
        canwu: &Canwu,
        stock: &DomainRecordVersionRef,
        admitted_at: BoundaryId,
    ) -> bool {
        if !canwu.domain_record_version_evidence_exists(stock) {
            return false;
        }
        let established = match &stock.established_by {
            DomainRecordVersionSource::InitialScenario => true,
            DomainRecordVersionSource::BoundaryChange {
                boundary,
                change_index,
            } => usize::try_from(*change_index)
                .is_ok_and(|index| self.visible(admitted_at, *boundary, index)),
        };
        if !established {
            return false;
        }
        let Some(next) = stock.version.checked_add(1) else {
            return true;
        };
        match self.versions.get(&(stock.record.clone(), next)) {
            Some((boundary, index)) => !self.visible(admitted_at, *boundary, *index),
            // A successor outside the retained history predates it.
            None => canwu
                .domain_record(&stock.record)
                .is_some_and(|record| record.version == stock.version),
        }
    }
}

/// Rehydrates a snapshot with the society plugin and revalidates its semantic
/// payload-to-reference binding and every queued rebase or lifecycle delta
/// against the ingress journal before returning the simulation.
///
/// # Errors
///
/// Returns an error when the engine snapshot contract or the society record is
/// invalid.
pub fn from_society_snapshot_json(json: &str) -> Result<Canwu, CanwuError> {
    let plugin = SocietyPlugin;
    let canwu = Canwu::from_snapshot_json_with_plugins(json, &[&plugin])?;
    validate_society_runtime(&canwu)?;
    Ok(canwu)
}

/// Validates restored society state: the root record (as
/// [`load_society_state`] does) and every queued rebase or lifecycle delta,
/// re-derived from the ingress journal. Hosts that restore society together
/// with other plugins call this after `Canwu::from_snapshot_json_with_plugins`.
///
/// # Errors
///
/// Returns an error when the society record is invalid or a queued packet is
/// not the exact admitted packet its entry claims, including a rebase whose
/// recorded admission verdict disagrees with the stock history.
pub fn validate_society_runtime(canwu: &Canwu) -> Result<(), CanwuError> {
    load_society_state(canwu)?;
    validate_queued_ingress(canwu)
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
