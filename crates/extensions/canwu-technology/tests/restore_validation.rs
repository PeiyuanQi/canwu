//! Restore validation over either run handle. A run restored from a
//! continuation checkpoint holds no journal before its cut;
//! `validate_technology_runtime_on` checks its technology state without
//! reading any, and agrees with the full-journal validator on a full run.

#![allow(clippy::too_many_lines)]

use std::cell::RefCell;
use std::collections::BTreeMap;

use canwu_api::{
    BoundaryId, BoundaryRecord, BoundaryRequest, Canwu, CanwuError, CanwuRun, Command,
    CommandEnvelope, CommandRequest, CommandRequestId, CompactedCanwu, DomainEntityKindClass,
    DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordKind, DomainRecordLifecycle,
    DomainRecordRef, DomainRecordSchema, DomainRecordType, DomainRecordVersionRef,
    DomainRecordVersionSource, DomainValueKindClass, EntityRef, ErrorCode, EvidenceCursor,
    EvidenceRef, Government, GovernmentId, HolderLedger, IngressId, IngressRecord, Issuer,
    KnowledgeHolderPolicy, KnowledgeHolderRef, KnowledgeSnapshot, MapPoint,
    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD, PayloadRequiredEvidenceContinuationV1, Person,
    PersonId, PluginIngressRequest, PluginRegistrar, Scenario, SimTime, SimulationPlugin,
    Territory, TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_technology::{
    AttemptObservation, AttemptObservationPayload, CapabilityQualification,
    CapabilityQualificationPayload, ExperimentAttempt, ExperimentAttemptPayload,
    ExternalTransmissionSourceV1, ImplementationPayload, ImplementationRecord, MetricComparison,
    MetricContext, MetricSchema, MetricSchemaPayload, MetricThreshold, MetricValue, ProgramMode,
    ProgramStatus, QualificationRule, REFERENCE_EVALUATOR_V1, RequirementGroup, TECHNOLOGY_COMMAND,
    TECHNOLOGY_RESULT_INGRESS, TechnicalClaim, TechnicalClaimPayload, TechnicalProgram,
    TechnicalProgramPayload, TechniqueRevision, TechniqueRevisionPayload, TechniqueSpec,
    TechniqueSpecPayload, TechnologyCatalogRecord, TechnologyCommandEnvelope,
    TechnologyExecutionIntent, TechnologyExecutionIntentPayload, TechnologyIntentRequest,
    TechnologyIntentState, TechnologyOperation, TechnologyOperationStatus, TechnologyPlugin,
    TechnologyRecordChange, TechnologyRecordPayload, TechnologyRestoreAccess,
    TechnologyResultEnvelope, TransmissionMode, TransmissionOpportunity,
    TransmissionOpportunityPayload, evaluate_attempt, initial_record_version,
    validate_technology_runtime, validate_technology_runtime_on,
};
use serde::{Deserialize, Serialize};

const OPERATOR: PersonId = PersonId::new(1);
const LEARNER: PersonId = PersonId::new(2);
const WORKSHOP: TerritoryId = TerritoryId::new(1);
const TRAINING_GROUND: TerritoryId = TerritoryId::new(2);
const CONTENT_PLUGIN: &str = "test-content";
const LAB: &str = "reference-lab";
const METER: &str = "meter-reader";

fn plugins() -> [&'static dyn SimulationPlugin; 2] {
    [&TechnologyPlugin, &ContentPlugin]
}

#[test]
fn restore_validator_agrees_with_the_full_journal_validator_on_generated_runs() {
    let plugins = plugins();
    for seed in 0..6 {
        let mut canwu =
            Canwu::new_with_plugins(seed, scenario(), &plugins).expect("technology run");
        let mut fixture = Fixture::default();
        for step in generated_steps(seed, 10) {
            fixture.run(&mut canwu, step);
        }
        assert!(
            !consumed_ingress(&canwu).is_empty(),
            "seed {seed}: the run consumes execution intents"
        );
        assert!(
            canwu
                .knowledge()
                .for_holder(&guild_holder())
                .is_some_and(|ledger| !ledger.is_empty()),
            "seed {seed}: a domain entity holds technology knowledge"
        );
        assert!(
            external_transmissions(&canwu) > 0,
            "seed {seed}: the run admits external-source transmissions"
        );
        let full = validate_technology_runtime(&canwu);
        let restore = validate_technology_runtime_on(&canwu);
        assert!(full.is_ok(), "seed {seed}: {full:?}");
        assert!(restore.is_ok(), "seed {seed}: {restore:?}");
        let dynamic: &dyn TechnologyRestoreAccess = &canwu;
        validate_technology_runtime_on(dynamic).expect("the trait object reads the same run");
        assert_eq!(
            TechnologyRestoreAccess::history_cut(&canwu).expect("history cut"),
            EvidenceCursor::default(),
            "seed {seed}: a full run holds its journal from the scenario"
        );

        // The same state sealed, and restored from its continuation
        // checkpoint: neither retains a journal entry, so the validator asks
        // for none.
        let mut sealed = canwu.fork().into_compacted().expect("compact the run");
        sealed
            .seal_evidence()
            .expect("seal the run")
            .expect("a retained tail to seal");
        let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
        let restored =
            CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
                .expect("restore from the continuation checkpoint");
        for (label, run) in [("sealed", &sealed), ("restored", &restored)] {
            let probe = Probe::new(run);
            let result = validate_technology_runtime_on(&probe);
            assert!(result.is_ok(), "seed {seed} {label}: {result:?}");
            assert!(
                probe.ingress.borrow().is_empty() && probe.boundaries.borrow().is_empty(),
                "seed {seed} {label}: no journal entry before the cut is read"
            );
        }
    }
}

#[test]
fn continuation_restored_run_validates_without_its_history() {
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(7, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    for step in [
        Step::Experiment,
        Step::OpenTransmission,
        Step::Claim,
        Step::Qualify,
        Step::PauseProgram,
        Step::Experiment,
        Step::CloseTransmission,
        Step::Rejected,
        Step::OpenTransmission,
        // The provider result is queued before the cut and consumes its
        // intent after it.
        Step::QueueExperiment,
    ] {
        fixture.run(&mut canwu, step);
    }
    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    let cut = restored.continuation_base().expect("continuation base").cut;
    assert_eq!(
        TechnologyRestoreAccess::history_cut(&restored).expect("history cut"),
        cut,
        "the restored run's history begins at its continuation cut"
    );
    assert!(cut.boundary_count > 0 && cut.ingress_count > 0);
    assert!(restored.ingress_log().is_empty() && restored.boundaries().is_empty());
    assert!(
        knowledge_without_bodies(&restored) > 0,
        "knowledge cites a superseded version the restored run knows by identity"
    );
    validate_technology_runtime_on(&restored).expect("the restored run validates at its cut");

    // The same operations after the cut, on the restored run and on the
    // uninterrupted run.
    let mut twin = canwu;
    let mut twin_fixture = fixture.clone();
    for step in [
        Step::SettleQueued,
        Step::Experiment,
        Step::ClaimFirstAttempt,
        Step::ObserveFirstAttempt,
        Step::CloseTransmission,
        Step::PauseProgram,
        Step::Qualify,
        Step::OpenTransmission,
        Step::Rejected,
    ] {
        fixture.run(&mut restored, step);
        twin_fixture.run(&mut twin, step);
    }
    let consumed = consumed_ingress(&restored);
    assert!(
        consumed.iter().any(|id| id.get() <= cut.ingress_count)
            && consumed.iter().any(|id| id.get() > cut.ingress_count),
        "intents are consumed both before and after the cut"
    );
    validate_technology_runtime_on(&twin).expect("the uninterrupted run validates");
    validate_technology_runtime(&twin).expect("the uninterrupted run replays");

    // No journal entry at or before the cut is read; the entries after it
    // are, for the intents consumed after it.
    let probe = Probe::new(&restored);
    validate_technology_runtime_on(&probe).expect("the restored run validates after the cut");
    {
        let ingress = probe.ingress.borrow();
        let boundaries = probe.boundaries.borrow();
        assert!(!ingress.is_empty() && !boundaries.is_empty());
        assert!(ingress.iter().all(|id| id.get() > cut.ingress_count));
        assert!(boundaries.iter().all(|id| id.get() > cut.boundary_count));
    }

    // A seal after the restore moves the cut again; the run still
    // validates and reads no journal entry before the new cut.
    restored
        .seal_evidence()
        .expect("seal the restored run")
        .expect("a retained tail to seal");
    let sealed_cut = TechnologyRestoreAccess::history_cut(&restored).expect("history cut");
    assert!(sealed_cut.boundary_count > cut.boundary_count);
    let probe = Probe::new(&restored);
    validate_technology_runtime_on(&probe).expect("the sealed run validates");
    assert!(
        probe
            .ingress
            .borrow()
            .iter()
            .all(|id| id.get() > sealed_cut.ingress_count)
            && probe
                .boundaries
                .borrow()
                .iter()
                .all(|id| id.get() > sealed_cut.boundary_count)
    );
}

#[test]
fn superseded_versions_are_accepted_by_identity_and_forged_identities_refused() {
    // A claim cites a transmission's first version as generic evidence; the
    // transmission closes before the cut, and no current record declares
    // the superseded version, so the restored run no longer knows it.
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(13, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    fixture.run(&mut canwu, Step::Experiment);
    fixture.run(&mut canwu, Step::OpenTransmission);
    let transmission =
        TypedDomainRecordRef::<TransmissionOpportunity>::new("transmission-2").into_untyped();
    let cited = current(&canwu, &transmission);
    let asserted_at = canwu.time();
    fixture.command_applied(
        &mut canwu,
        OPERATOR,
        "assert-transmission-claim",
        TechnologyRecordChange::Create {
            id: "transmission-claim".to_owned(),
            value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                asserted_by: KnowledgeHolderRef::Person(OPERATOR),
                proposition: "the transmission was offered".to_owned(),
                scope: vec![],
                source_evidence: vec![EvidenceRef::DomainRecordVersion(cited.clone())],
                relations: vec![],
                asserted_at,
            }),
        },
    );
    fixture.run(&mut canwu, Step::CloseTransmission);
    let closed = current(&canwu, &transmission);
    // One more boundary, so the cut follows the one that closed it.
    fixture.run(&mut canwu, Step::Rejected);
    let now = canwu.time();
    validate_technology_runtime(&canwu).expect("the full run replays");
    validate_technology_runtime_on(&canwu).expect("the full run validates");

    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    assert!(!restored.domain_record_version_evidence_exists(&cited));
    validate_technology_runtime_on(&restored).expect("the restored run validates");

    // Forged identities that the identity rule refuses, each beside the
    // real one it imitates.
    let DomainRecordVersionSource::BoundaryChange {
        boundary: closed_at,
        change_index,
    } = closed.established_by.clone()
    else {
        panic!("a boundary closed the transmission");
    };
    let cut = TechnologyRestoreAccess::history_cut(&restored).expect("history cut");
    assert!(cut.boundary_count > closed_at.get());
    let forged = [
        // The current version number, from another change of its boundary.
        DomainRecordVersionRef {
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: closed_at,
                change_index: change_index + 1,
            },
            ..closed
        },
        // The superseded version number, from a boundary before the cut but
        // after the one that established the current version.
        DomainRecordVersionRef {
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: BoundaryId::new(cut.boundary_count),
                change_index: 0,
            },
            ..cited.clone()
        },
        // A version of a record that was never current.
        DomainRecordVersionRef {
            record: TypedDomainRecordRef::<TechnicalProgram>::new("ghost").into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::BoundaryChange {
                boundary: BoundaryId::new(1),
                change_index: 0,
            },
        },
    ];
    let claim_citing = |evidence: DomainRecordVersionRef| {
        seeded_record(
            TypedDomainRecordRef::<TechnicalClaim>::new("overlay-claim"),
            &TechnicalClaimPayload {
                asserted_by: KnowledgeHolderRef::Person(OPERATOR),
                proposition: "a claim presented by the host".to_owned(),
                scope: vec![],
                source_evidence: vec![EvidenceRef::DomainRecordVersion(evidence)],
                relations: vec![],
                asserted_at: now,
            },
            PayloadRequiredEvidenceContinuationV1::completed(),
        )
    };
    let assert_forged_refused = |label: &str, run: &dyn TechnologyRestoreAccess| {
        for evidence in &forged {
            let error = validate_technology_runtime_on(&Probe::with_extra(
                run,
                claim_citing(evidence.clone()),
            ))
            .expect_err("a forged identity must be refused");
            assert_eq!(error.code, ErrorCode::InvalidDomainRecord, "{label}");
            assert!(
                error
                    .message
                    .contains("unavailable at its semantic time cut"),
                "{label}: {}",
                error.message
            );
        }
    };
    validate_technology_runtime_on(&Probe::with_extra(&restored, claim_citing(cited.clone())))
        .expect("the real identity is accepted");
    assert_forged_refused("full", &canwu);
    assert_forged_refused("restored", &restored);

    let mut sealed = canwu.into_compacted().expect("compact the run");
    sealed
        .seal_evidence()
        .expect("seal the run")
        .expect("a retained tail to seal");
    assert!(!sealed.domain_record_version_evidence_exists(&cited));
    validate_technology_runtime_on(&sealed).expect("the sealed run validates");
    validate_technology_runtime_on(&Probe::with_extra(&sealed, claim_citing(cited.clone())))
        .expect("the real identity is accepted after a seal");
    assert_forged_refused("sealed", &sealed);
}

#[test]
fn sealed_and_restored_runs_decide_later_operations_as_their_uninterrupted_twin() {
    // A live claim cites a transmission's first version as generic evidence,
    // and the transmission closes: the version is superseded and no current
    // record declares it, so a seal or a continuation restore drops it.
    // Every later technology operation re-checks the live records'
    // citations, and a new claim cites the same version again.
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(17, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    fixture.run(&mut canwu, Step::Experiment);
    fixture.run(&mut canwu, Step::OpenTransmission);
    let transmission =
        TypedDomainRecordRef::<TransmissionOpportunity>::new("transmission-2").into_untyped();
    let cited = current(&canwu, &transmission);
    let asserted_at = canwu.time();
    fixture.command_applied(
        &mut canwu,
        OPERATOR,
        "assert-transmission-claim",
        claim_citing("transmission-claim", &cited, asserted_at),
    );
    fixture.run(&mut canwu, Step::CloseTransmission);

    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    let mut sealed = canwu.fork().into_compacted().expect("compact the run");
    sealed
        .seal_evidence()
        .expect("seal the run")
        .expect("a retained tail to seal");
    assert!(!restored.domain_record_version_evidence_exists(&cited));
    assert!(!sealed.domain_record_version_evidence_exists(&cited));

    let uninterrupted = later_operations(&mut fixture.clone(), &mut canwu, &cited);
    assert_eq!(
        uninterrupted,
        [
            TechnologyOperationStatus::Applied,
            TechnologyOperationStatus::Applied
        ],
        "the uninterrupted run applies both operations"
    );
    for (label, statuses, hash) in [
        (
            "restored",
            later_operations(&mut fixture.clone(), &mut restored, &cited),
            restored.checkpoint_hash().to_owned(),
        ),
        (
            "sealed",
            later_operations(&mut fixture.clone(), &mut sealed, &cited),
            sealed.checkpoint_hash().to_owned(),
        ),
    ] {
        assert_eq!(statuses, uninterrupted, "{label}");
        assert_eq!(hash, canwu.checkpoint_hash(), "{label}");
    }
    validate_technology_runtime(&canwu).expect("the uninterrupted run replays");
    for run in [&restored, &sealed] {
        validate_technology_runtime_on(run).expect("the run validates");
    }
}

#[test]
fn sealed_and_restored_runs_keep_the_bodies_a_superseded_version_cites() {
    // A transmission's source is an implementation's first version, whose
    // body cites the qualification's first version. The qualification is
    // renewed and the implementation moved onto the renewal: both first
    // versions are superseded, and only the superseded implementation body
    // cites the first qualification. Every later technology operation reads
    // that body through the transmission's source. One run is sealed before
    // the move and again after it, others after it.
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(23, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    let source = install_source(&mut fixture, &mut canwu);
    let opened_at = canwu.time();
    fixture.command_applied(
        &mut canwu,
        OPERATOR,
        "open-taught-transmission",
        TechnologyRecordChange::Create {
            id: "taught-transmission".to_owned(),
            value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
                source: Some(KnowledgeHolderRef::Person(OPERATOR)),
                source_site: Some(EntityRef::Territory(WORKSHOP)),
                source_capability: Some(source.clone()),
                external_source: None,
                destination: KnowledgeHolderRef::Person(LEARNER),
                destination_site: EntityRef::Territory(TRAINING_GROUND),
                revision: Some(revision()),
                mode: TransmissionMode::Demonstration,
                evidence: vec![],
                resulting_program: None,
                opened_at,
                active: true,
            }),
        },
    );
    let mut resealed = canwu.fork().into_compacted().expect("compact the run");
    resealed
        .seal_evidence()
        .expect("seal the run")
        .expect("a retained tail to seal");
    renew_and_move_source(&mut fixture.clone(), &mut resealed);
    renew_and_move_source(&mut fixture, &mut canwu);
    resealed
        .seal_evidence()
        .expect("seal the run again")
        .expect("a retained tail to seal");
    assert_ne!(current(&canwu, &source.record), source);

    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    let mut sealed = canwu.fork().into_compacted().expect("compact the run");
    sealed
        .seal_evidence()
        .expect("seal the run")
        .expect("a retained tail to seal");

    let uninterrupted = later_transmission(&mut fixture.clone(), &mut canwu);
    assert_eq!(uninterrupted, TechnologyOperationStatus::Applied);
    for (label, run) in [
        ("restored", &mut restored),
        ("sealed", &mut sealed),
        ("resealed", &mut resealed),
    ] {
        assert_eq!(
            later_transmission(&mut fixture.clone(), run),
            uninterrupted,
            "{label}"
        );
        assert_eq!(run.checkpoint_hash(), canwu.checkpoint_hash(), "{label}");
        validate_technology_runtime_on(&*run).expect("the run validates");
    }
    validate_technology_runtime(&canwu).expect("the uninterrupted run replays");
}

#[test]
fn a_rejected_result_names_no_intent_the_run_does_not_keep() {
    // A provider result names the superseded first version of an
    // implementation as its intent, and is rejected. An outcome record
    // declares the intent it names, and loading follows that body, whose
    // first qualification no current record declares; so the outcome names
    // none, and later operations apply on every run.
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(31, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    let source = install_source(&mut fixture, &mut canwu);
    renew_and_move_source(&mut fixture, &mut canwu);
    let attempt = fixture.attempts.last().cloned().expect("an attempt");
    let observed_at = canwu.time();
    let misdirected = provider_result(
        &mut canwu,
        TechnologyResultEnvelope {
            id: "misdirected-result".to_owned(),
            provider: METER.to_owned(),
            execution_intent: Some(source),
            change: TechnologyRecordChange::Create {
                id: "misdirected-observation".to_owned(),
                value: TechnologyRecordPayload::AttemptObservation(AttemptObservationPayload {
                    attempt,
                    observer: guild_holder(),
                    method: "metered fuel accounting".to_owned(),
                    values: vec![MetricValue {
                        metric: fuel_cost(),
                        value: 400,
                    }],
                    uncertainty_per_mille: 50,
                    observed_at,
                }),
            },
        },
    );
    assert_eq!(misdirected, TechnologyOperationStatus::Rejected);
    let outcome = canwu_api::typed_domain_record(
        &canwu,
        &TypedDomainRecordRef::<TechnologyOperation>::new("misdirected-result"),
    )
    .expect("terminal operation outcome")
    .decode_payload::<TechnologyOperation>()
    .expect("operation payload");
    assert_eq!(outcome.execution_intent, None);

    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    let mut sealed = canwu.fork().into_compacted().expect("compact the run");
    sealed
        .seal_evidence()
        .expect("seal the run")
        .expect("a retained tail to seal");
    let uninterrupted = later_transmission(&mut fixture.clone(), &mut canwu);
    assert_eq!(uninterrupted, TechnologyOperationStatus::Applied);
    for (label, run) in [("restored", &mut restored), ("sealed", &mut sealed)] {
        assert_eq!(
            later_transmission(&mut fixture.clone(), run),
            uninterrupted,
            "{label}"
        );
        assert_eq!(run.checkpoint_hash(), canwu.checkpoint_hash(), "{label}");
        validate_technology_runtime_on(&*run).expect("the run validates");
    }
    validate_technology_runtime(&canwu).expect("the uninterrupted run replays");
}

/// Qualifies the operator by the latest attempt and installs an
/// implementation on that qualification; the implementation's first version.
fn install_source<R: Drive + ?Sized>(fixture: &mut Fixture, run: &mut R) -> DomainRecordVersionRef {
    fixture.run(run, Step::Experiment);
    fixture.command_applied(
        run,
        OPERATOR,
        "qualify-source",
        TechnologyRecordChange::Create {
            id: "source-qualification".to_owned(),
            value: TechnologyRecordPayload::Capability(source_qualification(
                fixture.attempts.clone(),
            )),
        },
    );
    let qualification = current(&*run, &source_qualification_ref());
    fixture.command_applied(
        run,
        OPERATOR,
        "install-source",
        TechnologyRecordChange::Create {
            id: "source-implementation".to_owned(),
            value: TechnologyRecordPayload::Implementation(source_implementation(qualification)),
        },
    );
    current(
        &*run,
        &TypedDomainRecordRef::<ImplementationRecord>::new("source-implementation").into_untyped(),
    )
}

/// Renews the source qualification with a further attempt and moves the
/// implementation onto the renewal, superseding both first versions.
fn renew_and_move_source<R: Drive + ?Sized>(fixture: &mut Fixture, run: &mut R) {
    fixture.run(run, Step::Experiment);
    fixture.command_applied(
        run,
        OPERATOR,
        "renew-source-qualification",
        TechnologyRecordChange::Update {
            id: "source-qualification".to_owned(),
            expected_version: 1,
            value: TechnologyRecordPayload::Capability(source_qualification(
                fixture.attempts.clone(),
            )),
        },
    );
    let renewed = current(&*run, &source_qualification_ref());
    fixture.command_applied(
        run,
        OPERATOR,
        "move-source-implementation",
        TechnologyRecordChange::Update {
            id: "source-implementation".to_owned(),
            expected_version: 1,
            value: TechnologyRecordPayload::Implementation(source_implementation(renewed)),
        },
    );
}

fn source_qualification_ref() -> DomainRecordRef {
    TypedDomainRecordRef::<CapabilityQualification>::new("source-qualification").into_untyped()
}

fn source_qualification(attempts: Vec<DomainRecordVersionRef>) -> CapabilityQualificationPayload {
    CapabilityQualificationPayload {
        holder: KnowledgeHolderRef::Person(OPERATOR),
        operator: Some(EntityRef::Person(OPERATOR)),
        site: EntityRef::Territory(WORKSHOP),
        revision: revision(),
        operation: "operate".to_owned(),
        reliability_per_mille: 1_000,
        attempts,
        last_practiced_at: SimTime::EPOCH,
        valid_from: SimTime::EPOCH,
        valid_until: None,
        active: true,
    }
}

fn source_implementation(qualification: DomainRecordVersionRef) -> ImplementationPayload {
    ImplementationPayload {
        owner: KnowledgeHolderRef::Person(OPERATOR),
        site: EntityRef::Territory(WORKSHOP),
        revision: revision(),
        qualification,
        assets: vec![],
        installed_at: SimTime::EPOCH,
        capacity: 10,
        unit: "runs_per_month".to_owned(),
        reliability_per_mille: 1_000,
        maintenance_provider: None,
        active: true,
    }
}

#[test]
fn new_exact_citations_of_superseded_versions_decide_alike_on_every_handle() {
    // Two destination programs are paused after a transmission names the
    // first as its outcome: a current record holds the first program's first
    // version, and nothing holds the second's. A new transmission naming the
    // held version applies on every run. One naming the other is rejected on
    // every run, though the uninterrupted run still retains that body.
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(29, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    let started_at = canwu.time();
    let destination = TechnicalProgramPayload {
        sponsor: KnowledgeHolderRef::Person(LEARNER),
        site: EntityRef::Territory(TRAINING_GROUND),
        started_at,
        ..program_payload()
    };
    let program = |id: &str| TypedDomainRecordRef::<TechnicalProgram>::new(id).into_untyped();
    for id in ["held-program", "unheld-program"] {
        fixture.command_applied(
            &mut canwu,
            LEARNER,
            &format!("create-{id}"),
            TechnologyRecordChange::Create {
                id: id.to_owned(),
                value: TechnologyRecordPayload::TechnicalProgram(destination.clone()),
            },
        );
    }
    let held = current(&canwu, &program("held-program"));
    let unheld = current(&canwu, &program("unheld-program"));
    fixture.command_applied(
        &mut canwu,
        LEARNER,
        "open-outcome-transmission",
        outcome_transmission("outcome-transmission", &held, started_at),
    );
    for id in ["held-program", "unheld-program"] {
        fixture.command_applied(
            &mut canwu,
            LEARNER,
            &format!("pause-{id}"),
            TechnologyRecordChange::Update {
                id: id.to_owned(),
                expected_version: 1,
                value: TechnologyRecordPayload::TechnicalProgram(TechnicalProgramPayload {
                    status: ProgramStatus::Paused,
                    ..destination.clone()
                }),
            },
        );
    }
    assert!(canwu.domain_record_version_evidence_exists(&unheld));

    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    let mut sealed = canwu.fork().into_compacted().expect("compact the run");
    sealed
        .seal_evidence()
        .expect("seal the run")
        .expect("a retained tail to seal");
    assert!(!restored.domain_record_version_evidence_exists(&unheld));
    assert!(!sealed.domain_record_version_evidence_exists(&unheld));

    let cited = [&held, &unheld];
    let uninterrupted = outcome_operations(&mut fixture.clone(), &mut canwu, cited, started_at);
    assert_eq!(
        uninterrupted,
        [
            TechnologyOperationStatus::Applied,
            TechnologyOperationStatus::Rejected
        ]
    );
    assert_eq!(
        outcome_operations(&mut fixture.clone(), &mut restored, cited, started_at),
        uninterrupted,
        "restored"
    );
    assert_eq!(
        outcome_operations(&mut fixture.clone(), &mut sealed, cited, started_at),
        uninterrupted,
        "sealed"
    );
    for (label, hash) in [
        ("restored", restored.checkpoint_hash()),
        ("sealed", sealed.checkpoint_hash()),
    ] {
        assert_eq!(hash, canwu.checkpoint_hash(), "{label}");
    }
    validate_technology_runtime(&canwu).expect("the uninterrupted run replays");
    for run in [&restored, &sealed] {
        validate_technology_runtime_on(run).expect("the run validates");
    }
}

/// A transmission naming each of `programs` as its outcome; the statuses.
fn outcome_operations<R: Drive + ?Sized>(
    fixture: &mut Fixture,
    run: &mut R,
    programs: [&DomainRecordVersionRef; 2],
    opened_at: SimTime,
) -> [TechnologyOperationStatus; 2] {
    programs.map(|program| {
        let id = format!("outcome-of-{}", program.record.id);
        let change = outcome_transmission(&id, program, opened_at);
        fixture.command(run, LEARNER, &format!("open-{id}"), change)
    })
}

/// An external transmission to the learner whose outcome is `program`.
fn outcome_transmission(
    id: &str,
    program: &DomainRecordVersionRef,
    opened_at: SimTime,
) -> TechnologyRecordChange {
    TechnologyRecordChange::Create {
        id: id.to_owned(),
        value: TechnologyRecordPayload::Transmission(TransmissionOpportunityPayload {
            resulting_program: Some(program.clone()),
            ..external_opening(opened_at)
        }),
    }
}

/// An external transmission opened; the operation's status.
fn later_transmission<R: Drive + ?Sized>(
    fixture: &mut Fixture,
    run: &mut R,
) -> TechnologyOperationStatus {
    let opening = TechnologyRecordChange::Create {
        id: "later-transmission".to_owned(),
        value: TechnologyRecordPayload::Transmission(external_opening(run.time())),
    };
    fixture.command(run, LEARNER, "open-later-transmission", opening)
}

/// A transmission opened, then a claim citing `cited` again; the statuses of
/// both operations.
fn later_operations<R: Drive + ?Sized>(
    fixture: &mut Fixture,
    run: &mut R,
    cited: &DomainRecordVersionRef,
) -> [TechnologyOperationStatus; 2] {
    let opening = TechnologyRecordChange::Create {
        id: "later-transmission".to_owned(),
        value: TechnologyRecordPayload::Transmission(external_opening(run.time())),
    };
    let opened = fixture.command(run, LEARNER, "open-later-transmission", opening);
    let at = run.time();
    let claimed = fixture.command(
        run,
        OPERATOR,
        "assert-later-claim",
        claim_citing("later-claim", cited, at),
    );
    [opened, claimed]
}

fn claim_citing(id: &str, cited: &DomainRecordVersionRef, at: SimTime) -> TechnologyRecordChange {
    TechnologyRecordChange::Create {
        id: id.to_owned(),
        value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
            asserted_by: KnowledgeHolderRef::Person(OPERATOR),
            proposition: "the transmission was offered".to_owned(),
            scope: vec![],
            source_evidence: vec![EvidenceRef::DomainRecordVersion(cited.clone())],
            relations: vec![],
            asserted_at: at,
        }),
    }
}

#[test]
fn technology_records_past_the_shared_cap_are_refused_on_both_handles() {
    // The per-kind cap equals the shared total cap, so one kind past it
    // trips the shared cap first.
    let mut scenario = scenario();
    let limit = canwu_technology::TechnologyLimitsV1::canonical().max_records_per_kind;
    scenario.domain_records.extend((0..=limit).map(|index| {
        TechnologyCatalogRecord::Metric(MetricSchemaPayload {
            label: format!("filler metric {index}"),
            unit: "units".to_owned(),
            scale: 1,
            minimum: 0,
            maximum: 1,
        })
        .into_initial_record(format!("filler-{index}"))
        .expect("filler metric")
    }));
    assert_refused_on_both_handles(scenario, "technology records exceed the shared total cap");
}

#[test]
fn a_noncanonical_payload_continuation_is_refused_on_both_handles() {
    // The kernel accepts an active continuation that names a version the run
    // establishes before its cut; the canonical continuation of a claim that
    // cites only scenario evidence is completed. Learn that version's
    // identity from an untampered run with the same history.
    let mut dry = Canwu::new_with_plugins(REFUSAL_SEED, scenario(), &plugins()).expect("dry run");
    let probe = settle_probe(&mut dry);
    let mut scenario = scenario();
    scenario.domain_records.push(seeded_record(
        TypedDomainRecordRef::<TechnicalClaim>::new("seeded-claim"),
        &TechnicalClaimPayload {
            asserted_by: KnowledgeHolderRef::Person(OPERATOR),
            proposition: "a seeded claim".to_owned(),
            scope: vec![],
            source_evidence: vec![EvidenceRef::DomainRecordVersion(revision())],
            relations: vec![],
            asserted_at: SimTime::EPOCH,
        },
        PayloadRequiredEvidenceContinuationV1::active(vec![EvidenceRef::DomainRecordVersion(
            probe,
        )]),
    ));
    assert_refused_on_both_handles(scenario, "noncanonical payload continuation");
}

#[test]
fn citations_of_versions_no_boundary_established_are_refused_on_both_handles() {
    let at = |record: DomainRecordRef, boundary, version| DomainRecordVersionRef {
        record,
        version,
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: BoundaryId::new(boundary),
            change_index: 0,
        },
    };
    let ghost = TypedDomainRecordRef::<TechnicalProgram>::new("ghost").into_untyped();
    // Generic evidence, as the scenario installs it: a version after the
    // cut, and two before it that the identity rule refuses: a later
    // version of a record whose current version the scenario established,
    // and version zero of a record that is not current.
    for missing in [
        at(revision().record, 1_000, 2),
        at(revision().record, 1, 2),
        at(ghost, 1, 0),
    ] {
        let mut scenario = scenario();
        scenario.domain_records.push(seeded_record(
            TypedDomainRecordRef::<TechnicalClaim>::new("seeded-claim"),
            &TechnicalClaimPayload {
                asserted_by: KnowledgeHolderRef::Person(OPERATOR),
                proposition: "a claim citing a version no boundary established".to_owned(),
                scope: vec![],
                source_evidence: vec![EvidenceRef::DomainRecordVersion(missing)],
                relations: vec![],
                asserted_at: SimTime::EPOCH,
            },
            PayloadRequiredEvidenceContinuationV1::completed(),
        ));
        assert_refused_on_both_handles(scenario, "unavailable at its semantic time cut");
    }

    // An exact version whose body is required. The kernel refuses to write a
    // continuation checkpoint that declares a missing body, so a host-side
    // overlay presents the record over each handle.
    let (full, restored) = full_and_restored(15, &[Step::Experiment]);
    let missing_attempt = DomainRecordVersionRef {
        record: TypedDomainRecordRef::<ExperimentAttempt>::new("attempt-1").into_untyped(),
        version: 2,
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: BoundaryId::new(1_000),
            change_index: 0,
        },
    };
    let observation = seeded_record(
        TypedDomainRecordRef::<AttemptObservation>::new("forged-observation"),
        &AttemptObservationPayload {
            attempt: missing_attempt.clone(),
            observer: guild_holder(),
            method: "forged".to_owned(),
            values: vec![],
            uncertainty_per_mille: 50,
            observed_at: SimTime::EPOCH,
        },
        PayloadRequiredEvidenceContinuationV1::active(vec![EvidenceRef::DomainRecordVersion(
            missing_attempt,
        )]),
    );
    for (label, result) in [
        (
            "full",
            validate_technology_runtime_on(&Probe::with_extra(&full, observation.clone())),
        ),
        (
            "restored",
            validate_technology_runtime_on(&Probe::with_extra(&restored, observation.clone())),
        ),
    ] {
        let error = result.expect_err("a missing exact body must be refused");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord, "{label}");
        assert!(
            error.message.contains("exact technology record body"),
            "{label}: {}",
            error.message
        );
    }
}

#[test]
fn an_external_source_the_scenario_did_not_establish_is_refused_on_both_handles() {
    let established_later = DomainRecordVersionRef {
        record: content_reference().into_untyped(),
        version: 2,
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: BoundaryId::new(1),
            change_index: 0,
        },
    };
    let never_established = DomainRecordVersionRef {
        record: content_reference().into_untyped(),
        version: 2,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    for (evidence, message) in [
        (established_later, "manifest-bound content record"),
        (never_established, "unavailable"),
    ] {
        let mut scenario = scenario();
        scenario.domain_records.push(seeded_record(
            TypedDomainRecordRef::<TransmissionOpportunity>::new("seeded-transmission"),
            &TransmissionOpportunityPayload {
                external_source: Some(ExternalTransmissionSourceV1 {
                    evidence: EvidenceRef::DomainRecordVersion(evidence),
                    declared_reliability_per_mille: 650,
                }),
                ..external_opening(SimTime::EPOCH)
            },
            PayloadRequiredEvidenceContinuationV1::completed(),
        ));
        assert_refused_on_both_handles(scenario, message);
    }
}

#[test]
fn an_intent_consumed_twice_is_refused_on_both_handles() {
    // The kernel and the plugin admit no second consumption, so the second
    // applied operation is presented by a host-side overlay over each
    // handle, as a corrupted persisted state would present it. Episode 1
    // consumes its intent before the cut, episode 2 after it.
    let plugins = plugins();
    let mut full = Canwu::new_with_plugins(9, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    fixture.run(&mut full, Step::Experiment);
    let (checkpoint, hash) = full.continuation_checkpoint().expect("continuation cut");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    let cut = restored.continuation_base().expect("continuation base").cut;
    fixture.clone().run(&mut full, Step::Experiment);
    fixture.run(&mut restored, Step::Experiment);

    for episode in ["1", "2"] {
        let replayed = replayed_operation(&restored, episode);
        let on_full = validate_technology_runtime_on(&Probe::with_extra(&full, replayed.clone()));
        let overlay = Probe::with_extra(&restored, replayed);
        let on_restored = validate_technology_runtime_on(&overlay);
        assert!(
            overlay
                .ingress
                .borrow()
                .iter()
                .all(|id| id.get() > cut.ingress_count)
        );
        for (label, result) in [("full", on_full), ("restored", on_restored)] {
            let error = result.expect_err("a second consumption must be refused");
            assert_eq!(error.code, ErrorCode::InvalidDomainRecord, "{label}");
            assert!(
                error
                    .message
                    .contains("consumed technology intent points to another operation"),
                "{label} episode {episode}: {}",
                error.message
            );
        }
    }
}

/// Runs the steps on a full run and restores a second handle from its
/// continuation checkpoint at their end.
fn full_and_restored(seed: u64, steps: &[Step]) -> (Canwu, CompactedCanwu) {
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(seed, scenario(), &plugins).expect("technology run");
    let mut fixture = Fixture::default();
    for step in steps {
        fixture.run(&mut canwu, *step);
    }
    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("restore from the continuation checkpoint");
    (canwu, restored)
}

/// How many technology knowledge records cite a version whose body the run
/// no longer holds but whose identity it keeps.
fn knowledge_without_bodies(run: &CompactedCanwu) -> usize {
    run.knowledge()
        .records
        .values()
        .flat_map(HolderLedger::values)
        .filter(|record| record.schema.kind.namespace == "canwu.technology")
        .filter(|record| {
            record.origin.evidence.iter().any(|evidence| {
                matches!(
                    evidence,
                    EvidenceRef::DomainRecordVersion(version)
                        if run.domain_record_version(version).is_none()
                            && run.domain_record_version_evidence_exists(version)
                )
            })
        })
        .count()
}

const REFUSAL_SEED: u64 = 11;

/// Builds the run, settles a rejected probe operation so the continuation
/// cut is past the scenario, and expects the same refusal from both
/// validators on the full run and from the restore validator on the run
/// restored from its continuation checkpoint.
fn assert_refused_on_both_handles(scenario: Scenario, message: &str) {
    let plugins = plugins();
    let mut canwu = Canwu::new_with_plugins(REFUSAL_SEED, scenario, &plugins)
        .expect("the kernel accepts the schema-valid record");
    // Over the cap the plugin writes no outcome record; the probe settles
    // its boundaries either way.
    Fixture::default().submit_rejected(&mut canwu);
    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("continuation cut");
    let restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the kernel restores the schema-valid state");
    assert!(
        restored
            .continuation_base()
            .is_some_and(|base| base.cut.boundary_count > 0)
    );
    for (label, result) in [
        ("full journal", validate_technology_runtime(&canwu)),
        ("restore on full", validate_technology_runtime_on(&canwu)),
        (
            "restore on restored",
            validate_technology_runtime_on(&restored),
        ),
    ] {
        let error = result.expect_err("tampered technology state must be refused");
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord, "{label}");
        assert!(
            error.message.contains(message),
            "{label}: {}",
            error.message
        );
    }
}

/// Settles one always-rejected operation and returns the exact version of
/// its outcome record.
fn settle_probe(canwu: &mut Canwu) -> DomainRecordVersionRef {
    let mut fixture = Fixture::default();
    fixture.run(canwu, Step::Rejected);
    current(
        canwu,
        &TypedDomainRecordRef::<TechnologyOperation>::new("reject-1").into_untyped(),
    )
}

/// A second applied operation consuming the intent of one experiment
/// episode, with its own canonical payload continuation.
fn replayed_operation(run: &CompactedCanwu, episode: &str) -> DomainRecord {
    let original = canwu_api::typed_domain_record(
        run,
        &TypedDomainRecordRef::<TechnologyOperation>::new(format!("result-attempt-{episode}")),
    )
    .expect("the consuming operation")
    .decode_payload::<TechnologyOperation>()
    .expect("operation payload");
    assert_eq!(original.status, TechnologyOperationStatus::Applied);
    let pending = original
        .execution_intent
        .clone()
        .expect("the consumed intent");
    let mut replayed = original;
    replayed.id = format!("replayed-{episode}");
    seeded_record(
        TypedDomainRecordRef::<TechnologyOperation>::new(replayed.id.clone()),
        &replayed,
        PayloadRequiredEvidenceContinuationV1::active(vec![EvidenceRef::DomainRecordVersion(
            pending,
        )]),
    )
}

// Generated operations, driven on either handle.

#[derive(Clone, Copy, Debug)]
enum Step {
    /// A program, an authorized experiment intent, the provider's result
    /// that consumes it, and a passive observation by a domain entity.
    Experiment,
    /// An experiment whose provider result is queued and not yet settled.
    QueueExperiment,
    /// Settles the queued provider result and observes its attempt.
    SettleQueued,
    OpenTransmission,
    CloseTransmission,
    Claim,
    ClaimFirstAttempt,
    ObserveFirstAttempt,
    /// A capability qualified by the latest attempt, then deactivated, so
    /// its published knowledge cites a superseded version.
    Qualify,
    PauseProgram,
    Rejected,
}

fn generated_steps(seed: u64, count: usize) -> Vec<Step> {
    let mut state = seed;
    let mut steps = vec![Step::Experiment, Step::OpenTransmission];
    while steps.len() < count {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        steps.push(match (state >> 33) % 9 {
            0 | 1 => Step::Experiment,
            2 => Step::OpenTransmission,
            3 => Step::CloseTransmission,
            4 => Step::Claim,
            5 => Step::ObserveFirstAttempt,
            6 => Step::PauseProgram,
            7 => Step::Qualify,
            _ => Step::Rejected,
        });
    }
    steps
}

/// An experiment whose provider result is queued: its episode, program, and
/// attempt IDs.
#[derive(Clone)]
struct QueuedExperiment {
    episode: u64,
    program: String,
    attempt: String,
}

#[derive(Clone, Default)]
struct Fixture {
    request: u64,
    episode: u64,
    active_programs: Vec<String>,
    attempts: Vec<DomainRecordVersionRef>,
    open_transmissions: Vec<String>,
    queued: Option<QueuedExperiment>,
}

impl Fixture {
    fn next(&mut self) -> u64 {
        self.episode += 1;
        self.episode
    }

    fn run<R: Drive + ?Sized>(&mut self, run: &mut R, step: Step) {
        match step {
            Step::Experiment => {
                let queued = self.queue_experiment(run);
                self.settle_experiment(run, queued);
            }
            Step::QueueExperiment => {
                let queued = self.queue_experiment(run);
                self.queued = Some(queued);
            }
            Step::SettleQueued => {
                let queued = self.queued.take().expect("a queued experiment");
                self.settle_experiment(run, queued);
            }
            Step::Qualify => match self.attempts.last().cloned() {
                Some(attempt) => self.qualify(run, attempt),
                None => self.run(run, Step::Experiment),
            },
            Step::OpenTransmission => self.open_transmission(run),
            Step::CloseTransmission => {
                if self.open_transmissions.is_empty() {
                    self.open_transmission(run);
                } else {
                    let id = self.open_transmissions.remove(0);
                    let closed = TransmissionOpportunityPayload {
                        active: false,
                        ..external_opening(SimTime::EPOCH)
                    };
                    self.command_applied(
                        run,
                        LEARNER,
                        &format!("close-{id}"),
                        TechnologyRecordChange::Update {
                            id,
                            expected_version: 1,
                            value: TechnologyRecordPayload::Transmission(closed),
                        },
                    );
                }
            }
            Step::Claim | Step::ClaimFirstAttempt => {
                let n = self.next();
                let cited = if matches!(step, Step::Claim) {
                    self.attempts.last()
                } else {
                    self.attempts.first()
                }
                .cloned()
                .unwrap_or_else(revision);
                let asserted_at = run.time();
                self.command_applied(
                    run,
                    OPERATOR,
                    &format!("assert-claim-{n}"),
                    TechnologyRecordChange::Create {
                        id: format!("claim-{n}"),
                        value: TechnologyRecordPayload::TechnicalClaim(TechnicalClaimPayload {
                            asserted_by: KnowledgeHolderRef::Person(OPERATOR),
                            proposition: format!("claim {n} holds"),
                            scope: vec![],
                            source_evidence: vec![EvidenceRef::DomainRecordVersion(cited)],
                            relations: vec![],
                            asserted_at,
                        }),
                    },
                );
            }
            Step::ObserveFirstAttempt => match self.attempts.first().cloned() {
                Some(attempt) => {
                    let n = self.next();
                    observe(run, n, attempt);
                }
                None => self.run(run, Step::Experiment),
            },
            Step::PauseProgram => {
                if self.active_programs.is_empty() {
                    self.run(run, Step::Experiment);
                } else {
                    let id = self.active_programs.remove(0);
                    let mut paused = program_payload();
                    paused.status = ProgramStatus::Paused;
                    self.command_applied(
                        run,
                        OPERATOR,
                        &format!("pause-{id}"),
                        TechnologyRecordChange::Update {
                            id,
                            expected_version: 1,
                            value: TechnologyRecordPayload::TechnicalProgram(paused),
                        },
                    );
                }
            }
            Step::Rejected => {
                let id = self.submit_rejected(run);
                assert_eq!(status(run, &id), TechnologyOperationStatus::Rejected);
            }
        }
    }

    /// Submits and settles a transmission citing both a live capability and
    /// an external source, which is always rejected, and returns its
    /// operation ID.
    fn submit_rejected<R: Drive + ?Sized>(&mut self, run: &mut R) -> String {
        let n = self.next();
        let both = TransmissionOpportunityPayload {
            source_capability: Some(revision()),
            ..external_opening(run.time())
        };
        let id = format!("reject-{n}");
        self.submit(
            run,
            LEARNER,
            &id,
            TechnologyRecordChange::Create {
                id: format!("rejected-{n}"),
                value: TechnologyRecordPayload::Transmission(both),
            },
        );
        id
    }

    /// Creates a program, authorizes an experiment intent, and queues the
    /// provider result that will consume it.
    fn queue_experiment<R: Drive + ?Sized>(&mut self, run: &mut R) -> QueuedExperiment {
        let n = self.next();
        let program_id = format!("program-{n}");
        self.command_applied(
            run,
            OPERATOR,
            &format!("create-{program_id}"),
            TechnologyRecordChange::Create {
                id: program_id.clone(),
                value: TechnologyRecordPayload::TechnicalProgram(program_payload()),
            },
        );
        let program = current(
            run,
            &TypedDomainRecordRef::<TechnicalProgram>::new(program_id.clone()).into_untyped(),
        );
        let intent_id = format!("intent-{n}");
        let attempt_id = format!("attempt-{n}");
        let not_before = run.time();
        self.command_applied(
            run,
            OPERATOR,
            &format!("authorize-{intent_id}"),
            TechnologyRecordChange::Create {
                id: intent_id.clone(),
                value: TechnologyRecordPayload::ExecutionIntent(TechnologyExecutionIntentPayload {
                    authorized_by: KnowledgeHolderRef::Person(OPERATOR),
                    program: program.clone(),
                    provider: LAB.to_owned(),
                    request: TechnologyIntentRequest::Experiment {
                        result_id: attempt_id.clone(),
                        revision: revision(),
                        operation: "operate".to_owned(),
                        site: EntityRef::Territory(WORKSHOP),
                        operator: Some(KnowledgeHolderRef::Person(OPERATOR)),
                        required_assets: vec![],
                    },
                    not_before,
                    expires_at: None,
                    state: TechnologyIntentState::Pending,
                }),
            },
        );
        let intent = current(
            run,
            &TypedDomainRecordRef::<TechnologyExecutionIntent>::new(intent_id).into_untyped(),
        );
        let attempt = ExperimentAttemptPayload {
            execution_intent: intent.clone(),
            program,
            revision: revision(),
            operator: KnowledgeHolderRef::Person(OPERATOR),
            site: EntityRef::Territory(WORKSHOP),
            operation: "operate".to_owned(),
            inputs: vec![],
            environment: vec![],
            outputs: vec![MetricValue {
                metric: reliability(),
                value: 820,
            }],
            assets: vec![],
            started_at: SimTime::EPOCH,
            ended_at: SimTime::EPOCH,
            evaluation: evaluate_attempt(
                &revision_payload(),
                &spec_payload(),
                &BTreeMap::from([(reliability().record, reliability_payload())]),
                &MetricContext {
                    values: BTreeMap::from([(reliability().record, 820)]),
                },
            )
            .expect("attempt evaluates"),
        };
        submit_result(
            run,
            TechnologyResultEnvelope {
                id: format!("result-{attempt_id}"),
                provider: LAB.to_owned(),
                execution_intent: Some(intent),
                change: TechnologyRecordChange::Create {
                    id: attempt_id.clone(),
                    value: TechnologyRecordPayload::ExperimentAttempt(attempt),
                },
            },
        );
        QueuedExperiment {
            episode: n,
            program: program_id,
            attempt: attempt_id,
        }
    }

    /// Settles a queued provider result, which consumes its intent, and
    /// observes the attempt.
    fn settle_experiment<R: Drive + ?Sized>(&mut self, run: &mut R, queued: QueuedExperiment) {
        run.settle();
        assert_eq!(
            status(run, &format!("result-{}", queued.attempt)),
            TechnologyOperationStatus::Applied
        );
        let attempt = current(
            run,
            &TypedDomainRecordRef::<ExperimentAttempt>::new(queued.attempt).into_untyped(),
        );
        observe(run, queued.episode, attempt.clone());
        self.active_programs.push(queued.program);
        self.attempts.push(attempt);
    }

    fn qualify<R: Drive + ?Sized>(&mut self, run: &mut R, attempt: DomainRecordVersionRef) {
        let n = self.next();
        let id = format!("capability-{n}");
        let qualification = CapabilityQualificationPayload {
            holder: KnowledgeHolderRef::Person(OPERATOR),
            operator: Some(EntityRef::Person(OPERATOR)),
            site: EntityRef::Territory(WORKSHOP),
            revision: revision(),
            operation: "operate".to_owned(),
            reliability_per_mille: 1_000,
            attempts: vec![attempt],
            last_practiced_at: SimTime::EPOCH,
            valid_from: SimTime::EPOCH,
            valid_until: None,
            active: true,
        };
        self.command_applied(
            run,
            OPERATOR,
            &format!("qualify-{n}"),
            TechnologyRecordChange::Create {
                id: id.clone(),
                value: TechnologyRecordPayload::Capability(qualification.clone()),
            },
        );
        self.command_applied(
            run,
            OPERATOR,
            &format!("deactivate-{n}"),
            TechnologyRecordChange::Update {
                id,
                expected_version: 1,
                value: TechnologyRecordPayload::Capability(CapabilityQualificationPayload {
                    active: false,
                    ..qualification
                }),
            },
        );
    }

    fn open_transmission<R: Drive + ?Sized>(&mut self, run: &mut R) {
        let n = self.next();
        let id = format!("transmission-{n}");
        let opening = external_opening(run.time());
        self.command_applied(
            run,
            LEARNER,
            &format!("open-{id}"),
            TechnologyRecordChange::Create {
                id: id.clone(),
                value: TechnologyRecordPayload::Transmission(opening),
            },
        );
        self.open_transmissions.push(id);
    }

    fn command_applied<R: Drive + ?Sized>(
        &mut self,
        run: &mut R,
        actor: PersonId,
        id: &str,
        change: TechnologyRecordChange,
    ) {
        let status = self.command(run, actor, id, change);
        assert_eq!(status, TechnologyOperationStatus::Applied, "{id}");
    }

    fn command<R: Drive + ?Sized>(
        &mut self,
        run: &mut R,
        actor: PersonId,
        id: &str,
        change: TechnologyRecordChange,
    ) -> TechnologyOperationStatus {
        self.submit(run, actor, id, change);
        status(run, id)
    }

    /// Submits one technology command and settles its admission and
    /// application boundaries.
    fn submit<R: Drive + ?Sized>(
        &mut self,
        run: &mut R,
        actor: PersonId,
        id: &str,
        change: TechnologyRecordChange,
    ) {
        self.request += 1;
        let request = CommandRequest::new(
            CommandRequestId::new(self.request),
            run.revision(),
            CommandEnvelope::new(
                Issuer::Actor(actor),
                Command::Plugin {
                    plugin: "canwu-technology".to_owned(),
                    command: TECHNOLOGY_COMMAND.to_owned(),
                    payload: serde_json::to_value(TechnologyCommandEnvelope {
                        id: id.to_owned(),
                        subject: KnowledgeHolderRef::Person(actor),
                        change,
                    })
                    .expect("command payload"),
                },
            )
            .at_time(run.time()),
        );
        run.submit_command(request);
        run.settle();
        run.settle();
    }
}

/// A passive provider's observation of an attempt by a domain-entity
/// knowledge holder.
fn observe<R: Drive + ?Sized>(run: &mut R, n: u64, attempt: DomainRecordVersionRef) {
    let observed_at = run.time();
    assert_eq!(
        provider_result(
            run,
            TechnologyResultEnvelope {
                id: format!("observe-{n}"),
                provider: METER.to_owned(),
                execution_intent: None,
                change: TechnologyRecordChange::Create {
                    id: format!("observation-{n}"),
                    value: TechnologyRecordPayload::AttemptObservation(AttemptObservationPayload {
                        attempt,
                        observer: guild_holder(),
                        method: "metered fuel accounting".to_owned(),
                        values: vec![MetricValue {
                            metric: fuel_cost(),
                            value: 400,
                        }],
                        uncertainty_per_mille: 50,
                        observed_at,
                    }),
                },
            },
        ),
        TechnologyOperationStatus::Applied
    );
}

fn provider_result<R: Drive + ?Sized>(
    run: &mut R,
    envelope: TechnologyResultEnvelope,
) -> TechnologyOperationStatus {
    let id = envelope.id.clone();
    submit_result(run, envelope);
    run.settle();
    status(run, &id)
}

/// Queues a provider result without settling it.
fn submit_result<R: Drive + ?Sized>(run: &mut R, envelope: TechnologyResultEnvelope) {
    run.submit_ingress(PluginIngressRequest::new(
        "canwu-technology",
        TECHNOLOGY_RESULT_INGRESS,
        run.time(),
        serde_json::to_value(envelope).expect("result payload"),
    ));
}

fn status<R: CanwuRun + ?Sized>(run: &R, id: &str) -> TechnologyOperationStatus {
    canwu_api::typed_domain_record(run, &TypedDomainRecordRef::<TechnologyOperation>::new(id))
        .expect("terminal operation outcome")
        .decode_payload::<TechnologyOperation>()
        .expect("operation payload")
        .status
}

fn current<R: CanwuRun + ?Sized>(run: &R, reference: &DomainRecordRef) -> DomainRecordVersionRef {
    run.current_domain_record_version(reference)
        .expect("current provenance")
        .expect("current record")
}

fn consumed_ingress<R: CanwuRun + ?Sized>(run: &R) -> Vec<IngressId> {
    run.domain_records_of_kind(&DomainRecordKind::for_type::<TechnologyExecutionIntent>())
        .filter_map(|record| {
            match record
                .decode_payload::<TechnologyExecutionIntent>()
                .expect("intent payload")
                .state
            {
                TechnologyIntentState::Consumed {
                    ingress: EvidenceRef::Ingress(id),
                    ..
                } => Some(id),
                _ => None,
            }
        })
        .collect()
}

fn external_transmissions<R: CanwuRun + ?Sized>(run: &R) -> usize {
    run.domain_records_of_kind(&DomainRecordKind::for_type::<TransmissionOpportunity>())
        .filter(|record| {
            record
                .decode_payload::<TransmissionOpportunity>()
                .expect("transmission payload")
                .external_source
                .is_some()
        })
        .count()
}

/// The mutating calls the fixture makes, on either run handle.
trait Drive: CanwuRun {
    fn submit_command(&mut self, request: CommandRequest);
    fn submit_ingress(&mut self, request: PluginIngressRequest);
    fn settle(&mut self);
}

macro_rules! impl_drive {
    ($handle:ty) => {
        impl Drive for $handle {
            fn submit_command(&mut self, request: CommandRequest) {
                let due_at = CanwuRun::time(self);
                <$handle>::enqueue_command(self, due_at, 0, request).expect("command enqueues");
            }

            fn submit_ingress(&mut self, request: PluginIngressRequest) {
                <$handle>::enqueue_plugin_ingress(self, request).expect("ingress enqueues");
            }

            fn settle(&mut self) {
                let at = CanwuRun::time(self);
                <$handle>::settle_boundary(self, BoundaryRequest::at(at))
                    .expect("boundary settles");
            }
        }
    };
}

impl_drive!(Canwu);
impl_drive!(CompactedCanwu);

/// Reads one handle through [`TechnologyRestoreAccess`], records each
/// journal entry the validator asks for, and can present extra current
/// records, as a host whose persisted state was altered would.
struct Probe<'a, H: ?Sized> {
    run: &'a H,
    extra: Vec<DomainRecord>,
    ingress: RefCell<Vec<IngressId>>,
    boundaries: RefCell<Vec<BoundaryId>>,
}

impl<'a, H: TechnologyRestoreAccess + ?Sized> Probe<'a, H> {
    fn new(run: &'a H) -> Self {
        Self {
            run,
            extra: Vec::new(),
            ingress: RefCell::default(),
            boundaries: RefCell::default(),
        }
    }

    fn with_extra(run: &'a H, record: DomainRecord) -> Self {
        Self {
            extra: vec![record],
            ..Self::new(run)
        }
    }
}

impl<H: TechnologyRestoreAccess + ?Sized> TechnologyRestoreAccess for Probe<'_, H> {
    fn committed_time(&self) -> SimTime {
        self.run.committed_time()
    }

    fn current_records_of_kind(
        &self,
        kind: &DomainRecordKind,
    ) -> Box<dyn Iterator<Item = &DomainRecord> + '_> {
        let wanted = kind.clone();
        Box::new(
            self.run.current_records_of_kind(kind).chain(
                self.extra
                    .iter()
                    .filter(move |record| record.reference.kind == wanted),
            ),
        )
    }

    fn current_record_version(
        &self,
        reference: &DomainRecordRef,
    ) -> Result<Option<DomainRecordVersionRef>, CanwuError> {
        self.run.current_record_version(reference)
    }

    fn record_version_body(&self, reference: &DomainRecordVersionRef) -> Option<DomainRecord> {
        self.run.record_version_body(reference)
    }

    fn record_version_known(&self, reference: &DomainRecordVersionRef) -> bool {
        self.run.record_version_known(reference)
    }

    fn evidence_known(&self, reference: &EvidenceRef) -> bool {
        self.run.evidence_known(reference)
    }

    fn evidence_established_at(&self, reference: &EvidenceRef) -> Option<SimTime> {
        self.run.evidence_established_at(reference)
    }

    fn committed_knowledge(&self) -> &KnowledgeSnapshot {
        self.run.committed_knowledge()
    }

    fn history_cut(&self) -> Result<EvidenceCursor, CanwuError> {
        self.run.history_cut()
    }

    fn retained_ingress(&self, id: IngressId) -> Option<&IngressRecord> {
        self.ingress.borrow_mut().push(id);
        self.run.retained_ingress(id)
    }

    fn retained_boundary(&self, id: BoundaryId) -> Option<&BoundaryRecord> {
        self.boundaries.borrow_mut().push(id);
        self.run.retained_boundary(id)
    }
}

// The scenario, its plugins, and record helpers.

fn reliability() -> DomainRecordVersionRef {
    initial_record_version::<MetricSchema>("reliability")
}

fn fuel_cost() -> DomainRecordVersionRef {
    initial_record_version::<MetricSchema>("fuel-cost")
}

fn revision() -> DomainRecordVersionRef {
    initial_record_version::<TechniqueRevision>("revision")
}

fn reliability_payload() -> MetricSchemaPayload {
    MetricSchemaPayload {
        label: "reliability".to_owned(),
        unit: "permille".to_owned(),
        scale: 1_000,
        minimum: 0,
        maximum: 1_000,
    }
}

fn spec_payload() -> TechniqueSpecPayload {
    TechniqueSpecPayload {
        label: "neutral pressure converter".to_owned(),
        function: "convert pressure into useful work".to_owned(),
        requirements: vec![RequirementGroup {
            id: "reliable_output".to_owned(),
            any_of: vec![MetricThreshold {
                id: "reliability_floor".to_owned(),
                metric: reliability(),
                comparison: MetricComparison::AtLeast,
                value: 700,
            }],
        }],
        qualification_rules: vec![QualificationRule {
            operation: "operate".to_owned(),
            minimum_successful_attempts: 1,
            minimum_reliability_per_mille: 700,
            independent_reproduction_required: false,
        }],
    }
}

fn revision_payload() -> TechniqueRevisionPayload {
    TechniqueRevisionPayload {
        label: "neutral revision".to_owned(),
        spec: initial_record_version::<TechniqueSpec>("technique"),
        parents: vec![],
        parameters: vec![],
        evaluator: REFERENCE_EVALUATOR_V1.to_owned(),
        produced_by: None,
        execution_intent: None,
        discovery_evidence: vec![],
    }
}

fn program_payload() -> TechnicalProgramPayload {
    TechnicalProgramPayload {
        sponsor: KnowledgeHolderRef::Person(OPERATOR),
        site: EntityRef::Territory(WORKSHOP),
        revision: Some(revision()),
        mode: ProgramMode::Adaptation,
        status: ProgramStatus::Active,
        requirements: vec![],
        started_at: SimTime::EPOCH,
        due_at: None,
    }
}

fn external_opening(opened_at: SimTime) -> TransmissionOpportunityPayload {
    TransmissionOpportunityPayload {
        source: None,
        source_site: None,
        source_capability: None,
        external_source: Some(ExternalTransmissionSourceV1 {
            evidence: EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
                record: content_reference().into_untyped(),
                version: 1,
                established_by: DomainRecordVersionSource::InitialScenario,
            }),
            declared_reliability_per_mille: 650,
        }),
        destination: KnowledgeHolderRef::Person(LEARNER),
        destination_site: EntityRef::Territory(TRAINING_GROUND),
        revision: Some(revision()),
        mode: TransmissionMode::Demonstration,
        evidence: vec![],
        resulting_program: None,
        opened_at,
        active: true,
    }
}

/// A technology record installed by the scenario with the given payload
/// continuation, which the kernel accepts and the technology domain judges.
fn seeded_record<T: DomainRecordType>(
    reference: TypedDomainRecordRef<T>,
    payload: &T::Payload,
    continuation: PayloadRequiredEvidenceContinuationV1,
) -> DomainRecord
where
    T::Payload: Serialize,
{
    let mut draft = DomainRecordDraft::from_typed(reference, payload).expect("record draft");
    draft
        .payload
        .as_object_mut()
        .expect("payload object")
        .insert(
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
            serde_json::to_value(continuation).expect("continuation"),
        );
    DomainRecord {
        reference: draft.reference,
        owner: "canwu-technology".to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    }
}

fn scenario() -> Scenario {
    let government = GovernmentId::new(1);
    let world = WorldSnapshot {
        people: vec![
            Person {
                id: OPERATOR,
                name: "Operator".to_owned(),
                government,
                current_location: WORKSHOP,
                roles: vec![],
                transit: None,
            },
            Person {
                id: LEARNER,
                name: "Learner".to_owned(),
                government,
                current_location: TRAINING_GROUND,
                roles: vec![],
                transit: None,
            },
        ],
        governments: vec![Government {
            id: government,
            name: "Workshop authority".to_owned(),
            capital: WORKSHOP,
        }],
        territories: vec![
            Territory {
                id: WORKSHOP,
                name: "Workshop".to_owned(),
                controller: government,
                position: MapPoint::default(),
            },
            Territory {
                id: TRAINING_GROUND,
                name: "Training ground".to_owned(),
                controller: government,
                position: MapPoint { x: 1.0, y: 0.0 },
            },
        ],
        routes: vec![],
        armies: vec![],
        letters: vec![],
    };
    Scenario {
        start_time: SimTime::EPOCH,
        entities: world.entities(),
        world,
        knowledge: KnowledgeSnapshot::default(),
        domain_records: vec![
            TechnologyCatalogRecord::Metric(reliability_payload())
                .into_initial_record("reliability")
                .expect("metric"),
            TechnologyCatalogRecord::Metric(MetricSchemaPayload {
                label: "fuel cost".to_owned(),
                unit: "cost_units".to_owned(),
                scale: 1,
                minimum: 0,
                maximum: 1_000,
            })
            .into_initial_record("fuel-cost")
            .expect("metric"),
            TechnologyCatalogRecord::Technique(spec_payload())
                .into_initial_record("technique")
                .expect("technique"),
            TechnologyCatalogRecord::Revision(revision_payload())
                .into_initial_record("revision")
                .expect("revision"),
            content_record(),
            guild_record(),
        ],
    }
}

/// Application content describing an off-map source.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct ExternalSourceContent {
    label: String,
}

struct ExternalSourceContentRecord;

impl DomainRecordType for ExternalSourceContentRecord {
    type Payload = ExternalSourceContent;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "test.content";
    const NAME: &'static str = "external-source";
}

/// An application-owned domain entity that may hold knowledge.
struct GuildRecord;

impl DomainRecordType for GuildRecord {
    type Payload = serde_json::Value;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "test.content";
    const NAME: &'static str = "guild";
}

fn content_reference() -> TypedDomainRecordRef<ExternalSourceContentRecord> {
    TypedDomainRecordRef::new("distant-foundry")
}

fn guild_reference() -> DomainRecordRef {
    TypedDomainRecordRef::<GuildRecord>::new("instrument-guild").into_untyped()
}

fn guild_holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Domain(guild_reference()))
}

fn guild_schema() -> DomainRecordSchema {
    let mut schema = DomainRecordSchema::for_entity::<GuildRecord>();
    schema.holder_policy = KnowledgeHolderPolicy::Allowed;
    schema
}

fn content_record() -> DomainRecord {
    let draft = DomainRecordDraft::from_typed(
        content_reference(),
        &ExternalSourceContent {
            label: "a distant foundry".to_owned(),
        },
    )
    .expect("content draft");
    DomainRecord {
        reference: draft.reference,
        owner: CONTENT_PLUGIN.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: Vec::new(),
    }
}

fn guild_record() -> DomainRecord {
    DomainRecord {
        reference: guild_reference(),
        owner: CONTENT_PLUGIN.to_owned(),
        class: DomainRecordClass::Entity,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: serde_json::json!({}),
        references: Vec::new(),
    }
}

struct ContentPlugin;

impl SimulationPlugin for ContentPlugin {
    fn name(&self) -> &'static str {
        CONTENT_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "6e1a3c5f7b9d0f2a4c6e8a0b2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a2c4e6b8d0f"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::for_record::<
            ExternalSourceContentRecord,
        >())?;
        registrar.register_record_schema(guild_schema())
    }
}
