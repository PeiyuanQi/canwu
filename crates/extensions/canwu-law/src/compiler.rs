use crate::model::*;
use canwu_api::{CanwuError, ErrorCode, canonical_hash};
use std::collections::{BTreeMap, BTreeSet};

/// Compile authored legal content into a deterministic, immutable plan.
pub fn compile_law(definition: &LegalDefinition) -> Result<CompiledLawPlan, CanwuError> {
    validate_definition(definition)?;
    let mut canonical = definition.clone();
    canonical.orders.sort_by(|a, b| a.id.cmp(&b.id));
    canonical.jurisdictions.sort_by(|a, b| a.id.cmp(&b.id));
    canonical.institutions.sort_by(|a, b| a.id.cmp(&b.id));
    canonical.procedures.sort_by(|a, b| a.id.cmp(&b.id));
    canonical.clauses.sort_by(|a, b| a.id.cmp(&b.id));
    canonical.source_profiles.sort_by(|a, b| a.id.cmp(&b.id));
    canonical
        .signal_providers
        .sort_by(|a, b| a.signal_kind.cmp(&b.signal_kind));
    canonical
        .applicability_profiles
        .sort_by(|a, b| a.id.cmp(&b.id));
    canonical.predicates.sort_by(|a, b| a.id.cmp(&b.id));
    canonical.forums.sort_by(|a, b| a.id.cmp(&b.id));
    canonical
        .precedence_profiles
        .sort_by(|a, b| a.id.cmp(&b.id));
    for institution in &mut canonical.institutions {
        institution.jurisdictions.sort();
        institution.jurisdictions.dedup();
        institution.procedures.sort();
        institution.procedures.dedup();
        institution.seats.sort_by(|a, b| a.id.cmp(&b.id));
        for competence in &mut institution.competences {
            competence.legal_orders.sort();
            competence.legal_orders.dedup();
            competence.jurisdictions.sort();
            competence.jurisdictions.dedup();
            competence.subject_matters.sort();
            competence.subject_matters.dedup();
            competence.source_modes.sort_by_key(|mode| *mode as u8);
            competence.source_modes.dedup();
            competence
                .operations
                .sort_by_key(|operation| *operation as u8);
            competence.operations.dedup();
            competence.procedures.sort();
            competence.procedures.dedup();
            competence.forums.sort();
            competence.forums.dedup();
        }
        institution.competences.sort_by(|left, right| {
            (
                &left.legal_orders,
                &left.jurisdictions,
                &left.subject_matters,
                &left.source_modes,
                &left.operations,
                &left.procedures,
                &left.forums,
                left.can_adjudicate,
            )
                .cmp(&(
                    &right.legal_orders,
                    &right.jurisdictions,
                    &right.subject_matters,
                    &right.source_modes,
                    &right.operations,
                    &right.procedures,
                    &right.forums,
                    right.can_adjudicate,
                ))
        });
        institution.competences.dedup();
    }
    for procedure in &mut canonical.procedures {
        for stage in &mut procedure.stages {
            stage.seats.sort();
            stage.seats.dedup();
            stage.allowed_ballots.sort();
            stage.allowed_ballots.dedup();
        }
    }
    for clause in &mut canonical.clauses {
        clause.operation_kinds.sort();
        clause.operation_kinds.dedup();
    }
    for profile in &mut canonical.source_profiles {
        profile.required_signal_kinds.sort();
        profile.required_signal_kinds.dedup();
    }
    for jurisdiction in &mut canonical.jurisdictions {
        jurisdiction
            .relations
            .sort_by_key(|r| (r.from.clone(), r.to.clone(), r.kind));
    }
    for profile in &mut canonical.applicability_profiles {
        profile.jurisdiction_traversal.sort();
        profile.jurisdiction_traversal.dedup();
    }
    for forum in &mut canonical.forums {
        forum.legal_orders.sort();
        forum.legal_orders.dedup();
        forum.subject_matters.sort();
        forum.subject_matters.dedup();
        forum.institutions.sort();
        forum.institutions.dedup();
        forum.proof_profiles.sort();
        forum.proof_profiles.dedup();
        forum.standing_profiles.sort();
        forum.standing_profiles.dedup();
        forum.remedy_profiles.sort();
        forum.remedy_profiles.dedup();
        forum.precedent_profiles.sort();
        forum.precedent_profiles.dedup();
    }
    let jurisdiction_adjacency_by_profile = canonical
        .applicability_profiles
        .iter()
        .map(|profile| {
            let mut adjacency = BTreeMap::<String, Vec<String>>::new();
            for relation in canonical
                .jurisdictions
                .iter()
                .flat_map(|jurisdiction| &jurisdiction.relations)
            {
                for traversal in profile
                    .jurisdiction_traversal
                    .iter()
                    .filter(|traversal| traversal.kind == relation.kind)
                {
                    let (from, to) = match traversal.direction {
                        RelationTraversalDirection::Forward => (&relation.from, &relation.to),
                        RelationTraversalDirection::Reverse => (&relation.to, &relation.from),
                    };
                    adjacency.entry(from.clone()).or_default().push(to.clone());
                }
            }
            for targets in adjacency.values_mut() {
                targets.sort();
                targets.dedup();
            }
            (profile.id.clone(), adjacency)
        })
        .collect();
    let mut seat_authority_by_procedure = BTreeMap::new();
    for procedure in &canonical.procedures {
        let mut authorities = BTreeMap::new();
        for seat_id in procedure
            .stages
            .iter()
            .flat_map(|stage| &stage.seats)
            .collect::<BTreeSet<_>>()
        {
            let (institution, seat) = resolve_procedure_seat(&canonical, &procedure.id, seat_id)?;
            authorities.insert(
                seat_id.clone(),
                CompiledProcedureSeatAuthority {
                    institution: institution.id.clone(),
                    seat: seat.id.clone(),
                    holder: seat.holder.clone(),
                    permission_profile: seat.permission_profile.clone(),
                    decision_controller_id: crate::runtime::decision_controller_id(
                        &institution.id,
                        &seat.id,
                    ),
                },
            );
        }
        seat_authority_by_procedure.insert(procedure.id.clone(), authorities);
    }
    let content_hash = canonical_hash(LAW_PLAN_HASH_DOMAIN, &canonical)?;

    let mut order_by_id = BTreeMap::new();
    let orders = canonical
        .orders
        .into_iter()
        .enumerate()
        .map(|(i, order)| {
            order_by_id.insert(order.id.clone(), DenseKey::from_raw(i as u32));
            CompiledLegalOrder {
                key: DenseKey::from_raw(i as u32),
                source_id: order.id,
                precedence_profile: order.precedence_profile,
            }
        })
        .collect();
    let mut jurisdiction_by_id = BTreeMap::new();
    let jurisdictions = canonical
        .jurisdictions
        .into_iter()
        .enumerate()
        .map(|(i, j)| {
            jurisdiction_by_id.insert(j.id.clone(), DenseKey::from_raw(i as u32));
            CompiledJurisdiction {
                key: DenseKey::from_raw(i as u32),
                source_id: j.id,
                relations: j.relations,
                metadata: j.metadata,
            }
        })
        .collect();
    let mut procedure_by_id = BTreeMap::new();
    let procedures = canonical
        .procedures
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            procedure_by_id.insert(p.id.clone(), DenseKey::from_raw(i as u32));
            CompiledProcedure {
                key: DenseKey::from_raw(i as u32),
                source_id: p.id,
                stages: p.stages,
                deterministic_tie_break: p.deterministic_tie_break,
                reservation_pool: p.reservation_pool,
                reservation_quantity: p.reservation_quantity,
            }
        })
        .collect();
    let mut source_profile_by_id = BTreeMap::new();
    let source_profiles = canonical
        .source_profiles
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            source_profile_by_id.insert(p.id.clone(), DenseKey::from_raw(i as u32));
            CompiledSourceProfile {
                key: DenseKey::from_raw(i as u32),
                source_id: p.id,
                mode: p.mode,
                procedure: p.procedure,
                applicability_profile: p.applicability_profile,
                origin_policy: p.origin_policy,
                authority_policy: p.authority_policy,
                publicity_policy: p.publicity_policy,
                publicity_signal_kind: p.publicity_signal_kind,
                required_signal_kinds: p.required_signal_kinds,
                min_evidence: p.min_evidence,
                max_evidence: p.max_evidence,
                require_claimant: p.require_claimant,
                allow_retroactive: p.allow_retroactive,
                agreement_namespace: p.agreement_namespace,
                agreement_kind: p.agreement_kind,
                min_agreement_parties: p.min_agreement_parties,
                require_agreement_ratification: p.require_agreement_ratification,
            }
        })
        .collect();
    let signal_provider_by_kind = canonical
        .signal_providers
        .iter()
        .cloned()
        .map(|provider| (provider.signal_kind.clone(), provider))
        .collect();
    let mut predicate_by_id = BTreeMap::new();
    for (i, predicate) in canonical.predicates.iter().enumerate() {
        predicate_by_id.insert(predicate.id.clone(), DenseKey::from_raw(i as u32));
    }
    let mut forum_by_id = BTreeMap::new();
    for (i, forum) in canonical.forums.iter().enumerate() {
        forum_by_id.insert(forum.id.clone(), DenseKey::from_raw(i as u32));
    }
    let mut precedence_profile_by_id = BTreeMap::new();
    for (i, profile) in canonical.precedence_profiles.iter().enumerate() {
        precedence_profile_by_id.insert(profile.id.clone(), DenseKey::from_raw(i as u32));
    }
    let mut institution_by_id = BTreeMap::new();
    for (i, institution) in canonical.institutions.iter().enumerate() {
        institution_by_id.insert(institution.id.clone(), DenseKey::from_raw(i as u32));
    }
    let plan = CompiledLawPlan {
        plan_version: LAW_PLAN_VERSION,
        definition_id: definition.id.clone(),
        content_hash,
        budgets: definition.budgets.clone(),
        id_blocks: definition.id_blocks.clone(),
        orders,
        jurisdictions,
        institutions: canonical.institutions,
        institution_by_id,
        procedures,
        clauses: canonical.clauses,
        source_profiles,
        signal_provider_by_kind,
        applicability_profiles: canonical.applicability_profiles,
        predicates: canonical.predicates,
        forums: canonical.forums,
        precedence_profiles: canonical.precedence_profiles,
        order_by_id,
        jurisdiction_by_id,
        procedure_by_id,
        source_profile_by_id,
        predicate_by_id,
        forum_by_id,
        precedence_profile_by_id,
        jurisdiction_adjacency_by_profile,
        seat_authority_by_procedure,
    };
    Ok(plan)
}

/// Rebuilds a compiled plan from its semantic content and requires exact equality.
///
/// This is the cold-load defense against tampered dense keys, reverse indexes,
/// controller bindings, plan versions, and content hashes.
pub fn validate_compiled_law_plan(plan: &CompiledLawPlan) -> Result<(), CanwuError> {
    let definition = LegalDefinition {
        id: plan.definition_id.clone(),
        orders: plan
            .orders
            .iter()
            .map(|order| LegalOrderDefinition {
                id: order.source_id.clone(),
                precedence_profile: order.precedence_profile.clone(),
            })
            .collect(),
        jurisdictions: plan
            .jurisdictions
            .iter()
            .map(|jurisdiction| LegalJurisdictionDefinition {
                id: jurisdiction.source_id.clone(),
                relations: jurisdiction.relations.clone(),
                metadata: jurisdiction.metadata.clone(),
            })
            .collect(),
        institutions: plan.institutions.clone(),
        procedures: plan
            .procedures
            .iter()
            .map(|procedure| ProcedureProfileDefinition {
                id: procedure.source_id.clone(),
                stages: procedure.stages.clone(),
                deterministic_tie_break: procedure.deterministic_tie_break.clone(),
                reservation_pool: procedure.reservation_pool.clone(),
                reservation_quantity: procedure.reservation_quantity,
            })
            .collect(),
        clauses: plan.clauses.clone(),
        source_profiles: plan
            .source_profiles
            .iter()
            .map(|profile| LegalSourceProfileDefinition {
                id: profile.source_id.clone(),
                mode: profile.mode,
                procedure: profile.procedure.clone(),
                applicability_profile: profile.applicability_profile.clone(),
                origin_policy: profile.origin_policy,
                authority_policy: profile.authority_policy,
                publicity_policy: profile.publicity_policy,
                publicity_signal_kind: profile.publicity_signal_kind.clone(),
                required_signal_kinds: profile.required_signal_kinds.clone(),
                min_evidence: profile.min_evidence,
                max_evidence: profile.max_evidence,
                require_claimant: profile.require_claimant,
                allow_retroactive: profile.allow_retroactive,
                agreement_namespace: profile.agreement_namespace.clone(),
                agreement_kind: profile.agreement_kind.clone(),
                min_agreement_parties: profile.min_agreement_parties,
                require_agreement_ratification: profile.require_agreement_ratification,
            })
            .collect(),
        signal_providers: plan.signal_provider_by_kind.values().cloned().collect(),
        applicability_profiles: plan.applicability_profiles.clone(),
        predicates: plan.predicates.clone(),
        forums: plan.forums.clone(),
        precedence_profiles: plan.precedence_profiles.clone(),
        id_blocks: plan.id_blocks.clone(),
        budgets: plan.budgets.clone(),
    };
    let rebuilt = compile_law(&definition)?;
    if &rebuilt != plan {
        return Err(invalid(
            "compiled legal plan does not match its canonical semantic content",
        ));
    }
    Ok(())
}

pub fn validate_definition(definition: &LegalDefinition) -> Result<(), CanwuError> {
    if !canonical_id(&definition.id) {
        return Err(invalid("legal definition ID is not canonical"));
    }
    check_ids(&definition.orders, |x| &x.id, "legal order")?;
    check_ids(&definition.jurisdictions, |x| &x.id, "jurisdiction")?;
    check_ids(&definition.institutions, |x| &x.id, "institution")?;
    check_ids(&definition.procedures, |x| &x.id, "procedure")?;
    check_ids(&definition.clauses, |x| &x.id, "clause")?;
    check_ids(&definition.source_profiles, |x| &x.id, "source profile")?;
    check_ids(
        &definition.signal_providers,
        |provider| &provider.signal_kind,
        "legal signal provider",
    )?;
    check_ids(
        &definition.applicability_profiles,
        |x| &x.id,
        "applicability profile",
    )?;
    check_ids(&definition.predicates, |x| &x.id, "legal predicate")?;
    check_ids(&definition.forums, |x| &x.id, "legal forum profile")?;
    check_ids(
        &definition.precedence_profiles,
        |x| &x.id,
        "legal precedence profile",
    )?;
    if definition.orders.len() > definition.budgets.max_orders
        || definition.jurisdictions.len() > definition.budgets.max_jurisdictions
        || definition.institutions.len() > definition.budgets.max_institutions
        || definition.procedures.len() > definition.budgets.max_procedures
        || definition.clauses.len() > definition.budgets.max_rules
        || definition.source_profiles.len() > definition.budgets.max_sources
        || definition.signal_providers.len() > definition.budgets.max_sources
        || definition.applicability_profiles.len() > definition.budgets.max_jurisdictions
        || definition.predicates.len() > definition.budgets.max_rules
        || definition.forums.len() > definition.budgets.max_jurisdictions
        || definition.precedence_profiles.len() > definition.budgets.max_orders
    {
        return Err(invalid("legal authoring collection budget exceeded"));
    }
    if definition.budgets.max_mutations_per_boundary == 0
        || definition.budgets.max_clauses_per_proposal == 0
        || definition.budgets.max_jurisdictions_per_proposal == 0
        || definition.budgets.max_nested_items_per_record == 0
        || definition.budgets.max_applicability_entries_per_boundary == 0
        || definition.budgets.max_applicability_query_work == 0
        || definition.budgets.max_retirement_dependency_records == 0
        || definition.budgets.max_text_bytes == 0
        || definition.budgets.max_state_bytes == 0
        || definition.budgets.max_memory_bytes == 0
        || definition.budgets.max_state_bytes > MAX_LEGAL_STATE_BYTES
        || definition.budgets.max_memory_bytes > MAX_LEGAL_MEMORY_BYTES
    {
        return Err(invalid("legal budgets must be greater than zero"));
    }
    let authored_value = serde_json::to_value(definition)
        .map_err(|error| invalid(format!("legal definition cannot be encoded: {error}")))?;
    validate_text_budget(&authored_value, definition.budgets.max_text_bytes)?;
    definition
        .id_blocks
        .decision_tickets
        .validate("decision ticket")?;
    definition
        .id_blocks
        .decision_requests
        .validate("decision request")?;
    definition
        .id_blocks
        .command_requests
        .validate("command request")?;
    validate_id_blocks(definition)?;
    let order_ids: BTreeSet<_> = definition.orders.iter().map(|x| x.id.as_str()).collect();
    let jurisdiction_ids: BTreeSet<_> = definition
        .jurisdictions
        .iter()
        .map(|x| x.id.as_str())
        .collect();
    let procedure_ids: BTreeSet<_> = definition
        .procedures
        .iter()
        .map(|x| x.id.as_str())
        .collect();
    let applicability_ids: BTreeSet<_> = definition
        .applicability_profiles
        .iter()
        .map(|x| x.id.as_str())
        .collect();
    let forum_ids: BTreeSet<_> = definition.forums.iter().map(|x| x.id.as_str()).collect();
    let institution_ids: BTreeSet<_> = definition
        .institutions
        .iter()
        .map(|x| x.id.as_str())
        .collect();
    let precedence_ids: BTreeSet<_> = definition
        .precedence_profiles
        .iter()
        .map(|x| x.id.as_str())
        .collect();
    if definition
        .orders
        .iter()
        .any(|order| !precedence_ids.contains(order.precedence_profile.as_str()))
    {
        return Err(invalid(
            "legal order references an unknown precedence profile",
        ));
    }
    for jurisdiction in &definition.jurisdictions {
        for relation in &jurisdiction.relations {
            if relation.from != jurisdiction.id || !jurisdiction_ids.contains(relation.to.as_str())
            {
                return Err(invalid(format!(
                    "jurisdiction relation {} -> {} is dangling",
                    relation.from, relation.to
                )));
            }
        }
    }
    validate_acyclic_relations(definition)?;
    for institution in &definition.institutions {
        if institution.seats.len() > definition.budgets.max_seats_per_procedure {
            return Err(invalid("institution seat budget exceeded"));
        }
        for id in &institution.jurisdictions {
            if !jurisdiction_ids.contains(id.as_str()) {
                return Err(invalid(format!(
                    "institution {} references unknown jurisdiction {id}",
                    institution.id
                )));
            }
        }
        for id in &institution.procedures {
            if !procedure_ids.contains(id.as_str()) {
                return Err(invalid(format!(
                    "institution {} references unknown procedure {id}",
                    institution.id
                )));
            }
        }
        check_ids(&institution.seats, |x| &x.id, "authority seat")?;
        if institution.competences.is_empty() {
            return Err(invalid(format!(
                "institution {} has no compiled legal competence",
                institution.id
            )));
        }
        for competence in &institution.competences {
            if competence.legal_orders.is_empty()
                || competence.jurisdictions.is_empty()
                || competence.subject_matters.is_empty()
                || competence.source_modes.is_empty()
                || competence.operations.is_empty()
                || competence.procedures.is_empty()
                || competence
                    .legal_orders
                    .iter()
                    .any(|id| id != "*" && !order_ids.contains(id.as_str()))
                || competence
                    .jurisdictions
                    .iter()
                    .any(|id| id != "*" && !jurisdiction_ids.contains(id.as_str()))
                || competence
                    .procedures
                    .iter()
                    .any(|id| id != "*" && !procedure_ids.contains(id.as_str()))
                || competence
                    .forums
                    .iter()
                    .any(|id| id != "*" && !forum_ids.contains(id.as_str()))
            {
                return Err(invalid(format!(
                    "institution {} has an invalid legal competence scope",
                    institution.id
                )));
            }
        }
    }
    for procedure in &definition.procedures {
        if procedure.stages.is_empty()
            || procedure.stages.len() > definition.budgets.max_stages_per_procedure
        {
            return Err(invalid(format!(
                "procedure {} has invalid stage count",
                procedure.id
            )));
        }
        if procedure.deterministic_tie_break.is_empty() {
            return Err(invalid(format!(
                "procedure {} has no tie-break",
                procedure.id
            )));
        }
        let mut stage_ids = BTreeSet::new();
        for stage in &procedure.stages {
            if !canonical_id(&stage.id) || !stage_ids.insert(stage.id.clone()) {
                return Err(invalid(format!(
                    "procedure {} has duplicate stage",
                    procedure.id
                )));
            }
            if stage.quorum > 1000 || stage.threshold > 1000 || stage.deadline_minutes < 0 {
                return Err(invalid(format!(
                    "procedure {} stage {} has invalid threshold",
                    procedure.id, stage.id
                )));
            }
            if stage.allowed_ballots.is_empty()
                || !stage.allowed_ballots.contains(&Ballot::For)
                || (stage.kind != ProcedureStageKind::Veto
                    && stage.allowed_ballots.contains(&Ballot::Veto))
            {
                return Err(invalid(format!(
                    "procedure {} stage {} has invalid ballot permissions",
                    procedure.id, stage.id
                )));
            }
            if stage.seats.len() > definition.budgets.max_seats_per_procedure {
                return Err(invalid("procedure seat budget exceeded"));
            }
            for seat in &stage.seats {
                resolve_procedure_seat(definition, &procedure.id, seat)?;
            }
        }
        if procedure.reservation_quantity > 0
            && procedure
                .reservation_pool
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(invalid("procedure reservation quantity requires a pool"));
        }
    }
    for clause in &definition.clauses {
        if clause.schema.is_empty()
            || clause.operation_kinds.len() > definition.budgets.max_stages_per_procedure
        {
            return Err(invalid(format!("clause {} has invalid schema", clause.id)));
        }
    }
    for predicate in &definition.predicates {
        if predicate.knowledge_schema.is_some() != predicate.payload_pointer.is_some()
            || predicate
                .payload_pointer
                .as_deref()
                .is_some_and(|pointer| !pointer.starts_with('/'))
        {
            return Err(invalid(format!(
                "predicate {} has an invalid knowledge binding",
                predicate.id
            )));
        }
    }
    for profile in &definition.source_profiles {
        if let Some(procedure) = &profile.procedure {
            if !procedure_ids.contains(procedure.as_str()) {
                return Err(invalid(format!(
                    "source profile {} references unknown procedure",
                    profile.id
                )));
            }
        }
        if !applicability_ids.contains(profile.applicability_profile.as_str()) {
            return Err(invalid(format!(
                "source profile {} references unknown applicability profile",
                profile.id
            )));
        }
        let expected_origin = match profile.mode {
            SourceMode::Promulgated | SourceMode::Accreted => SourceOriginPolicy::NoOrigin,
            SourceMode::Adjudicated => SourceOriginPolicy::Ruling,
            SourceMode::Agreed => SourceOriginPolicy::Agreement,
            SourceMode::Received => SourceOriginPolicy::Reception,
        };
        if profile.origin_policy != expected_origin
            || profile.authority_policy
                != if profile.procedure.is_some() {
                    SourceAuthorityPolicy::ProceduralInstitution
                } else {
                    SourceAuthorityPolicy::EvidenceClaim
                }
            || profile.min_evidence > profile.max_evidence
            || profile.max_evidence > definition.budgets.max_evidence_per_record
            || (profile.mode == SourceMode::Promulgated && profile.procedure.is_none())
            || (profile.publicity_policy == PublicityPolicy::NotRequired
                && profile.publicity_signal_kind.is_some())
            || (profile.publicity_policy != PublicityPolicy::NotRequired
                && profile
                    .publicity_signal_kind
                    .as_ref()
                    .is_none_or(String::is_empty))
            || profile.publicity_signal_kind.as_ref().is_some_and(|kind| {
                !definition
                    .signal_providers
                    .iter()
                    .any(|provider| &provider.signal_kind == kind)
            })
        {
            return Err(invalid("legal source admission contract is invalid"));
        }
        let agreement_contract = (
            profile.agreement_namespace.as_deref(),
            profile.agreement_kind.as_deref(),
            profile.min_agreement_parties,
            profile.require_agreement_ratification,
        );
        if match profile.mode {
            SourceMode::Agreed => {
                agreement_contract
                    .0
                    .is_none_or(|value| !canonical_id(value))
                    || agreement_contract
                        .1
                        .is_none_or(|value| !canonical_id(value))
                    || agreement_contract.2 < 2
                    || !agreement_contract.3
            }
            _ => agreement_contract != (None, None, 0, false),
        } {
            return Err(invalid("legal agreement source contract is invalid"));
        }
        if profile.required_signal_kinds.iter().any(|kind| {
            !definition
                .signal_providers
                .iter()
                .any(|provider| &provider.signal_kind == kind)
        }) {
            return Err(invalid(format!(
                "source profile {} requires an unbound signal kind",
                profile.id
            )));
        }
    }
    for provider in &definition.signal_providers {
        if !canonical_id(&provider.plugin) || !canonical_id(&provider.packet_type) {
            return Err(invalid(format!(
                "signal provider {} has an invalid plugin or packet type",
                provider.signal_kind
            )));
        }
    }
    for profile in &definition.applicability_profiles {
        let pipeline = profile
            .pipeline
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !order_ids.contains(profile.legal_order.as_str())
            || pipeline != ["scope", "jurisdiction", "validity", "conflict"]
            || !matches!(
                profile.temporal_conflict_rule.as_str(),
                "later-in-time" | "later-valid-source"
            )
            || profile.max_candidates == 0
            || profile.jurisdiction_traversal.iter().any(|traversal| {
                !matches!(
                    traversal.kind,
                    JurisdictionRelationKind::Delegation
                        | JurisdictionRelationKind::TerritorialContainment
                )
            })
        {
            return Err(invalid(format!(
                "applicability profile {} is invalid",
                profile.id
            )));
        }
    }
    for forum in &definition.forums {
        if !jurisdiction_ids.contains(forum.jurisdiction.as_str())
            || forum.legal_orders.is_empty()
            || forum.subject_matters.is_empty()
            || forum.institutions.is_empty()
            || forum.proof_profiles.is_empty()
            || forum.standing_profiles.is_empty()
            || forum.remedy_profiles.is_empty()
            || forum.precedent_profiles.is_empty()
            || forum
                .legal_orders
                .iter()
                .any(|id| id != "*" && !order_ids.contains(id.as_str()))
            || forum
                .institutions
                .iter()
                .any(|id| !institution_ids.contains(id.as_str()))
        {
            return Err(invalid(format!(
                "legal forum profile {} is invalid",
                forum.id
            )));
        }
    }
    for profile in &definition.precedence_profiles {
        let unique = profile
            .ordered_bases
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if profile.ordered_bases.is_empty()
            || unique.len() != profile.ordered_bases.len()
            || profile
                .ordered_bases
                .iter()
                .position(|basis| *basis == ConflictResolutionBasis::Temporal)
                .is_some_and(|position| position + 1 != profile.ordered_bases.len())
        {
            return Err(invalid(format!(
                "legal precedence profile {} is invalid",
                profile.id
            )));
        }
    }
    let estimate = serde_json::to_vec(definition)
        .map_err(|e| invalid(format!("legal definition cannot be measured: {e}")))?
        .len()
        .saturating_mul(2)
        .saturating_add(definition.orders.len().saturating_mul(192))
        .saturating_add(definition.jurisdictions.len().saturating_mul(256))
        .saturating_add(definition.procedures.len().saturating_mul(512));
    if estimate > definition.budgets.max_memory_bytes {
        return Err(invalid(format!(
            "compiled legal plan exceeds memory budget: {estimate} > {}",
            definition.budgets.max_memory_bytes
        )));
    }
    Ok(())
}

fn resolve_procedure_seat<'a>(
    definition: &'a LegalDefinition,
    procedure_id: &str,
    seat_id: &str,
) -> Result<(&'a LegalInstitutionDefinition, &'a AuthoritySeatDefinition), CanwuError> {
    let mut matches = definition.institutions.iter().filter_map(|institution| {
        institution
            .procedures
            .iter()
            .any(|id| id == procedure_id)
            .then(|| {
                institution
                    .seats
                    .iter()
                    .find(|seat| seat.id == seat_id)
                    .map(|seat| (institution, seat))
            })
            .flatten()
    });
    let binding = matches.next().ok_or_else(|| {
        invalid(format!(
            "procedure {procedure_id} seat {seat_id} has no institution binding"
        ))
    })?;
    if matches.next().is_some() {
        return Err(invalid(format!(
            "procedure {procedure_id} seat {seat_id} has multiple institution bindings"
        )));
    }
    Ok(binding)
}

fn validate_id_blocks(definition: &LegalDefinition) -> Result<(), CanwuError> {
    let blocks = [
        ("decision ticket", &definition.id_blocks.decision_tickets),
        ("decision request", &definition.id_blocks.decision_requests),
        ("command request", &definition.id_blocks.command_requests),
    ];
    for (index, (label, block)) in blocks.iter().enumerate() {
        let end = block
            .start
            .checked_add(block.capacity)
            .ok_or_else(|| invalid(format!("{label} ID block overflows")))?;
        for (other_label, other) in blocks.iter().skip(index + 1) {
            let other_end = other
                .start
                .checked_add(other.capacity)
                .ok_or_else(|| invalid(format!("{other_label} ID block overflows")))?;
            if block.start < other_end && other.start < end {
                return Err(invalid(format!(
                    "{label} and {other_label} ID blocks overlap"
                )));
            }
        }
    }
    let outbox = u64::try_from(definition.budgets.max_outbox)
        .map_err(|_| invalid("outbox budget does not fit the ID allocator"))?;
    let decision_requests = outbox
        .checked_mul(3)
        .ok_or_else(|| invalid("outbox decision-request budget overflows"))?;
    if definition.id_blocks.decision_tickets.capacity < outbox
        || definition.id_blocks.command_requests.capacity < outbox
        || definition.id_blocks.decision_requests.capacity < decision_requests
    {
        return Err(invalid(
            "legal ID blocks do not cover the declared lifetime outbox budget",
        ));
    }
    Ok(())
}

fn validate_text_budget(value: &serde_json::Value, max: usize) -> Result<(), CanwuError> {
    match value {
        serde_json::Value::String(text) if text.len() > max => {
            Err(invalid("legal authoring text budget exceeded"))
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_text_budget(value, max)?;
            }
            Ok(())
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if key.len() > max {
                    return Err(invalid("legal authoring text budget exceeded"));
                }
                validate_text_budget(value, max)?;
            }
            Ok(())
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => Ok(()),
    }
}

fn validate_acyclic_relations(definition: &LegalDefinition) -> Result<(), CanwuError> {
    let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for jurisdiction in &definition.jurisdictions {
        for relation in &jurisdiction.relations {
            if matches!(
                relation.kind,
                JurisdictionRelationKind::Supremacy | JurisdictionRelationKind::Appeal
            ) {
                edges
                    .entry(relation.from.as_str())
                    .or_default()
                    .push(relation.to.as_str());
            }
        }
    }
    fn visit<'a>(
        node: &'a str,
        edges: &BTreeMap<&'a str, Vec<&'a str>>,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> bool {
        if visiting.contains(node) {
            return false;
        }
        if !visited.insert(node) {
            return true;
        }
        visiting.insert(node);
        let acyclic = edges.get(node).is_none_or(|children| {
            children
                .iter()
                .all(|child| visit(child, edges, visiting, visited))
        });
        visiting.remove(node);
        acyclic
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    if edges
        .keys()
        .all(|node| visit(node, &edges, &mut visiting, &mut visited))
    {
        Ok(())
    } else {
        Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "supremacy and appeal jurisdiction relations must be acyclic",
        ))
    }
}
