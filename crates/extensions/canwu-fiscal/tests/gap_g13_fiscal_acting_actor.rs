//! Gap G-13: a fiscal authority binding admits an acting actor only while the
//! exact authority-basis record version is current, and keeps admitting its
//! principal after the basis advances.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, CommandAttemptOutcome, CommandEnvelope,
    CommandRequest, CommandRequestId, CompactedCanwu, DomainRecord, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation,
    DomainRecordSchema, DomainRecordType, DomainRecordVersionRef, DomainRecordVersionSource,
    DomainValueKindClass, EntityRef, ErrorCode, EvidenceRef, Government, GovernmentId,
    IngressClass, IngressPayload, Issuer, KnowledgeSnapshot, MapPoint, PayloadSchema, Person,
    PersonId, PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, Scenario,
    SimDuration, SimTime, SimulationGranularity, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_fiscal::{
    FISCAL_ACTING_BASIS_NOT_CURRENT, FiscalAction, FiscalActionDisposition, FiscalActionRequest,
    FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessmentBasis, FiscalAuthorityBinding,
    FiscalCommutationPolicy, FiscalContentPack, FiscalContentSelection, FiscalCoverageDeclaration,
    FiscalCoverageSelector, FiscalCoverageStatus, FiscalHistoricalMode, FiscalMechanism,
    FiscalPackManifest, FiscalPaymentForm, FiscalPeriodDefinition, FiscalPlugin, FiscalProvenance,
    FiscalRegionDefinition, FiscalRuleDefinition, FiscalScopeBinding, FiscalState,
    FiscalStateRecord, HistoricalConfidence, HistoricalYearWindow, compile_fiscal_content,
    fiscal_action_command, fiscal_state_reference,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const GRANT_PLUGIN: &str = "fixture-acting-authority";
const RENEW_GRANT_INGRESS: &str = "renew_acting_grant_v1";
const AUTHORITY_ID: &str = "authority.treasury";
const PRINCIPAL: PersonId = PersonId::new(1);
const ACTING: PersonId = PersonId::new(2);

#[derive(Debug, Deserialize, Serialize)]
struct ActingGrantPayload {
    holder: PersonId,
    term: u32,
}

struct ActingGrant;

impl DomainRecordType for ActingGrant {
    type Payload = ActingGrantPayload;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.authority";
    const NAME: &'static str = "acting_grant";
}

fn grant_reference() -> TypedDomainRecordRef<ActingGrant> {
    TypedDomainRecordRef::new("grant.treasury")
}

fn grant_state_key() -> StateKey {
    DomainRecordSchema::for_record::<ActingGrant>().state_key()
}

/// Application-owned grant record: each renewal advances its version, which
/// makes any fiscal binding that cites the previous version stale.
struct ActingGrantPlugin;

impl SimulationPlugin for ActingGrantPlugin {
    fn name(&self) -> &'static str {
        GRANT_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000c1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::for_record::<ActingGrant>())?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: RENEW_GRANT_INGRESS.to_owned(),
            description: "Renew the acting grant as a new record version".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut renew = BoundarySystemContract::new(
            "renew-acting-grant-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        renew.reads = vec![StateKey::core_ingress(), grant_state_key()];
        renew.writes = vec![grant_state_key()];
        renew.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(renew, renew_grant)
    }
}

fn renew_grant(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut renewals = 0_u32;
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                ..
            } = &ingress.payload
            && plugin == GRANT_PLUGIN
            && packet_type == RENEW_GRANT_INGRESS
        {
            renewals += 1;
        }
    }
    if renewals == 0 {
        return Ok(BoundaryProposal::default());
    }
    let record = view
        .typed_domain_record(&grant_reference())?
        .expect("acting grant record")
        .clone();
    let mut payload = record.decode_payload::<ActingGrant>()?;
    payload.term += renewals;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::from_typed(grant_reference(), &payload)?,
                expected_version: record.version,
            },
            summary: "Renew the acting grant".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn fiscal_pack() -> FiscalContentPack {
    let window = HistoricalYearWindow {
        start: 100,
        end: 200,
    };
    FiscalContentPack {
        manifest: FiscalPackManifest {
            pack_id: "fixture.acting".to_owned(),
            pack_version: "1.0.0".to_owned(),
            schema_version: 1,
            license: "Apache-2.0".to_owned(),
            title: "Acting authority fixture".to_owned(),
            historical_scope: window.clone(),
            period_ids: vec!["period.a".to_owned()],
            region_ids: vec!["region.a".to_owned()],
            mechanisms: vec![FiscalMechanism::LandTax],
        },
        periods: vec![FiscalPeriodDefinition {
            id: "period.a".to_owned(),
            label: "Fixture period".to_owned(),
            window: window.clone(),
            branch: None,
        }],
        regions: vec![FiscalRegionDefinition {
            id: "region.a".to_owned(),
            label: "Fixture region".to_owned(),
        }],
        institutions: Vec::new(),
        rules: vec![FiscalRuleDefinition {
            id: "rule.land".to_owned(),
            revision: 1,
            label: "Land levy".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            legal_window: window,
            jurisdiction_ids: BTreeSet::from(["region.a".to_owned()]),
            subject_scope: "registered_land".to_owned(),
            assessment_basis: FiscalAssessmentBasis::RegisteredLand,
            payment_forms: BTreeSet::from([FiscalPaymentForm::Grain]),
            commutation: FiscalCommutationPolicy::Disabled,
            earmark_ids: BTreeSet::new(),
            provenance_ids: BTreeSet::from(["source.fixture".to_owned()]),
            confidence: HistoricalConfidence::Medium,
        }],
        transitions: Vec::new(),
        coverage: vec![FiscalCoverageDeclaration {
            id: "coverage.a".to_owned(),
            priority: 1,
            selector: FiscalCoverageSelector {
                period_ids: BTreeSet::new(),
                region_ids: BTreeSet::new(),
                mechanisms: BTreeSet::new(),
            },
            status: FiscalCoverageStatus::Supported,
            definition_ids: BTreeSet::from(["rule.land".to_owned()]),
            provenance_ids: BTreeSet::from(["source.fixture".to_owned()]),
        }],
        provenance: vec![FiscalProvenance {
            id: "source.fixture".to_owned(),
            citation: "Synthetic fixture".to_owned(),
            url: "https://example.invalid/fixture".to_owned(),
            claim_scope: "test content only".to_owned(),
            confidence: HistoricalConfidence::Medium,
            forbidden_inferences: vec!["no historical claim".to_owned()],
        }],
    }
}

fn initial_basis() -> DomainRecordVersionRef {
    DomainRecordVersionRef {
        record: grant_reference().into_untyped(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    }
}

fn scenario() -> Scenario {
    let government = GovernmentId::new(1);
    let territory = TerritoryId::new(1);
    let person = |id: PersonId, name: &str| Person {
        id,
        name: name.to_owned(),
        government,
        current_location: territory,
        roles: Vec::new(),
        transit: None,
    };
    let world = WorldSnapshot {
        people: vec![person(PRINCIPAL, "Principal"), person(ACTING, "Deputy")],
        governments: vec![Government {
            id: government,
            name: "Fixture authority".to_owned(),
            capital: territory,
        }],
        territories: vec![Territory {
            id: territory,
            name: "Fixture district".to_owned(),
            controller: government,
            position: MapPoint::default(),
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    let institution = EntityRef::Government(government);
    let catalog = compile_fiscal_content(
        &fiscal_pack(),
        FiscalContentSelection {
            historical_year: 150,
            ..FiscalContentSelection::default()
        },
    )
    .expect("fixture fiscal catalog");
    let mut state = FiscalState::new(150, FiscalHistoricalMode::Counterfactual, SimTime::EPOCH);
    state.authority_bindings.insert(
        AUTHORITY_ID.to_owned(),
        FiscalAuthorityBinding {
            id: AUTHORITY_ID.to_owned(),
            institution: institution.clone(),
            authorized_actor: Some(PRINCIPAL),
            acting_actor: Some(ACTING),
            authority_basis: Some(initial_basis()),
        },
    );
    state.scope_bindings.insert(
        "scope.a".to_owned(),
        FiscalScopeBinding {
            id: "scope.a".to_owned(),
            institution,
            jurisdiction_id: "region.a".to_owned(),
            subject_scope: "registered_land".to_owned(),
            mechanism: FiscalMechanism::LandTax,
            authoritative_granularity: SimulationGranularity::Aggregate,
        },
    );
    state.adoptions.insert(
        "adopt.a".to_owned(),
        FiscalAdoptionState {
            id: "adopt.a".to_owned(),
            rule_id: "rule.land".to_owned(),
            scope_binding_id: "scope.a".to_owned(),
            stage: FiscalAdoptionStage::Implemented,
            generation: 1,
            changed_at: SimTime::EPOCH,
            source_action_id: None,
        },
    );
    let grant = DomainRecordDraft::from_typed(
        grant_reference(),
        &ActingGrantPayload {
            holder: ACTING,
            term: 1,
        },
    )
    .expect("grant draft");
    Scenario {
        start_time: SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![
            DomainRecord {
                reference: grant.reference,
                owner: GRANT_PLUGIN.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: grant.payload,
                references: grant.references,
            },
            catalog.clone().into_record().expect("catalog record"),
            state.into_record(&catalog).expect("fiscal state record"),
        ],
    }
}

fn fiscal_state(canwu: &Canwu) -> FiscalState {
    canwu
        .typed_domain_record(&fiscal_state_reference())
        .expect("fiscal state record")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
}

/// Submits one tracked assessment action as `issuer` and settles it.
fn submit_assessment(canwu: &mut Canwu, issuer: PersonId, action_id: &str, cycle: &str) {
    let request = FiscalActionRequest {
        action_id: action_id.to_owned(),
        authority_binding_id: AUTHORITY_ID.to_owned(),
        expected_procedure_revision: fiscal_state(canwu).procedure_revision,
        action: FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: "rule.land".to_owned(),
            scope_binding_id: "scope.a".to_owned(),
            accounting_cycle_id: cycle.to_owned(),
            quantity: 100,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    };
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(canwu.revision() + 1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(issuer),
                    fiscal_action_command(&request).expect("fiscal command"),
                )
                .at_time(canwu.time()),
            ),
        )
        .expect("tracked fiscal command");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("fiscal command boundary");
}

fn applied(canwu: &Canwu, action_id: &str) -> bool {
    fiscal_state(canwu)
        .action_outcomes
        .get(action_id)
        .is_some_and(|outcome| outcome.disposition == FiscalActionDisposition::Applied)
}

#[test]
fn gap_g13_fiscal_acting_actor() {
    let grant_plugin = ActingGrantPlugin;
    // A basis kind the fiscal plugin was not configured to read fails closed.
    assert!(
        Canwu::new_with_plugins(13, scenario(), &[&grant_plugin, &FiscalPlugin::default()])
            .is_err()
    );
    let fiscal_plugin = FiscalPlugin::default()
        .with_authority_basis_kinds([DomainRecordKind::for_type::<ActingGrant>()]);
    let plugins: [&dyn SimulationPlugin; 2] = [&grant_plugin, &fiscal_plugin];
    let mut canwu = Canwu::new_with_plugins(13, scenario(), &plugins).expect("fixture runtime");

    // The acting holder is admitted while the cited grant version is current.
    submit_assessment(&mut canwu, ACTING, "action.acting.current", "cycle.1");
    assert!(
        applied(&canwu, "action.acting.current"),
        "acting actor with a current basis must be admitted: {:?}",
        fiscal_state(&canwu).action_outcomes
    );
    assert!(
        fiscal_state(&canwu)
            .assessments
            .contains_key("action.acting.current.assessment")
    );

    // The application renews its grant in the boundary that admits the next
    // acting command: admission saw the current basis, but settlement re-checks
    // it and records a rejected outcome with the stable reason.
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            GRANT_PLUGIN,
            RENEW_GRANT_INGRESS,
            canwu.time(),
            serde_json::json!({}),
        ))
        .expect("grant renewal ingress");
    submit_assessment(&mut canwu, ACTING, "action.acting.renewed", "cycle.2");
    assert!(matches!(
        canwu
            .command_attempts()
            .last()
            .expect("acting attempt during renewal")
            .outcome,
        CommandAttemptOutcome::Accepted { .. }
    ));
    let outcome = fiscal_state(&canwu).action_outcomes["action.acting.renewed"].clone();
    assert_eq!(outcome.disposition, FiscalActionDisposition::Rejected);
    assert!(outcome.reason.ends_with(FISCAL_ACTING_BASIS_NOT_CURRENT));
    let current = canwu
        .current_domain_record_version(&grant_reference().into_untyped())
        .expect("current grant version query")
        .expect("current grant version");
    assert_eq!(current.version, 2);
    assert_ne!(
        Some(current),
        fiscal_state(&canwu).authority_bindings[AUTHORITY_ID].authority_basis
    );

    // With the basis stale, command admission itself rejects the acting holder.
    submit_assessment(&mut canwu, ACTING, "action.acting.stale", "cycle.3");
    let CommandAttemptOutcome::Rejected { error } = &canwu
        .command_attempts()
        .last()
        .expect("stale acting attempt")
        .outcome
    else {
        panic!("a stale authority basis must reject the acting actor");
    };
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    assert_eq!(error.message, FISCAL_ACTING_BASIS_NOT_CURRENT);
    assert!(
        !fiscal_state(&canwu)
            .action_outcomes
            .contains_key("action.acting.stale")
    );

    // The principal is unaffected by the acting basis.
    submit_assessment(&mut canwu, PRINCIPAL, "action.principal", "cycle.4");
    assert!(applied(&canwu, "action.principal"));

    // Save/load and exact replay reproduce the same authority outcome history.
    let settled = fiscal_state(&canwu);
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    assert_eq!(fiscal_state(&restored), settled);
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
    assert_eq!(fiscal_state(&replayed), settled);
    assert_eq!(replayed.command_attempts(), canwu.command_attempts());
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

fn renew_grant_now(canwu: &mut Canwu) {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            GRANT_PLUGIN,
            RENEW_GRANT_INGRESS,
            canwu.time(),
            serde_json::json!({}),
        ))
        .expect("grant renewal ingress");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("grant renewal boundary");
}

fn current_grant(canwu: &Canwu) -> DomainRecordVersionRef {
    canwu
        .current_domain_record_version(&grant_reference().into_untyped())
        .expect("current grant version query")
        .expect("current grant version")
}

fn sealed_fiscal_state(sealed: &CompactedCanwu) -> FiscalState {
    sealed
        .typed_domain_record(&fiscal_state_reference())
        .expect("fiscal state record")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
}

/// Submits one tracked assessment action that cites `quote` as its
/// commutation quote, as the principal, and runs the boundary that admits it.
fn submit_quoted_assessment(
    sealed: &mut CompactedCanwu,
    action_id: &str,
    quote: DomainRecordVersionRef,
) {
    let request = FiscalActionRequest {
        action_id: action_id.to_owned(),
        authority_binding_id: AUTHORITY_ID.to_owned(),
        expected_procedure_revision: sealed_fiscal_state(sealed).procedure_revision,
        action: FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: "rule.land".to_owned(),
            scope_binding_id: "scope.a".to_owned(),
            accounting_cycle_id: action_id.to_owned(),
            quantity: 100,
            unit: "grain".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: Some(quote),
        },
    };
    sealed
        .enqueue_command(
            sealed.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(sealed.revision() + 1),
                sealed.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(PRINCIPAL),
                    fiscal_action_command(&request).expect("fiscal command"),
                )
                .at_time(sealed.time()),
            ),
        )
        .expect("tracked fiscal command");
    sealed
        .advance_canonical(SimDuration::minutes(1))
        .expect("a quote check must not fail the boundary");
}

/// A commutation quote is admitted only while no seal can change its
/// answer, so a sealed run rejects a superseded quote version exactly as its
/// full-retention replay does. The acting grant stands in for an application
/// quote record of a kind the fiscal plugin reads.
#[test]
fn sealed_run_rejects_a_superseded_commutation_quote_like_its_replay() {
    let grant_plugin = ActingGrantPlugin;
    let fiscal_plugin = FiscalPlugin::default()
        .with_authority_basis_kinds([DomainRecordKind::for_type::<ActingGrant>()]);
    let plugins: [&dyn SimulationPlugin; 2] = [&grant_plugin, &fiscal_plugin];
    let mut canwu = Canwu::new_with_plugins(13, scenario(), &plugins).expect("fixture runtime");

    // Version 2 is established by a boundary and then superseded, and nothing
    // live depends on it, so the next seal drops its evidence.
    renew_grant_now(&mut canwu);
    let superseded = current_grant(&canwu);
    assert_eq!(superseded.version, 2);
    renew_grant_now(&mut canwu);
    let current = current_grant(&canwu);
    assert_eq!(current.version, 3);
    // Later boundaries admit the renewals' emissions, so the history can seal.
    for _ in 0..2 {
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("a quiet boundary before sealing");
    }

    let mut sealed = canwu.into_compacted().expect("compact mode should start");
    let segments = vec![
        sealed
            .seal_evidence()
            .expect("settled history should seal")
            .expect("settled history should contain evidence"),
    ];
    assert!(
        sealed
            .archived_evidence_receipt(&EvidenceRef::DomainRecordVersion(superseded.clone()))
            .is_none()
    );

    submit_quoted_assessment(&mut sealed, "action.quote.superseded", superseded);
    submit_quoted_assessment(&mut sealed, "action.quote.current", current);
    sealed
        .advance_canonical(SimDuration::minutes(1))
        .expect("the admitted quote action settles");
    let outcomes = sealed_fiscal_state(&sealed).action_outcomes;
    assert!(!outcomes.contains_key("action.quote.superseded"));
    assert!(outcomes.contains_key("action.quote.current"));

    let replayed = Canwu::replay_from_journal(
        &plugins,
        &sealed
            .replay_journal_with_segments(segments.clone())
            .expect("the sealed run should produce an exact replay journal"),
    )
    .expect("the sealed run should replay exactly");
    assert_eq!(
        replayed.snapshot(),
        sealed
            .snapshot_with_segments(segments)
            .expect("the sealed archive should reconstruct a full snapshot")
    );
    let quote_rejections = replayed
        .command_attempts()
        .iter()
        .filter(|attempt| {
            matches!(
                &attempt.outcome,
                CommandAttemptOutcome::Rejected { error } if error.code == ErrorCode::EntityNotFound
            )
        })
        .count();
    assert_eq!(quote_rejections, 1);
}
