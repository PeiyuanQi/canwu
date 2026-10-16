//! Gap G-58, as the fiscal extension meets it: two boundaries at one instant
//! each change the fiscal state, so the records of the observer's second
//! report supersede records learned at the same instant in the earlier
//! boundary. The run continues, and a save taken after it restores and
//! replays exactly.

use canwu_api::{
    Canwu, CommandEnvelope, CommandRequest, CommandRequestId, EntityRef, Government, GovernmentId,
    Issuer, KnowledgeHolderRef, KnowledgeRecord, KnowledgeSnapshot, MapPoint, Person, PersonId,
    Scenario, SimDuration, SimTime, SimulationGranularity, Territory, TerritoryId, WorldSnapshot,
};
use canwu_fiscal::{
    FiscalAction, FiscalActionBatch, FiscalActionDisposition, FiscalActionEntry,
    FiscalActionTarget, FiscalAdoptionStage, FiscalAdoptionState, FiscalAssessmentBasis,
    FiscalAuthorityBinding, FiscalCommutationPolicy, FiscalContentPack, FiscalContentSelection,
    FiscalCoverageDeclaration, FiscalCoverageSelector, FiscalCoverageStatus, FiscalHistoricalMode,
    FiscalMechanism, FiscalObserverBinding, FiscalPackManifest, FiscalPaymentForm,
    FiscalPeriodDefinition, FiscalPlugin, FiscalProvenance, FiscalRegionDefinition,
    FiscalRuleDefinition, FiscalScopeBinding, FiscalState, HistoricalConfidence,
    HistoricalYearWindow, audit_fiscal_records, compile_fiscal_content, fiscal_action_command,
    fiscal_target_revision,
};
use std::collections::BTreeSet;

const AUTHORITY_ID: &str = "authority.treasury";
const TREASURER: PersonId = PersonId::new(1);
const AUDITOR: PersonId = PersonId::new(2);

fn fiscal_pack() -> FiscalContentPack {
    let window = HistoricalYearWindow {
        start: 100,
        end: 200,
    };
    FiscalContentPack {
        manifest: FiscalPackManifest {
            pack_id: "fixture.reports".to_owned(),
            pack_version: "1.0.0".to_owned(),
            schema_version: 1,
            license: "Apache-2.0".to_owned(),
            title: "Same-instant report fixture".to_owned(),
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
        people: vec![person(TREASURER, "Treasurer"), person(AUDITOR, "Auditor")],
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
    state.add_authority_binding(FiscalAuthorityBinding {
        id: AUTHORITY_ID.to_owned(),
        institution: institution.clone(),
        authorized_actor: Some(TREASURER),
        acting_actor: None,
        authority_basis: None,
        rebound_by: None,
    });
    state.add_scope(FiscalScopeBinding {
        id: "scope.a".to_owned(),
        institution: institution.clone(),
        jurisdiction_id: "region.a".to_owned(),
        subject_scope: "registered_land".to_owned(),
        mechanism: FiscalMechanism::LandTax,
        authoritative_granularity: SimulationGranularity::Aggregate,
    });
    state.add_observer(FiscalObserverBinding {
        id: "observer.auditor".to_owned(),
        actor: Some(AUDITOR),
        knowledge_holder: KnowledgeHolderRef::Person(AUDITOR),
        visible_institutions: BTreeSet::from([institution]),
        confidence_per_mille: 900,
        controlled_by: None,
        rebound_by: None,
        removed: None,
    });
    state
        .add_adoption(FiscalAdoptionState {
            id: "adopt.a".to_owned(),
            rule_id: "rule.land".to_owned(),
            scope_binding_id: "scope.a".to_owned(),
            stage: FiscalAdoptionStage::Implemented,
            generation: 1,
            changed_at: SimTime::EPOCH,
            source_action_id: None,
        })
        .expect("adoption of the scope");
    let mut domain_records = vec![catalog.clone().into_record().expect("catalog record")];
    domain_records.extend(state.into_records(&catalog).expect("fiscal records"));
    Scenario {
        start_time: SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records,
    }
}

fn fiscal_state(canwu: &Canwu) -> FiscalState {
    audit_fiscal_records(canwu).expect("fiscal records audit")
}

fn scope() -> FiscalActionTarget {
    FiscalActionTarget::Ledger("scope.a".to_owned())
}

/// Submits one assessment action, of one more partition of the one book,
/// and settles it at the current instant, without advancing time: one
/// boundary admits the command and a second one settles the action and
/// publishes the report.
fn assess_now(canwu: &mut Canwu, action_id: &str) {
    let request = FiscalActionEntry {
        action_id: action_id.to_owned(),
        expected_revision: fiscal_target_revision(canwu, &scope())
            .expect("revision read")
            .expect("the scope's ledger"),
        action: FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: "rule.land".to_owned(),
            scope_binding_id: "scope.a".to_owned(),
            accounting_cycle_id: "cycle.1".to_owned(),
            quantity: 100,
            unit: action_id.to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    };
    let now = canwu.time();
    canwu
        .enqueue_command(
            now,
            0,
            CommandRequest::new(
                CommandRequestId::new(canwu.revision() + 1),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::Actor(TREASURER),
                    fiscal_action_command(&FiscalActionBatch::single(AUTHORITY_ID, request))
                        .expect("fiscal command"),
                )
                .at_time(now),
            ),
        )
        .expect("tracked fiscal command");
    canwu
        .advance_canonical(SimDuration::ZERO)
        .expect("fiscal boundaries settle");
    assert_eq!(canwu.time(), now);
    assert_eq!(
        fiscal_state(canwu)
            .recent_outcome(&scope(), action_id)
            .expect("the assessment's outcome")
            .disposition,
        FiscalActionDisposition::Applied
    );
}

/// The auditor's report records, in the order the run published them. Each
/// report here is a summary followed by its one part.
fn reports(canwu: &Canwu) -> Vec<KnowledgeRecord> {
    let auditor = KnowledgeHolderRef::Person(AUDITOR);
    canwu
        .boundaries()
        .iter()
        .flat_map(|boundary| &boundary.knowledge_changes)
        .flat_map(|change| change.records_of(&auditor).collect::<Vec<_>>())
        .collect()
}

#[test]
fn gap_g58_fiscal_same_instant_reports() {
    let fiscal = FiscalPlugin::default();
    let mut canwu = Canwu::new_with_plugins(58, scenario(), &[&fiscal]).expect("fiscal run");

    // Two fiscal boundaries at one instant: each changes the fiscal state and
    // publishes the auditor's report, and each record of the second supersedes
    // the record of the same line in the first.
    assess_now(&mut canwu, "action.first");
    assess_now(&mut canwu, "action.second");
    let published = reports(&canwu);
    assert_eq!(published.len(), 4);
    assert!(
        published
            .iter()
            .all(|record| record.learned_at == published[0].learned_at)
    );
    assert_eq!(published[2].supersedes, vec![published[0].id]);
    assert_eq!(published[3].supersedes, vec![published[1].id]);

    // The run continues past that instant.
    canwu
        .advance_canonical(SimDuration::days(1))
        .expect("the run continues");
    assess_now(&mut canwu, "action.third");
    let published = reports(&canwu);
    assert_eq!(published.len(), 6);
    assert_eq!(published[4].supersedes, vec![published[2].id]);
    assert_eq!(published[5].supersedes, vec![published[3].id]);

    // A save taken after it restores, and the restored run settles a further
    // same-instant fiscal boundary exactly as the live run does.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let mut restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &[&fiscal])
        .expect("a snapshot the run continued past must restore");
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    for run in [&mut canwu, &mut restored] {
        assess_now(run, "action.fourth");
    }
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(restored.snapshot(), canwu.snapshot());

    let replayed =
        Canwu::replay_from_journal(&[&fiscal], &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}
