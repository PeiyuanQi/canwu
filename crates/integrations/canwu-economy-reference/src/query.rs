use crate::{
    EconomyObservationHeadV1, EconomyPriceObservationV1, EconomyReferenceRuntimeRecord,
    EconomyReferenceStateV1, EconomyRouteObservationV1, PriceEvidenceApplicabilityV1,
    economy_reference_runtime_reference,
};
use canwu_api::{Canwu, CanwuError, ErrorCode, KnowledgeHolderRef, SimDuration, SimTime};
use canwu_resource::ResourceScopeId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_ARCHIVED_QUERY_RECORDS: usize = 16_384;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EconomyObservationWitnessV1 {
    pub(crate) provider_plugin: String,
    pub(crate) provider_version: String,
    pub(crate) provider_semantic_hash: String,
    pub(crate) provider_state_revision: u64,
    pub(crate) holder: KnowledgeHolderRef,
    pub(crate) scope: ResourceScopeId,
    pub(crate) observed_at: SimTime,
    pub(crate) materialized_at: SimTime,
    pub(crate) confidence_per_mille: u16,
    pub(crate) head: EconomyObservationHeadV1,
    pub(crate) routes: Vec<EconomyRouteObservationV1>,
    pub(crate) prices: Vec<EconomyPriceObservationV1>,
    pub(crate) price_applicability: PriceEvidenceApplicabilityV1,
    pub(crate) profile_revision: String,
    pub(crate) compiled_content_hash: String,
    pub(crate) adapter_revision: String,
    pub(crate) canonical_digest: String,
}

impl EconomyObservationWitnessV1 {
    #[must_use]
    pub fn holder(&self) -> &KnowledgeHolderRef {
        &self.holder
    }

    #[must_use]
    pub fn scope(&self) -> &ResourceScopeId {
        &self.scope
    }

    #[must_use]
    pub const fn observed_at(&self) -> SimTime {
        self.observed_at
    }

    #[must_use]
    pub const fn materialized_at(&self) -> SimTime {
        self.materialized_at
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.canonical_digest
    }
}

pub fn economy_reference_state(
    canwu: &Canwu,
) -> Result<Option<(canwu_api::DomainRecord, EconomyReferenceStateV1)>, CanwuError> {
    let reference = economy_reference_runtime_reference();
    let Some(record) = canwu.typed_domain_record(&reference).cloned() else {
        return Ok(None);
    };
    let state = record.decode_payload::<EconomyReferenceRuntimeRecord>()?;
    Ok(Some((record, state)))
}

/// Validates the economy runtime and its complete provider-backed archive and
/// retention closure before restored state is allowed to resume.
pub fn validate_economy_reference_runtime_with_archive_store(
    canwu: &Canwu,
    store: &dyn canwu_force_supply_reference::PackageArchiveStore,
) -> Result<(), CanwuError> {
    let Some((record, state)) = economy_reference_state(canwu)? else {
        return Ok(());
    };
    state.validate()?;
    if record.version != state.revision || state.draft()?.payload != record.payload {
        return Err(invalid(
            "economy runtime root version or canonical encoding differs",
        ));
    }
    canwu_force_supply_reference::validate_package_archive_store::<
        crate::EconomyArchiveKeyV1,
        crate::EconomyArchivePayloadV1,
    >(
        crate::ECONOMY_ARCHIVE_DOMAIN,
        &state.archive_head,
        &state.archive_retention_handles,
        &state.archive_maintenance_receipts,
        store,
    )
}

#[allow(clippy::too_many_lines)]
pub fn economy_observation_witness(
    canwu: &Canwu,
    holder: &KnowledgeHolderRef,
    scope: &ResourceScopeId,
) -> Result<EconomyObservationWitnessV1, CanwuError> {
    let (_, state) = economy_reference_state(canwu)?
        .ok_or_else(|| unavailable("economy-reference runtime is unavailable"))?;
    let grant_id = state
        .observation_grant_by_holder_scope
        .get(&crate::holder_scope_index_key(holder, scope)?)
        .ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidAuthority,
                "holder has no economy observation grant for this scope",
            )
        })?;
    let grant = state
        .observation_grants
        .get(grant_id)
        .ok_or_else(|| invalid("economy holder/scope grant index is orphaned"))?;
    let delay = i64::try_from(grant.delay_minutes)
        .map_err(|_| invalid("economy observation delay exceeds simulation time"))?;
    let observed_at = canwu
        .time()
        .checked_add(SimDuration::minutes(-delay))
        .ok_or_else(|| unavailable("economy observation delay precedes scenario start"))?;
    let holder_scope = crate::holder_scope_index_key(holder, scope)?;
    let hot_head = state
        .observation_temporal_by_holder_scope
        .get(&holder_scope)
        .and_then(|history| latest_observation_head_at(history, observed_at).cloned());
    let cold = canwu_force_supply_reference::load_package_archive_records::<
        crate::EconomyArchiveKeyV1,
        crate::EconomyArchivePayloadV1,
    >(
        crate::ECONOMY_ARCHIVE_DOMAIN,
        &state.archive_head,
        canwu,
        MAX_ARCHIVED_QUERY_RECORDS,
    )?;
    let cold_head = cold
        .iter()
        .filter_map(|record| match (&record.key, &record.payload) {
            (
                crate::EconomyArchiveKeyV1::ObservationHead {
                    holder_scope: archived_scope,
                    ..
                },
                crate::EconomyArchivePayloadV1::ObservationHead(head),
            ) if archived_scope == &holder_scope && head.observed_at <= observed_at => Some(head),
            _ => None,
        })
        .max_by(|left, right| {
            (left.observed_at, &left.semantic_digest)
                .cmp(&(right.observed_at, &right.semantic_digest))
        })
        .cloned();
    let head = hot_head
        .into_iter()
        .chain(cold_head)
        .max_by(|left, right| {
            (left.observed_at, &left.semantic_digest)
                .cmp(&(right.observed_at, &right.semantic_digest))
        })
        .ok_or_else(|| unavailable("no persisted economy observation head exists at this cut"))?;
    let economy = state
        .local_economies
        .get(&head.economy)
        .ok_or_else(|| invalid("economy observation head references a missing local economy"))?;
    let profile = state
        .profiles
        .get(&economy.profile)
        .ok_or_else(|| invalid("economy observation profile is unavailable"))?;
    let mut route_heads = BTreeMap::new();
    for route in state
        .route_observations
        .values()
        .chain(cold.iter().filter_map(|record| match &record.payload {
            crate::EconomyArchivePayloadV1::RouteObservation(route) => Some(route),
            _ => None,
        }))
        .filter(|route| {
            &route.holder == holder
                && &route.target_scope == scope
                && route.observed_at <= observed_at
        })
    {
        let fact = crate::route_head_index_key(&route.route_key, &route.source_scope);
        let replace = route_heads
            .get(&fact)
            .is_none_or(|current: &&EconomyRouteObservationV1| {
                (current.observed_at, &current.id) < (route.observed_at, &route.id)
            });
        if replace {
            route_heads.insert(fact, route);
        }
    }
    if route_heads.len() > crate::MAX_OBSERVATION_FACTS {
        return Err(CanwuError::new(
            ErrorCode::QueryBudgetExceeded,
            "economy route observation adapter exceeded its fact budget",
        ));
    }
    let mut routes = route_heads.into_values().cloned().collect::<Vec<_>>();
    routes.sort_by(|left, right| left.source_scope.cmp(&right.source_scope));
    let mut price_heads = BTreeMap::new();
    for price in state
        .price_observations
        .values()
        .chain(cold.iter().filter_map(|record| match &record.payload {
            crate::EconomyArchivePayloadV1::PriceObservation(price) => Some(price),
            _ => None,
        }))
        .filter(|price| {
            &price.holder == holder
                && &price.scope == scope
                && price.observed_at <= observed_at
                && observed_at >= price.effective_from
                && observed_at < price.effective_until
        })
    {
        let fact = crate::price_head_index_key(price.kind, &price.interpretation_rule_revision);
        let replace = price_heads
            .get(&fact)
            .is_none_or(|current: &&EconomyPriceObservationV1| {
                (current.observed_at, &current.id) < (price.observed_at, &price.id)
            });
        if replace {
            price_heads.insert(fact, price);
        }
    }
    if price_heads.len() > crate::MAX_PRICE_FACTORS {
        return Err(CanwuError::new(
            ErrorCode::QueryBudgetExceeded,
            "economy price observation adapter exceeded its factor budget",
        ));
    }
    let mut prices = price_heads.into_values().cloned().collect::<Vec<_>>();
    prices.sort_by(|left, right| {
        left.kind.cmp(&right.kind).then_with(|| {
            left.interpretation_rule_revision
                .cmp(&right.interpretation_rule_revision)
        })
    });
    let mut witness = EconomyObservationWitnessV1 {
        provider_plugin: crate::PLUGIN_NAME.to_owned(),
        provider_version: env!("CARGO_PKG_VERSION").to_owned(),
        provider_semantic_hash: crate::ECONOMY_SEMANTIC_HASH.to_owned(),
        provider_state_revision: state.revision,
        holder: holder.clone(),
        scope: scope.clone(),
        observed_at,
        materialized_at: canwu.time(),
        confidence_per_mille: grant.confidence_per_mille,
        head,
        routes,
        prices,
        price_applicability: profile.price_applicability,
        profile_revision: format!("{}@{}", profile.id, profile.revision),
        compiled_content_hash: profile.compiled_content_hash.clone(),
        adapter_revision: "canwu.economy.observation-adapter.v1".to_owned(),
        canonical_digest: String::new(),
    };
    witness.canonical_digest =
        canwu_api::canonical_hash("canwu.economy.observation-witness.v1", &witness)?;
    Ok(witness)
}

fn latest_observation_head_at(
    history: &BTreeMap<crate::EconomyObservationTemporalKeyV1, EconomyObservationHeadV1>,
    observed_at: SimTime,
) -> Option<&EconomyObservationHeadV1> {
    history
        .iter()
        .rev()
        .find_map(|(key, head)| (key.observed_at <= observed_at).then_some(head))
}

#[cfg(test)]
fn latest_fact_at<T>(
    history: &BTreeMap<crate::EconomyFactTemporalKeyV1, T>,
    observed_at: SimTime,
) -> Option<&T> {
    history
        .iter()
        .rev()
        .find_map(|(key, fact)| (key.observed_at <= observed_at).then_some(fact))
}

fn unavailable(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::DomainRecordNotFound, message)
}

fn invalid(message: impl Into<String>) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidDomainRecord, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EconomyFactTemporalKeyV1, EconomyObservationTemporalKeyV1};

    #[test]
    fn delayed_cut_falls_back_to_the_latest_fact_not_after_the_cut() {
        let history = BTreeMap::from([
            (
                EconomyFactTemporalKeyV1 {
                    observed_at: SimTime::from_minutes(10),
                    observation_id: "older".to_owned(),
                },
                "reachable",
            ),
            (
                EconomyFactTemporalKeyV1 {
                    observed_at: SimTime::from_minutes(20),
                    observation_id: "newer".to_owned(),
                },
                "blocked",
            ),
        ]);
        assert_eq!(
            latest_fact_at(&history, SimTime::from_minutes(15)),
            Some(&"reachable")
        );
    }

    #[test]
    fn delayed_cut_falls_back_to_the_latest_economy_head_not_after_the_cut() {
        fn head(at: SimTime, digest: &str) -> EconomyObservationHeadV1 {
            EconomyObservationHeadV1 {
                economy: crate::LocalEconomyId::new(format!(
                    "canwu.economy-reference:economy:{digest}"
                ))
                .expect("economy"),
                scope: ResourceScopeId::new("canwu.economy-reference:scope:delayed-cut")
                    .expect("scope"),
                observed_at: at,
                population_wellbeing_per_mille: 700,
                cooperation_per_mille: 600,
                relief_open: false,
                rationed: false,
                requisitioned: false,
                reserve_release_allowed: false,
                source_versions: Vec::new(),
                semantic_digest: digest.repeat(64),
            }
        }
        let older = head(SimTime::from_minutes(10), "a");
        let newer = head(SimTime::from_minutes(20), "b");
        let history = BTreeMap::from([
            (
                EconomyObservationTemporalKeyV1 {
                    observed_at: older.observed_at,
                    canonical_digest: older.semantic_digest.clone(),
                },
                older.clone(),
            ),
            (
                EconomyObservationTemporalKeyV1 {
                    observed_at: newer.observed_at,
                    canonical_digest: newer.semantic_digest.clone(),
                },
                newer,
            ),
        ]);
        assert_eq!(
            latest_observation_head_at(&history, SimTime::from_minutes(15)),
            Some(&older)
        );
    }
}
