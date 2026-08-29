use canwu_api::{
    Canwu, CommandEnvelope, CommandId, CommandRequest, CommandRequestId, DomainRecordVersionRef,
    DomainRecordVersionSource, EntityRef, IngressId, Issuer, KnowledgeQuery, PluginIngressRequest,
    ResourceId, SimDuration, SimTime,
};
use canwu_fiscal::{
    FISCAL_ACTION_INGRESS, FISCAL_EXECUTION_RECEIPT_INGRESS, FiscalAction, FiscalActionDisposition,
    FiscalActionRequest, FiscalAdoptionStage, FiscalAssessment, FiscalCatalogRecord,
    FiscalExecutionKind, FiscalExecutionReceipt, FiscalExecutionReceiptPacket,
    FiscalExecutionRequest, FiscalExternalOperationRef, FiscalHistoricalContextPacket,
    FiscalHistoricalMode, FiscalPaymentForm, FiscalProjection, FiscalReceiptDisposition,
    FiscalStateRecord, MAX_FISCAL_ASSESSMENTS, MAX_FISCAL_EVIDENCE_PER_RECORD, PLUGIN_NAME,
    compute_aggregates, enqueue_execution_receipt, fiscal_action_command, fiscal_catalog_reference,
    fiscal_historical_context_ingress, fiscal_report_knowledge_schema_id, fiscal_state_reference,
};
use canwu_ming_fiscal_reference::{
    DEFAULT_SEED, ReferenceFiscalExecutionEvidence, enqueue_reference_execution_result,
    fixture_ids, ming_fiscal_reference_scenario, new_ming_fiscal_reference,
    reference_execution_evidence_ref, reference_execution_evidence_version,
    replay_ming_fiscal_reference, restore_ming_fiscal_reference, run_ming_fiscal_sample_cycle,
};

#[test]
fn all_reference_starts_load_with_world_adapter_and_fiscal_plugins() {
    for fixture in fixture_ids() {
        let canwu = new_ming_fiscal_reference(DEFAULT_SEED, fixture).expect("reference start");
        assert!(
            canwu
                .domain_record(&fiscal_state_reference().into_untyped())
                .is_some()
        );
    }
}

#[test]
fn every_fixture_completes_the_reference_fiscal_vertical_slice() {
    for fixture in fixture_ids() {
        let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, fixture).expect("reference start");
        run_ming_fiscal_sample_cycle(&mut canwu, &format!("test.{fixture}"))
            .expect("sample fiscal cycle");
        let state = fiscal_state(&canwu);
        assert_eq!(state.assessments.len(), 1);
        assert_eq!(state.execution_requests.len(), 1);
        assert_eq!(state.execution_receipts.len(), 1);

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
    assert_eq!(state.assessments.len(), 2);
    assert_eq!(state.execution_requests.len(), 2);
    assert_eq!(state.execution_receipts.len(), 2);
    assert_eq!(state.aggregates.len(), 2);
    assert!(state.aggregates.values().all(|aggregate| {
        aggregate.assessed == 100 && aggregate.collected == 70 && aggregate.outstanding == 30
    }));
    let snapshot = canwu.snapshot_json().expect("two-cycle snapshot");
    let restored = restore_ming_fiscal_reference(&snapshot).expect("two-cycle restore");
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());
}

#[test]
fn maximum_supported_cycles_use_linear_aggregate_indexes() {
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
    let institution = state.scope_bindings[scope_id].institution.clone();
    let evidence_kind = reference_execution_evidence_ref("capacity")
        .into_untyped()
        .kind;
    for index in 0..MAX_FISCAL_ASSESSMENTS {
        let assessment_id = format!("capacity.assessment.{index}");
        let request_id = format!("capacity.request.{index}");
        let evidence_id = format!("capacity.evidence.{index}");
        state.assessments.insert(
            assessment_id.clone(),
            FiscalAssessment {
                id: assessment_id.clone(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: scope_id.to_owned(),
                accounting_cycle_id: format!("capacity.cycle.{index}"),
                quantity: 100,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
                created_at: SimTime::EPOCH,
            },
        );
        state.execution_requests.insert(
            request_id.clone(),
            FiscalExecutionRequest {
                id: request_id.clone(),
                assessment_id,
                institution: institution.clone(),
                kind: FiscalExecutionKind::Collect,
                quantity: 70,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                resource: ResourceId::new(1),
                source: EntityRef::Person(actor),
                target: institution.clone(),
                purpose: "capacity aggregate guard".to_owned(),
                requested_at: SimTime::EPOCH,
            },
        );
        let evidence = DomainRecordVersionRef {
            record: reference_execution_evidence_ref(&evidence_id).into_untyped(),
            version: 1,
            established_by: DomainRecordVersionSource::InitialScenario,
        };
        state.execution_receipts.insert(
            format!("capacity.receipt.{index}"),
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
        );
    }

    state.validate(&catalog).expect("capacity state");
    let aggregates = compute_aggregates(&state, &catalog).expect("linear aggregate pass");
    assert_eq!(aggregates.len(), MAX_FISCAL_ASSESSMENTS);
    assert_eq!(
        aggregates.values().map(|value| value.assessed).sum::<u64>(),
        MAX_FISCAL_ASSESSMENTS as u64 * 100
    );
    assert_eq!(
        aggregates
            .values()
            .map(|value| value.collected)
            .sum::<u64>(),
        MAX_FISCAL_ASSESSMENTS as u64 * 70
    );
}

#[test]
fn hongguang_start_preserves_five_distinct_fiscal_authority_domains() {
    let reference = ming_fiscal_reference_scenario("hongguang-1644").expect("fixture");
    let state = reference
        .scenario
        .domain_records
        .iter()
        .find(|record| record.reference == fiscal_state_reference().into_untyped())
        .expect("fiscal state")
        .decode_payload::<FiscalStateRecord>()
        .expect("payload");
    let institutions = state
        .scope_bindings
        .values()
        .map(|scope| scope.institution.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(institutions.len(), 5);
    assert!(institutions.contains(&EntityRef::Government(reference.world_ids.government)));
    assert!(institutions.contains(&EntityRef::Army(reference.world_ids.army)));
    assert_eq!(state.authority_bindings.len(), 5);
    for transition_id in [
        "fragment_southern_ming_military_levy",
        "fragment_southern_ming_merchant_credit",
        "fragment_southern_ming_regional_treasury",
    ] {
        assert!(
            state
                .transition_candidates
                .values()
                .any(|candidate| candidate.transition_id == transition_id)
        );
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
        let procedure_revision = fiscal_state_version(&canwu);
        enqueue_action(
            &mut canwu,
            actor,
            request_id,
            &FiscalActionRequest {
                action_id: format!("hongguang.transition.{request_id}"),
                authority_binding_id: authority_id.to_owned(),
                expected_procedure_revision: procedure_revision,
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
    assert_eq!(state.authority_bindings.len(), 5);
    assert_eq!(
        state.adoptions["adopt.southeast.legacy-taicang"].stage,
        FiscalAdoptionStage::Suspended
    );
    for rule_id in [
        "southern_ming_emergency_levies",
        "southern_ming_merchant_credit",
        "southern_ming_regional_treasuries",
    ] {
        assert!(state.adoptions.values().any(|adoption| {
            adoption.rule_id == rule_id && adoption.stage == FiscalAdoptionStage::Implemented
        }));
    }
}

#[test]
fn wanli_start_exposes_regional_reform_candidates_without_auto_adoption() {
    let canwu = new_ming_fiscal_reference(DEFAULT_SEED, "wanli-1581").expect("Wanli start");
    let state = canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("state")
        .decode_payload::<FiscalStateRecord>()
        .expect("payload");
    let transition_ids = state
        .transition_candidates
        .values()
        .map(|candidate| candidate.transition_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(transition_ids.contains("adopt_single_whip_north"));
    assert!(transition_ids.contains("adopt_single_whip_southwest"));
    assert_eq!(
        state.adoptions["adopt.southwest.single-whip"].stage,
        canwu_fiscal::FiscalAdoptionStage::Accepted
    );
}

#[test]
fn authority_bound_assessment_survives_snapshot_and_replay() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let state_version = canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("state")
        .version;
    let command = fiscal_action_command(&FiscalActionRequest {
        action_id: "test.assessment.action".to_owned(),
        authority_binding_id: "authority.revenue-minister".to_owned(),
        expected_procedure_revision: state_version,
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
    })
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
    let record = canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("fiscal state");
    let state = record
        .decode_payload::<FiscalStateRecord>()
        .expect("state payload");
    assert_eq!(
        state.action_outcomes["test.assessment.action"].disposition,
        FiscalActionDisposition::Applied
    );
    assert!(state.assessments.contains_key("test.assessment"));

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
    let version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "stale.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: version,
            action: FiscalAction::OpenAssessment {
                assessment_id: "stale.assessment".to_owned(),
                rule_id: "yellow_register_land_assessment".to_owned(),
                scope_binding_id: "scope.lower-yangzi.land".to_owned(),
                accounting_cycle_id: "hongwu-1391.cycle-1".to_owned(),
                quantity: 10,
                unit: "shi_grain_equivalent".to_owned(),
                payment_form: FiscalPaymentForm::Grain,
                commutation_quote: None,
            },
        },
    );
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
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("stale settlement");
    let state = fiscal_state(&canwu);
    assert_eq!(
        state.action_outcomes["stale.action"].disposition,
        FiscalActionDisposition::Rejected
    );
    assert!(
        state.action_outcomes["stale.action"]
            .reason
            .contains("stale")
    );
    assert!(!state.assessments.contains_key("stale.assessment"));
}

#[test]
fn single_whip_transition_atomically_suspends_the_superseded_rule() {
    let reference = ming_fiscal_reference_scenario("wanli-1581").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "wanli-1581").expect("runtime");
    let state_version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "transition.north.single-whip".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: state_version,
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
        state.adoptions["adopt.north.silver-commutation"].stage,
        FiscalAdoptionStage::Suspended
    );
    assert!(state.adoptions.values().any(|adoption| {
        adoption.rule_id == "single_whip_north"
            && adoption.stage == FiscalAdoptionStage::Implemented
    }));
    assert!(
        !state
            .transition_candidates
            .values()
            .any(|candidate| candidate.transition_id == "adopt_single_whip_north")
    );
}

#[test]
fn fiscal_reports_are_imperfect_holder_relative_knowledge() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let state_version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "report.assessment.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: state_version,
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
    let result = canwu
        .viewer_for_actor(actor)
        .expect("actor viewer")
        .query_knowledge(&KnowledgeQuery {
            schemas: vec![fiscal_report_knowledge_schema_id()],
            ..KnowledgeQuery::default()
        })
        .expect("actor-visible holder report");
    assert_eq!(result.records.len(), 1);
    let report: FiscalProjection =
        serde_json::from_value(result.records[0].payload.clone()).expect("typed fiscal report");
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
        request_id: "missing.request".to_owned(),
        external_evidence: [unavailable].into_iter().collect(),
    };
    let now = canwu.time();
    let error = canwu_fiscal::enqueue_execution_receipt(&mut canwu, now, &packet)
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
        request_id: "oversized.request".to_owned(),
        external_evidence,
    };
    let now = canwu.time();
    let error = enqueue_execution_receipt(&mut canwu, now, &packet)
        .expect_err("oversized evidence set must fail before enqueue");
    assert!(error.message.contains("evidence count"));
}

#[test]
fn forged_action_ingress_without_an_admitted_command_is_rejected() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let request = FiscalActionRequest {
        action_id: "forged.assessment.action".to_owned(),
        authority_binding_id: "authority.revenue-minister".to_owned(),
        expected_procedure_revision: fiscal_state_version(&canwu),
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
    let payload = serde_json::json!({
        "request": request,
        "command": CommandId::new(999),
    });
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_ACTION_INGRESS,
            now,
            payload,
        ))
        .expect("enqueue forged ingress");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("forged action must fail");
    assert!(error.message.contains("caused by an admitted command"));
}

#[test]
fn generic_receipt_ingress_cannot_bypass_exact_evidence_validation() {
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");
    let unavailable = DomainRecordVersionRef {
        record: reference_execution_evidence_ref("forged.evidence").into_untyped(),
        version: 99,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    let packet = canwu_fiscal::FiscalExecutionReceiptPacket {
        receipt_id: "forged.receipt".to_owned(),
        request_id: "forged.request".to_owned(),
        external_evidence: [unavailable].into_iter().collect(),
    };
    let now = canwu.time();
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_EXECUTION_RECEIPT_INGRESS,
            now,
            serde_json::to_value(packet).expect("receipt payload"),
        ))
        .expect("enqueue forged receipt");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("unavailable evidence must fail");
    assert!(error.message.contains("unavailable"));
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
    canwu_fiscal::enqueue_execution_receipt(
        &mut canwu,
        now,
        &canwu_fiscal::FiscalExecutionReceiptPacket {
            receipt_id: "receipt.semantic-mismatch".to_owned(),
            request_id: request_b,
            external_evidence: [evidence].into_iter().collect(),
        },
    )
    .expect("receipt ingress");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("cross-request evidence must fail at the live boundary");
    assert!(error.message.contains("does not match"));
    assert!(fiscal_state(&canwu).execution_receipts.is_empty());
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
            FISCAL_EXECUTION_RECEIPT_INGRESS,
            now,
            serde_json::json!({
                "receipt_id": "receipt.forged-quantity",
                "request_id": request_id,
                "quantity": 50,
                "disposition": "fulfilled",
                "external_evidence": [evidence],
            }),
        ))
        .expect("raw receipt ingress");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("receipt declarations must be derived from evidence");
    assert!(error.message.contains("could not be decoded"));
    assert!(fiscal_state(&canwu).execution_receipts.is_empty());
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
    enqueue_execution_receipt(
        &mut canwu,
        now,
        &FiscalExecutionReceiptPacket {
            receipt_id: "receipt.same-operation.second".to_owned(),
            request_id,
            external_evidence: [evidence].into_iter().collect(),
        },
    )
    .expect("second receipt ingress");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("one external operation must not settle through two records");
    assert!(error.message.contains("external fiscal operation"));
    let state = fiscal_state(&canwu);
    assert_eq!(state.execution_receipts.len(), 1);
    assert_eq!(
        state.execution_receipts.values().next().unwrap().quantity,
        50
    );
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
    enqueue_execution_receipt(
        &mut canwu,
        now,
        &FiscalExecutionReceiptPacket {
            receipt_id: "receipt.operation-b".to_owned(),
            request_id: request_b,
            external_evidence: [evidence].into_iter().collect(),
        },
    )
    .expect("cross-request receipt ingress");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("one external operation must not settle different requests");
    assert!(error.message.contains("external fiscal operation"));
    assert_eq!(fiscal_state(&canwu).execution_receipts.len(), 1);
}

#[test]
fn exact_external_evidence_settles_fulfillment_without_owning_resources() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");

    let assessment_version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "receipt.assess.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: assessment_version,
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

    let authorization_version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        2,
        &FiscalActionRequest {
            action_id: "receipt.authorize.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: authorization_version,
            action: FiscalAction::AuthorizeExecution {
                request_id: "receipt.execution".to_owned(),
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
        request_id: "receipt.execution".to_owned(),
        external_evidence: [evidence].into_iter().collect(),
    };
    let now = canwu.time();
    canwu_fiscal::enqueue_execution_receipt(&mut canwu, now, &packet)
        .expect("exact external evidence");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("receipt boundary");

    let state = canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("state")
        .decode_payload::<FiscalStateRecord>()
        .expect("payload");
    assert_eq!(state.execution_receipts["receipt.fulfillment"].quantity, 70);
    let aggregate = state.aggregates.values().next().expect("aggregate");
    assert_eq!(aggregate.collected, 70);
    assert_eq!(aggregate.outstanding, 30);

    assert_receipt_retry_is_idempotent(&mut canwu, &packet, state.procedure_revision);

    assert_restore_replay_and_reject_duplicate(&mut canwu, &adapter_result);
}

#[test]
fn rejected_external_receipt_records_evidence_without_fulfilling_quantity() {
    let reference = ming_fiscal_reference_scenario("hongwu-1391").expect("fixture");
    let actor = reference.world_ids.observer;
    let mut canwu = new_ming_fiscal_reference(DEFAULT_SEED, "hongwu-1391").expect("runtime");

    let assessment_version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        1,
        &FiscalActionRequest {
            action_id: "rejected.assess.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: assessment_version,
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
    let authorization_version = fiscal_state_version(&canwu);
    enqueue_action(
        &mut canwu,
        actor,
        2,
        &FiscalActionRequest {
            action_id: "rejected.authorize.action".to_owned(),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: authorization_version,
            action: FiscalAction::AuthorizeExecution {
                request_id: "rejected.execution".to_owned(),
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
        request_id: "rejected.execution".to_owned(),
        external_evidence: [evidence].into_iter().collect(),
    };
    let now = canwu.time();
    canwu_fiscal::enqueue_execution_receipt(&mut canwu, now, &packet)
        .expect("exact rejection evidence");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("receipt boundary");

    let state = canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("state")
        .decode_payload::<FiscalStateRecord>()
        .expect("payload");
    assert_eq!(state.execution_receipts["receipt.rejected"].quantity, 0);
    let aggregate = state.aggregates.values().next().expect("aggregate");
    assert_eq!(aggregate.collected, 0);
    assert_eq!(aggregate.outstanding, 100);
}

fn fiscal_state_version(canwu: &Canwu) -> u64 {
    canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("fiscal state")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
        .procedure_revision
}

fn fiscal_state(canwu: &Canwu) -> canwu_fiscal::FiscalState {
    canwu
        .domain_record(&fiscal_state_reference().into_untyped())
        .expect("fiscal state")
        .decode_payload::<FiscalStateRecord>()
        .expect("fiscal state payload")
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
        request_id: "receipt.execution".to_owned(),
        external_evidence: [evidence].into_iter().collect(),
    };
    let now = canwu.time();
    canwu_fiscal::enqueue_execution_receipt(canwu, now, &duplicate)
        .expect("enqueue duplicate receipt");
    let error = canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect_err("one adapter result cannot settle twice");
    assert!(error.message.contains("external fiscal operation"));
}

fn assert_receipt_retry_is_idempotent(
    canwu: &mut Canwu,
    packet: &FiscalExecutionReceiptPacket,
    procedure_revision: u64,
) {
    let now = canwu.time();
    enqueue_execution_receipt(canwu, now, packet).expect("idempotent receipt ingress");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("idempotent receipt boundary");
    assert_eq!(fiscal_state(canwu).procedure_revision, procedure_revision);
}

fn enqueue_action(
    canwu: &mut Canwu,
    actor: canwu_api::PersonId,
    request_id: u64,
    request: &FiscalActionRequest,
) {
    let command = fiscal_action_command(request).expect("fiscal command");
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
    let assessment_revision = fiscal_state_version(canwu);
    enqueue_action(
        canwu,
        actor,
        first_request_id,
        &FiscalActionRequest {
            action_id: format!("{prefix}.assess.action"),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: assessment_revision,
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
    let authorization_revision = fiscal_state_version(canwu);
    enqueue_action(
        canwu,
        actor,
        first_request_id + 1,
        &FiscalActionRequest {
            action_id: format!("{prefix}.authorize.action"),
            authority_binding_id: "authority.revenue-minister".to_owned(),
            expected_procedure_revision: authorization_revision,
            action: FiscalAction::AuthorizeExecution {
                request_id: request_id.clone(),
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
    enqueue_execution_receipt(
        canwu,
        now,
        &FiscalExecutionReceiptPacket {
            receipt_id: receipt_id.to_owned(),
            request_id: evidence.request_id.clone(),
            external_evidence: [version].into_iter().collect(),
        },
    )
    .expect("execution receipt ingress");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("execution receipt boundary");
}
