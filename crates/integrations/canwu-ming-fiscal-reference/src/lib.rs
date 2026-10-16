//! Ready-to-run integration of Canwu's Ming fiscal reference content.

#![allow(clippy::missing_errors_doc)]

mod adapter;
mod trace;
mod trace_viewer;

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
pub use trace_viewer::{TraceViewerError, TraceViewerHandle, start_trace_viewer};

use canwu_api::{
    Canwu, CanwuError, CommandEnvelope, CommandRequest, CommandRequestId, DomainRecordKind,
    DomainRecordVersionRef, DomainRecordVersionSource, EntityRef, EvidenceCursor, EvidenceRef,
    Issuer, OrganizationId, PersonId, ResourceId, Scenario, SimDuration, SimTime,
    SimulationGranularity,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionBatch, FiscalActionDisposition, FiscalActionEntry,
    FiscalActionTarget, FiscalAdoptionState, FiscalArchiveError, FiscalArchiveStore,
    FiscalAuthorityBinding, FiscalContentSelection, FiscalExecutionKind,
    FiscalExternalOperationRef, FiscalObserverBinding, FiscalPaymentForm, FiscalReceiptDisposition,
    FiscalReportCadence, FiscalReportGranularity, FiscalScopeBinding, FiscalSettledAction,
    FiscalState, MAX_FISCAL_ARCHIVE_COMMIT_RECORDS, audit_fiscal_records,
    enqueue_execution_receipts, enqueue_fiscal_archive, finalize_fiscal_archive_retention,
    fiscal_action_command, fiscal_catalog_reference, fiscal_target_revision, load_fiscal_state,
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
        .root
        .execution_evidence_kinds
        .insert(DomainRecordKind::for_type::<
            ReferenceFiscalExecutionEvidenceRecord,
        >());
    let institutions = reference_institutions(&fixture.id, &world_ids);
    for institution in institutions.values() {
        if let EntityRef::Organization(id) = &institution.entity {
            scenario.entities.push(EntityRef::Organization(*id));
        }
        state.add_authority_binding(FiscalAuthorityBinding {
            id: institution.authority_id.to_owned(),
            institution: institution.entity.clone(),
            authorized_actor: Some(institution.actor),
            acting_actor: None,
            authority_basis: None,
            rebound_by: None,
        });
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
        if state.ledger(&adoption.scope_id).is_none() {
            state.add_scope(FiscalScopeBinding {
                id: adoption.scope_id.clone(),
                institution,
                jurisdiction_id,
                subject_scope: rule.subject_scope.clone(),
                mechanism: rule.mechanism,
                authoritative_granularity: SimulationGranularity::Aggregate,
            });
        }
        state.add_adoption(FiscalAdoptionState {
            id: adoption.adoption_id.clone(),
            rule_id: adoption.rule_id.clone(),
            scope_binding_id: adoption.scope_id.clone(),
            stage: adoption.stage,
            generation: 1,
            changed_at: SimTime::EPOCH,
            source_action_id: None,
        })?;
    }
    let visible_institutions = state
        .scope_bindings()
        .map(|scope| scope.institution.clone())
        .collect();
    state.add_observer(FiscalObserverBinding {
        id: "observer.revenue-minister".to_owned(),
        actor: Some(world_ids.observer),
        knowledge_holder: canwu_api::KnowledgeHolderRef::Person(world_ids.observer),
        visible_institutions,
        confidence_per_mille: 850,
        granularity: FiscalReportGranularity::Scope,
        cadence: FiscalReportCadence::OnChange,
        controlled_by: None,
        rebound_by: None,
        removed: None,
    });
    scenario.domain_records.push(catalog.clone().into_record()?);
    scenario
        .domain_records
        .extend(state.into_records(&catalog)?);
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

/// Checks that the fiscal state of a reference run is consistent with its
/// catalog and with the evidence it cites.
///
/// It takes either runtime handle, [`Canwu`] or [`canwu_api::CompactedCanwu`],
/// and reads only what a seal cannot change: committed current records
/// ([`canwu_api::CanwuRun::domain_record`],
/// [`canwu_api::CanwuRun::current_domain_record_version`]), replay-stable
/// versions ([`canwu_api::CanwuRun::replay_stable_domain_record_version`]),
/// and the journal counters ([`canwu_api::CanwuRun::evidence_cursor`]);
/// never [`canwu_api::CanwuRun::domain_record_version_evidence_exists`]. So
/// a sealed run, an unsealed fork of it, a restore from its sealed
/// segments, and its exact replay all give the same answer:
///
/// - each execution receipt is checked against the typed adapter evidence it
///   cites, which must still be the current version of its record (adapter
///   evidence records are create-only), with that version's establishment
///   time;
/// - a commutation quote or an audit's domain-record evidence must be a
///   version the run established: the current or initial-scenario version of
///   its record, or a superseded version established by a settled boundary.
///   While the record exists, a superseded version must be below its current
///   version and its boundary no later than the one that established the
///   current version. The body of a superseded version is not read; the
///   fiscal plugin checked
///   it when it admitted the action, and the fiscal state declares quotes
///   and receipt evidence as payload-required dependencies, which the seal
///   and the restore keep provable;
/// - an audit's other evidence (an event, command, command attempt, ingress,
///   boundary, or random draw) must have an identity the run issued, by the
///   journal counters.
pub fn validate_ming_fiscal_reference<R: canwu_api::CanwuRun + ?Sized>(
    canwu: &R,
) -> Result<(), CanwuError> {
    let catalog_record = canwu
        .domain_record(fiscal_catalog_reference().as_untyped())
        .ok_or_else(|| invalid_reference("Ming fiscal catalog is unavailable"))?;
    let catalog = catalog_record.decode_payload::<canwu_fiscal::FiscalCatalogRecord>()?;
    catalog.validate()?;
    // The fiscal records are exactly the partition of a valid state, every
    // derived field included.
    let state = audit_fiscal_records(canwu)?;
    for (_, book) in state.books() {
        for receipt in book.execution_receipts.values() {
            let request = book
                .execution_requests
                .get(&receipt.request_id)
                .ok_or_else(|| invalid_reference("Ming fiscal receipt lost its request"))?;
            validate_reference_receipt(canwu, request, receipt)?;
        }
    }
    let cursor = canwu.evidence_cursor()?;
    for assessment in state.assessments() {
        if let Some(quote) = &assessment.commutation_quote
            && !established_version(canwu, &cursor, quote)?
        {
            return Err(invalid_reference(
                "Ming fiscal commutation quote is not a version the run established",
            ));
        }
    }
    for audit in state.audits() {
        for evidence in &audit.evidence {
            if !issued_evidence(canwu, &cursor, evidence)? {
                return Err(invalid_reference(
                    "Ming fiscal audit evidence is unavailable",
                ));
            }
        }
    }
    Ok(())
}

/// Whether the run established `version`: it resolves as the current or an
/// initial-scenario version of its record, or a settled boundary established
/// it and its record has moved past it. When the record still exists, the
/// cited version must be below its current version and its boundary no later
/// than the one that established the current version. Only current state and
/// counters are read, so the answer does not depend on what a seal kept.
fn established_version<R: canwu_api::CanwuRun + ?Sized>(
    canwu: &R,
    cursor: &EvidenceCursor,
    version: &DomainRecordVersionRef,
) -> Result<bool, CanwuError> {
    if canwu.replay_stable_domain_record_version(version).is_some() {
        return Ok(true);
    }
    let DomainRecordVersionSource::BoundaryChange { boundary, .. } = version.established_by else {
        return Ok(false);
    };
    if version.version == 0 || boundary.get() == 0 || boundary.get() > cursor.boundary_count {
        return Ok(false);
    }
    Ok(
        match canwu.current_domain_record_version(&version.record)? {
            None => true,
            Some(current) => {
                version.version < current.version
                    && match current.established_by {
                        DomainRecordVersionSource::BoundaryChange {
                            boundary: current_boundary,
                            ..
                        } => boundary <= current_boundary,
                        DomainRecordVersionSource::InitialScenario => false,
                    }
            }
        },
    )
}

/// Whether the run issued the evidence identity `evidence`. Journal
/// identities are contiguous from one, so an identity was issued exactly when
/// it is at most its journal's count, whether its item is retained or sealed.
fn issued_evidence<R: canwu_api::CanwuRun + ?Sized>(
    canwu: &R,
    cursor: &EvidenceCursor,
    evidence: &EvidenceRef,
) -> Result<bool, CanwuError> {
    let issued = |id: u64, count: u64| id != 0 && id <= count;
    Ok(match evidence {
        EvidenceRef::Event(id) => issued(id.get(), cursor.event_count),
        EvidenceRef::Command(id) => issued(id.get(), cursor.command_count),
        EvidenceRef::CommandAttempt(id) => issued(id.get(), cursor.command_attempt_count),
        EvidenceRef::Ingress(id) => issued(id.get(), cursor.ingress_count),
        EvidenceRef::Boundary(id) => issued(id.get(), cursor.boundary_count),
        EvidenceRef::RandomDraw(id) => issued(id.get(), cursor.random_draw_count),
        EvidenceRef::DomainRecordVersion(version) => {
            return established_version(canwu, cursor, version);
        }
    })
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
        accounting_cycle_id,
        payment_form,
    } = plan;
    submit_reference_action_with_trace(
        canwu,
        actor,
        &reference_batch(
            canwu,
            &authority_id,
            &scope_id,
            format!("{id_prefix}.assess.action"),
            FiscalAction::OpenAssessment {
                assessment_id: assessment_id.clone(),
                rule_id,
                scope_binding_id: scope_id.clone(),
                accounting_cycle_id,
                quantity: 100,
                unit: sample_unit(payment_form).to_owned(),
                payment_form,
                commutation_quote: None,
            },
        )?,
        MingFiscalTracePhase::OpenAssessment,
        &mut on_boundary,
    )?;

    let request_id = format!("{id_prefix}.execution");
    submit_reference_action_with_trace(
        canwu,
        actor,
        &reference_batch(
            canwu,
            &authority_id,
            &scope_id,
            format!("{id_prefix}.authorize.action"),
            FiscalAction::AuthorizeExecution {
                request_id: request_id.clone(),
                scope_binding_id: scope_id.clone(),
                assessment_id,
                kind: FiscalExecutionKind::Collect,
                quantity: 70,
                unit: sample_unit(payment_form).to_owned(),
                resource: ResourceId::new(1),
                source: EntityRef::Person(actor),
                target: institution.clone(),
                purpose: "reference fiscal sample collection".to_owned(),
            },
        )?,
        MingFiscalTracePhase::AuthorizeExecution,
        &mut on_boundary,
    )?;

    settle_sample_execution_with_trace(
        canwu,
        id_prefix,
        (scope_id, request_id),
        actor,
        institution,
        payment_form,
        &mut on_boundary,
    )?;
    validate_ming_fiscal_reference(canwu)
}

/// Closes the accounting cycle book of the sample cycle `id_prefix`, so that
/// an archive commit can take it out of the fiscal state.
pub fn close_ming_fiscal_sample_cycle(
    canwu: &mut Canwu,
    id_prefix: &str,
) -> Result<(), CanwuError> {
    let plan = sample_cycle_plan(canwu, id_prefix)?;
    let batch = reference_batch(
        canwu,
        &plan.authority_id,
        &plan.scope_id,
        format!("{id_prefix}.close.action"),
        FiscalAction::CloseAccountingCycle {
            scope_binding_id: plan.scope_id.clone(),
            accounting_cycle_id: plan.accounting_cycle_id,
        },
    )?;
    submit_reference_action_with_trace(
        canwu,
        plan.actor,
        &batch,
        MingFiscalTracePhase::CanonicalBoundary,
        &mut |_canwu, _phase, _receipt| Ok(()),
    )
}

/// Archives every closed book the fiscal state holds into `store`, in
/// commits of up to [`MAX_FISCAL_ARCHIVE_COMMIT_RECORDS`] records: each is
/// prepared, stored, verified, queued, settled, and its retention finalized
/// and acknowledged. Returns how many books left the state.
pub fn archive_ming_fiscal_books(
    canwu: &mut Canwu,
    store: &dyn FiscalArchiveStore,
) -> Result<u64, FiscalArchiveError> {
    let before = fiscal_state(canwu)?.archive.archived_book_count;
    loop {
        let state = fiscal_state(canwu)?;
        if !state.books().any(|(_, book)| !book.is_open()) {
            break;
        }
        let prepared =
            state.prepare_fiscal_archive(MAX_FISCAL_ARCHIVE_COMMIT_RECORDS, store, None)?;
        let receipt = enqueue_fiscal_archive(canwu, &prepared, store, None)?;
        canwu.advance_canonical(SimDuration::minutes(1))?;
        finalize_fiscal_archive_retention(canwu, store, &receipt)?;
        canwu.advance_canonical(SimDuration::minutes(1))?;
    }
    Ok(fiscal_state(canwu)?.archive.archived_book_count - before)
}

fn fiscal_state(canwu: &Canwu) -> Result<FiscalState, CanwuError> {
    load_fiscal_state(canwu)?.ok_or_else(|| invalid_reference("Ming fiscal state is unavailable"))
}

/// A one-entry batch of `action` on `scope`, under `authority_id`, against
/// the revision the scope's ledger has now.
fn reference_batch(
    canwu: &Canwu,
    authority_id: &str,
    scope: &str,
    action_id: String,
    action: FiscalAction,
) -> Result<FiscalActionBatch, CanwuError> {
    let expected_revision =
        fiscal_target_revision(canwu, &FiscalActionTarget::Ledger(scope.to_owned()))?
            .ok_or_else(|| invalid_reference("the sample scope has no ledger"))?;
    Ok(FiscalActionBatch::single(
        authority_id,
        FiscalActionEntry {
            action_id,
            expected_revision,
            action,
        },
    ))
}

struct SampleCyclePlan {
    actor: PersonId,
    authority_id: String,
    institution: EntityRef,
    assessment_id: String,
    rule_id: String,
    scope_id: String,
    accounting_cycle_id: String,
    payment_form: FiscalPaymentForm,
}

fn sample_cycle_plan(canwu: &Canwu, id_prefix: &str) -> Result<SampleCyclePlan, CanwuError> {
    let catalog = canwu
        .typed_domain_record(&fiscal_catalog_reference())
        .ok_or_else(|| invalid_reference("Ming fiscal catalog is unavailable"))?
        .decode_payload::<canwu_fiscal::FiscalCatalogRecord>()?;
    let initial_state = fiscal_state(canwu)?;
    let (adoption, scope, authority) = initial_state
        .adoptions()
        .filter(|adoption| {
            adoption.stage.is_operational()
                && catalog.rules.get(&adoption.rule_id).is_some_and(|rule| {
                    rule.legal_window
                        .contains(initial_state.root.historical_context.year)
                })
        })
        .find_map(|adoption| {
            let scope = initial_state.scope_binding(&adoption.scope_binding_id)?;
            let authority = initial_state.authority_bindings().find(|authority| {
                authority.institution == scope.institution && authority.authorized_actor.is_some()
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
        .active_period_ids(initial_state.root.historical_context.year)
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
        accounting_cycle_id: format!("{period}.{id_prefix}"),
        payment_form,
    })
}

fn settle_sample_execution_with_trace<F>(
    canwu: &mut Canwu,
    id_prefix: &str,
    (scope_id, request_id): (String, String),
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
        scope_binding_id: scope_id.clone(),
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
    enqueue_execution_receipts(
        canwu,
        now,
        &[canwu_fiscal::FiscalExecutionReceiptPacket {
            receipt_id: format!("{id_prefix}.receipt"),
            scope_binding_id: scope_id,
            request_id,
            external_evidence: [evidence_version].into_iter().collect(),
        }],
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
    batch: &FiscalActionBatch,
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
    let command = fiscal_action_command(batch)
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
    for entry in &batch.entries {
        let outcome = FiscalSettledAction::from_events(canwu.events())
            .into_iter()
            .rev()
            .find(|settled| settled.action_id == entry.action_id)
            .ok_or_else(|| invalid_reference("sample fiscal action produced no durable outcome"))?;
        if outcome.disposition != FiscalActionDisposition::Applied {
            return Err(invalid_reference(format!(
                "sample fiscal action {} was rejected: {}",
                entry.action_id, outcome.reason
            )));
        }
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

fn validate_reference_receipt<R: canwu_api::CanwuRun + ?Sized>(
    canwu: &R,
    request: &canwu_fiscal::FiscalExecutionRequest,
    receipt: &canwu_fiscal::FiscalExecutionReceipt,
) -> Result<(), CanwuError> {
    let mut evidenced_quantity = 0_u64;
    let mut external_operations = BTreeSet::new();
    for version in &receipt.external_evidence {
        // Adapter evidence is create-only, so the version a receipt cites
        // stays its record's current version, whose body and time no seal
        // removes.
        let (record, established_at) = canwu
            .replay_stable_domain_record_version(version)
            .ok_or_else(|| {
                invalid_reference(
                    "Ming fiscal execution evidence is not the current version of its record",
                )
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
        if established_at < request.requested_at {
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
