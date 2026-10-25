use canwu_api::{
    Canwu, CommandAttemptOutcome, CommandEnvelope, CommandId, CommandRequest, CommandRequestId,
    DomainRecordKind, DomainRecordRef, DomainRecordVersionRef, DomainRecordVersionSource,
    EntityRef, ErrorCode, IngressId, Issuer, PluginIngressRequest, ResourceId, SimDuration,
    SimTime,
};
use canwu_fiscal::{
    FISCAL_ACTION_INGRESS, FISCAL_EXECUTION_RECEIPTS_INGRESS, FISCAL_INGRESS_MALFORMED_REJECTION,
    FISCAL_INGRESS_REJECTED_EVENT, FISCAL_RECEIPT_CLAIM_MISMATCH,
    FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT, FISCAL_RECEIPT_EVIDENCE_REUSED,
    FISCAL_RECEIPT_OPERATION_REUSED, FiscalAction, FiscalActionBatch, FiscalActionDisposition,
    FiscalActionEntry, FiscalActionIssuer, FiscalActionTarget, FiscalAdoptionStage,
    FiscalAssessment, FiscalCatalogRecord, FiscalCycleTotal, FiscalExecutionKind,
    FiscalExecutionReceipt, FiscalExecutionReceiptPacket, FiscalExecutionRequest,
    FiscalExternalOperationRef, FiscalHistoricalContextPacket, FiscalHistoricalMode,
    FiscalIssuerRole, FiscalPaymentForm, FiscalProjection, FiscalReceiptDisposition,
    FiscalSettledAction, FiscalState, MAX_FISCAL_EVIDENCE_PER_RECORD,
    MAX_FISCAL_HOT_BOOKS_PER_LEDGER, MAX_FISCAL_PARTITION_KEYS, PLUGIN_NAME, audit_fiscal_records,
    enqueue_execution_receipts, fiscal_action_command, fiscal_action_target,
    fiscal_catalog_reference, fiscal_historical_context_ingress, fiscal_reports,
    fiscal_target_revision,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, ReferenceFiscalExecutionEvidence, enqueue_reference_execution_result,
    fixture_ids, ming_fiscal_reference_scenario, new_ming_fiscal_reference,
    reference_execution_evidence_ref, reference_execution_evidence_version,
    replay_ming_fiscal_reference, restore_ming_fiscal_reference, run_ming_fiscal_sample_cycle,
};

#[test]
fn every_fixture_completes_the_reference_fiscal_vertical_slice() {
    for fixture in fixture_ids() {
        let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, fixture).expect("reference start");
        run_ming_fiscal_sample_cycle(&mut canwu, &format!("test.{fixture}"))
            .expect("sample fiscal cycle");
        let state = fiscal_state(&canwu);
        assert_eq!(state.assessments().count(), 1);
        assert_eq!(state.execution_requests().count(), 1);
        assert_eq!(state.execution_receipts().count(), 1);

        let snapshot = canwu.snapshot_json().expect("fixture snapshot");
        let restored = restore_ming_fiscal_reference(&snapshot)
            .unwrap_or_else(|error| panic!("fixture {fixture} restore failed: {error}"));
        let replayed = replay_ming_fiscal_reference(&canwu.replay_journal())
            .unwrap_or_else(|error| panic!("fixture {fixture} replay failed: {error}"));
        assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
        assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    }
}

#[test]
fn one_historical_period_supports_consecutive_accounting_cycles() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    run_ming_fiscal_sample_cycle(&mut canwu, "annual-1391").expect("first annual cycle");
    run_ming_fiscal_sample_cycle(&mut canwu, "annual-1392").expect("second annual cycle");
    let state = fiscal_state(&canwu);
    assert_eq!(state.assessments().count(), 2);
    assert_eq!(state.execution_requests().count(), 2);
    assert_eq!(state.execution_receipts().count(), 2);
    let totals = totals(&state);
    assert_eq!(totals.len(), 2);
    assert!(totals.iter().all(|total| {
        total.assessed == 100 && total.collected == 70 && total.outstanding == 30
    }));
    let snapshot = canwu.snapshot_json().expect("two-cycle snapshot");
    let restored = restore_ming_fiscal_reference(&snapshot).expect("two-cycle restore");
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
}

/// A ledger at its capacity: every book it may hold, each with as many
/// partitions as one book may use, each collected; the state validates and
/// its totals are one linear pass over the books.
#[test]
fn a_ledger_at_its_book_capacity_validates_and_totals_linearly() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let catalog = canwu
        .domain_record(&fiscal_catalog_reference().into_untyped())
        .expect("fiscal catalog")
        .decode_payload::<FiscalCatalogRecord>()
        .expect("fiscal catalog payload");
    let mut state = fiscal_state(&canwu);
    let scope_id = "scope.lower-yangzi.land";
    let institution = state
        .scope_binding(scope_id)
        .expect("land scope")
        .institution
        .clone();
    let evidence_kind = reference_execution_evidence_ref("capacity")
        .into_untyped()
        .kind;
    let partitions = MAX_FISCAL_PARTITION_KEYS;
    let assessments = MAX_FISCAL_HOT_BOOKS_PER_LEDGER * partitions;
    for index in 0..assessments {
        let assessment_id = format!("capacity.assessment.{index}");
        let request_id = format!("capacity.request.{index}");
        let evidence_id = format!("capacity.evidence.{index}");
        let unit = format!("shi_grain_equivalent.{}", index % partitions);
        state
            .add_assessment(FiscalAssessment {
                id: assessment_id.clone(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: scope_id.to_owned(),
                accounting_cycle_id: format!("capacity.cycle.{:02}", index / partitions),
                quantity: 100,
                unit: unit.clone(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
                source_action_id: None,
            })
            .expect("capacity assessment");
        state
            .add_execution_request(
                scope_id,
                FiscalExecutionRequest {
                    id: request_id.clone(),
                    assessment_id,
                    institution: institution.clone(),
                    kind: FiscalExecutionKind::Collect,
                    quantity: 70,
                    unit,
                    payment_form: FiscalPaymentForm::Grain,
                    resource: ResourceId::new(1),
                    source: EntityRef::Person(actor),
                    target: institution.clone(),
                    purpose: "capacity totals guard".to_owned(),
                    requested_at: SimTime::EPOCH,
                    source_action_id: None,
                },
            )
            .expect("capacity request");
        let evidence = DomainRecordVersionRef {
            record: reference_execution_evidence_ref(&evidence_id).into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
        state
            .add_execution_receipt(
                scope_id,
                FiscalExecutionReceipt {
                    id: format!("capacity.receipt.{index}"),
                    request_id,
                    quantity: 70,
                    disposition: FiscalReceiptDisposition::Fulfilled,
                    external_evidence: [evidence].into_iter().collect(),
                    external_operations: [FiscalExternalOperationRef {
                        evidence_kind: evidence_kind.clone(),
                        external_operation_id: format!("capacity.operation.{index}"),
                    }]
                    .into_iter()
                    .collect(),
                    accepted_ingress: IngressId::new(index as u64 + 1),
                    observed_at: SimTime::EPOCH,
                },
            )
            .expect("capacity receipt");
    }

    state.validate(&catalog).expect("capacity state");
    let totals = totals(&state);
    assert_eq!(totals.len(), assessments);
    assert_eq!(
        totals.iter().map(|total| total.assessed).sum::<u64>(),
        assessments as u64 * 100
    );
    assert_eq!(
        totals.iter().map(|total| total.collected).sum::<u64>(),
        assessments as u64 * 70
    );
}

#[test]
fn hongguang_start_preserves_five_distinct_fiscal_authority_domains() {
    let reference = ming_fiscal_reference_scenario("hongguang-1644").expect("fixture");
    let state = FiscalState::from_records(&reference.scenario.domain_records).expect("state");
    let institutions = state
        .scope_bindings()
        .map(|scope| scope.institution.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(institutions.len(), 5);
    assert!(institutions.contains(&EntityRef::Government(reference.world_ids.government)));
    assert!(institutions.contains(&EntityRef::Army(reference.world_ids.army)));
    assert_eq!(state.authority_bindings().count(), 5);
    for transition_id in [
        "fragment_southern_ming_military_levy",
        "fragment_southern_ming_merchant_credit",
        "fragment_southern_ming_regional_treasury",
    ] {
        assert!(candidates(&state).any(|candidate| candidate.transition_id == transition_id));
    }
}

#[test]
fn hongguang_fragmentation_is_executable_without_collapsing_authority_domains() {
    let reference = ming_fiscal_reference_scenario("hongguang-1644").expect("fixture");
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongguang-1644").expect("runtime");
    for (request_id, actor, authority_id, transition_id, rule_id, scope_id) in [
        (
            1,
            reference.world_ids.commander,
            "authority.field-commander",
            "fragment_southern_ming_military_levy",
            "southern_ming_emergency_levies",
            "scope.southeast.commander-surcharge",
        ),
        (
            2,
            reference.world_ids.observer,
            "authority.merchant-credit",
            "fragment_southern_ming_merchant_credit",
            "southern_ming_merchant_credit",
            "scope.southeast.credit",
        ),
        (
            3,
            reference.world_ids.observer,
            "authority.regional-treasury",
            "fragment_southern_ming_regional_treasury",
            "southern_ming_regional_treasuries",
            "scope.southeast.treasury",
        ),
    ] {
        enqueue_action(
            &mut canwu,
            actor,
            request_id,
            &FiscalActionRequest {
                action_id: format!("hongguang.transition.{request_id}"),
                authority_binding_id: authority_id.to_owned(),
                expected_revision: None,
                action: FiscalAction::ApplyTransition {
                    transition_id: transition_id.to_owned(),
                    target_scope_bindings: [(rule_id.to_owned(), scope_id.to_owned())]
                        .into_iter()
                        .collect(),
                },
            },
        );
        canwu
            .advance_canonical(SimDuration::minutes(1))
            .expect("fragmentation transition");
    }
    let state = fiscal_state(&canwu);
    assert_eq!(state.authority_bindings().count(), 5);
    assert_eq!(
        adoption(&state, "adopt.southeast.legacy-taicang").stage,
        FiscalAdoptionStage::Suspended
    );
    for rule_id in [
        "southern_ming_emergency_levies",
        "southern_ming_merchant_credit",
        "southern_ming_regional_treasuries",
    ] {
        assert!(state.adoptions().any(|adoption| {
            adoption.rule_id == rule_id && adoption.stage == FiscalAdoptionStage::Implemented
        }));
    }
}

#[test]
fn wanli_start_exposes_regional_reform_candidates_without_auto_adoption() {
    let canwu = new_ming_fiscal_reference(DEFAULT_SEED, "wanli-1581").expect("Wanli start");
    let state = fiscal_state(&canwu);
    let transition_ids = candidates(&state)
        .map(|candidate| candidate.transition_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(transition_ids.contains("adopt_single_whip_north"));
    assert!(transition_ids.contains("adopt_single_whip_southwest"));
    assert_eq!(
        adoption(&state, "adopt.southwest.single-whip").stage,
        canwu_fiscal::FiscalAdoptionStage::Accepted
    );
}

#[test]
fn authority_bound_assessment_survives_snapshot_and_replay() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let command = fiscal_action_command(&batch_of(
        &canwu,
        &FiscalActionRequest {
            action_id: "test.assessment.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::OpenAssessment {
                assessment_id: "test.assessment".to_owned(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
                quantity: 800,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        },
    ))
    .expect("command");
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(1),
                canwu.revision(),
                CommandEnvelope::new(Issuer::Actor(actor), command).at_time(canwu.time()),
            ),
        )
        .expect("enqueue command");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("settle");
    let state = fiscal_state(&canwu);
    assert_eq!(
        settled(&canwu, "test.assessment.action").disposition,
        FiscalActionDisposition::Applied
    );
    assert!(
        state
            .assessments()
            .any(|assessment| assessment.id == "test.assessment")
    );

    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = restore_ming_fiscal_reference(&snapshot).expect("restore");
    let replayed = replay_ming_fiscal_reference(&canwu.replay_journal()).expect("replay");
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
}

#[test]
fn action_stale_at_settlement_is_persisted_as_a_rejected_outcome() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    // The issuer decides against the scope as it is now; another action on
    // the scope settles first.
    let decided = scope_revision(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &land_assessment_in("interposed.action", "hongwu-1391.cycle-0", None, None),
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("interposed settlement");
    // A historical-context change alone makes no action stale.
    let context = fiscal_historical_context_ingress(
        canwu.time(),
        &FiscalHistoricalContextPacket {
            year: 1391,
            mode: FiscalHistoricalMode::Counterfactual,
        },
    )
    .expect("historical context ingress");
    canwu
        .enqueue_plugin_ingress(context)
        .expect("interposed historical context");
    enqueue_action(
        &mut canwu,
        actor,
        2,
        &land_assessment("stale.action", Some(decided), None),
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("stale settlement");
    let state = fiscal_state(&canwu);
    let outcome = settled(&canwu, "stale.action");
    assert_eq!(outcome.disposition, FiscalActionDisposition::Rejected);
    assert!(outcome.reason.contains("stale"));
    assert!(
        !state
            .assessments()
            .any(|assessment| assessment.id == "stale.action.assessment")
    );
    // An action decided against the scope as it is now applies.
    enqueue_action(
        &mut canwu,
        actor,
        3,
        &land_assessment("current.action", None, None),
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("current settlement");
    assert_eq!(
        settled(&canwu, "current.action").disposition,
        FiscalActionDisposition::Applied
    );
}

#[test]
fn rejected_fiscal_actions_do_not_block_later_boundaries() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");

    // A request built before another action on its scope settled is stale
    // when it settles.
    let stale_revision = scope_revision(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        10,
        &land_assessment_in("first.action", "hongwu-1391.cycle-0", None, None),
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("first boundary");
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &land_assessment("stale.action", Some(stale_revision), None),
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("a stale action settles as an outcome");
    let outcome = settled(&canwu, "stale.action");
    assert_eq!(outcome.disposition, FiscalActionDisposition::Rejected);
    assert!(outcome.reason.starts_with("domain_record_version_conflict"));

    // A quote version that does not exist, or whose kind the plugin does not
    // read, is a recorded command rejection.
    let missing_quotes = [
        reference_execution_evidence_ref("quote.missing").into_untyped(),
        DomainRecordRef {
            kind: DomainRecordKind::new("host.market", "quote"),
            id: "quote.unreadable".to_owned(),
        },
    ];
    for (request_id, record) in (2..).zip(missing_quotes) {
        let quote = DomainRecordVersionRef {
            record,
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
        let quote_action =
            land_assessment(&format!("quote.action.{request_id}"), None, Some(quote));
        enqueue_action(&mut canwu, actor, request_id, &quote_action);
        canwu
            .advance_canonical(SimDuration::minutes(1))
            .expect("a missing quote is rejected at admission");
        let CommandAttemptOutcome::Rejected { error } = &canwu
            .command_attempts()
            .last()
            .expect("quote attempt")
            .outcome
        else {
            panic!("a missing quote must be a rejected command attempt");
        };
        assert_eq!(error.code, ErrorCode::EntityNotFound);
        assert!(
            !FiscalSettledAction::from_events(canwu.events())
                .iter()
                .any(|settled| settled.action_id == quote_action.action_id)
        );
    }

    // Neither rejection holds up the next action.
    let valid_action = land_assessment("valid.action", None, None);
    enqueue_action(&mut canwu, actor, 4, &valid_action);
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("valid action boundary");
    assert_eq!(
        settled(&canwu, "valid.action").disposition,
        FiscalActionDisposition::Applied
    );

    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored = restore_ming_fiscal_reference(&snapshot).expect("restore");
    let replayed = replay_ming_fiscal_reference(&canwu.replay_journal()).expect("replay");
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.command_attempts(), canwu.command_attempts());
}

fn land_assessment(
    action_id: &str,
    revision: Option<u64>,
    commutation_quote: Option<DomainRecordVersionRef>,
) -> FiscalActionRequest {
    land_assessment_in(
        action_id,
        "hongwu-1391.cycle-1",
        revision,
        commutation_quote,
    )
}

/// A land assessment of the cycle `cycle`.
fn land_assessment_in(
    action_id: &str,
    cycle: &str,
    revision: Option<u64>,
    commutation_quote: Option<DomainRecordVersionRef>,
) -> FiscalActionRequest {
    FiscalActionRequest {
        action_id: action_id.to_owned(),
        authority_binding_id: "authority.revenue-minister".to_owned(),
        expected_revision: revision,
        action: FiscalAction::OpenAssessment {
            assessment_id: format!("{action_id}.assessment"),
            rule_id: "yellow_register_land_assessment".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            accounting_cycle_id: cycle.to_owned(),
            quantity: 10,
            unit: "shi_grain_equivalent".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote,
        },
    }
}

#[test]
fn single_whip_transition_atomically_suspends_the_superseded_rule() {
    let reference = ming_fiscal_reference_scenario("wanli-1581").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "wanli-1581").expect("runtime");
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "transition.north.single-whip".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::ApplyTransition {
                transition_id: "adopt_single_whip_north".to_owned(),
                target_scope_bindings: [(
                    "single_whip_north".to_owned(),
                    "scope.north.land".to_owned(),
                )]
                .into_iter()
                .collect(),
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("transition settlement");
    let state = fiscal_state(&canwu);
    assert_eq!(
        adoption(&state, "adopt.north.silver-commutation").stage,
        FiscalAdoptionStage::Suspended
    );
    assert!(state.adoptions().any(|adoption| {
        adoption.rule_id == "single_whip_north"
            && adoption.stage == FiscalAdoptionStage::Implemented
    }));
    assert!(
        !candidates(&state).any(|candidate| candidate.transition_id == "adopt_single_whip_north")
    );
}

#[test]
fn fiscal_reports_are_imperfect_holder_relative_knowledge() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "report.assessment.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::OpenAssessment {
                assessment_id: "report.assessment".to_owned(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
                quantity: 800,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("assessment and report");
    let viewer = canwu.viewer_for_actor(actor).expect("actor viewer");
    let reports = fiscal_reports(&viewer).expect("actor-visible holder report");
    assert_eq!(reports.len(), 1);
    let report = reports.values().next().expect("the actor's report");
    assert!(report.is_complete());
    let report: &FiscalProjection = &report.projection;
    let fact = report.facts.values().next().expect("reported fact");
    assert!(fact.assessed.minimum <= 800);
    assert!(fact.assessed.maximum >= 800);
    assert!(fact.assessed.minimum < fact.assessed.maximum);
    assert_ne!(
        u64::midpoint(fact.assessed.minimum, fact.assessed.maximum),
        800
    );
}

#[test]
fn receipt_helper_rejects_unavailable_external_evidence() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let unavailable = DomainRecordVersionRef {
        record: reference_execution_evidence_ref("missing.evidence").into_untyped(),
        version: 99,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let packet = canwu_fiscal::FiscalExecutionReceiptPacket {
        receipt_id: "missing.evidence".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "missing.request".to_owned(),
        external_evidence: [unavailable].into_iter().collect(),
    };
    let now = canwu.time();
    let error = enqueue_execution_receipts(&mut canwu, now, std::slice::from_ref(&packet))
        .expect_err("unavailable exact version");
    assert!(error.message.contains("exact available"));
}

#[test]
fn receipt_helper_rejects_an_unbounded_evidence_set() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let external_evidence = (0..=MAX_FISCAL_EVIDENCE_PER_RECORD)
        .map(|index| DomainRecordVersionRef {
            record: reference_execution_evidence_ref(&format!("oversized.evidence.{index}"))
                .into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        })
        .collect();
    let packet = FiscalExecutionReceiptPacket {
        receipt_id: "oversized.evidence".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "oversized.request".to_owned(),
        external_evidence,
    };
    let now = canwu.time();
    let error = enqueue_execution_receipts(&mut canwu, now, std::slice::from_ref(&packet))
        .expect_err("oversized evidence set must fail before enqueue");
    assert!(error.message.contains("evidence count"));
}

#[test]
fn a_host_cannot_enqueue_a_fiscal_action_packet() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request = FiscalActionRequest {
        action_id: "forged.assessment.action".to_owned(),
        authority_binding_id: "authority.revenue-minister".to_owned(),
        expected_revision: None,
        action: FiscalAction::OpenAssessment {
            assessment_id: "forged.assessment".to_owned(),
            rule_id: "yellow_register_land_assessment".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
            quantity: 1,
            unit: "shi_grain_equivalent".to_owned(),
            payment_form: FiscalPaymentForm::Grain,
            commutation_quote: None,
        },
    };
    let batch = batch_of(&canwu, &request);
    let entry = &batch.entries[0];
    let payload = serde_json::json!({
        "authority_binding_id": batch.authority_binding_id,
        "command": CommandId::new(999),
        "entries": [{
            "action_id": entry.action_id,
            "expected_revision": entry.expected_revision,
            "action": entry.action,
            "issuer": FiscalActionIssuer {
                binding_id: "authority.revenue-minister".to_owned(),
                role: FiscalIssuerRole::Principal,
                actor: None,
                authority_basis: None,
            },
            "target": {"type": "ledger", "id": "scope.lower-yangzi.land"},
        }],
    });
    // The action packet is internal ingress: a host cannot enqueue one, so
    // nothing reaches settlement and the run goes on.
    let now = canwu.time();
    let error = canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_ACTION_INGRESS,
            now,
            payload,
        ))
        .expect_err("a host cannot enqueue a fiscal action packet");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("the run continues");
    assert!(
        !FiscalSettledAction::from_events(canwu.events())
            .iter()
            .any(|settled| settled.action_id == "forged.assessment.action")
    );
}

#[test]
fn generic_receipt_ingress_cannot_bypass_exact_evidence_validation() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request_id = open_and_authorize_collection(
        &mut canwu,
        reference.world_ids.observer,
        reference.world_ids.government,
        "forged",
        40,
        1,
    );
    let unavailable = DomainRecordVersionRef {
        record: reference_execution_evidence_ref("forged.evidence").into_untyped(),
        version: 99,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let packet = canwu_fiscal::FiscalExecutionReceiptPacket {
        receipt_id: "forged.receipt".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id,
        external_evidence: [unavailable].into_iter().collect(),
    };
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_EXECUTION_RECEIPTS_INGRESS,
            now,
            serde_json::json!({ "receipts": [packet] }),
        ))
        .expect("enqueue forged receipt");
    assert_receipt_refused(
        &mut canwu,
        "current version",
        FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT,
    );
}

#[test]
fn live_receipt_rejects_evidence_for_another_authorized_request() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request_a = open_and_authorize_collection(
        &mut canwu,
        actor,
        reference.world_ids.government,
        "semantic-a",
        40,
        1,
    );
    let request_b = open_and_authorize_collection(
        &mut canwu,
        actor,
        reference.world_ids.government,
        "semantic-b",
        60,
        3,
    );
    let adapter_result = ReferenceFiscalExecutionEvidence {
        id: "evidence.semantic-a".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        external_operation_id: "resource-transfer.semantic-a".to_owned(),
        request_id: request_a,
        quantity: 40,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: EntityRef::Government(reference.world_ids.government),
        disposition: FiscalReceiptDisposition::Fulfilled,
    };
    let now = canwu.time();
    enqueue_reference_execution_result(&mut canwu, now, &adapter_result).expect("adapter result");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("adapter boundary");
    let evidence =
        reference_execution_evidence_version(&canwu, &adapter_result.id).expect("evidence version");
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[canwu_fiscal::FiscalExecutionReceiptPacket {
            receipt_id: "receipt.semantic-mismatch".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            request_id: request_b,
            external_evidence: [evidence].into_iter().collect(),
        }],
    )
    .expect("receipt ingress");
    assert_receipt_refused(&mut canwu, "does not match", FISCAL_RECEIPT_CLAIM_MISMATCH);
    assert_eq!(fiscal_state(&canwu).execution_receipts().count(), 0);
}

#[test]
fn receipt_payload_cannot_redeclare_evidence_quantity_or_disposition() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request_id = open_and_authorize_collection(
        &mut canwu,
        actor,
        reference.world_ids.government,
        "derived-receipt",
        70,
        1,
    );
    let adapter_result = ReferenceFiscalExecutionEvidence {
        id: "evidence.derived-receipt".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        external_operation_id: "resource-transfer.derived-receipt".to_owned(),
        request_id: request_id.clone(),
        quantity: 70,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: EntityRef::Government(reference.world_ids.government),
        disposition: FiscalReceiptDisposition::Fulfilled,
    };
    let now = canwu.time();
    enqueue_reference_execution_result(&mut canwu, now, &adapter_result).expect("adapter result");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("adapter boundary");
    let evidence =
        reference_execution_evidence_version(&canwu, &adapter_result.id).expect("evidence version");
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_EXECUTION_RECEIPTS_INGRESS,
            now,
            serde_json::json!({ "receipts": [{
                "receipt_id": "receipt.forged-quantity",
                "request_id": request_id,
                "quantity": 50,
                "disposition": "fulfilled",
                "external_evidence": [evidence],
            }] }),
        ))
        .expect("raw receipt ingress");
    // A receipt that declares its own quantity and disposition does not
    // decode: both are derived from the evidence.
    assert_receipt_refused(
        &mut canwu,
        FISCAL_EXECUTION_RECEIPTS_INGRESS,
        FISCAL_INGRESS_MALFORMED_REJECTION,
    );
    assert_eq!(fiscal_state(&canwu).execution_receipts().count(), 0);
}

#[test]
fn different_records_cannot_reuse_one_external_operation_for_the_same_request() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let government = reference.world_ids.government;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request_id =
        open_and_authorize_collection(&mut canwu, actor, government, "same-operation", 100, 1);
    let first = execution_evidence(
        "evidence.same-operation.first",
        "resource-transfer.same-operation",
        &request_id,
        50,
        actor,
        government,
    );
    publish_and_settle_receipt(&mut canwu, &first, "receipt.same-operation.first");

    let second = execution_evidence(
        "evidence.same-operation.second",
        "resource-transfer.same-operation",
        &request_id,
        50,
        actor,
        government,
    );
    let evidence = publish_execution_evidence(&mut canwu, &second);
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[FiscalExecutionReceiptPacket {
            receipt_id: "receipt.same-operation.second".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            request_id,
            external_evidence: [evidence].into_iter().collect(),
        }],
    )
    .expect("second receipt ingress");
    assert_receipt_refused(
        &mut canwu,
        "external fiscal operation",
        FISCAL_RECEIPT_OPERATION_REUSED,
    );
    let state = fiscal_state(&canwu);
    assert_eq!(state.execution_receipts().count(), 1);
    assert_eq!(state.execution_receipts().next().unwrap().quantity, 50);
}

#[test]
fn different_requests_cannot_reuse_one_external_operation() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let government = reference.world_ids.government;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request_a =
        open_and_authorize_collection(&mut canwu, actor, government, "operation-a", 50, 1);
    let request_b =
        open_and_authorize_collection(&mut canwu, actor, government, "operation-b", 50, 3);
    let first = execution_evidence(
        "evidence.operation-a",
        "resource-transfer.shared-operation",
        &request_a,
        50,
        actor,
        government,
    );
    publish_and_settle_receipt(&mut canwu, &first, "receipt.operation-a");

    let second = execution_evidence(
        "evidence.operation-b",
        "resource-transfer.shared-operation",
        &request_b,
        50,
        actor,
        government,
    );
    let evidence = publish_execution_evidence(&mut canwu, &second);
    let now = canwu.time();
    enqueue_execution_receipts(
        &mut canwu,
        now,
        &[FiscalExecutionReceiptPacket {
            receipt_id: "receipt.operation-b".to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            request_id: request_b,
            external_evidence: [evidence].into_iter().collect(),
        }],
    )
    .expect("cross-request receipt ingress");
    assert_receipt_refused(
        &mut canwu,
        "external fiscal operation",
        FISCAL_RECEIPT_OPERATION_REUSED,
    );
    assert_eq!(fiscal_state(&canwu).execution_receipts().count(), 1);
}

#[test]
fn exact_external_evidence_settles_fulfillment_without_owning_resources() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");

    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "receipt.assess.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::OpenAssessment {
                assessment_id: "receipt.assessment".to_owned(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
                quantity: 100,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("assessment boundary");

    enqueue_action(
        &mut canwu,
        actor,
        2,
        &FiscalActionRequest {
            action_id: "receipt.authorize.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::AuthorizeExecution {
                request_id: "receipt.execution".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                assessment_id: "receipt.assessment".to_owned(),
                kind: FiscalExecutionKind::Collect,
                quantity: 70,
                unit: "shi_grain_equivalent".to_owned(),
                resource: ResourceId::new(1),
                source: EntityRef::Person(actor),
                target: EntityRef::Government(reference.world_ids.government),
                purpose: "test external resource adapter".to_owned(),
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("authorization boundary");

    let adapter_result = ReferenceFiscalExecutionEvidence {
        id: "evidence.fulfillment".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "receipt.execution".to_owned(),
        quantity: 70,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: EntityRef::Government(reference.world_ids.government),
        disposition: FiscalReceiptDisposition::Fulfilled,
        external_operation_id: "resource-transfer.1".to_owned(),
    };
    let now = canwu.time();
    enqueue_reference_execution_result(&mut canwu, now, &adapter_result)
        .expect("enqueue adapter result");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("adapter boundary");
    let evidence = reference_execution_evidence_version(&canwu, &adapter_result.id)
        .expect("exact adapter evidence version");
    let packet = canwu_fiscal::FiscalExecutionReceiptPacket {
        receipt_id: "receipt.fulfillment".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "receipt.execution".to_owned(),
        external_evidence: [evidence].into_iter().collect(),
    };
    let now = canwu.time();
    enqueue_execution_receipts(&mut canwu, now, std::slice::from_ref(&packet))
        .expect("exact external evidence");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("receipt boundary");

    let state = fiscal_state(&canwu);
    assert_eq!(receipt(&state, "receipt.fulfillment").quantity, 70);
    let total = totals(&state).into_iter().next().expect("the book's total");
    assert_eq!(total.collected, 70);
    assert_eq!(total.outstanding, 30);

    let revision = scope_revision(&canwu);
    assert_receipt_retry_is_idempotent(&mut canwu, &packet, revision);

    assert_restore_replay_and_reject_duplicate(&mut canwu, &adapter_result);
}

#[test]
fn rejected_external_receipt_records_evidence_without_fulfilling_quantity() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");

    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "rejected.assess.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::OpenAssessment {
                assessment_id: "rejected.assessment".to_owned(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
                quantity: 100,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("assessment boundary");
    enqueue_action(
        &mut canwu,
        actor,
        2,
        &FiscalActionRequest {
            action_id: "rejected.authorize.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::AuthorizeExecution {
                request_id: "rejected.execution".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                assessment_id: "rejected.assessment".to_owned(),
                kind: FiscalExecutionKind::Collect,
                quantity: 100,
                unit: "shi_grain_equivalent".to_owned(),
                resource: ResourceId::new(1),
                source: EntityRef::Person(actor),
                target: EntityRef::Government(reference.world_ids.government),
                purpose: "test rejected external settlement".to_owned(),
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("authorization boundary");

    let adapter_result = ReferenceFiscalExecutionEvidence {
        id: "evidence.rejected".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "rejected.execution".to_owned(),
        quantity: 0,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: EntityRef::Government(reference.world_ids.government),
        disposition: FiscalReceiptDisposition::Rejected,
        external_operation_id: "resource-transfer.rejected".to_owned(),
    };
    let now = canwu.time();
    enqueue_reference_execution_result(&mut canwu, now, &adapter_result)
        .expect("enqueue rejected adapter result");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("adapter boundary");
    let evidence = reference_execution_evidence_version(&canwu, &adapter_result.id)
        .expect("exact rejected adapter evidence version");
    let packet = canwu_fiscal::FiscalExecutionReceiptPacket {
        receipt_id: "receipt.rejected".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "rejected.execution".to_owned(),
        external_evidence: [evidence].into_iter().collect(),
    };
    let now = canwu.time();
    enqueue_execution_receipts(&mut canwu, now, std::slice::from_ref(&packet))
        .expect("exact rejection evidence");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("receipt boundary");

    let state = fiscal_state(&canwu);
    assert_eq!(receipt(&state, "receipt.rejected").quantity, 0);
    let total = totals(&state).into_iter().next().expect("the book's total");
    assert_eq!(total.collected, 0);
    assert_eq!(total.outstanding, 100);
}

/// One action as these tests build it: stamped, when it is enqueued, with
/// the revision its target has then, unless it names an earlier one.
#[derive(Clone)]
struct FiscalActionRequest {
    action_id: String,
    authority_binding_id: String,
    expected_revision: Option<u64>,
    action: FiscalAction,
}

/// The one-entry batch of a request on `canwu` now.
fn batch_of(canwu: &Canwu, request: &FiscalActionRequest) -> FiscalActionBatch {
    let revision = request.expected_revision.unwrap_or_else(|| {
        fiscal_action_target(canwu, &request.authority_binding_id, &request.action)
            .ok()
            .and_then(|target| fiscal_target_revision(canwu, &target).expect("revision read"))
            .unwrap_or(1)
    });
    FiscalActionBatch::single(
        request.authority_binding_id.clone(),
        FiscalActionEntry {
            action_id: request.action_id.clone(),
            expected_revision: revision,
            action: request.action.clone(),
        },
    )
}

/// The revision of the land scope these tests act on.
fn scope_revision(canwu: &Canwu) -> u64 {
    fiscal_target_revision(
        canwu,
        &FiscalActionTarget::Ledger("scope.lower-yangzi.land".to_owned()),
    )
    .expect("revision read")
    .expect("the land scope")
}

/// The run's fiscal state, after checking that its records are exactly the
/// partition of a valid state.
fn fiscal_state(canwu: &Canwu) -> FiscalState {
    audit_fiscal_records(canwu).expect("fiscal records audit")
}

/// The totals of every hot book, by scope, cycle, and partition.
fn totals(state: &FiscalState) -> Vec<FiscalCycleTotal> {
    state
        .books()
        .flat_map(|(ledger, book)| book.totals(ledger.scope.mechanism).expect("totals"))
        .collect()
}

/// Every transition candidate of every jurisdiction.
fn candidates(
    state: &FiscalState,
) -> impl Iterator<Item = &canwu_fiscal::FiscalTransitionCandidate> {
    state
        .jurisdictions
        .values()
        .flat_map(|jurisdiction| jurisdiction.transition_candidates.values())
}

/// The settled outcome of `action_id`, as its event records it.
fn settled(canwu: &Canwu, action_id: &str) -> FiscalSettledAction {
    FiscalSettledAction::from_events(canwu.events())
        .into_iter()
        .rev()
        .find(|settled| settled.action_id == action_id)
        .unwrap_or_else(|| panic!("{action_id} has no settled event"))
}

fn adoption<'a>(
    state: &'a canwu_fiscal::FiscalState,
    id: &str,
) -> &'a canwu_fiscal::FiscalAdoptionState {
    state
        .adoptions()
        .find(|adoption| adoption.id == id)
        .unwrap_or_else(|| panic!("adoption {id}"))
}

fn receipt<'a>(state: &'a canwu_fiscal::FiscalState, id: &str) -> &'a FiscalExecutionReceipt {
    state
        .execution_receipts()
        .find(|receipt| receipt.id == id)
        .unwrap_or_else(|| panic!("receipt {id}"))
}

fn assert_restore_replay_and_reject_duplicate(
    canwu: &mut Canwu,
    adapter_result: &ReferenceFiscalExecutionEvidence,
) {
    let snapshot = canwu.snapshot_json().expect("snapshot with exact evidence");
    let restored = restore_ming_fiscal_reference(&snapshot).expect("validated restore");
    let replayed = replay_ming_fiscal_reference(&canwu.replay_journal()).expect("validated replay");
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());

    let evidence = reference_execution_evidence_version(canwu, &adapter_result.id)
        .expect("same exact adapter evidence");
    let duplicate = canwu_fiscal::FiscalExecutionReceiptPacket {
        receipt_id: "receipt.duplicate".to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        request_id: "receipt.execution".to_owned(),
        external_evidence: [evidence].into_iter().collect(),
    };
    let now = canwu.time();
    enqueue_execution_receipts(canwu, now, &[duplicate]).expect("enqueue duplicate receipt");
    assert_receipt_refused(
        canwu,
        "cannot settle multiple",
        FISCAL_RECEIPT_EVIDENCE_REUSED,
    );
}

/// Settles what is due and asserts that the boundary committed and refused
/// one receipt: its only `canwu.fiscal.ingress_rejected.v1` event names
/// `detail` and ends with `reason`. Settlement never fails a boundary for a
/// receipt a host sent.
fn assert_receipt_refused(canwu: &mut Canwu, detail: &str, reason: &str) {
    let events = canwu.events().len();
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("the boundary commits and refuses the receipt");
    let refusals: Vec<_> = canwu.events()[events..]
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, FISCAL_INGRESS_REJECTED_EVENT))
        })
        .collect();
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(
        refusals[0].summary.contains(detail) && refusals[0].summary.ends_with(reason),
        "{}",
        refusals[0].summary
    );
}

fn assert_receipt_retry_is_idempotent(
    canwu: &mut Canwu,
    packet: &FiscalExecutionReceiptPacket,
    revision: u64,
) {
    let now = canwu.time();
    enqueue_execution_receipts(canwu, now, std::slice::from_ref(packet))
        .expect("idempotent receipt ingress");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("idempotent receipt boundary");
    assert_eq!(scope_revision(canwu), revision);
}

fn enqueue_action(
    canwu: &mut Canwu,
    actor: canwu_api::PersonId,
    request_id: u64,
    request: &FiscalActionRequest,
) {
    let command = fiscal_action_command(&batch_of(canwu, request)).expect("fiscal command");
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request_id),
                canwu.revision(),
                CommandEnvelope::new(Issuer::Actor(actor), command).at_time(canwu.time()),
            ),
        )
        .expect("enqueue fiscal action");
}

fn open_and_authorize_collection(
    canwu: &mut Canwu,
    actor: canwu_api::PersonId,
    government: canwu_api::GovernmentId,
    prefix: &str,
    quantity: u64,
    first_request_id: u64,
) -> String {
    let assessment_id = format!("{prefix}.assessment");
    let request_id = format!("{prefix}.execution");
    enqueue_action(
        canwu,
        actor,
        first_request_id,
        &FiscalActionRequest {
            action_id: format!("{prefix}.assess.action"),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::OpenAssessment {
                assessment_id: assessment_id.clone(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                accounting_cycle_id: format!("hongwu-1391.{prefix}"),
                quantity,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("assessment boundary");
    enqueue_action(
        canwu,
        actor,
        first_request_id + 1,
        &FiscalActionRequest {
            action_id: format!("{prefix}.authorize.action"),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_revision: None,
            action: FiscalAction::AuthorizeExecution {
                request_id: request_id.clone(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                assessment_id,
                kind: FiscalExecutionKind::Collect,
                quantity,
                unit: "shi_grain_equivalent".to_owned(),
                resource: ResourceId::new(1),
                source: EntityRef::Person(actor),
                target: EntityRef::Government(government),
                purpose: "typed evidence validation".to_owned(),
            },
        },
    );
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("authorization boundary");
    request_id
}

fn execution_evidence(
    id: &str,
    external_operation_id: &str,
    request_id: &str,
    quantity: u64,
    actor: canwu_api::PersonId,
    government: canwu_api::GovernmentId,
) -> ReferenceFiscalExecutionEvidence {
    ReferenceFiscalExecutionEvidence {
        id: id.to_owned(),
        scope_binding_id: "scope.lower-yangzi.land".to_owned(),
        external_operation_id: external_operation_id.to_owned(),
        request_id: request_id.to_owned(),
        quantity,
        unit: "shi_grain_equivalent".to_owned(),
        payment_form: FiscalPaymentForm::Grain,
        execution_kind: FiscalExecutionKind::Collect,
        resource: ResourceId::new(1),
        source: EntityRef::Person(actor),
        target: EntityRef::Government(government),
        disposition: FiscalReceiptDisposition::Fulfilled,
    }
}

fn publish_execution_evidence(
    canwu: &mut Canwu,
    evidence: &ReferenceFiscalExecutionEvidence,
) -> DomainRecordVersionRef {
    let now = canwu.time();
    enqueue_reference_execution_result(canwu, now, evidence).expect("execution evidence ingress");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("execution evidence boundary");
    reference_execution_evidence_version(canwu, &evidence.id).expect("execution evidence version")
}

fn publish_and_settle_receipt(
    canwu: &mut Canwu,
    evidence: &ReferenceFiscalExecutionEvidence,
    receipt_id: &str,
) {
    let version = publish_execution_evidence(canwu, evidence);
    let now = canwu.time();
    enqueue_execution_receipts(
        canwu,
        now,
        &[FiscalExecutionReceiptPacket {
            receipt_id: receipt_id.to_owned(),
            scope_binding_id: "scope.lower-yangzi.land".to_owned(),
            request_id: evidence.request_id.clone(),
            external_evidence: [version].into_iter().collect(),
        }],
    )
    .expect("execution receipt ingress");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("execution receipt boundary");
}
