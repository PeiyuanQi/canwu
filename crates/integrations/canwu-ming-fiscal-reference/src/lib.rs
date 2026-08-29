//! Ready-to-run integration of Canwu's Ming fiscal reference content.

#![allow(clippy::missing_errors_doc)]

mod adapter;
mod trace;

pub use adapter::{
    MingFiscalExecutionAdapterPlugin, ReferenceFiscalExecutionEvidence,
    ReferenceFiscalExecutionEvidenceRecord, enqueue_reference_execution_result,
    reference_execution_evidence_ref, reference_execution_evidence_version,
};
pub use trace::{
    DEFAULT_TRACE_DIRECTORY, MingFiscalTraceCounts, MingFiscalTraceFiscalState,
    MingFiscalTraceFrame, MingFiscalTraceManifest, MingFiscalTracePaths, MingFiscalTracePhase,
    MingFiscalTraceWriter, TRACE_FORMAT_VERSION, TRACE_MANIFEST_FILE, TRACE_STEPS_FILE,
    TraceDumpError, capture_ming_fiscal_trace_frame, default_trace_directory, trace_error,
};

use canwu_api::{
    Canwu, CanwuError, CommandEnvelope, CommandRequest, CommandRequestId, DomainRecordKind,
    EntityRef, Issuer, OrganizationId, PersonId, ResourceId, Scenario, SimDuration, SimTime,
    SimulationGranularity,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionRequest, FiscalAdoptionState, FiscalAuthorityBinding,
    FiscalContentSelection, FiscalExecutionKind, FiscalExternalOperationRef, FiscalObserverBinding,
    FiscalPaymentForm, FiscalReceiptDisposition, FiscalScopeBinding, FiscalState,
    compute_aggregates, compute_transition_candidates, enqueue_execution_receipt,
    fiscal_action_command, fiscal_catalog_reference, fiscal_state_reference, recompute_derived,
};
use canwu_ming_fiscal::{MingFiscalFixture, compile_ming_fiscal, ming_fiscal_fixture};
use canwu_reference_world::{ReferenceWorldIds, ReferenceWorldPlugin, demo_scenario};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_SEED: u64 = 0x4d49_4e47;

#[derive(Clone, Debug)]
pub struct MingFiscalReferenceScenario {
    pub scenario: Scenario,
    pub world_ids: ReferenceWorldIds,
    pub fixture: MingFiscalFixture,
}

pub fn ming_fiscal_reference_scenario(
    fixture_id: &str,
) -> Result<MingFiscalReferenceScenario, CanwuError> {
    let fixture = ming_fiscal_fixture(fixture_id).map_err(|error| {
        CanwuError::new(
            canwu_api::ErrorCode::InvalidPayload,
            format!("Ming fiscal fixture could not be decoded: {error}"),
        )
    })?;
    let catalog = compile_ming_fiscal(FiscalContentSelection {
        historical_year: fixture.historical_year,
        region_ids: fixture.region_ids.clone(),
        ..FiscalContentSelection::default()
    })?;
    let (mut scenario, world_ids) = demo_scenario()?;
    scenario
        .entities
        .push(EntityRef::Resource(ResourceId::new(1)));
    let mut state = FiscalState::new(fixture.historical_year, fixture.mode, SimTime::EPOCH);
    state
        .execution_evidence_kinds
        .insert(DomainRecordKind::for_type::<
            ReferenceFiscalExecutionEvidenceRecord,
        >());
    let institutions = reference_institutions(&fixture.id, &world_ids);
    for institution in institutions.values() {
        if let EntityRef::Organization(id) = &institution.entity {
            scenario.entities.push(EntityRef::Organization(*id));
        }
        state.authority_bindings.insert(
            institution.authority_id.to_owned(),
            FiscalAuthorityBinding {
                id: institution.authority_id.to_owned(),
                institution: institution.entity.clone(),
                authorized_actor: Some(institution.actor),
            },
        );
    }

    for adoption in &fixture.adoptions {
        let rule = catalog.rules.get(&adoption.rule_id).ok_or_else(|| {
            CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!(
                    "fixture adoption references unknown rule {}",
                    adoption.rule_id
                ),
            )
        })?;
        let jurisdiction_id = jurisdiction_from_scope(&adoption.scope_id)?;
        let institution = institution_for_scope(&adoption.scope_id, &institutions)?;
        state
            .scope_bindings
            .entry(adoption.scope_id.clone())
            .or_insert(FiscalScopeBinding {
                id: adoption.scope_id.clone(),
                institution,
                jurisdiction_id,
                subject_scope: rule.subject_scope.clone(),
                mechanism: rule.mechanism,
                authoritative_granularity: SimulationGranularity::Aggregate,
            });
        state.adoptions.insert(
            adoption.adoption_id.clone(),
            FiscalAdoptionState {
                id: adoption.adoption_id.clone(),
                rule_id: adoption.rule_id.clone(),
                scope_binding_id: adoption.scope_id.clone(),
                stage: adoption.stage,
                generation: 1,
                changed_at: SimTime::EPOCH,
                source_action_id: None,
            },
        );
    }
    state.observer_bindings.insert(
        "observer.revenue-minister".to_owned(),
        FiscalObserverBinding {
            id: "observer.revenue-minister".to_owned(),
            actor: world_ids.observer,
            knowledge_holder: canwu_api::KnowledgeHolderRef::Person(world_ids.observer),
            visible_institutions: state
                .scope_bindings
                .values()
                .map(|scope| scope.institution.clone())
                .collect(),
            confidence_per_mille: 850,
        },
    );
    recompute_derived(&mut state, &catalog, SimTime::EPOCH)?;
    state.validate(&catalog)?;
    scenario
        .domain_records
        .extend([catalog.clone().into_record()?, state.into_record(&catalog)?]);
    Ok(MingFiscalReferenceScenario {
        scenario,
        world_ids,
        fixture,
    })
}

#[derive(Clone)]
struct ReferenceInstitution {
    authority_id: &'static str,
    entity: EntityRef,
    actor: PersonId,
}

fn reference_institutions(
    fixture_id: &str,
    world_ids: &ReferenceWorldIds,
) -> BTreeMap<&'static str, ReferenceInstitution> {
    let mut institutions = BTreeMap::from([(
        "court",
        ReferenceInstitution {
            authority_id: "authority.revenue-minister",
            entity: EntityRef::Government(world_ids.government),
            actor: world_ids.observer,
        },
    )]);
    if fixture_id == "hongguang-1644" {
        institutions.extend([
            (
                "commander",
                ReferenceInstitution {
                    authority_id: "authority.field-commander",
                    entity: EntityRef::Army(world_ids.army),
                    actor: world_ids.commander,
                },
            ),
            (
                "regional-treasury",
                ReferenceInstitution {
                    authority_id: "authority.regional-treasury",
                    entity: EntityRef::Organization(OrganizationId::new(1)),
                    actor: world_ids.observer,
                },
            ),
            (
                "salt-administration",
                ReferenceInstitution {
                    authority_id: "authority.salt-administration",
                    entity: EntityRef::Organization(OrganizationId::new(2)),
                    actor: world_ids.observer,
                },
            ),
            (
                "merchant-credit",
                ReferenceInstitution {
                    authority_id: "authority.merchant-credit",
                    entity: EntityRef::Organization(OrganizationId::new(3)),
                    actor: world_ids.observer,
                },
            ),
        ]);
    }
    institutions
}

fn institution_for_scope(
    scope_id: &str,
    institutions: &BTreeMap<&str, ReferenceInstitution>,
) -> Result<EntityRef, CanwuError> {
    let institution_key = if scope_id.contains("commander") {
        "commander"
    } else if scope_has_suffix(scope_id, "treasury") {
        "regional-treasury"
    } else if scope_has_suffix(scope_id, "salt") {
        "salt-administration"
    } else if scope_has_suffix(scope_id, "credit") {
        "merchant-credit"
    } else {
        "court"
    };
    institutions
        .get(institution_key)
        .map(|institution| institution.entity.clone())
        .ok_or_else(|| {
            CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("fiscal scope {scope_id} has no reference institution"),
            )
        })
}

fn scope_has_suffix(scope_id: &str, suffix: &str) -> bool {
    scope_id.rsplit('.').next() == Some(suffix)
}

pub fn new_ming_fiscal_reference(seed: u64, fixture_id: &str) -> Result<Canwu, CanwuError> {
    let reference = ming_fiscal_reference_scenario(fixture_id)?;
    let world = ReferenceWorldPlugin;
    let adapter = MingFiscalExecutionAdapterPlugin;
    let fiscal = ming_fiscal_plugin();
    let canwu = Canwu::new_with_plugins(seed, reference.scenario, &[&world, &adapter, &fiscal])?;
    validate_ming_fiscal_reference(&canwu)?;
    Ok(canwu)
}

pub fn restore_ming_fiscal_reference(json: &str) -> Result<Canwu, CanwuError> {
    let world = ReferenceWorldPlugin;
    let adapter = MingFiscalExecutionAdapterPlugin;
    let fiscal = ming_fiscal_plugin();
    let canwu = Canwu::from_snapshot_json_with_plugins(json, &[&world, &adapter, &fiscal])?;
    validate_ming_fiscal_reference(&canwu)?;
    Ok(canwu)
}

pub fn replay_ming_fiscal_reference(
    journal: &canwu_api::ReplayJournal,
) -> Result<Canwu, CanwuError> {
    let world = ReferenceWorldPlugin;
    let adapter = MingFiscalExecutionAdapterPlugin;
    let fiscal = ming_fiscal_plugin();
    let canwu = Canwu::replay_from_journal(&[&world, &adapter, &fiscal], journal)?;
    validate_ming_fiscal_reference(&canwu)?;
    Ok(canwu)
}

pub fn validate_ming_fiscal_reference(canwu: &Canwu) -> Result<(), CanwuError> {
    let catalog_record = canwu
        .typed_domain_record(&fiscal_catalog_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal catalog is unavailable"))?;
    let catalog = catalog_record.decode_payload::<canwu_fiscal::FiscalCatalogRecord>()?;
    catalog.validate()?;
    let state_record = canwu
        .typed_domain_record(&fiscal_state_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal state is unavailable"))?;
    let state = state_record.decode_payload::<canwu_fiscal::FiscalStateRecord>()?;
    state.validate(&catalog)?;
    state.validate_record_binding(state_record)?;
    if state.aggregates != compute_aggregates(&state, &catalog)? {
        return Err(invalid_reference(
            "Ming fiscal strategic aggregates do not match authoritative procedure state",
        ));
    }
    let candidate_time = state
        .transition_candidates
        .values()
        .map(|candidate| candidate.evaluated_at)
        .max()
        .unwrap_or(state.historical_context.updated_at);
    if state.transition_candidates
        != compute_transition_candidates(&state, &catalog, candidate_time)
    {
        return Err(invalid_reference(
            "Ming fiscal transition candidates do not match authoritative procedure state",
        ));
    }
    for receipt in state.execution_receipts.values() {
        validate_reference_receipt(canwu, &state, receipt)?;
    }
    for assessment in state.assessments.values() {
        if let Some(quote) = &assessment.commutation_quote
            && canwu.domain_record_version(quote).is_none()
        {
            return Err(invalid_reference(
                "Ming fiscal commutation quote payload is unavailable",
            ));
        }
    }
    for audit in state.audits.values() {
        if audit
            .evidence
            .iter()
            .any(|evidence| !canwu.evidence_exists(evidence))
        {
            return Err(invalid_reference(
                "Ming fiscal audit evidence is unavailable",
            ));
        }
    }
    Ok(())
}

pub fn run_ming_fiscal_sample_cycle(canwu: &mut Canwu, id_prefix: &str) -> Result<(), CanwuError> {
    run_ming_fiscal_sample_cycle_with_trace(canwu, id_prefix, |_canwu, _phase, _receipt| Ok(()))
}

pub fn run_ming_fiscal_sample_cycle_with_trace<F>(
    canwu: &mut Canwu,
    id_prefix: &str,
    mut on_boundary: F,
) -> Result<(), CanwuError>
where
    F: FnMut(&Canwu, MingFiscalTracePhase, &canwu_api::BoundaryReceipt) -> Result<(), CanwuError>,
{
    let plan = sample_cycle_plan(canwu, id_prefix)?;
    let SampleCyclePlan {
        actor,
        authority_id,
        institution,
        assessment_id,
        rule_id,
        scope_id,
        period_id,
        payment_form,
    } = plan;
    submit_reference_action_with_trace(
        canwu,
        actor,
        &FiscalActionRequest {
            action_id: format!("{id_prefix}.assess.action"),
            authority_binding_id: authority_id.clone(),
            expected_procedure_revision: fiscal_state_version(canwu)?,
            action: FiscalAction::OpenAssessment {
                assessment_id: assessment_id.clone(),
                rule_id,
                scope_binding_id: scope_id,
                accounting_cycle_id: format!("{period_id}.{id_prefix}"),
                quantity: 100,
                unit: sample_unit(payment_form).to_owned(),
                payment_form,
                commutation_quote: None,
            },
        },
        MingFiscalTracePhase::OpenAssessment,
        &mut on_boundary,
    )?;

    let request_id = format!("{id_prefix}.execution");
    submit_reference_action_with_trace(
        canwu,
        actor,
        &FiscalActionRequest {
            action_id: format!("{id_prefix}.authorize.action"),
            authority_binding_id: authority_id,
            expected_procedure_revision: fiscal_state_version(canwu)?,
            action: FiscalAction::AuthorizeExecution {
                request_id: request_id.clone(),
                assessment_id,
                kind: FiscalExecutionKind::Collect,
                quantity: 70,
                unit: sample_unit(payment_form).to_owned(),
                resource: ResourceId::new(1),
                source: EntityRef::Person(actor),
                target: institution.clone(),
                purpose: "reference fiscal sample collection".to_owned(),
            },
        },
        MingFiscalTracePhase::AuthorizeExecution,
        &mut on_boundary,
    )?;

    settle_sample_execution_with_trace(
        canwu,
        id_prefix,
        request_id,
        actor,
        institution,
        payment_form,
        &mut on_boundary,
    )?;
    validate_ming_fiscal_reference(canwu)
}

struct SampleCyclePlan {
    actor: PersonId,
    authority_id: String,
    institution: EntityRef,
    assessment_id: String,
    rule_id: String,
    scope_id: String,
    period_id: String,
    payment_form: FiscalPaymentForm,
}

fn sample_cycle_plan(canwu: &Canwu, id_prefix: &str) -> Result<SampleCyclePlan, CanwuError> {
    let catalog = canwu
        .typed_domain_record(&fiscal_catalog_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal catalog is unavailable"))?
        .decode_payload::<canwu_fiscal::FiscalCatalogRecord>()?;
    let initial_state = canwu
        .typed_domain_record(&fiscal_state_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal state is unavailable"))?
        .decode_payload::<canwu_fiscal::FiscalStateRecord>()?;
    let (adoption, scope, authority) = initial_state
        .adoptions
        .values()
        .filter(|adoption| {
            adoption.stage.is_operational()
                && catalog.rules.get(&adoption.rule_id).is_some_and(|rule| {
                    rule.legal_window
                        .contains(initial_state.historical_context.year)
                })
        })
        .find_map(|adoption| {
            let scope = initial_state
                .scope_bindings
                .get(&adoption.scope_binding_id)?;
            let authority = initial_state
                .authority_bindings
                .values()
                .find(|authority| {
                    authority.institution == scope.institution
                        && authority.authorized_actor.is_some()
                })?;
            Some((adoption.clone(), scope.clone(), authority.clone()))
        })
        .ok_or_else(|| invalid_reference("fixture has no executable fiscal adoption"))?;
    let rule = &catalog.rules[&adoption.rule_id];
    let payment_form = *rule
        .payment_forms
        .iter()
        .next()
        .ok_or_else(|| invalid_reference("fixture fiscal rule has no payment form"))?;
    let period = catalog
        .active_period_ids(initial_state.historical_context.year)
        .into_iter()
        .next()
        .ok_or_else(|| invalid_reference("fixture has no active fiscal period"))?;
    let actor = authority
        .authorized_actor
        .ok_or_else(|| invalid_reference("fixture fiscal authority has no actor"))?;
    Ok(SampleCyclePlan {
        actor,
        authority_id: authority.id,
        institution: authority.institution,
        assessment_id: format!("{id_prefix}.assessment"),
        rule_id: adoption.rule_id,
        scope_id: scope.id,
        period_id: period,
        payment_form,
    })
}

fn settle_sample_execution_with_trace<F>(
    canwu: &mut Canwu,
    id_prefix: &str,
    request_id: String,
    actor: PersonId,
    institution: EntityRef,
    payment_form: FiscalPaymentForm,
    on_boundary: &mut F,
) -> Result<(), CanwuError>
where
    F: FnMut(&Canwu, MingFiscalTracePhase, &canwu_api::BoundaryReceipt) -> Result<(), CanwuError>,
{
    let evidence = ReferenceFiscalExecutionEvidence {
        id: format!("{id_prefix}.execution-evidence"),
        request_id: request_id.clone(),
        quantity: 70,
        unit: sample_unit(payment_form).to_owned(),
        payment_form,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: institution,
        disposition: FiscalReceiptDisposition::Fulfilled,
        external_operation_id: format!("{id_prefix}.resource-operation"),
    };
    let now = canwu.time();
    enqueue_reference_execution_result(canwu, now, &evidence)?;
    let receipts = canwu.advance_canonical(SimDuration::minutes(1))?;
    notify_trace(
        on_boundary,
        canwu,
        MingFiscalTracePhase::AdapterEvidence,
        &receipts,
    )?;
    let evidence_version = reference_execution_evidence_version(canwu, &evidence.id)
        .ok_or_else(|| invalid_reference("sample execution evidence version is unavailable"))?;
    let now = canwu.time();
    enqueue_execution_receipt(
        canwu,
        now,
        &canwu_fiscal::FiscalExecutionReceiptPacket {
            receipt_id: format!("{id_prefix}.receipt"),
            request_id,
            external_evidence: [evidence_version].into_iter().collect(),
        },
    )?;
    let receipts = canwu.advance_canonical(SimDuration::minutes(1))?;
    notify_trace(
        on_boundary,
        canwu,
        MingFiscalTracePhase::FiscalExecutionReceipt,
        &receipts,
    )?;
    Ok(())
}

fn submit_reference_action_with_trace<F>(
    canwu: &mut Canwu,
    actor: PersonId,
    request: &FiscalActionRequest,
    phase: MingFiscalTracePhase,
    on_boundary: &mut F,
) -> Result<(), CanwuError>
where
    F: FnMut(&Canwu, MingFiscalTracePhase, &canwu_api::BoundaryReceipt) -> Result<(), CanwuError>,
{
    let request_id = canwu
        .revision()
        .checked_add(1)
        .ok_or_else(|| invalid_reference("sample command request identity overflowed"))?;
    let command = fiscal_action_command(request)
        .map_err(|error| invalid_reference(format!("sample action encoding failed: {error}")))?;
    canwu.enqueue_command(
        canwu.time(),
        0,
        CommandRequest::new(
            CommandRequestId::new(request_id),
            canwu.revision(),
            CommandEnvelope::new(Issuer::Actor(actor), command).at_time(canwu.time()),
        ),
    )?;
    let receipts = canwu.advance_canonical(SimDuration::minutes(1))?;
    notify_trace(on_boundary, canwu, phase, &receipts)?;
    let state = canwu
        .typed_domain_record(&fiscal_state_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal state is unavailable"))?
        .decode_payload::<canwu_fiscal::FiscalStateRecord>()?;
    let outcome = state
        .action_outcomes
        .get(&request.action_id)
        .ok_or_else(|| invalid_reference("sample fiscal action produced no durable outcome"))?;
    if outcome.disposition != canwu_fiscal::FiscalActionDisposition::Applied {
        return Err(invalid_reference(format!(
            "sample fiscal action {} was rejected: {}",
            request.action_id, outcome.reason
        )));
    }
    Ok(())
}

fn notify_trace<F>(
    on_boundary: &mut F,
    canwu: &Canwu,
    phase: MingFiscalTracePhase,
    receipts: &[canwu_api::BoundaryReceipt],
) -> Result<(), CanwuError>
where
    F: FnMut(&Canwu, MingFiscalTracePhase, &canwu_api::BoundaryReceipt) -> Result<(), CanwuError>,
{
    for receipt in receipts {
        on_boundary(canwu, phase, receipt)?;
    }
    Ok(())
}

fn fiscal_state_version(canwu: &Canwu) -> Result<u64, CanwuError> {
    canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .ok_or_else(|| invalid_reference("Ming fiscal state is unavailable"))?
        .decode_payload::<canwu_fiscal::FiscalStateRecord>()
        .map(|state| state.procedure_revision)
}

const fn sample_unit(payment_form: FiscalPaymentForm) -> &'static str {
    match payment_form {
        FiscalPaymentForm::Grain => "shi_grain_equivalent",
        FiscalPaymentForm::Silver => "liang_silver",
        FiscalPaymentForm::Labor => "labor_days",
        FiscalPaymentForm::SaltCertificate => "salt_certificates",
        FiscalPaymentForm::Coin => "wen_coin",
        FiscalPaymentForm::Goods => "goods_equivalent",
        FiscalPaymentForm::Credit => "liang_credit_equivalent",
    }
}

fn validate_reference_receipt(
    canwu: &Canwu,
    state: &FiscalState,
    receipt: &canwu_fiscal::FiscalExecutionReceipt,
) -> Result<(), CanwuError> {
    let request = &state.execution_requests[&receipt.request_id];
    let mut evidenced_quantity = 0_u64;
    let mut external_operations = BTreeSet::new();
    for version in &receipt.external_evidence {
        let record = canwu.domain_record_version(version).ok_or_else(|| {
            invalid_reference("Ming fiscal execution evidence payload is unavailable")
        })?;
        let evidence = record.decode_payload::<ReferenceFiscalExecutionEvidenceRecord>()?;
        let operation = FiscalExternalOperationRef {
            evidence_kind: version.record.kind.clone(),
            external_operation_id: evidence.external_operation_id.clone(),
        };
        if evidence.id != version.record.id
            || !external_operations.insert(operation)
            || evidence.request_id != request.id
            || evidence.unit != request.unit
            || evidence.payment_form != request.payment_form
            || evidence.execution_kind != request.kind
            || evidence.resource != request.resource
            || evidence.source != request.source
            || evidence.target != request.target
            || evidence.disposition != receipt.disposition
        {
            return Err(invalid_reference(
                "Ming fiscal execution evidence does not match its receipt and request",
            ));
        }
        evidenced_quantity = evidenced_quantity
            .checked_add(evidence.quantity)
            .ok_or_else(|| invalid_reference("Ming fiscal evidence quantity overflowed"))?;
        if canwu
            .evidence_time(&canwu_api::EvidenceRef::DomainRecordVersion(
                version.clone(),
            ))
            .is_none_or(|at| at < request.requested_at)
        {
            return Err(invalid_reference(
                "Ming fiscal execution evidence predates its request",
            ));
        }
    }
    if evidenced_quantity != receipt.quantity || external_operations != receipt.external_operations
    {
        return Err(invalid_reference(
            "Ming fiscal receipt operations or quantity are not proven by its execution evidence",
        ));
    }
    Ok(())
}

fn invalid_reference(message: impl Into<String>) -> CanwuError {
    CanwuError::new(canwu_api::ErrorCode::InvalidDomainRecord, message)
}

#[must_use]
pub fn ming_fiscal_plugin() -> canwu_fiscal::FiscalPlugin {
    canwu_fiscal::FiscalPlugin::new([DomainRecordKind::for_type::<
        ReferenceFiscalExecutionEvidenceRecord,
    >()])
}

#[must_use]
pub fn fixture_ids() -> [&'static str; 3] {
    ["hongwu-1391", "wanli-1581", "hongguang-1644"]
}

fn jurisdiction_from_scope(scope_id: &str) -> Result<String, CanwuError> {
    let mappings = BTreeMap::from([
        ("scope.lower-yangzi.", "lower_yangzi"),
        ("scope.north.", "north_china"),
        ("scope.southeast.", "southeast_coast"),
        ("scope.southwest.", "southwest"),
        ("scope.southern-court.", "southern_ming_courts"),
    ]);
    mappings
        .into_iter()
        .find_map(|(prefix, region)| scope_id.starts_with(prefix).then(|| region.to_owned()))
        .ok_or_else(|| {
            CanwuError::new(
                canwu_api::ErrorCode::InvalidDomainRecord,
                format!("fixture scope {scope_id} has no canonical region mapping"),
            )
        })
}
