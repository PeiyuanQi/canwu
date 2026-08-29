use canwu_api::{
    BoundaryId, Canwu, Command, DecisionAction, DecisionContext, DecisionOption,
    DecisionTicketDraft, DecisionTicketId, EntityRef, EvidenceRef, GovernmentId, IngressId,
    KnowledgeHolderRef, Scenario, SimTime,
};
use canwu_law::{
    ApplicabilityProfileDefinition, AuthoritySeatDefinition, Ballot, ClauseDefinition,
    ClauseOperation, JurisdictionRelationDefinition, JurisdictionRelationKind, LawBudgets,
    LawPlugin, LegalCompetenceDisposition, LegalDefinition, LegalInstitutionDefinition,
    LegalJurisdictionDefinition, LegalOrderDefinition, LegalProposal, LegalPublicityEvent,
    LegalRecordRef, LegalRuntime, LegalSourceProfileDefinition, NormativeModality,
    OperativeDisposition, PendingLegalIntent, ProcedureProfileDefinition, ProcedureStageDefinition,
    ProcedureStageKind, ProposalStatus, SourceMode, compile_law,
};
use std::collections::{BTreeMap, BTreeSet};

#[allow(clippy::too_many_lines)]
fn definition() -> LegalDefinition {
    let mut definition = LegalDefinition::new("constitutional-order");
    definition.orders.push(LegalOrderDefinition {
        id: "republic".to_owned(),
        precedence_profile: "constitutional".to_owned(),
    });
    definition.jurisdictions.push(LegalJurisdictionDefinition {
        id: "national".to_owned(),
        relations: Vec::new(),
        metadata: BTreeMap::default(),
    });
    definition.procedures.push(ProcedureProfileDefinition {
        id: "legislature".to_owned(),
        stages: vec![ProcedureStageDefinition {
            id: "vote".to_owned(),
            kind: ProcedureStageKind::Deliberation,
            seats: vec!["member-1".to_owned()],
            allowed_ballots: vec![Ballot::Abstain, Ballot::Against, Ballot::For],
            quorum: 1,
            threshold: 500,
            deadline_minutes: 1_000,
            allow_replacement: false,
        }],
        deterministic_tie_break: "reject".to_owned(),
        reservation_pool: None,
        reservation_quantity: 0,
    });
    definition.institutions.push(LegalInstitutionDefinition {
        id: "assembly".to_owned(),
        organization: None,
        jurisdictions: vec!["national".to_owned()],
        seats: vec![AuthoritySeatDefinition {
            id: "member-1".to_owned(),
            holder: Some(KnowledgeHolderRef::Entity(EntityRef::Government(
                GovernmentId::new(1),
            ))),
            permission_profile: "vote".to_owned(),
        }],
        procedures: vec!["legislature".to_owned()],
        competences: vec![canwu_law::LegalCompetenceDefinition {
            legal_orders: vec!["republic".to_owned()],
            jurisdictions: vec!["national".to_owned()],
            subject_matters: vec!["voting".to_owned()],
            source_modes: vec![SourceMode::Promulgated],
            operations: vec![canwu_law::LawOperation::Establish],
            procedures: vec!["legislature".to_owned()],
            forums: vec!["*".to_owned()],
            can_adjudicate: false,
        }],
    });
    definition.clauses.push(ClauseDefinition {
        id: "claim-right".to_owned(),
        schema: "canwu.law.claim-right.v1".to_owned(),
        modality: NormativeModality::ClaimRight,
        operation_kinds: vec!["right".to_owned()],
    });
    definition
        .applicability_profiles
        .push(ApplicabilityProfileDefinition {
            id: "national-rights".to_owned(),
            legal_order: "republic".to_owned(),
            temporal_conflict_rule: "later-valid-source".to_owned(),
            pipeline: ["scope", "jurisdiction", "validity", "conflict"]
                .map(str::to_owned)
                .to_vec(),
            jurisdiction_traversal: Vec::new(),
            max_candidates: 16,
        });
    definition.predicates.extend([
        canwu_law::LegalPredicateDefinition {
            id: "adult".to_owned(),
            knowledge_schema: None,
            payload_pointer: None,
        },
        canwu_law::LegalPredicateDefinition {
            id: "citizen".to_owned(),
            knowledge_schema: None,
            payload_pointer: None,
        },
    ]);
    definition
        .precedence_profiles
        .push(canwu_law::PrecedenceProfileDefinition {
            id: "constitutional".to_owned(),
            ordered_bases: vec![
                canwu_law::ConflictResolutionBasis::Competence,
                canwu_law::ConflictResolutionBasis::Supremacy,
                canwu_law::ConflictResolutionBasis::Ruling,
                canwu_law::ConflictResolutionBasis::Temporal,
            ],
        });
    definition
        .source_profiles
        .push(LegalSourceProfileDefinition {
            id: "legislature".to_owned(),
            mode: SourceMode::Promulgated,
            procedure: Some("legislature".to_owned()),
            applicability_profile: "national-rights".to_owned(),
            origin_policy: canwu_law::SourceOriginPolicy::NoOrigin,
            authority_policy: canwu_law::SourceAuthorityPolicy::ProceduralInstitution,
            publicity_policy: canwu_law::PublicityPolicy::ValidityCondition,
            publicity_signal_kind: Some("law.publication".to_owned()),
            required_signal_kinds: Vec::new(),
            min_evidence: 0,
            max_evidence: 8,
            require_claimant: false,
            allow_retroactive: false,
            agreement_namespace: None,
            agreement_kind: None,
            min_agreement_parties: 0,
            require_agreement_ratification: false,
        });
    definition
        .signal_providers
        .push(canwu_law::LegalSignalProviderDefinition {
            signal_kind: "law.publication".to_owned(),
            plugin: "test-gazette".to_owned(),
            packet_type: "publication".to_owned(),
        });
    definition
}

fn stage_required_contexts(runtime: &mut LegalRuntime, plan: &canwu_law::CompiledLawPlan) {
    let canwu = Canwu::new(7, Scenario::new(SimTime::EPOCH, Vec::new())).expect("knowledge host");
    for requirement in runtime
        .pending_actor_context_requirements(plan)
        .expect("actor context requirements")
    {
        runtime
            .stage_actor_context_from_canwu(
                plan,
                &requirement,
                &canwu,
                &canwu_api::KnowledgeQuery::default(),
            )
            .expect("stage actor context");
    }
}

#[test]
fn compilation_is_order_independent_and_rejects_hierarchical_cycles() {
    let first = definition();
    let mut reordered = first.clone();
    reordered.orders.reverse();
    reordered.institutions.reverse();
    assert_eq!(
        compile_law(&first).expect("first plan").content_hash,
        compile_law(&reordered)
            .expect("reordered plan")
            .content_hash
    );

    let mut cyclic = first;
    cyclic.jurisdictions.push(LegalJurisdictionDefinition {
        id: "province".to_owned(),
        relations: vec![JurisdictionRelationDefinition {
            from: "province".to_owned(),
            to: "national".to_owned(),
            kind: JurisdictionRelationKind::Appeal,
        }],
        metadata: BTreeMap::default(),
    });
    cyclic.jurisdictions[0]
        .relations
        .push(JurisdictionRelationDefinition {
            from: "national".to_owned(),
            to: "province".to_owned(),
            kind: JurisdictionRelationKind::Appeal,
        });
    assert!(compile_law(&cyclic).is_err());
}

#[test]
#[allow(clippy::too_many_lines)]
fn authorized_intent_becomes_versioned_law_on_a_later_boundary() {
    let plan = compile_law(&definition()).expect("plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(
            &plan,
            LegalProposal {
                id: "women-suffrage".to_owned(),
                sponsor: Some(EntityRef::Government(GovernmentId::new(1))),
                legal_order: "republic".to_owned(),
                jurisdictions: vec!["national".to_owned()],
                subjects: Vec::new(),
                cultural_dependencies: Vec::new(),
                clauses: vec![ClauseOperation {
                    clause: "claim-right".to_owned(),
                    operation: "establish".to_owned(),
                    content_hash: "a".repeat(64),
                    value: serde_json::json!({"holder": "adult_women", "action": "vote"}),
                    holders: vec!["status:adult-women".to_owned()],
                    duty_bearers: vec!["institution:election-administration".to_owned()],
                    subject_matters: vec!["voting".to_owned()],
                    conditions: vec!["adult".to_owned(), "citizen".to_owned()],
                    exceptions: Vec::new(),
                    standing: Vec::new(),
                    forum: None,
                    remedy_profile: None,
                    territories: Vec::new(),
                }],
                source_profile: "legislature".to_owned(),
                procedure_profile: "legislature".to_owned(),
                procedure_profile_hash: plan.content_hash.clone(),
                deadline: SimTime::from_minutes(10),
                effective_at: SimTime::from_minutes(10),
                operation: canwu_law::LawOperation::Establish,
                rule_id: "rule:women-suffrage".to_owned(),
                competence: LegalCompetenceDisposition::Confirmed,
                defects: Vec::new(),
                validity: OperativeDisposition::Operative,
                origin: None,
                publicity: None,
                retrospective_from: None,
                status: ProposalStatus::Draft,
                adopted_at: None,
                source_version: None,
                law_version: None,
                admitted_signal_kinds: BTreeSet::new(),
                evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
                expected_rule_head: None,
                expected_versions: Vec::new(),
                active_procedure: None,
            },
        )
        .expect("proposal");
    stage_required_contexts(&mut runtime, &plan);

    let first = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("outbox boundary");
    assert_eq!(first.emitted_outbox.len(), 1);
    assert!(runtime.sources.is_empty());

    runtime
        .record_publicity(
            &plan,
            LegalPublicityEvent {
                id: "publicity:women-suffrage".to_owned(),
                proposal: LegalRecordRef {
                    kind: "proposal".to_owned(),
                    id: "women-suffrage".to_owned(),
                },
                at: SimTime::from_minutes(1),
                signal_kind: "law.publication".to_owned(),
                medium: "official-gazette".to_owned(),
                scope: vec!["national".to_owned()],
                evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
            },
        )
        .expect("publicize enacted proposal");

    let item = &first.emitted_outbox[0];
    let persisted = runtime.outbox.get_mut(&item.sequence).expect("test outbox");
    persisted.enqueue_expected_revision = Some(0);
    persisted.enqueue_ingress = Some(EvidenceRef::Ingress(IngressId::new(item.sequence)));
    persisted.enqueue_outcome_commitment = Some("test-outcome".to_owned());
    persisted.dispatch = canwu_law::DispatchState::Enqueued;
    runtime.pending_outbox_sequences.remove(&item.sequence);
    let option = item
        .draft
        .options
        .iter()
        .find(|option| option.id == "for")
        .expect("for option");
    let DecisionAction::Command { command } = &option.action else {
        panic!("legal vote must execute a command");
    };
    let Command::Plugin { payload, .. } =
        serde_json::from_value::<Command>(command.clone()).expect("legal command")
    else {
        panic!("legal vote must execute a plugin command");
    };
    let intent: PendingLegalIntent =
        serde_json::from_value(payload["intent"].clone()).expect("legal intent");
    runtime.queue_pending_intent(&plan, intent).expect("intent");
    let adopted = runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("adoption boundary");
    assert_eq!(adopted.adopted_proposals, vec!["women-suffrage"]);
    assert_eq!(runtime.sources.len(), 1);
    assert_eq!(runtime.law_versions.len(), 1);
    assert!(
        runtime
            .operative_rules()
            .all(|rule| rule.operative_version.is_none())
    );

    runtime
        .settle_boundary(&plan, SimTime::from_minutes(10), &[])
        .expect("effective boundary");
    assert!(
        runtime
            .operative_rules()
            .any(|rule| rule.operative_version.is_some())
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn culture_retirement_preserves_enacted_law_and_blocks_live_dependencies() {
    let plan = compile_law(&definition()).expect("plan");
    let mut runtime = LegalRuntime::new(&plan);
    let historical = canwu_law::CulturalTargetGenerationRef {
        target: "human-rights".to_owned(),
        generation: 1,
    };
    runtime
        .retire_cultural_target_for_plan(&plan, &historical, SimTime::EPOCH, "support is cold")
        .expect("cold target retires");
    runtime
        .retire_cultural_target_for_plan(&plan, &historical, SimTime::EPOCH, "idempotent retry")
        .expect("cold target retirement is idempotent");
    assert_eq!(runtime.retirements.len(), 1);
    assert!(runtime.retired_cultural_targets.contains(&historical));

    let active = canwu_law::CulturalTargetGenerationRef {
        target: "active-rights".to_owned(),
        generation: 3,
    };
    let culture_evidence = EvidenceRef::Boundary(BoundaryId::new(1));

    runtime.proposals.insert(
        "live".to_owned(),
        LegalProposal {
            id: "live".to_owned(),
            sponsor: None,
            legal_order: "republic".to_owned(),
            jurisdictions: vec!["national".to_owned()],
            subjects: Vec::new(),
            cultural_dependencies: vec![canwu_law::LegalCulturalDependency {
                target: active.clone(),
                kind: canwu_law::CulturalDependencyKind::AdoptionEvidence,
                evidence: culture_evidence.clone(),
            }],
            clauses: Vec::new(),
            source_profile: "legislature".to_owned(),
            procedure_profile: "legislature".to_owned(),
            procedure_profile_hash: plan.content_hash.clone(),
            deadline: SimTime::from_minutes(10),
            effective_at: SimTime::from_minutes(10),
            operation: canwu_law::LawOperation::Establish,
            rule_id: "rule:live".to_owned(),
            competence: LegalCompetenceDisposition::Confirmed,
            defects: Vec::new(),
            validity: OperativeDisposition::Operative,
            origin: None,
            publicity: None,
            retrospective_from: None,
            status: ProposalStatus::Deliberating,
            adopted_at: None,
            source_version: None,
            law_version: None,
            admitted_signal_kinds: BTreeSet::new(),
            evidence: vec![culture_evidence],
            expected_rule_head: None,
            expected_versions: Vec::new(),
            active_procedure: None,
        },
    );
    runtime.outbox.insert(
        1,
        canwu_law::LegalDecisionOutboxItem {
            sequence: 1,
            id: "outbox:1".to_owned(),
            operation: "create".to_owned(),
            ticket_id: 1,
            create_request_id: 1,
            refresh_request_id: None,
            resolution_request_id: 2,
            nested_command_request_id: 1,
            enqueue_expected_revision: None,
            enqueue_ingress: None,
            enqueue_outcome_commitment: None,
            proposal: LegalRecordRef {
                kind: "proposal".to_owned(),
                id: "live".to_owned(),
            },
            procedure: LegalRecordRef {
                kind: "procedure".to_owned(),
                id: "procedure:live".to_owned(),
            },
            stage: 0,
            round: 0,
            seat: "member-1".to_owned(),
            decision_controller_id: canwu_law::decision_controller_id("assembly", "member-1"),
            permission_profile_id: "vote".to_owned(),
            source_boundary: None,
            controller: KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(1))),
            command_subject: None,
            draft: DecisionTicketDraft {
                id: DecisionTicketId::new(1),
                definition: "canwu.law.test.v1".to_owned(),
                decision_maker: EntityRef::Government(GovernmentId::new(1)),
                assigned_controller: canwu_law::decision_controller_id("assembly", "member-1"),
                summary: "Test legal decision".to_owned(),
                context: DecisionContext::new(
                    "canwu.law.test-context.v1",
                    serde_json::json!({"culture": "active-rights"}),
                ),
                options: vec![DecisionOption::new("hold", "Hold")],
                deadline: Some(SimTime::from_minutes(10)),
            },
            knowledge_read_cut: canwu_api::KnowledgeReadCut {
                boundary: Some(BoundaryId::new(1)),
                holder_projection_root: "root".to_owned(),
                holder_overlay_root: None,
            },
            knowledge_record_ids: Vec::new(),
            context_hash: String::new(),
            due_at: SimTime::EPOCH,
            priority: 0,
            dispatch: canwu_law::DispatchState::Pending,
            expires_at: SimTime::from_minutes(10),
            acknowledgement: None,
        },
    );
    assert!(
        runtime
            .retire_cultural_target_for_plan(&plan, &active, SimTime::EPOCH, "premature",)
            .is_err()
    );
    let item = runtime.outbox.get_mut(&1).expect("live outbox");
    item.dispatch = canwu_law::DispatchState::Acknowledged;
    item.acknowledgement = Some(EvidenceRef::Boundary(BoundaryId::new(1)));
    runtime
        .retire_cultural_target_for_plan(&plan, &active, SimTime::EPOCH, "decision completed")
        .expect("acknowledged history no longer blocks retirement");
}

#[test]
fn budget_failures_are_admission_errors() {
    let mut value = definition();
    value.budgets = LawBudgets {
        max_memory_bytes: 1,
        ..LawBudgets::conservative()
    };
    assert!(compile_law(&value).is_err());
    let _ = Ballot::For;
}

#[test]
fn plugin_registers_owned_records_and_the_pending_intent_command() {
    let mut canwu = Canwu::demo(7).expect("demo");
    canwu.register_plugin(&LawPlugin).expect("law plugin");
    let descriptor = canwu
        .plugin_descriptors()
        .find(|descriptor| descriptor.name == "canwu-law")
        .expect("law descriptor");
    assert_eq!(descriptor.commands.len(), 1);
    assert_eq!(descriptor.record_schemas.len(), 1);
    assert_eq!(descriptor.commands[0].name, "submit_pending_intent");
}
