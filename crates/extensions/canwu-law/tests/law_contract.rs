use canwu_api::{
    ArchiveProvider, ArchiveStore, ArchiveStoreOutcome, BoundaryContext, BoundaryDirective,
    BoundaryId, BoundaryPhase, BoundaryProposal, BoundaryRequest, BoundarySystemContract, Canwu,
    CanwuError, Command, DecisionAction, DecisionAuthority, DecisionControllerBinding,
    DecisionIngressRequest, DecisionMutation, DecisionPolicyIdentity, DecisionPolicyKind,
    DecisionRequestId, DecisionTicketId, DomainRecord, DomainRecordClass, DomainRecordLifecycle,
    DomainRecordRef, EntityRef, ErrorCode, EvidenceJournalSegment, EvidenceRef,
    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD, IngressClass, IngressId, IngressPayload,
    KnowledgeHistoryView, KnowledgeHolderRef, KnowledgeOrigin, KnowledgeQuery,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeWriteGrant,
    PayloadSchema, PersonId, PluginIngressDescriptor, PluginIngressRequest, PluginKnowledgeSchema,
    PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey,
    StateVisibility, SystemCadence, TerritoryId,
};
use canwu_law::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct MemoryArchive {
    segments: RefCell<BTreeMap<String, EvidenceJournalSegment>>,
}

impl ArchiveProvider for MemoryArchive {
    fn load_evidence_segment(
        &self,
        segment_id: &str,
    ) -> Result<Option<EvidenceJournalSegment>, CanwuError> {
        Ok(self.segments.borrow().get(segment_id).cloned())
    }
}

impl ArchiveStore for MemoryArchive {
    fn store_evidence_segment(
        &self,
        segment: &EvidenceJournalSegment,
    ) -> Result<ArchiveStoreOutcome, CanwuError> {
        let id = segment
            .archive
            .as_ref()
            .ok_or_else(|| CanwuError::new(ErrorCode::InvalidArchive, "missing archive index"))?
            .header
            .segment_id
            .clone();
        let mut segments = self.segments.borrow_mut();
        if let Some(existing) = segments.get(&id) {
            return if existing == segment {
                Ok(ArchiveStoreOutcome::AlreadyPresent)
            } else {
                Err(CanwuError::new(
                    ErrorCode::InvalidArchive,
                    "archive ID is already bound to different bytes",
                ))
            };
        }
        segments.insert(id, segment.clone());
        Ok(ArchiveStoreOutcome::Stored)
    }
}

#[allow(clippy::too_many_lines)]
fn definition() -> LegalDefinition {
    let seats = vec!["east".to_owned(), "west".to_owned()];
    LegalDefinition {
        id: "suffrage-law".to_owned(),
        orders: vec![LegalOrderDefinition {
            id: "state-code".to_owned(),
            precedence_profile: "constitutional".to_owned(),
        }],
        jurisdictions: vec![LegalJurisdictionDefinition {
            id: "assembly-forum".to_owned(),
            relations: Vec::new(),
            metadata: BTreeMap::default(),
        }],
        institutions: vec![LegalInstitutionDefinition {
            id: "assembly".to_owned(),
            organization: None,
            jurisdictions: vec!["assembly-forum".to_owned()],
            seats: seats
                .iter()
                .map(|seat| AuthoritySeatDefinition {
                    id: seat.clone(),
                    holder: Some(holder(seat)),
                    permission_profile: "legislator".to_owned(),
                })
                .collect(),
            procedures: vec!["ordinary-bill".to_owned()],
            competences: vec![LegalCompetenceDefinition {
                legal_orders: vec!["state-code".to_owned()],
                jurisdictions: vec!["assembly-forum".to_owned()],
                subject_matters: vec!["voting".to_owned()],
                source_modes: vec![SourceMode::Promulgated, SourceMode::Adjudicated],
                operations: vec![
                    LawOperation::Establish,
                    LawOperation::Recognize,
                    LawOperation::Amend,
                    LawOperation::Suspend,
                    LawOperation::Resume,
                    LawOperation::Displace,
                    LawOperation::Annul,
                    LawOperation::Repeal,
                    LawOperation::Expire,
                ],
                procedures: vec!["ordinary-bill".to_owned()],
                forums: vec!["eligibility-court".to_owned()],
                can_adjudicate: true,
            }],
        }],
        procedures: vec![ProcedureProfileDefinition {
            id: "ordinary-bill".to_owned(),
            stages: vec![ProcedureStageDefinition {
                id: "vote".to_owned(),
                kind: ProcedureStageKind::Deliberation,
                seats,
                allowed_ballots: vec![Ballot::Abstain, Ballot::Against, Ballot::For],
                quorum: 2,
                threshold: 500,
                deadline_minutes: 10,
                allow_replacement: false,
            }],
            deterministic_tie_break: "seat-id".to_owned(),
            reservation_pool: None,
            reservation_quantity: 0,
        }],
        clauses: vec![ClauseDefinition {
            id: "voting-eligibility".to_owned(),
            schema: "canwu.test.eligibility.v1".to_owned(),
            modality: NormativeModality::Eligibility,
            operation_kinds: vec!["right".to_owned()],
        }],
        source_profiles: vec![
            LegalSourceProfileDefinition {
                id: "statute".to_owned(),
                mode: SourceMode::Promulgated,
                procedure: Some("ordinary-bill".to_owned()),
                applicability_profile: "state-choice".to_owned(),
                origin_policy: SourceOriginPolicy::NoOrigin,
                authority_policy: SourceAuthorityPolicy::ProceduralInstitution,
                publicity_policy: PublicityPolicy::ValidityCondition,
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
            },
            LegalSourceProfileDefinition {
                id: "custom".to_owned(),
                mode: SourceMode::Accreted,
                procedure: None,
                applicability_profile: "state-choice".to_owned(),
                origin_policy: SourceOriginPolicy::NoOrigin,
                authority_policy: SourceAuthorityPolicy::EvidenceClaim,
                publicity_policy: PublicityPolicy::NotRequired,
                publicity_signal_kind: None,
                required_signal_kinds: vec!["practice.recognized".to_owned()],
                min_evidence: 1,
                max_evidence: 8,
                require_claimant: true,
                allow_retroactive: false,
                agreement_namespace: None,
                agreement_kind: None,
                min_agreement_parties: 0,
                require_agreement_ratification: false,
            },
        ],
        signal_providers: vec![
            LegalSignalProviderDefinition {
                signal_kind: "law.publication".to_owned(),
                plugin: "test-gazette".to_owned(),
                packet_type: "publication".to_owned(),
            },
            LegalSignalProviderDefinition {
                signal_kind: "practice.recognized".to_owned(),
                plugin: "test-society".to_owned(),
                packet_type: "recognized-practice".to_owned(),
            },
            LegalSignalProviderDefinition {
                signal_kind: "culture.support".to_owned(),
                plugin: "test-society".to_owned(),
                packet_type: "culture-support".to_owned(),
            },
        ],
        applicability_profiles: vec![ApplicabilityProfileDefinition {
            id: "state-choice".to_owned(),
            legal_order: "state-code".to_owned(),
            temporal_conflict_rule: "later-in-time".to_owned(),
            pipeline: ["scope", "jurisdiction", "validity", "conflict"]
                .map(str::to_owned)
                .to_vec(),
            jurisdiction_traversal: Vec::new(),
            max_candidates: 64,
        }],
        predicates: vec![
            LegalPredicateDefinition {
                id: "adult".to_owned(),
                knowledge_schema: None,
                payload_pointer: None,
            },
            LegalPredicateDefinition {
                id: "citizen".to_owned(),
                knowledge_schema: None,
                payload_pointer: None,
            },
            LegalPredicateDefinition {
                id: "disqualified".to_owned(),
                knowledge_schema: None,
                payload_pointer: None,
            },
        ],
        forums: vec![LegalForumProfileDefinition {
            id: "eligibility-court".to_owned(),
            jurisdiction: "assembly-forum".to_owned(),
            legal_orders: vec!["state-code".to_owned()],
            subject_matters: vec!["voting".to_owned()],
            institutions: vec!["assembly".to_owned()],
            proof_profiles: vec!["preponderance".to_owned()],
            standing_profiles: vec![
                "affected-voter".to_owned(),
                "excluded-eligible-voter".to_owned(),
            ],
            remedy_profiles: vec!["declaration".to_owned(), "restore-registration".to_owned()],
            precedent_profiles: vec!["persuasive".to_owned()],
        }],
        precedence_profiles: vec![PrecedenceProfileDefinition {
            id: "constitutional".to_owned(),
            ordered_bases: vec![
                ConflictResolutionBasis::Competence,
                ConflictResolutionBasis::Supremacy,
                ConflictResolutionBasis::Specificity,
                ConflictResolutionBasis::Ruling,
                ConflictResolutionBasis::Temporal,
            ],
        }],
        id_blocks: LawIdBlocks::default(),
        budgets: LawBudgets::default(),
    }
}

fn holder(id: &str) -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Domain(DomainRecordRef::new("test", "seat", id)))
}

fn applicable_facts() -> BTreeMap<String, bool> {
    BTreeMap::from([
        ("adult".to_owned(), true),
        ("citizen".to_owned(), true),
        ("disqualified".to_owned(), false),
    ])
}

fn fact_evidence_for(facts: &BTreeMap<String, bool>) -> BTreeMap<String, EvidenceRef> {
    facts
        .keys()
        .cloned()
        .map(|predicate| (predicate, EvidenceRef::Boundary(BoundaryId::new(1))))
        .collect()
}

fn applicable_fact_evidence() -> BTreeMap<String, EvidenceRef> {
    fact_evidence_for(&applicable_facts())
}

#[derive(Clone, Copy, Debug)]
struct TestSocietyPlugin;

fn generate_recognized_practice(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(record) = view.ingress(*ingress_id)? else {
            continue;
        };
        if let IngressPayload::Plugin {
            plugin,
            packet_type,
            ..
        } = &record.payload
            && plugin == "test-society"
            && matches!(
                packet_type.as_str(),
                "observe-practice" | "observe-delayed-practice"
            )
        {
            directives.push(BoundaryDirective::ScheduleIngress {
                after: if packet_type == "observe-delayed-practice" {
                    SimDuration::minutes(5)
                } else {
                    SimDuration::ZERO
                },
                packet_type: "recognized-practice".to_owned(),
                priority: 1,
                payload: serde_json::json!({"practice": "repeated public conduct"}),
                affected: Vec::new(),
            });
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn legal_fact_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new("test-law", "predicate-facts"), 1)
}

#[allow(clippy::unnecessary_wraps)]
fn publish_legal_facts(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            holder: KnowledgeHolderRef::Person(PersonId::new(1)),
            visibility: StateVisibility::SameBoundary,
            producer_correlation: Some("legal-facts:person-1".to_owned()),
            records: vec![KnowledgeRecordDraft {
                schema: legal_fact_schema(),
                subjects: Vec::new(),
                payload: serde_json::json!({
                    "facts": {"adult": true, "citizen": true, "disqualified": false}
                }),
                as_of: None,
                confidence_per_mille: 1_000,
                origin: KnowledgeOrigin {
                    method: "test-observation".to_owned(),
                    evidence: Vec::new(),
                },
                supersedes: Vec::new(),
                contradicts: Vec::new(),
            }],
            summary: "Publish holder-relative legal predicate facts".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct ActorLegalFactsPlugin;

impl SimulationPlugin for ActorLegalFactsPlugin {
    fn name(&self) -> &'static str {
        "test-law-facts"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000099"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: legal_fact_schema(),
            schema_hash: "1000000000000000000000000000000000000000000000000000000000000099"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
        })?;
        let mut contract = BoundarySystemContract::new(
            "publish-legal-facts",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: legal_fact_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, publish_legal_facts)
    }
}

impl SimulationPlugin for TestSocietyPlugin {
    fn name(&self) -> &'static str {
        "test-society"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000042"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for name in [
            "observe-practice",
            "observe-delayed-practice",
            "recognized-practice",
            "culture-support",
        ] {
            registrar.register_ingress(PluginIngressDescriptor {
                name: name.to_owned(),
                description: "Test-only legal signal provenance".to_owned(),
                class: IngressClass::Information,
                payload_schema: PayloadSchema::Any,
            })?;
        }
        let mut contract = BoundarySystemContract::new(
            "recognize-practice",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress()];
        registrar.register_boundary_system(contract, generate_recognized_practice)
    }
}

fn proposal() -> LegalProposal {
    LegalProposal {
        id: "suffrage".to_owned(),
        sponsor: Some(EntityRef::Person(PersonId::new(1))),
        legal_order: "state-code".to_owned(),
        jurisdictions: vec!["assembly-forum".to_owned()],
        subjects: Vec::new(),
        cultural_dependencies: Vec::new(),
        clauses: vec![ClauseOperation {
            clause: "voting-eligibility".to_owned(),
            operation: "establish".to_owned(),
            content_hash: "a".repeat(64),
            value: serde_json::json!({"eligible": "adult-citizens"}),
            holders: vec!["status:adult-women".to_owned()],
            duty_bearers: vec!["institution:election-administration".to_owned()],
            subject_matters: vec!["voting".to_owned()],
            conditions: vec!["adult".to_owned(), "citizen".to_owned()],
            exceptions: vec!["disqualified".to_owned()],
            standing: vec!["excluded-eligible-voter".to_owned()],
            forum: Some("eligibility-court".to_owned()),
            remedy_profile: Some("restore-registration".to_owned()),
            territories: Vec::new(),
        }],
        source_profile: "statute".to_owned(),
        procedure_profile: "ordinary-bill".to_owned(),
        procedure_profile_hash: "b".repeat(64),
        deadline: SimTime::from_minutes(10),
        effective_at: SimTime::from_minutes(10),
        operation: LawOperation::Establish,
        rule_id: "rule:suffrage".to_owned(),
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
        evidence: Vec::new(),
        expected_rule_head: None,
        expected_versions: Vec::new(),
        active_procedure: None,
    }
}

fn intent_from_outbox(item: &LegalDecisionOutboxItem, option_id: &str) -> PendingLegalIntent {
    let option = item
        .draft
        .options
        .iter()
        .find(|option| option.id == option_id)
        .expect("legal decision option");
    let DecisionAction::Command { command } = &option.action else {
        panic!("legal option must submit a command");
    };
    let Command::Plugin { payload, .. } =
        serde_json::from_value::<Command>(command.clone()).expect("decode legal command")
    else {
        panic!("legal option must submit a plugin command");
    };
    serde_json::from_value(payload["intent"].clone()).expect("decode pending legal intent")
}

fn stage_required_contexts(runtime: &mut LegalRuntime, plan: &CompiledLawPlan) {
    let canwu = canwu_api::Canwu::new(7, Scenario::new(SimTime::EPOCH, Vec::new()))
        .expect("knowledge host");
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

fn proposal_change(id: &str, operation: LawOperation, effective_at: i64) -> LegalProposal {
    let mut proposal = proposal();
    id.clone_into(&mut proposal.id);
    proposal.operation = operation;
    match operation {
        LawOperation::Amend => "amend",
        LawOperation::Repeal => "repeal",
        _ => "establish",
    }
    .clone_into(&mut proposal.clauses[0].operation);
    proposal.clauses[0].value = serde_json::json!({"change": id});
    proposal.deadline = SimTime::from_minutes(effective_at + 20);
    proposal.effective_at = SimTime::from_minutes(effective_at);
    proposal
}

fn submit_two_votes(
    runtime: &mut LegalRuntime,
    plan: &CompiledLawPlan,
    emitted: &[LegalDecisionOutboxItem],
) {
    assert_eq!(emitted.len(), 2);
    let proposal = emitted.first().expect("legal outbox").proposal.clone();
    if runtime
        .proposals
        .get(&proposal.id)
        .is_some_and(|record| record.publicity.is_none())
    {
        runtime
            .record_publicity(
                plan,
                LegalPublicityEvent {
                    id: format!("publicity:{}", proposal.id),
                    proposal,
                    at: runtime.last_settled_at,
                    signal_kind: "law.publication".to_owned(),
                    medium: "official-gazette".to_owned(),
                    scope: vec!["assembly-forum".to_owned()],
                    evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
                },
            )
            .expect("record statutory publicity");
    }
    for item in emitted {
        mark_test_outbox(runtime, item.sequence, IngressId::new(item.sequence));
        runtime
            .queue_pending_intent(plan, intent_from_outbox(item, "for"))
            .expect("queue typed legal vote");
    }
}

fn mark_test_outbox(runtime: &mut LegalRuntime, sequence: u64, ingress: IngressId) {
    let item = runtime.outbox.get_mut(&sequence).expect("test outbox");
    item.enqueue_expected_revision = Some(0);
    item.enqueue_ingress = Some(EvidenceRef::Ingress(ingress));
    item.enqueue_outcome_commitment = Some("test-outcome".to_owned());
    item.dispatch = DispatchState::Enqueued;
    runtime.pending_outbox_sequences.remove(&sequence);
}

#[allow(clippy::too_many_lines)]
fn resolve_contested_amendment(
    runtime: &mut LegalRuntime,
    plan: &CompiledLawPlan,
    suffix: &str,
    selected: LegalRecordRef,
) -> ApplicabilityResult {
    let case_id = format!("case:contested-{suffix}");
    runtime
        .record_case(
            plan,
            LegalCase {
                id: case_id.clone(),
                legal_order: "state-code".to_owned(),
                subject_matters: vec!["voting".to_owned()],
                parties: Vec::new(),
                claims: vec!["validity".to_owned()],
                forum: "eligibility-court".to_owned(),
                standing: Some("affected-voter".to_owned()),
                proof_profile: "preponderance".to_owned(),
                issues: vec!["validity".to_owned()],
                deadline: SimTime::from_minutes(20),
                remedies: vec!["declaration".to_owned()],
                allegations: vec![EvidenceRef::Boundary(BoundaryId::new(2))],
            },
        )
        .expect("record contested-law case");
    let finding_id = format!("finding:contested-{suffix}");
    runtime
        .record_finding(
            plan,
            LegalFindingVersion {
                id: finding_id.clone(),
                case_id: case_id.clone(),
                issue: "validity".to_owned(),
                finding: "compiled court resolved the competing validity claims".to_owned(),
                accepted: true,
                burden: "preponderance".to_owned(),
                evidence: vec![EvidenceRef::Boundary(BoundaryId::new(2))],
                at: SimTime::from_minutes(2),
                predecessor: None,
            },
        )
        .expect("record contested-law finding");
    let versions = [1_u64, 2]
        .map(|ordinal| LegalRecordRef {
            kind: "law_version".to_owned(),
            id: format!("law-version:rule:contested-amendment:{ordinal}"),
        })
        .to_vec();
    let displaced = versions
        .iter()
        .filter(|reference| **reference != selected)
        .cloned()
        .collect::<Vec<_>>();
    let ruling_id = format!("ruling:contested-{suffix}");
    let mut sources = runtime
        .sources
        .values()
        .map(|source| LegalRecordRef {
            kind: "source_version".to_owned(),
            id: source.id.clone(),
        })
        .collect::<Vec<_>>();
    sources.sort();
    runtime
        .record_ruling(
            plan,
            LegalRulingVersion {
                id: ruling_id.clone(),
                case_id,
                institution: "assembly".to_owned(),
                issues: vec!["validity".to_owned()],
                findings: vec![LegalRecordRef {
                    kind: "finding".to_owned(),
                    id: finding_id,
                }],
                sources,
                resolved_versions: versions.clone(),
                selected_versions: vec![selected.clone()],
                scope: vec!["assembly-forum".to_owned()],
                precedent_profile: Some("persuasive".to_owned()),
                effective_from: SimTime::from_minutes(2),
                effective_until: None,
                remedy: Some("declaration".to_owned()),
                predecessors: Vec::new(),
                disposition: OperativeDisposition::Operative,
                evidence: vec![EvidenceRef::Boundary(BoundaryId::new(2))],
            },
        )
        .expect("record competent conflict ruling");
    runtime
        .record_conflict(
            plan,
            LegalConflict {
                id: format!("conflict:contested-{suffix}"),
                versions: versions.clone(),
                governing_versions: vec![selected],
                displaced_versions: displaced,
                jurisdiction: Some("assembly-forum".to_owned()),
                recorded_at: SimTime::from_minutes(2),
                effective_from: SimTime::from_minutes(2),
                effective_until: None,
                resolution: ApplicabilityOutcome::Displaced,
                basis: ConflictResolutionBasis::Ruling,
                rationale: "competent court resolves competing validity claims".to_owned(),
                ruling: Some(LegalRecordRef {
                    kind: "ruling".to_owned(),
                    id: ruling_id,
                }),
                trace: versions,
            },
        )
        .expect("record ruling-backed conflict");
    runtime
        .query_applicability_for_plan(
            plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(2),
                read_at: SimTime::from_minutes(2),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("resolve ruling-backed validity conflict")
}

#[test]
fn compiler_is_order_invariant_and_rejects_hierarchy_cycles() {
    let first = compile_law(&definition()).expect("compile legal plan");
    let mut reordered = definition();
    reordered.institutions[0].seats.reverse();
    reordered.procedures[0].stages[0].seats.reverse();
    reordered.signal_providers.reverse();
    let second = compile_law(&reordered).expect("compile reordered plan");
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first.order_by_id["state-code"].get(), 0);

    let mut non_procedural_statute = definition();
    non_procedural_statute.source_profiles[0].procedure = None;
    non_procedural_statute.source_profiles[0].authority_policy =
        SourceAuthorityPolicy::EvidenceClaim;
    assert!(compile_law(&non_procedural_statute).is_err());

    let mut untyped_agreement = definition();
    let mut agreed = untyped_agreement.source_profiles[1].clone();
    agreed.id = "agreed".to_owned();
    agreed.mode = SourceMode::Agreed;
    agreed.origin_policy = SourceOriginPolicy::Agreement;
    untyped_agreement.source_profiles.push(agreed);
    assert!(compile_law(&untyped_agreement).is_err());

    let mut cyclic = definition();
    cyclic.jurisdictions = vec![
        LegalJurisdictionDefinition {
            id: "a".to_owned(),
            relations: vec![JurisdictionRelationDefinition {
                from: "a".to_owned(),
                to: "b".to_owned(),
                kind: JurisdictionRelationKind::Supremacy,
            }],
            metadata: BTreeMap::default(),
        },
        LegalJurisdictionDefinition {
            id: "b".to_owned(),
            relations: vec![JurisdictionRelationDefinition {
                from: "b".to_owned(),
                to: "a".to_owned(),
                kind: JurisdictionRelationKind::Supremacy,
            }],
            metadata: BTreeMap::default(),
        },
    ];
    cyclic.institutions[0].jurisdictions = vec!["a".to_owned()];
    assert!(compile_law(&cyclic).is_err());

    let mut ambiguous = definition();
    let mut duplicate_institution = ambiguous.institutions[0].clone();
    duplicate_institution.id = "second-assembly".to_owned();
    ambiguous.institutions.push(duplicate_institution);
    assert!(compile_law(&ambiguous).is_err());

    let mut missing = definition();
    missing.procedures[0].stages[0].seats = vec!["unknown-seat".to_owned()];
    assert!(compile_law(&missing).is_err());

    let mut missing_provider = definition();
    missing_provider.signal_providers.clear();
    assert!(compile_law(&missing_provider).is_err());

    let mut duplicate_provider = definition();
    duplicate_provider
        .signal_providers
        .push(duplicate_provider.signal_providers[0].clone());
    assert!(compile_law(&duplicate_provider).is_err());

    let mut invalid_provider = definition();
    invalid_provider.signal_providers[0].plugin = "Test Society".to_owned();
    assert!(compile_law(&invalid_provider).is_err());

    assert_ne!(
        decision_controller_id("a.b", "c"),
        decision_controller_id("a", "b.c")
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn per_seat_intents_create_immutable_future_effective_law() {
    let mut authored = definition();
    authored.budgets.max_outbox = 2;
    let plan = compile_law(&authored).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    stage_required_contexts(&mut runtime, &plan);

    let initial = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("materialize outbox");
    assert_eq!(initial.emitted_outbox.len(), 2);
    let ids: Vec<_> = initial
        .emitted_outbox
        .iter()
        .map(|item| item.ticket_id)
        .collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(initial.emitted_outbox[0].create_request_id, 100_000);
    assert_eq!(initial.emitted_outbox[0].refresh_request_id, Some(100_001));
    assert_eq!(initial.emitted_outbox[0].resolution_request_id, 100_002);
    assert_eq!(initial.emitted_outbox[1].create_request_id, 100_003);
    assert!(
        initial
            .emitted_outbox
            .iter()
            .all(|item| { item.draft.options.iter().all(|option| option.id != "veto") })
    );

    submit_two_votes(&mut runtime, &plan, &initial.emitted_outbox);
    let adopted = runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("adopt law");
    assert_eq!(adopted.adopted_proposals, vec!["suffrage"]);
    assert_eq!(runtime.sources.len(), 1);
    assert_eq!(runtime.law_versions.len(), 1);
    let effect = &runtime
        .law_versions
        .values()
        .next()
        .expect("suffrage law version")
        .deltas[0];
    assert_eq!(effect.modality, NormativeModality::Eligibility);
    assert_eq!(effect.holders, ["status:adult-women"]);
    assert_eq!(effect.duty_bearers, ["institution:election-administration"]);
    assert_eq!(effect.standing, ["excluded-eligible-voter"]);
    assert_eq!(
        effect.remedy_profile.as_deref(),
        Some("restore-registration")
    );
    assert!(runtime.rules["rule:suffrage"].operative_version.is_none());

    let early = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(2),
                read_at: SimTime::from_minutes(2),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("early applicability");
    assert_eq!(early.outcome, ApplicabilityOutcome::NotApplicable);

    runtime
        .settle_boundary(&plan, SimTime::from_minutes(10), &[])
        .expect("activate scheduled law");
    let operative = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(10),
                read_at: SimTime::from_minutes(10),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("operative applicability");
    assert_eq!(operative.outcome, ApplicabilityOutcome::Applicable);

    let mut missing_facts = applicable_facts();
    missing_facts.remove("citizen");
    let missing_fact_evidence = fact_evidence_for(&missing_facts);
    let indeterminate = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(10),
                read_at: SimTime::from_minutes(10),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: missing_facts,
                fact_evidence: missing_fact_evidence,
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("missing predicate fact");
    assert_eq!(indeterminate.outcome, ApplicabilityOutcome::Indeterminate);

    let mut excepted_facts = applicable_facts();
    excepted_facts.insert("disqualified".to_owned(), true);
    let excepted_fact_evidence = fact_evidence_for(&excepted_facts);
    let excepted = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(10),
                read_at: SimTime::from_minutes(10),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: excepted_facts,
                fact_evidence: excepted_fact_evidence,
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("express legal exception");
    assert_eq!(excepted.outcome, ApplicabilityOutcome::NotApplicable);

    runtime
        .retire_cultural_target_for_plan(
            &plan,
            &CulturalTargetGenerationRef {
                target: "women-political-equality".to_owned(),
                generation: 1,
            },
            SimTime::from_minutes(11),
            "institutionalized",
        )
        .expect("retire source culture target");
    assert_eq!(runtime.law_versions.len(), 1);
    runtime.validate_against_plan(&plan).expect("valid runtime");
}

#[test]
#[allow(clippy::too_many_lines)]
fn required_publicity_is_an_atomic_adoption_guard_and_retroactivity_is_explicit() {
    let mut authored = definition();
    authored.source_profiles[0].allow_retroactive = true;
    authored.budgets.max_sources = 3;
    let plan = compile_law(&authored).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut retrospective = proposal();
    retrospective.effective_at = SimTime::from_minutes(1);
    retrospective.retrospective_from = Some(SimTime::EPOCH);
    runtime
        .submit_proposal(&plan, retrospective)
        .expect("submit retrospective proposal");
    stage_required_contexts(&mut runtime, &plan);
    let opened = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("open vote");
    for item in &opened.emitted_outbox {
        mark_test_outbox(&mut runtime, item.sequence, IngressId::new(item.sequence));
        runtime
            .queue_pending_intent(&plan, intent_from_outbox(item, "for"))
            .expect("queue vote");
    }
    let before_failed_adoption = runtime.clone();
    assert!(
        runtime
            .settle_boundary(&plan, SimTime::from_minutes(2), &[])
            .is_err()
    );
    assert_eq!(runtime, before_failed_adoption);

    let future_publicity = LegalPublicityEvent {
        id: "publicity:suffrage:future".to_owned(),
        proposal: LegalRecordRef {
            kind: "proposal".to_owned(),
            id: "suffrage".to_owned(),
        },
        at: SimTime::from_minutes(10),
        signal_kind: "law.publication".to_owned(),
        medium: "official-gazette".to_owned(),
        scope: vec!["assembly-forum".to_owned()],
        evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
    };
    assert!(runtime.record_publicity(&plan, future_publicity).is_err());
    let mut wrong_provider = LegalPublicityEvent {
        id: "publicity:suffrage:wrong-provider".to_owned(),
        proposal: LegalRecordRef {
            kind: "proposal".to_owned(),
            id: "suffrage".to_owned(),
        },
        at: runtime.last_settled_at,
        signal_kind: "practice.recognized".to_owned(),
        medium: "official-gazette".to_owned(),
        scope: vec!["assembly-forum".to_owned()],
        evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
    };
    assert!(
        runtime
            .record_publicity(&plan, wrong_provider.clone())
            .is_err()
    );
    wrong_provider.id = "publicity:suffrage".to_owned();
    wrong_provider.signal_kind = "law.publication".to_owned();

    runtime
        .record_publicity(&plan, wrong_provider)
        .expect("record immutable publicity event");
    let mut oversized_publicity = runtime.clone();
    let publicity = oversized_publicity.publicity_events["publicity:suffrage"].clone();
    for suffix in 2..=4 {
        let mut duplicate = publicity.clone();
        duplicate.id = format!("publicity:suffrage:{suffix}");
        oversized_publicity
            .publicity_events
            .insert(duplicate.id.clone(), duplicate);
    }
    oversized_publicity
        .reaccount_state_budget()
        .expect("reaccount oversized cold fixture");
    assert!(oversized_publicity.validate_against_plan(&plan).is_err());
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("adopt explicitly retrospective law");
    let version = runtime.law_versions.values().next().expect("law version");
    assert_eq!(version.retrospective_from, Some(SimTime::EPOCH));
    assert_eq!(version.promulgated_at, Some(SimTime::from_minutes(1)));
    let retrospective_result = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::EPOCH,
                read_at: SimTime::from_minutes(2),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("query retrospective effect at historical event time");
    assert_eq!(
        retrospective_result.outcome,
        ApplicabilityOutcome::Applicable
    );
    runtime.validate_against_plan(&plan).expect("valid runtime");
}

fn effectiveness_publicity_plan() -> CompiledLawPlan {
    let mut authored = definition();
    authored.source_profiles[0].publicity_policy = PublicityPolicy::EffectivenessCondition;
    compile_law(&authored).expect("compile effectiveness plan")
}

fn adopt_effectiveness_bill(runtime: &mut LegalRuntime, plan: &CompiledLawPlan, effective_at: i64) {
    let mut bill = proposal();
    bill.effective_at = SimTime::from_minutes(effective_at);
    runtime.submit_proposal(plan, bill).expect("submit bill");
    stage_required_contexts(runtime, plan);
    let opened = runtime
        .settle_boundary(plan, SimTime::from_minutes(1), &[])
        .expect("open bill");
    for item in &opened.emitted_outbox {
        mark_test_outbox(runtime, item.sequence, IngressId::new(item.sequence));
        runtime
            .queue_pending_intent(plan, intent_from_outbox(item, "for"))
            .expect("queue vote");
    }
    runtime
        .settle_boundary(plan, SimTime::from_minutes(2), &[])
        .expect("adopt bill");
}

fn effectiveness_publicity_event(id: &str, at: i64) -> LegalPublicityEvent {
    LegalPublicityEvent {
        id: id.to_owned(),
        proposal: LegalRecordRef {
            kind: "proposal".to_owned(),
            id: "suffrage".to_owned(),
        },
        at: SimTime::from_minutes(at),
        signal_kind: "law.publication".to_owned(),
        medium: "official-gazette".to_owned(),
        scope: vec!["assembly-forum".to_owned()],
        evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
    }
}

fn effectiveness_query_outcome(
    runtime: &LegalRuntime,
    plan: &CompiledLawPlan,
    event_at: i64,
    read_at: i64,
) -> ApplicabilityOutcome {
    runtime
        .query_applicability_for_plan(
            plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(event_at),
                read_at: SimTime::from_minutes(read_at),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("query effectiveness lifecycle")
        .outcome
}

#[test]
fn effectiveness_publicity_can_follow_adoption_but_precedes_effective_time() {
    let plan = effectiveness_publicity_plan();
    let mut runtime = LegalRuntime::new(&plan);
    adopt_effectiveness_bill(&mut runtime, &plan, 5);
    let adopted = &runtime.proposals["suffrage"];
    assert_eq!(adopted.status, ProposalStatus::Adopted);
    assert_eq!(adopted.adopted_at, Some(SimTime::from_minutes(2)));
    assert!(adopted.publicity.is_none());
    assert!(runtime.rules["rule:suffrage"].operative_version.is_none());
    assert!(
        runtime
            .law_versions
            .values()
            .next()
            .unwrap()
            .promulgated_at
            .is_none()
    );
    assert_eq!(
        effectiveness_query_outcome(&runtime, &plan, 2, 2),
        ApplicabilityOutcome::NotApplicable
    );
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(3), &[])
        .expect("reach publication boundary");
    runtime
        .record_publicity(
            &plan,
            effectiveness_publicity_event("publicity:suffrage:effectiveness", 3),
        )
        .expect("publish adopted bill");
    assert_eq!(runtime.rules["rule:suffrage"].scheduled_versions.len(), 1);
    assert_eq!(
        effectiveness_query_outcome(&runtime, &plan, 2, 2),
        ApplicabilityOutcome::NotApplicable
    );
    assert!(
        runtime
            .sources
            .values()
            .next()
            .unwrap()
            .promulgated_at
            .is_none()
    );
    assert!(
        runtime
            .law_versions
            .values()
            .next()
            .unwrap()
            .promulgated_at
            .is_none()
    );
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(5), &[])
        .expect("activate published bill");
    assert!(runtime.rules["rule:suffrage"].operative_version.is_some());
    assert_eq!(
        effectiveness_query_outcome(&runtime, &plan, 5, 5),
        ApplicabilityOutcome::Applicable
    );
    runtime
        .validate_against_plan(&plan)
        .expect("valid lifecycle");
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(6), &[])
        .expect("advance for cold tamper fixture");
    let mut late_publicity = runtime.clone();
    late_publicity
        .publicity_events
        .get_mut("publicity:suffrage:effectiveness")
        .unwrap()
        .at = SimTime::from_minutes(6);
    late_publicity
        .reaccount_state_budget()
        .expect("reaccount tampered publicity");
    assert!(late_publicity.validate_against_plan(&plan).is_err());

    let mut immediate = LegalRuntime::new(&plan);
    adopt_effectiveness_bill(&mut immediate, &plan, 2);
    immediate
        .record_publicity(
            &plan,
            effectiveness_publicity_event("publicity:suffrage:immediate", 2),
        )
        .expect("publish at effective time");
    assert!(immediate.rules["rule:suffrage"].operative_version.is_some());
    immediate
        .validate_against_plan(&plan)
        .expect("immediate publicity keeps exact evidence topology");
}

#[test]
#[allow(clippy::too_many_lines)]
fn conflict_resolution_uses_the_recorded_exact_version_partition() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    for (id, rule, minute) in [
        ("conflict-a", "rule:conflict-a", 1_u64),
        ("conflict-b", "rule:conflict-b", 2_u64),
    ] {
        let mut claim = proposal_change(
            id,
            LawOperation::Recognize,
            i64::try_from(minute).expect("fixture minute"),
        );
        claim.source_profile = "custom".to_owned();
        claim.procedure_profile.clear();
        claim.rule_id = rule.to_owned();
        if id == "conflict-a" {
            claim.clauses[0].conditions.clear();
            claim.clauses[0].exceptions.clear();
        }
        claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(minute))];
        runtime
            .admit_non_procedural_source(
                &plan,
                claim,
                &["practice.recognized".to_owned()],
                SimTime::from_minutes(i64::try_from(minute).expect("fixture minute")),
            )
            .expect("admit conflicting source");
    }
    let earlier = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:conflict-a:1".to_owned(),
    };
    let later = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:conflict-b:1".to_owned(),
    };
    runtime
        .record_conflict(
            &plan,
            LegalConflict {
                id: "conflict:temporal".to_owned(),
                versions: vec![earlier.clone(), later.clone()],
                governing_versions: vec![later.clone()],
                displaced_versions: vec![earlier.clone()],
                jurisdiction: Some("assembly-forum".to_owned()),
                recorded_at: SimTime::from_minutes(2),
                effective_from: SimTime::from_minutes(3),
                effective_until: None,
                resolution: ApplicabilityOutcome::Displaced,
                basis: ConflictResolutionBasis::Temporal,
                rationale: "later effective source governs under compiled precedence".to_owned(),
                ruling: None,
                trace: vec![earlier.clone(), later.clone()],
            },
        )
        .expect("record exact conflict partition");
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(3), &[])
        .expect("advance through conflict effective time");
    let before_effective = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(2),
                read_at: SimTime::from_minutes(3),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("query before conflict effective time");
    assert_eq!(
        before_effective.versions,
        vec![earlier.clone(), later.clone()]
    );
    assert!(before_effective.conflicts.is_empty());

    let mut incomplete_facts = applicable_facts();
    incomplete_facts.remove("citizen");
    let mut incomplete_evidence = applicable_fact_evidence();
    incomplete_evidence.remove("citizen");
    let mixed_scope = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(2),
                read_at: SimTime::from_minutes(3),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: incomplete_facts,
                fact_evidence: incomplete_evidence,
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("resolve mixed known and unknown scope");
    assert_eq!(mixed_scope.outcome, ApplicabilityOutcome::Indeterminate);
    let result = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(3),
                read_at: SimTime::from_minutes(3),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("resolve exact conflict partition");
    assert_eq!(result.versions, vec![later]);
    assert_eq!(result.displaced, vec![earlier]);
    assert_eq!(result.conflicts, vec!["conflict:temporal"]);
    runtime.validate_against_plan(&plan).expect("valid runtime");
    let mut wrong_key = runtime.clone();
    let conflict = wrong_key
        .conflicts
        .remove("conflict:temporal")
        .expect("conflict fixture");
    wrong_key
        .conflicts
        .insert("conflict:forged".to_owned(), conflict);
    assert!(wrong_key.validate_against_plan(&plan).is_err());
    let mut inverted_time = runtime.clone();
    inverted_time
        .conflicts
        .get_mut("conflict:temporal")
        .expect("conflict fixture")
        .effective_until = Some(SimTime::from_minutes(2));
    assert!(inverted_time.validate_against_plan(&plan).is_err());
}

#[test]
fn amendment_and_repeal_keep_history_and_apply_only_at_future_effective_times() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);

    let mut establish = proposal_change("base-law", LawOperation::Establish, 2);
    establish.rule_id = "rule:future-law".to_owned();
    runtime
        .submit_proposal(&plan, establish)
        .expect("submit base law");
    stage_required_contexts(&mut runtime, &plan);
    let base_outbox = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("open base vote");
    submit_two_votes(&mut runtime, &plan, &base_outbox.emitted_outbox);
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("adopt base law");

    let mut amend = proposal_change("future-amendment", LawOperation::Amend, 20);
    amend.rule_id = "rule:future-law".to_owned();
    amend.expected_rule_head = Some(LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:future-law:1".to_owned(),
    });
    runtime
        .submit_proposal(&plan, amend)
        .expect("submit amendment");
    stage_required_contexts(&mut runtime, &plan);
    let amend_outbox = runtime
        .settle_boundary(&plan, SimTime::from_minutes(10), &[])
        .expect("open amendment vote");
    submit_two_votes(&mut runtime, &plan, &amend_outbox.emitted_outbox);
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(11), &[])
        .expect("adopt future amendment");
    assert_eq!(runtime.law_versions.len(), 2);
    assert_eq!(
        runtime.law_versions["law-version:rule:future-law:1"].operation,
        LawOperation::Establish
    );
    assert_eq!(
        runtime.rules["rule:future-law"]
            .operative_version
            .as_ref()
            .expect("base law operative")
            .id,
        "law-version:rule:future-law:1"
    );
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(20), &[])
        .expect("activate amendment");
    assert_eq!(
        runtime.rules["rule:future-law"]
            .operative_version
            .as_ref()
            .expect("amendment operative")
            .id,
        "law-version:rule:future-law:2"
    );

    let mut repeal = proposal_change("future-repeal", LawOperation::Repeal, 30);
    repeal.rule_id = "rule:future-law".to_owned();
    repeal.expected_rule_head = Some(LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:future-law:2".to_owned(),
    });
    runtime
        .submit_proposal(&plan, repeal)
        .expect("submit repeal");
    stage_required_contexts(&mut runtime, &plan);
    let repeal_outbox = runtime
        .settle_boundary(&plan, SimTime::from_minutes(21), &[])
        .expect("open repeal vote");
    submit_two_votes(&mut runtime, &plan, &repeal_outbox.emitted_outbox);
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(22), &[])
        .expect("adopt future repeal");
    assert!(!runtime.rules["rule:future-law"].retired);
    assert!(!runtime.applicability.is_empty());
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(30), &[])
        .expect("activate repeal");
    assert_eq!(runtime.law_versions.len(), 3);
    assert!(runtime.rules["rule:future-law"].retired);
    assert!(runtime.applicability.is_empty());
    assert!(runtime.applicability_by_rule.is_empty());
    runtime.validate_against_plan(&plan).expect("valid history");
    let mut tampered = runtime.clone();
    tampered
        .law_versions
        .get_mut("law-version:rule:future-law:2")
        .expect("amendment version")
        .predecessors
        .clear();
    assert!(tampered.validate_against_plan(&plan).is_err());
}

#[test]
fn same_time_versions_apply_by_ordinal_then_record_id_across_rules() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let practice = ["practice.recognized".to_owned()];

    let mut base = proposal_change("base-a", LawOperation::Establish, 1);
    base.rule_id = "rule:a".to_owned();
    base.source_profile = "custom".to_owned();
    base.procedure_profile.clear();
    base.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(&plan, base, &practice, SimTime::from_minutes(1))
        .expect("admit base rule");

    let mut amendment = proposal_change("future-amend-a", LawOperation::Amend, 100);
    amendment.rule_id = "rule:a".to_owned();
    amendment.expected_rule_head = Some(LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:a:1".to_owned(),
    });
    amendment.source_profile = "custom".to_owned();
    amendment.procedure_profile.clear();
    amendment.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(2))];
    runtime
        .admit_non_procedural_source(&plan, amendment, &practice, SimTime::from_minutes(2))
        .expect("schedule ordinal two");

    let mut establish = proposal_change("future-base-z", LawOperation::Establish, 100);
    establish.rule_id = "rule:z".to_owned();
    establish.source_profile = "custom".to_owned();
    establish.procedure_profile.clear();
    establish.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(3))];
    runtime
        .admit_non_procedural_source(&plan, establish, &practice, SimTime::from_minutes(3))
        .expect("schedule ordinal one");

    let boundary = runtime
        .settle_boundary(&plan, SimTime::from_minutes(100), &[])
        .expect("apply same-time versions");
    assert_eq!(
        boundary.applied_versions,
        vec![
            "law-version:rule:z:1".to_owned(),
            "law-version:rule:a:2".to_owned(),
        ]
    );
}

#[test]
fn rule_changes_require_exact_head_order_and_legal_state_transition() {
    let mut authored = definition();
    authored.orders.push(LegalOrderDefinition {
        id: "other-code".to_owned(),
        precedence_profile: "constitutional".to_owned(),
    });
    let plan = compile_law(&authored).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let practice = ["practice.recognized".to_owned()];

    let mut base = proposal_change("state-base", LawOperation::Establish, 1);
    base.rule_id = "rule:state-machine".to_owned();
    base.source_profile = "custom".to_owned();
    base.procedure_profile.clear();
    base.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(&plan, base, &practice, SimTime::from_minutes(1))
        .expect("establish rule");

    let base_head = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:state-machine:1".to_owned(),
    };
    let mut retrograde = proposal_change("retrograde", LawOperation::Amend, 0);
    retrograde.rule_id = "rule:state-machine".to_owned();
    retrograde.expected_rule_head = Some(base_head.clone());
    retrograde.source_profile = "custom".to_owned();
    retrograde.procedure_profile.clear();
    retrograde.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(2))];
    assert!(
        runtime
            .admit_non_procedural_source(&plan, retrograde, &practice, SimTime::from_minutes(2),)
            .is_err()
    );
    let mut wrong_order = proposal_change("wrong-order", LawOperation::Amend, 2);
    wrong_order.rule_id = "rule:state-machine".to_owned();
    wrong_order.legal_order = "other-code".to_owned();
    wrong_order.expected_rule_head = Some(base_head.clone());
    wrong_order.source_profile = "custom".to_owned();
    wrong_order.procedure_profile.clear();
    wrong_order.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(2))];
    assert!(
        runtime
            .admit_non_procedural_source(&plan, wrong_order, &practice, SimTime::from_minutes(2),)
            .is_err()
    );

    let mut suspend = proposal_change("suspend", LawOperation::Suspend, 2);
    suspend.rule_id = "rule:state-machine".to_owned();
    suspend.expected_rule_head = Some(base_head.clone());
    suspend.source_profile = "custom".to_owned();
    suspend.procedure_profile.clear();
    suspend.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(2))];
    runtime
        .admit_non_procedural_source(&plan, suspend, &practice, SimTime::from_minutes(2))
        .expect("suspend operative rule");

    let suspended_head = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:state-machine:2".to_owned(),
    };
    let mut stale = proposal_change("stale", LawOperation::Resume, 3);
    stale.rule_id = "rule:state-machine".to_owned();
    stale.expected_rule_head = Some(base_head);
    stale.source_profile = "custom".to_owned();
    stale.procedure_profile.clear();
    stale.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(3))];
    assert!(
        runtime
            .admit_non_procedural_source(&plan, stale, &practice, SimTime::from_minutes(3))
            .is_err()
    );

    let mut amend_suspended = proposal_change("amend-suspended", LawOperation::Amend, 3);
    amend_suspended.rule_id = "rule:state-machine".to_owned();
    amend_suspended.expected_rule_head = Some(suspended_head.clone());
    amend_suspended.source_profile = "custom".to_owned();
    amend_suspended.procedure_profile.clear();
    amend_suspended.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(3))];
    assert!(
        runtime
            .admit_non_procedural_source(
                &plan,
                amend_suspended,
                &practice,
                SimTime::from_minutes(3),
            )
            .is_err()
    );

    let mut resume = proposal_change("resume", LawOperation::Resume, 3);
    resume.rule_id = "rule:state-machine".to_owned();
    resume.expected_rule_head = Some(suspended_head);
    resume.source_profile = "custom".to_owned();
    resume.procedure_profile.clear();
    resume.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(3))];
    runtime
        .admit_non_procedural_source(&plan, resume, &practice, SimTime::from_minutes(3))
        .expect("resume suspended rule");
    assert_eq!(
        runtime.rules["rule:state-machine"]
            .latest_adopted_version
            .as_ref()
            .expect("resumed head")
            .id,
        "law-version:rule:state-machine:3"
    );
}

#[test]
fn boundary_state_budget_fails_before_any_hot_path_mutation() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    stage_required_contexts(&mut runtime, &plan);
    runtime.reserved_state_bytes = plan.budgets.max_state_bytes - 1;
    let boundary = runtime.boundary_index;
    let next_sequence = runtime.next_outbox_sequence;
    assert!(
        runtime
            .settle_boundary(&plan, SimTime::from_minutes(1), &[])
            .is_err()
    );
    assert_eq!(runtime.boundary_index, boundary);
    assert_eq!(runtime.next_outbox_sequence, next_sequence);
    assert!(runtime.outbox.is_empty());
}

#[test]
#[allow(clippy::too_many_lines)]
fn host_adapter_enqueues_typed_ticket_before_marking_outbox() {
    let mut authored = definition();
    let persons = [PersonId::new(1), PersonId::new(2)];
    for (seat, person) in authored.institutions[0].seats.iter_mut().zip(persons) {
        seat.holder = Some(KnowledgeHolderRef::Person(person));
    }
    let plan = compile_law(&authored).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    let mut second_proposal = proposal();
    second_proposal.id = "suffrage-second-reading".to_owned();
    second_proposal.rule_id = "rule:suffrage-second-reading".to_owned();
    runtime
        .submit_proposal(&plan, second_proposal)
        .expect("submit second proposal through the same seats");
    stage_required_contexts(&mut runtime, &plan);
    let boundary = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("materialize outbox");
    assert_eq!(boundary.emitted_outbox[0].draft.options.len(), 3);
    let initial = runtime.to_record_draft().expect("encode runtime record");
    let mut canwu = canwu_api::Canwu::new_with_plugins(
        7,
        Scenario::new(
            SimTime::from_minutes(1),
            persons.into_iter().map(EntityRef::Person).collect(),
        )
        .with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .expect("law plugin registration");
    let persisted_drafts = runtime
        .outbox
        .values()
        .map(|item| item.draft.clone())
        .collect::<Vec<_>>();
    let preparation_receipts = runtime
        .prepare_pending_decision_enqueues(&mut canwu)
        .expect("queue durable enqueue preparation");
    assert_eq!(preparation_receipts.len(), 4);
    canwu
        .step_canonical()
        .expect("persist enqueue revision")
        .expect("preparation boundary");
    let prepared = load_legal_runtime(&canwu, &plan)
        .expect("reload prepared runtime")
        .expect("runtime record");
    assert!(
        prepared
            .outbox
            .values()
            .all(|item| item.enqueue_expected_revision == Some(canwu.revision()))
    );

    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("unrelated boundary after prepare");
    assert!(prepared.enqueue_pending_decisions(&mut canwu).is_err());
    assert_eq!(
        prepared
            .prepare_pending_decision_enqueues(&mut canwu)
            .expect("queue safe reprepare")
            .len(),
        4
    );
    canwu
        .step_canonical()
        .expect("persist refreshed enqueue revision")
        .expect("reprepare boundary");
    let prepared = load_legal_runtime(&canwu, &plan)
        .expect("reload reprepared runtime")
        .expect("runtime record");

    let crashed_runtime = prepared.clone();
    let receipts = crashed_runtime
        .enqueue_pending_decisions(&mut canwu)
        .expect("enqueue legal decisions");
    assert_eq!(receipts.len(), 4);
    drop(crashed_runtime);

    let reloaded = load_legal_runtime(&canwu, &plan)
        .expect("reload pending runtime after simulated crash")
        .expect("runtime record");
    assert!(reloaded.pending_outbox().next().is_some());
    assert_eq!(
        reloaded
            .outbox
            .values()
            .map(|item| item.draft.clone())
            .collect::<Vec<_>>(),
        persisted_drafts
    );
    let retry_receipts = reloaded
        .enqueue_pending_decisions(&mut canwu)
        .expect("retry exact persisted drafts");
    assert_eq!(retry_receipts, receipts);

    canwu
        .step_canonical()
        .expect("settle decision ingress")
        .expect("decision boundary");
    assert!(
        canwu
            .decision_ticket(DecisionTicketId::new(boundary.emitted_outbox[0].ticket_id))
            .is_some(),
        "{:#?}",
        canwu.decision_attempts()
    );
    for item in &boundary.emitted_outbox {
        assert_eq!(
            canwu
                .decision_ticket(DecisionTicketId::new(item.ticket_id))
                .expect("settled ticket")
                .options,
            item.draft.options,
            "ticket option normalization drifted for {}",
            item.sequence
        );
    }
    let awaiting_ack = load_legal_runtime(&canwu, &plan)
        .expect("load persisted legal runtime")
        .expect("runtime record");
    let reused_item = awaiting_ack
        .pending_outbox()
        .find(|item| {
            item.refresh_request_id.is_some_and(|request_id| {
                canwu
                    .decision_state()
                    .attempt(DecisionRequestId::new(request_id))
                    .is_none()
            })
        })
        .expect("one shared-controller outbox item without a refresh attempt")
        .clone();
    let mut attacked = canwu.fork();
    let conflicting = DecisionControllerBinding::new(
        reused_item.decision_controller_id.clone(),
        DecisionPolicyIdentity::new(DecisionPolicyKind::Rule, "conflict", "1"),
        DecisionAuthority::Actor {
            actor: PersonId::new(1),
        },
    );
    let attacked_revision = attacked.revision();
    attacked
        .enqueue_decision(
            attacked.time(),
            reused_item.priority,
            DecisionIngressRequest::new(
                DecisionRequestId::new(
                    reused_item
                        .refresh_request_id
                        .expect("persisted refresh request"),
                ),
                attacked_revision,
                DecisionMutation::RegisterController {
                    controller: conflicting,
                },
            ),
        )
        .expect("enqueue rejected controller attempt");
    attacked
        .step_canonical()
        .expect("settle rejected controller attempt")
        .expect("decision boundary");
    assert!(
        awaiting_ack
            .acknowledge_enqueued_decisions(&mut attacked)
            .is_err(),
        "a rejected persisted refresh attempt must not be hidden as controller reuse",
    );
    let first_acknowledgements = awaiting_ack
        .acknowledge_enqueued_decisions(&mut canwu)
        .expect("acknowledge committed decisions");
    assert_eq!(first_acknowledgements.len(), 4);
    let duplicate_acknowledgements = awaiting_ack
        .acknowledge_enqueued_decisions(&mut canwu)
        .expect("repeat acknowledgements before settlement");
    assert_eq!(duplicate_acknowledgements.len(), 4);
    assert_ne!(first_acknowledgements, duplicate_acknowledgements);
    canwu
        .step_canonical()
        .expect("settle legal acknowledgements")
        .expect("acknowledgement boundary");
    let persisted = load_legal_runtime(&canwu, &plan)
        .expect("load acknowledged legal runtime")
        .expect("runtime record");
    assert!(persisted.pending_outbox().next().is_none());
    assert!(
        persisted
            .outbox
            .values()
            .all(|item| item.dispatch == DispatchState::Enqueued)
    );
    assert!(
        persisted
            .outbox
            .values()
            .zip(&first_acknowledgements)
            .all(|(item, receipt)| item.enqueue_ingress
                == Some(EvidenceRef::Ingress(receipt.ingress_id)))
    );
    assert!(
        persisted
            .enqueue_pending_decisions(&mut canwu)
            .expect("idempotent empty adapter pass")
            .is_empty()
    );
}

#[test]
fn veto_option_is_exposed_only_by_a_veto_stage() {
    let mut authored = definition();
    authored.procedures[0].stages[0].kind = ProcedureStageKind::Veto;
    authored.procedures[0].stages[0]
        .allowed_ballots
        .push(Ballot::Veto);
    let plan = compile_law(&authored).expect("compile veto procedure");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit veto-stage proposal");
    stage_required_contexts(&mut runtime, &plan);
    let boundary = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("materialize veto outbox");
    assert!(
        boundary
            .emitted_outbox
            .iter()
            .all(|item| { item.draft.options.iter().any(|option| option.id == "veto") })
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn outbox_acknowledgement_binds_reserved_ids_to_exact_decision_mutations() {
    let mut authored = definition();
    authored.institutions[0].seats.truncate(1);
    authored.institutions[0].seats[0].holder = Some(KnowledgeHolderRef::Person(PersonId::new(1)));
    authored.procedures[0].stages[0].seats.truncate(1);
    authored.procedures[0].stages[0].quorum = 1;
    let plan = compile_law(&authored).expect("compile one-seat plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    stage_required_contexts(&mut runtime, &plan);
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("materialize one outbox item");
    let initial = runtime.to_record_draft().expect("encode runtime");
    let mut canwu = Canwu::new_with_plugins(
        7,
        Scenario::new(
            SimTime::from_minutes(1),
            vec![EntityRef::Person(PersonId::new(1))],
        )
        .with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .expect("law host");
    runtime
        .prepare_pending_decision_enqueues(&mut canwu)
        .expect("prepare enqueue");
    canwu
        .step_canonical()
        .expect("settle preparation")
        .expect("preparation boundary");
    let prepared = load_legal_runtime(&canwu, &plan)
        .expect("load prepared runtime")
        .expect("runtime");
    let item = prepared
        .pending_outbox()
        .next()
        .expect("one pending item")
        .clone();
    let expected_revision = item.enqueue_expected_revision.expect("prepared revision");
    let expected_controller = DecisionControllerBinding::new(
        item.decision_controller_id.clone(),
        DecisionPolicyIdentity::new(DecisionPolicyKind::Human, "canwu-law-human-seat", "1"),
        DecisionAuthority::Actor {
            actor: PersonId::new(1),
        },
    )
    .with_seat(&item.seat, &item.permission_profile_id);

    canwu
        .enqueue_decision(
            item.due_at,
            item.priority,
            DecisionIngressRequest::new(
                DecisionRequestId::new(900_001),
                expected_revision,
                DecisionMutation::RegisterController {
                    controller: expected_controller,
                },
            ),
        )
        .expect("enqueue expected controller under wrong request ID");
    canwu
        .enqueue_decision(
            item.due_at,
            item.priority,
            DecisionIngressRequest::new(
                DecisionRequestId::new(900_002),
                expected_revision,
                DecisionMutation::Open {
                    ticket: item.draft.clone(),
                },
            ),
        )
        .expect("enqueue expected ticket under wrong request ID");
    for (request_id, controller_id) in [
        (
            item.refresh_request_id.expect("reserved refresh request"),
            "unrelated-refresh-controller",
        ),
        (item.create_request_id, "unrelated-create-controller"),
    ] {
        canwu
            .enqueue_decision(
                item.due_at,
                item.priority,
                DecisionIngressRequest::new(
                    DecisionRequestId::new(request_id),
                    expected_revision,
                    DecisionMutation::RegisterController {
                        controller: DecisionControllerBinding::new(
                            controller_id,
                            DecisionPolicyIdentity::new(DecisionPolicyKind::Rule, "wrong", "1"),
                            DecisionAuthority::Actor {
                                actor: PersonId::new(1),
                            },
                        ),
                    },
                ),
            )
            .expect("enqueue accepted wrong mutation under reserved ID");
    }
    canwu
        .step_canonical()
        .expect("settle colliding decision requests")
        .expect("decision boundary");
    assert!(
        canwu
            .decision_ticket(DecisionTicketId::new(item.ticket_id))
            .is_some()
    );
    assert!(
        prepared.acknowledge_enqueued_decisions(&mut canwu).is_err(),
        "matching final state cannot substitute for exact reserved request mutations",
    );
}

#[test]
fn live_actor_context_is_derived_and_persisted_by_the_kernel() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    let initial = runtime.to_record_draft().expect("encode runtime record");
    let mut canwu = canwu_api::Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .expect("law plugin host");
    let requirements = runtime
        .pending_actor_context_requirements(&plan)
        .expect("actor requirements");
    for requirement in &requirements {
        runtime
            .enqueue_actor_context(
                &plan,
                &mut canwu,
                requirement,
                &canwu_api::KnowledgeQuery::default(),
            )
            .expect("queue holder knowledge context");
    }
    canwu
        .step_canonical()
        .expect("settle actor contexts")
        .expect("actor context boundary");
    let persisted = load_legal_runtime(&canwu, &plan)
        .expect("load legal runtime")
        .expect("persisted legal runtime");

    assert!(persisted.staged_actor_contexts.is_empty());
    assert_eq!(persisted.outbox.len(), requirements.len());
    assert!(persisted.outbox.values().all(|item| {
        item.draft.context.payload["facts"] == serde_json::json!([])
            && item.knowledge_record_ids.is_empty()
            && !item.knowledge_read_cut.holder_projection_root.is_empty()
    }));
}

#[test]
fn live_proposal_mutation_is_settled_inside_kernel_record_cas() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let runtime = LegalRuntime::new(&plan);
    let initial = runtime.to_record_draft().expect("encode runtime record");
    let mut canwu = canwu_api::Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .expect("law plugin host");
    enqueue_legal_mutation(
        &mut canwu,
        &LegalMutation::SubmitProposal {
            proposal: proposal(),
        },
    )
    .expect("queue legal proposal");
    canwu
        .step_canonical()
        .expect("settle proposal ingress")
        .expect("proposal boundary");

    let persisted = load_legal_runtime(&canwu, &plan)
        .expect("load legal runtime")
        .expect("runtime record");
    assert_eq!(persisted.boundary_index, 1);
    assert!(persisted.proposals.contains_key("suffrage"));
    assert!(persisted.open_procedures.contains("procedure:suffrage"));
    assert_eq!(
        canwu
            .typed_domain_record(&legal_runtime_reference())
            .expect("legal aggregate")
            .version,
        2
    );
}

#[test]
fn non_procedural_custom_requires_admitted_practice_without_a_ticket() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("recognized-custom", LawOperation::Recognize, 5);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:recognized-custom".to_owned();
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];

    assert!(
        runtime
            .admit_non_procedural_source(&plan, claim.clone(), &[], SimTime::from_minutes(5))
            .is_err()
    );
    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(5),
        )
        .expect("admit evidence-backed custom");

    assert!(runtime.outbox.is_empty());
    assert!(runtime.procedures.is_empty());
    assert_eq!(runtime.sources.len(), 1);
    let source = runtime.sources.values().next().expect("custom source");
    assert_eq!(source.mode, SourceMode::Accreted);
    assert!(source.procedure.is_none());
    assert_eq!(source.evidence_kinds, vec!["practice.recognized"]);
    assert!(source.promulgated_at.is_none());
    runtime.validate_against_plan(&plan).expect("valid custom");
}

#[test]
fn authorized_but_ultra_vires_source_is_purported_not_operative() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("ultra-vires-decree", LawOperation::Recognize, 1);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:ultra-vires-decree".to_owned();
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    claim.competence = LegalCompetenceDisposition::Purported;
    claim.defects = vec!["issuer_lacked_subject_matter_competence".to_owned()];
    claim.validity = OperativeDisposition::Purported;

    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("record authorized but legally purported act");
    let source = runtime.sources.values().next().expect("purported source");
    assert_eq!(source.competence, LegalCompetenceDisposition::Purported);
    assert_eq!(source.validity, OperativeDisposition::Purported);
    assert_eq!(source.defects.len(), 1);
    let version = runtime
        .law_versions
        .values()
        .next()
        .expect("purported law version");
    assert_eq!(version.disposition, OperativeDisposition::Purported);
    assert_eq!(version.deltas[0].modality, NormativeModality::Eligibility);
    assert!(
        runtime.rules["rule:ultra-vires-decree"]
            .operative_version
            .is_none()
    );

    let result = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(1),
                read_at: SimTime::from_minutes(1),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: None,
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("resolve disputed validity");
    assert_eq!(result.outcome, ApplicabilityOutcome::Contested);
    assert!(result.versions.is_empty());
    assert_eq!(result.displaced.len(), 1);
    runtime
        .validate_against_plan(&plan)
        .expect("cold validation preserves competence and validity split");
}

#[test]
#[allow(clippy::too_many_lines)]
fn contested_amendment_preserves_the_prior_operative_rule_in_resolution() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut base = proposal_change("valid-base", LawOperation::Recognize, 1);
    base.source_profile = "custom".to_owned();
    base.procedure_profile.clear();
    base.rule_id = "rule:contested-amendment".to_owned();
    base.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(
            &plan,
            base,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit operative base rule");

    let mut amendment = proposal_change("contested-change", LawOperation::Amend, 2);
    amendment.source_profile = "custom".to_owned();
    amendment.procedure_profile.clear();
    amendment.rule_id = "rule:contested-amendment".to_owned();
    amendment.expected_rule_head = Some(LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:contested-amendment:1".to_owned(),
    });
    amendment.competence = LegalCompetenceDisposition::Contested;
    amendment.defects = vec!["constitutional_review_pending".to_owned()];
    amendment.validity = OperativeDisposition::Contested;
    amendment.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(2))];
    runtime
        .admit_non_procedural_source(
            &plan,
            amendment,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(2),
        )
        .expect("record contested amendment");

    assert_eq!(
        runtime.rules["rule:contested-amendment"]
            .operative_version
            .as_ref()
            .expect("base remains operative")
            .id,
        "law-version:rule:contested-amendment:1"
    );
    let result = runtime
        .query_applicability_for_plan(
            &plan,
            &ApplicabilityQuery {
                event_at: SimTime::from_minutes(2),
                read_at: SimTime::from_minutes(2),
                subject: None,
                actor: None,
                knowledge_read_cut: None,
                territory: None,
                subject_matter: Some("voting".to_owned()),
                legal_order: "state-code".to_owned(),
                profile: "state-choice".to_owned(),
                jurisdiction: Some("assembly-forum".to_owned()),
                facts: applicable_facts(),
                fact_evidence: applicable_fact_evidence(),
                fact_knowledge_records: BTreeMap::new(),
            },
        )
        .expect("resolve operative rule and rival amendment");
    assert_eq!(result.outcome, ApplicabilityOutcome::Contested);
    assert_eq!(
        result.versions[0].id,
        "law-version:rule:contested-amendment:1"
    );
    assert_eq!(
        result.displaced[0].id,
        "law-version:rule:contested-amendment:2"
    );
    assert_eq!(
        runtime
            .retained_evidence_dependency_counts
            .get(&EvidenceRef::Boundary(BoundaryId::new(2))),
        Some(&1),
        "a live contested claim must retain its identity evidence"
    );
    runtime
        .validate_against_plan(&plan)
        .expect("cold validation preserves rival validity positions");

    let operative = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:contested-amendment:1".to_owned(),
    };
    let contested = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:contested-amendment:2".to_owned(),
    };
    let mut rejecting_court = runtime.clone();
    let rejected =
        resolve_contested_amendment(&mut rejecting_court, &plan, "reject", operative.clone());
    assert_eq!(rejected.outcome, ApplicabilityOutcome::Applicable);
    assert_eq!(rejected.versions, vec![operative.clone()]);
    assert_eq!(rejected.displaced, vec![contested.clone()]);
    rejecting_court
        .validate_against_plan(&plan)
        .expect("cold validation preserves rejected claim resolution");

    let mut selecting_court = runtime.clone();
    let selected =
        resolve_contested_amendment(&mut selecting_court, &plan, "select", contested.clone());
    assert_eq!(selected.outcome, ApplicabilityOutcome::Applicable);
    assert_eq!(selected.versions, vec![contested.clone()]);
    assert_eq!(selected.displaced, vec![operative.clone()]);
    selecting_court
        .validate_against_plan(&plan)
        .expect("cold validation preserves selected claim resolution");

    let mut contradictory_courts = runtime.clone();
    let _ = resolve_contested_amendment(
        &mut contradictory_courts,
        &plan,
        "overlap-reject",
        operative.clone(),
    );
    let contradictory = resolve_contested_amendment(
        &mut contradictory_courts,
        &plan,
        "overlap-select",
        contested.clone(),
    );
    assert_eq!(contradictory.outcome, ApplicabilityOutcome::Contested);
    contradictory_courts
        .validate_against_plan(&plan)
        .expect("overlapping rulings remain an explicit contested state");

    let existing_conflict = rejecting_court
        .conflicts
        .values()
        .next()
        .expect("ruling-backed conflict")
        .clone();
    let existing_ruling = rejecting_court
        .rulings
        .values()
        .next()
        .expect("conflict ruling")
        .clone();
    let mut cold_nonoperative = rejecting_court.clone();
    cold_nonoperative
        .rulings
        .get_mut(&existing_ruling.id)
        .expect("cold conflict ruling")
        .disposition = OperativeDisposition::Claimed;
    assert!(cold_nonoperative.validate_against_plan(&plan).is_err());
    let mut cold_globalized = rejecting_court.clone();
    cold_globalized
        .conflicts
        .get_mut(&existing_conflict.id)
        .expect("cold ruling-backed conflict")
        .jurisdiction = None;
    assert!(cold_globalized.validate_against_plan(&plan).is_err());
    let mut nonoperative_ruling = existing_ruling.clone();
    nonoperative_ruling.id = "ruling:nonoperative".to_owned();
    nonoperative_ruling.disposition = OperativeDisposition::Claimed;
    rejecting_court
        .record_ruling(&plan, nonoperative_ruling)
        .expect("record non-operative ruling as a claim");
    let mut nonoperative_conflict = existing_conflict.clone();
    nonoperative_conflict.id = "conflict:nonoperative".to_owned();
    nonoperative_conflict.ruling = Some(LegalRecordRef {
        kind: "ruling".to_owned(),
        id: "ruling:nonoperative".to_owned(),
    });
    assert!(
        rejecting_court
            .record_conflict(&plan, nonoperative_conflict)
            .is_err()
    );
    let mut globalized_conflict = existing_conflict;
    globalized_conflict.id = "conflict:globalized".to_owned();
    globalized_conflict.jurisdiction = None;
    assert!(
        rejecting_court
            .record_conflict(&plan, globalized_conflict)
            .is_err()
    );

    let mut replacement = proposal_change("validated-change", LawOperation::Amend, 3);
    replacement.source_profile = "custom".to_owned();
    replacement.procedure_profile.clear();
    replacement.rule_id = "rule:contested-amendment".to_owned();
    replacement.expected_rule_head = Some(LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:contested-amendment:2".to_owned(),
    });
    replacement.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(3))];
    runtime
        .admit_non_procedural_source(
            &plan,
            replacement,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(3),
        )
        .expect("replace contested claim with an operative amendment");
    assert!(
        !runtime
            .retained_evidence_dependencies
            .contains(&EvidenceRef::Boundary(BoundaryId::new(1)))
    );
    assert!(
        !runtime
            .retained_evidence_dependencies
            .contains(&EvidenceRef::Boundary(BoundaryId::new(2)))
    );
    assert!(
        runtime
            .retained_evidence_dependencies
            .contains(&EvidenceRef::Boundary(BoundaryId::new(3)))
    );
    runtime
        .validate_against_plan(&plan)
        .expect("cold validation matches the replaced contested claim");
}

#[test]
#[allow(clippy::too_many_lines)]
fn succession_reception_is_rule_scoped_and_received_sources_keep_exact_origin() {
    let mut authored = definition();
    authored.orders.push(LegalOrderDefinition {
        id: "successor-code".to_owned(),
        precedence_profile: "constitutional".to_owned(),
    });
    authored
        .applicability_profiles
        .push(ApplicabilityProfileDefinition {
            id: "successor-choice".to_owned(),
            legal_order: "successor-code".to_owned(),
            temporal_conflict_rule: "later-in-time".to_owned(),
            pipeline: ["scope", "jurisdiction", "validity", "conflict"]
                .map(str::to_owned)
                .to_vec(),
            jurisdiction_traversal: Vec::new(),
            max_candidates: 64,
        });
    authored.source_profiles.push(LegalSourceProfileDefinition {
        id: "received".to_owned(),
        mode: SourceMode::Received,
        procedure: None,
        applicability_profile: "successor-choice".to_owned(),
        origin_policy: SourceOriginPolicy::Reception,
        authority_policy: SourceAuthorityPolicy::EvidenceClaim,
        publicity_policy: PublicityPolicy::NotRequired,
        publicity_signal_kind: None,
        required_signal_kinds: Vec::new(),
        min_evidence: 1,
        max_evidence: 8,
        require_claimant: true,
        allow_retroactive: false,
        agreement_namespace: None,
        agreement_kind: None,
        min_agreement_parties: 0,
        require_agreement_ratification: false,
    });
    let plan = compile_law(&authored).expect("compile succession plan");
    let mut runtime = LegalRuntime::new(&plan);

    for (id, rule, minute) in [
        ("received-base-a", "rule:received-a", 1),
        ("received-base-b", "rule:received-b", 2),
    ] {
        let mut claim = proposal_change(id, LawOperation::Recognize, minute);
        claim.source_profile = "custom".to_owned();
        claim.procedure_profile.clear();
        claim.rule_id = rule.to_owned();
        claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(
            u64::try_from(minute).expect("positive fixture minute"),
        ))];
        runtime
            .admit_non_procedural_source(
                &plan,
                claim,
                &["practice.recognized".to_owned()],
                SimTime::from_minutes(minute),
            )
            .expect("admit predecessor rule");
    }
    let territory = TerritoryId::new(9);
    runtime
        .record_succession_for_plan(
            &plan,
            LegalOrderSuccession {
                id: "succession:constitutional-replacement".to_owned(),
                kind: SuccessionKind::ConstitutionalReplacement,
                predecessors: vec!["state-code".to_owned()],
                successors: vec!["successor-code".to_owned()],
                effective_at: SimTime::from_minutes(3),
                territorial_scope: vec![territory.to_string()],
                personal_scope: Vec::new(),
                institutions: Vec::new(),
                liabilities: Vec::new(),
                archives: Vec::new(),
                reception: vec![
                    ReceptionRule {
                        rule_prefix: "rule:received-b".to_owned(),
                        action: ReceptionAction::Transform,
                        transform: Some("voting-eligibility".to_owned()),
                    },
                    ReceptionRule {
                        rule_prefix: "rule:received-a".to_owned(),
                        action: ReceptionAction::Continue,
                        transform: None,
                    },
                ],
                evidence: vec![EvidenceRef::Boundary(BoundaryId::new(3))],
            },
        )
        .expect("record bounded reception table");
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(3), &[])
        .expect("reach succession effective time");

    let query = |territory| ApplicabilityQuery {
        event_at: SimTime::from_minutes(3),
        read_at: SimTime::from_minutes(3),
        subject: None,
        actor: None,
        knowledge_read_cut: None,
        territory,
        subject_matter: None,
        legal_order: "successor-code".to_owned(),
        profile: "successor-choice".to_owned(),
        jurisdiction: Some("assembly-forum".to_owned()),
        facts: applicable_facts(),
        fact_evidence: applicable_fact_evidence(),
        fact_knowledge_records: BTreeMap::new(),
    };
    assert_eq!(
        runtime
            .query_applicability_for_plan(&plan, &query(None))
            .expect("out-of-scope succession")
            .outcome,
        ApplicabilityOutcome::NotApplicable
    );
    let received = runtime
        .query_applicability_for_plan(&plan, &query(Some(territory)))
        .expect("in-scope succession");
    assert_eq!(received.outcome, ApplicabilityOutcome::Applicable);
    assert_eq!(received.versions.len(), 1);
    assert_eq!(received.versions[0].id, "law-version:rule:received-a:1");

    let predecessor = runtime
        .law_versions
        .values()
        .find(|version| version.rule == "rule:received-b")
        .map(|version| LegalRecordRef {
            kind: "law_version".to_owned(),
            id: version.id.clone(),
        })
        .expect("transformable predecessor version");
    let origin = LegalOriginRef::Reception {
        succession: "succession:constitutional-replacement".to_owned(),
        predecessor: predecessor.clone(),
        transform: Some("voting-eligibility".to_owned()),
    };
    let mut received_proposal = proposal_change("received-copy", LawOperation::Receive, 4);
    received_proposal.legal_order = "successor-code".to_owned();
    received_proposal.source_profile = "received".to_owned();
    received_proposal.procedure_profile.clear();
    received_proposal.rule_id = "rule:successor-received-a".to_owned();
    received_proposal.origin = Some(origin.clone());
    received_proposal.clauses[0].forum = None;
    received_proposal.clauses[0].standing.clear();
    received_proposal.clauses[0].remedy_profile = None;
    received_proposal.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(3))];
    runtime
        .admit_non_procedural_source(&plan, received_proposal, &[], SimTime::from_minutes(4))
        .expect("materialize an explicitly received source");
    let source = runtime
        .sources
        .values()
        .find(|source| source.mode == SourceMode::Received)
        .expect("received source");
    assert_eq!(source.origin.as_ref(), Some(&origin));
    let version = runtime
        .law_versions
        .values()
        .find(|version| version.rule == "rule:successor-received-a")
        .expect("received law version");
    assert_eq!(version.origin.as_ref(), Some(&origin));
    runtime
        .validate_against_plan(&plan)
        .expect("cold validation preserves reception provenance");
}

#[test]
#[allow(clippy::too_many_lines)]
fn live_non_procedural_source_requires_exact_compiled_provider_ingress() {
    fn host(plan: &CompiledLawPlan) -> Canwu {
        let initial = LegalRuntime::new(plan)
            .to_record_draft()
            .expect("encode initial law runtime");
        Canwu::new_with_plugins(
            7,
            Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
                reference: initial.reference,
                owner: PLUGIN_NAME.to_owned(),
                class: DomainRecordClass::Record,
                version: 1,
                lifecycle: DomainRecordLifecycle::Active,
                payload: initial.payload,
                references: initial.references,
            }]),
            &[&LawPlugin, &TestSocietyPlugin],
        )
        .expect("law and provider plugins")
    }

    fn custom_claim(evidence: EvidenceRef) -> LegalProposal {
        let mut claim = proposal_change("live-custom", LawOperation::Recognize, 5);
        claim.source_profile = "custom".to_owned();
        claim.procedure_profile.clear();
        claim.rule_id = "rule:live-custom".to_owned();
        claim.admitted_signal_kinds = BTreeSet::from(["forged.self-claim".to_owned()]);
        claim.evidence = vec![evidence];
        claim
    }

    let plan = compile_law(&definition()).expect("compile provider-bound plan");
    let mut accepted = host(&plan);
    accepted
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "test-society",
            "observe-practice",
            accepted.time(),
            serde_json::json!({"practice": "repeated public conduct"}),
        ))
        .expect("enqueue observed practice");
    let generated = accepted
        .step_canonical()
        .expect("settle provider production boundary")
        .expect("observed practice boundary")
        .generated_ingress[0];
    accepted
        .step_canonical()
        .expect("settle generated provider ingress")
        .expect("recognized-practice boundary");
    enqueue_legal_mutation(
        &mut accepted,
        &LegalMutation::AdmitNonProceduralSource {
            proposal: custom_claim(EvidenceRef::Ingress(generated)),
        },
    )
    .expect("enqueue custom source");
    accepted
        .step_canonical()
        .expect("provider-backed legal boundary")
        .expect("provider-backed legal work");
    let persisted = load_legal_runtime(&accepted, &plan)
        .expect("load provider-backed law")
        .expect("law runtime");
    assert_eq!(
        persisted
            .sources
            .values()
            .next()
            .expect("custom source")
            .evidence_kinds,
        vec!["practice.recognized"]
    );
    assert!(
        !persisted.proposals["live-custom"]
            .admitted_signal_kinds
            .contains("forged.self-claim")
    );

    let mut rejected = host(&plan);
    let host_spoof = rejected
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "test-society",
            "recognized-practice",
            rejected.time(),
            serde_json::json!({"practice": "host-authored self-report"}),
        ))
        .expect("enqueue host spoof");
    rejected
        .settle_boundary(BoundaryRequest::at(rejected.time()))
        .expect("retain host-authored provider target");
    enqueue_legal_mutation(
        &mut rejected,
        &LegalMutation::AdmitNonProceduralSource {
            proposal: custom_claim(EvidenceRef::Ingress(host_spoof.ingress_id)),
        },
    )
    .expect("enqueue forged custom source");
    let error = rejected
        .step_canonical()
        .expect_err("host-authored provider target must fail closed");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);

    let mut premature = host(&plan);
    premature
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "test-society",
            "observe-delayed-practice",
            premature.time(),
            serde_json::json!({"practice": "scheduled public conduct"}),
        ))
        .expect("enqueue delayed observation");
    let future_generated = premature
        .step_canonical()
        .expect("settle delayed provider production")
        .expect("delayed observation boundary")
        .generated_ingress[0];
    enqueue_legal_mutation(
        &mut premature,
        &LegalMutation::AdmitNonProceduralSource {
            proposal: custom_claim(EvidenceRef::Ingress(future_generated)),
        },
    )
    .expect("enqueue premature custom source");
    let error = premature
        .settle_boundary(BoundaryRequest::at(premature.time()))
        .expect_err("future provider ingress must not authorize early");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);
}

#[test]
#[allow(clippy::too_many_lines)]
fn active_legal_provider_evidence_survives_compaction_as_a_compact_proof() {
    let mut authored = definition();
    authored.source_profiles[0].required_signal_kinds = vec!["practice.recognized".to_owned()];
    let plan = compile_law(&authored).expect("compile provider-bound procedure");
    let initial = LegalRuntime::new(&plan)
        .to_record_draft()
        .expect("encode initial law runtime");
    let mut canwu = Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin, &TestSocietyPlugin],
    )
    .expect("law and provider plugins");

    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "test-society",
            "observe-practice",
            canwu.time(),
            serde_json::json!({"practice": "repeated public conduct"}),
        ))
        .expect("enqueue observed practice");
    let generated = canwu
        .step_canonical()
        .expect("settle provider production")
        .expect("provider boundary")
        .generated_ingress[0];
    canwu
        .step_canonical()
        .expect("admit generated evidence")
        .expect("evidence boundary");

    let mut custom = proposal_change("archived-custom", LawOperation::Recognize, 0);
    custom.source_profile = "custom".to_owned();
    custom.procedure_profile.clear();
    custom.rule_id = "rule:archived-custom".to_owned();
    custom.evidence = vec![EvidenceRef::Ingress(generated)];
    enqueue_legal_mutation(
        &mut canwu,
        &LegalMutation::AdmitNonProceduralSource { proposal: custom },
    )
    .expect("queue evidence-bound custom");
    canwu
        .step_canonical()
        .expect("settle custom")
        .expect("custom boundary");
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("admit the final retained event before sealing");

    let mut compact = canwu.into_compacted().expect("enter compact mode");
    let prepared = compact
        .prepare_evidence_seal()
        .expect("prepare archive")
        .expect("retained evidence exists");
    let archive = MemoryArchive::default();
    assert_eq!(
        archive
            .store_evidence_segment(&prepared.segment)
            .expect("store archive"),
        ArchiveStoreOutcome::Stored
    );
    compact
        .commit_evidence_seal(&prepared.token, &archive)
        .expect("commit archive");

    let receipt = compact
        .archived_evidence_receipt(&EvidenceRef::Ingress(generated))
        .expect("active proposal retains provider receipt");
    let provenance = receipt
        .plugin_ingress_provenance
        .as_ref()
        .expect("generated ingress retains compact producer proof");
    assert_eq!(provenance.plugin, "test-society");
    assert_eq!(provenance.packet_type, "recognized-practice");

    let mut bill = proposal();
    bill.evidence = vec![EvidenceRef::Ingress(generated)];
    compact
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            LAW_MUTATION_INGRESS,
            compact.time(),
            serde_json::json!({
                "mutation": LegalMutation::SubmitProposal {
                    proposal: bill.clone(),
                }
            }),
        ))
        .expect("queue proposal backed by archived evidence");
    compact
        .step_canonical()
        .expect("admit proposal from archived evidence")
        .expect("proposal boundary");
    compact
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            LAW_MUTATION_INGRESS,
            compact.time(),
            serde_json::json!({
                "mutation": LegalMutation::Signal {
                    signal: LegalSignal {
                        kind: "practice.recognized".to_owned(),
                        proposal_id: bill.id,
                        evidence: vec![EvidenceRef::Ingress(generated)],
                    },
                }
            }),
        ))
        .expect("queue signal backed by archived provider evidence");
    compact
        .step_canonical()
        .expect("verify archived provider proof without payload hydration")
        .expect("signal boundary");
}

fn activation_error(runtime: &LegalRuntime) -> CanwuError {
    let initial = runtime.to_record_draft().expect("encode corrupt runtime");
    Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .err()
    .expect("semantic corruption must block plugin activation")
}

#[test]
fn law_plugin_activation_rejects_corrupt_aggregate_before_execution() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime.dirty_rules.insert("rule:missing".to_owned());
    let error = activation_error(&runtime);
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
}

#[test]
fn law_plugin_activation_rejects_a_forged_identity_evidence_declaration() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut submitted = proposal();
    submitted.evidence = vec![EvidenceRef::Ingress(IngressId::new(9))];
    runtime
        .submit_proposal(&plan, submitted)
        .expect("submit evidence-bound proposal");
    let mut initial = runtime.to_record_draft().expect("encode runtime");
    initial.payload[IDENTITY_EVIDENCE_DEPENDENCIES_FIELD]["dependencies"] = serde_json::json!([]);
    let error = Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .err()
    .expect("forged declaration must block activation");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
}

#[test]
fn compact_seal_rejects_a_declared_identity_without_evidence() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut submitted = proposal();
    submitted.evidence = vec![EvidenceRef::Ingress(IngressId::new(999))];
    runtime
        .submit_proposal(&plan, submitted)
        .expect("detached authoring records the declared evidence identity");
    let initial = runtime.to_record_draft().expect("encode runtime");
    let mut canwu = Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .expect("structurally valid detached runtime");
    canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("create a sealable boundary tail");
    let mut compact = canwu.into_compacted().expect("enter compact mode");
    let before = compact.checkpoint().expect("checkpoint before failed seal");
    let error = compact
        .seal_evidence()
        .expect_err("missing declared evidence must fail atomically");
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
    assert_eq!(
        compact.checkpoint().expect("checkpoint after failed seal"),
        before,
        "failed direct sealing must not mutate compacted state"
    );
}

#[test]
fn evidence_dependency_refcounts_follow_active_topology() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let initial = EvidenceRef::Ingress(IngressId::new(41));
    let added = EvidenceRef::Ingress(IngressId::new(42));
    let mut submitted = proposal();
    submitted.evidence = vec![initial.clone()];
    runtime
        .submit_proposal(&plan, submitted)
        .expect("submit evidence-backed proposal");
    assert_eq!(
        runtime.retained_evidence_dependency_counts.get(&initial),
        Some(&2),
        "the active proposal and its open procedure each own the evidence"
    );
    runtime
        .validate_against_plan(&plan)
        .expect("cold reconstruction matches submitted topology");

    runtime
        .settle_boundary(
            &plan,
            SimTime::from_minutes(1),
            &[LegalSignal {
                kind: "public_alignment".to_owned(),
                proposal_id: "suffrage".to_owned(),
                evidence: vec![added.clone()],
            }],
        )
        .expect("admit incremental evidence");
    assert_eq!(
        runtime.retained_evidence_dependency_counts.get(&added),
        Some(&1)
    );
    runtime
        .validate_against_plan(&plan)
        .expect("cold reconstruction matches signal delta");

    runtime
        .settle_boundary(&plan, SimTime::from_minutes(11), &[])
        .expect("expire unresolved procedure");
    assert!(runtime.retained_evidence_dependency_counts.is_empty());
    assert!(runtime.retained_evidence_dependencies.is_empty());
    runtime
        .validate_against_plan(&plan)
        .expect("cold reconstruction matches released topology");
}

#[test]
fn law_plugin_activation_rebuilds_compiled_and_ledger_derived_topology() {
    let plan = compile_law(&definition()).expect("compile legal plan");

    let mut corrupt_plan = LegalRuntime::new(&plan);
    corrupt_plan.plan.procedure_by_id.clear();
    assert_eq!(
        activation_error(&corrupt_plan).code,
        ErrorCode::InvalidDomainRecord
    );

    let mut corrupt_procedure = LegalRuntime::new(&plan);
    corrupt_procedure
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    corrupt_procedure
        .procedures
        .values_mut()
        .next()
        .expect("procedure")
        .stages[0]
        .quorum += 1;
    assert_eq!(
        activation_error(&corrupt_procedure).code,
        ErrorCode::InvalidDomainRecord
    );

    let mut corrupt_schedule = LegalRuntime::new(&plan);
    let mut future = proposal_change("future-custom", LawOperation::Recognize, 20);
    future.source_profile = "custom".to_owned();
    future.procedure_profile.clear();
    future.rule_id = "rule:future-custom".to_owned();
    future.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    corrupt_schedule
        .admit_non_procedural_source(
            &plan,
            future,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit future custom source");
    corrupt_schedule
        .rules
        .get_mut("rule:future-custom")
        .expect("scheduled rule")
        .scheduled_versions
        .clear();
    corrupt_schedule.scheduled_versions_by_time.clear();
    assert_eq!(
        activation_error(&corrupt_schedule).code,
        ErrorCode::InvalidDomainRecord
    );
}

#[test]
fn legal_persistence_exports_only_the_aggregate_runtime_record() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("mirrored-custom", LawOperation::Recognize, 1);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:mirrored-custom".to_owned();
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit custom source");
    let records = runtime
        .to_record_drafts()
        .expect("encode aggregate runtime")
        .into_iter()
        .map(|draft| DomainRecord {
            reference: draft.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: draft.payload,
            references: draft.references,
        })
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reference.kind.name, LAW_RUNTIME_STATE);
    let canwu = canwu_api::Canwu::new_with_plugins(
        11,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(records.clone()),
        &[&LawPlugin],
    )
    .expect("aggregate legal persistence");
    load_legal_runtime(&canwu, &plan)
        .expect("load aggregate runtime")
        .expect("runtime");
}

#[test]
fn procedure_waits_for_exact_host_reservation_allocation() {
    let mut authored = definition();
    authored.procedures[0].reservation_pool = Some("legislative-session".to_owned());
    authored.procedures[0].reservation_quantity = 2;
    let plan = compile_law(&authored).expect("compile capacity-bound plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit capacity-bound proposal");

    let waiting = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("capacity wait boundary");
    assert!(waiting.emitted_outbox.is_empty());
    assert_eq!(
        runtime
            .pending_capacity_requirements(&plan)
            .expect("capacity requirements"),
        vec![LegalCapacityRequirement {
            procedure: "procedure:suffrage".to_owned(),
            pool: "legislative-session".to_owned(),
            quantity: 2,
        }]
    );

    runtime
        .admit_capacity_allocation(
            &plan,
            LegalCapacityAllocation {
                procedure: "procedure:suffrage".to_owned(),
                pool: "legislative-session".to_owned(),
                quantity: 2,
                admitted_at: SimTime::from_minutes(1),
                evidence: EvidenceRef::Boundary(BoundaryId::new(1)),
            },
        )
        .expect("admit exact reservation allocation");
    stage_required_contexts(&mut runtime, &plan);
    let ready = runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("capacity-backed boundary");
    assert_eq!(ready.emitted_outbox.len(), 2);
    runtime
        .validate_against_plan(&plan)
        .expect("valid capacity state");
}

#[test]
fn adopted_vote_waits_for_required_cultural_evidence_kind() {
    let mut authored = definition();
    authored.source_profiles[0].required_signal_kinds = vec!["culture.support".to_owned()];
    let plan = compile_law(&authored).expect("compile signal-bound plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit signal-bound proposal");
    stage_required_contexts(&mut runtime, &plan);
    let opened = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("open vote");
    submit_two_votes(&mut runtime, &plan, &opened.emitted_outbox);

    let waiting = runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("wait for culture evidence");
    assert!(waiting.adopted_proposals.is_empty());
    assert!(runtime.sources.is_empty());

    let adopted = runtime
        .settle_boundary(
            &plan,
            SimTime::from_minutes(3),
            &[LegalSignal {
                kind: "culture.support".to_owned(),
                proposal_id: "suffrage".to_owned(),
                evidence: vec![EvidenceRef::Boundary(BoundaryId::new(2))],
            }],
        )
        .expect("admit culture evidence");
    assert_eq!(adopted.adopted_proposals, vec!["suffrage"]);
    assert_eq!(
        runtime
            .sources
            .values()
            .next()
            .expect("source")
            .evidence_kinds,
        vec!["culture.support"]
    );
}

#[test]
fn culture_retirement_preserves_law_but_respects_live_level_dependencies() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let evidence = EvidenceRef::Boundary(BoundaryId::new(1));
    let target = CulturalTargetGenerationRef {
        target: "rights-campaign".to_owned(),
        generation: 4,
    };
    let make_claim = |kind| {
        let mut claim = proposal_change("rights-custom", LawOperation::Recognize, 1);
        claim.source_profile = "custom".to_owned();
        claim.procedure_profile.clear();
        claim.rule_id = "rule:rights-custom".to_owned();
        claim.evidence = vec![evidence.clone()];
        claim.cultural_dependencies = vec![LegalCulturalDependency {
            target: target.clone(),
            kind,
            evidence: evidence.clone(),
        }];
        claim
    };

    let mut historical = LegalRuntime::new(&plan);
    historical
        .admit_non_procedural_source(
            &plan,
            make_claim(CulturalDependencyKind::AdoptionEvidence),
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit evidence-bound law");
    historical
        .retire_cultural_target_for_plan(
            &plan,
            &target,
            SimTime::from_minutes(2),
            "campaign no longer runs",
        )
        .expect("retire adoption evidence generation");
    assert!(
        historical.rules["rule:rights-custom"]
            .operative_version
            .is_some()
    );
    assert_eq!(historical.law_versions.len(), 1);

    let mut live = LegalRuntime::new(&plan);
    live.admit_non_procedural_source(
        &plan,
        make_claim(CulturalDependencyKind::LiveLevel),
        &["practice.recognized".to_owned()],
        SimTime::from_minutes(1),
    )
    .expect("admit level-dependent law");
    assert!(
        live.retire_cultural_target_for_plan(
            &plan,
            &target,
            SimTime::from_minutes(2),
            "still normatively live",
        )
        .is_err()
    );

    let mut scheduled = LegalRuntime::new(&plan);
    let mut future = make_claim(CulturalDependencyKind::LiveLevel);
    future.id = "future-rights-custom".to_owned();
    future.rule_id = "rule:future-rights-custom".to_owned();
    future.effective_at = SimTime::from_minutes(20);
    scheduled
        .admit_non_procedural_source(
            &plan,
            future,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("schedule level-dependent law");
    assert!(scheduled.scheduled_live_dependencies.contains_key(&target));
    assert!(
        scheduled
            .retire_cultural_target_for_plan(
                &plan,
                &target,
                SimTime::from_minutes(2),
                "future law still depends on this generation",
            )
            .is_err()
    );
}

#[test]
fn culture_retirement_scan_budget_fails_atomically() {
    let mut authored = definition();
    authored.budgets.max_retirement_dependency_records = 1;
    let plan = compile_law(&authored).expect("compile retirement-bounded plan");
    let target = CulturalTargetGenerationRef {
        target: "retirement-budget-target".to_owned(),
        generation: 1,
    };
    let mut runtime = LegalRuntime::new(&plan);
    for (id, minute) in [("first-custom", 1), ("second-custom", 2)] {
        let mut claim = proposal_change(id, LawOperation::Recognize, minute);
        claim.source_profile = "custom".to_owned();
        claim.procedure_profile.clear();
        claim.rule_id = format!("rule:{id}");
        claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
        claim.cultural_dependencies = vec![LegalCulturalDependency {
            target: target.clone(),
            kind: CulturalDependencyKind::AdoptionEvidence,
            evidence: EvidenceRef::Boundary(BoundaryId::new(1)),
        }];
        runtime
            .admit_non_procedural_source(
                &plan,
                claim,
                &["practice.recognized".to_owned()],
                SimTime::from_minutes(minute),
            )
            .expect("admit custom source");
    }
    let before = runtime.clone();
    assert!(
        runtime
            .retire_cultural_target_for_plan(
                &plan,
                &target,
                SimTime::from_minutes(3),
                "bounded maintenance",
            )
            .is_err()
    );
    assert_eq!(runtime, before);
}

#[test]
fn kernel_wake_expires_unresolved_procedure_and_pending_outbox() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit expiring proposal");
    let initial = runtime.to_record_draft().expect("encode runtime record");
    let mut canwu = canwu_api::Canwu::new_with_plugins(
        7,
        Scenario::new(SimTime::EPOCH, Vec::new()).with_domain_records(vec![DomainRecord {
            reference: initial.reference,
            owner: PLUGIN_NAME.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: initial.payload,
            references: initial.references,
        }]),
        &[&LawPlugin],
    )
    .expect("law plugin host");
    for requirement in runtime
        .pending_actor_context_requirements(&plan)
        .expect("actor requirements")
    {
        runtime
            .enqueue_actor_context(
                &plan,
                &mut canwu,
                &requirement,
                &canwu_api::KnowledgeQuery::default(),
            )
            .expect("queue actor context");
    }
    canwu
        .step_canonical()
        .expect("settle actor contexts")
        .expect("context boundary");
    assert_eq!(
        load_legal_runtime(&canwu, &plan)
            .expect("load open legal state")
            .expect("runtime")
            .outbox
            .len(),
        2
    );

    canwu
        .step_canonical()
        .expect("settle deadline wake")
        .expect("deadline boundary");
    assert_eq!(canwu.time(), SimTime::from_minutes(11));
    let expired = load_legal_runtime(&canwu, &plan)
        .expect("load expired legal state")
        .expect("runtime");
    assert_eq!(
        expired.proposals["suffrage"].status,
        ProposalStatus::Expired
    );
    assert!(expired.open_procedures.is_empty());
    assert!(expired.pending_outbox().next().is_none());
    assert!(
        expired
            .outbox
            .values()
            .all(|item| item.dispatch == DispatchState::Expired)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn case_finding_and_ruling_require_exact_create_only_topology() {
    let mut authored = definition();
    for (id, mode) in [
        ("adjudicated", SourceMode::Adjudicated),
        ("agreed", SourceMode::Agreed),
    ] {
        authored.source_profiles.push(LegalSourceProfileDefinition {
            id: id.to_owned(),
            mode,
            procedure: None,
            applicability_profile: "state-choice".to_owned(),
            origin_policy: match mode {
                SourceMode::Adjudicated => SourceOriginPolicy::Ruling,
                SourceMode::Agreed => SourceOriginPolicy::Agreement,
                _ => unreachable!("fixture contains only derived sources"),
            },
            authority_policy: SourceAuthorityPolicy::EvidenceClaim,
            publicity_policy: PublicityPolicy::NotRequired,
            publicity_signal_kind: None,
            required_signal_kinds: Vec::new(),
            min_evidence: 1,
            max_evidence: 8,
            require_claimant: true,
            allow_retroactive: false,
            agreement_namespace: (mode == SourceMode::Agreed).then(|| "treaty".to_owned()),
            agreement_kind: (mode == SourceMode::Agreed).then(|| "instrument".to_owned()),
            min_agreement_parties: usize::from(mode == SourceMode::Agreed) * 2,
            require_agreement_ratification: mode == SourceMode::Agreed,
        });
    }
    let plan = compile_law(&authored).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("case-source", LawOperation::Recognize, 1);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:case-source".to_owned();
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit source for ruling");
    let source = runtime
        .law_versions
        .values()
        .next()
        .expect("law version")
        .source
        .clone();
    let resolved_version = LegalRecordRef {
        kind: "law_version".to_owned(),
        id: "law-version:rule:case-source:1".to_owned(),
    };

    let case = LegalCase {
        id: "case:eligibility".to_owned(),
        legal_order: "state-code".to_owned(),
        subject_matters: vec!["voting".to_owned()],
        parties: Vec::new(),
        claims: vec!["eligibility".to_owned()],
        forum: "eligibility-court".to_owned(),
        standing: Some("affected-voter".to_owned()),
        proof_profile: "preponderance".to_owned(),
        issues: vec!["scope".to_owned()],
        deadline: SimTime::from_minutes(20),
        remedies: vec!["declaration".to_owned()],
        allegations: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
    };
    runtime
        .record_case(&plan, case.clone())
        .expect("record case");
    assert!(runtime.record_case(&plan, case).is_err());

    let finding = LegalFindingVersion {
        id: "finding:scope:1".to_owned(),
        case_id: "case:eligibility".to_owned(),
        issue: "scope".to_owned(),
        finding: "practice applies".to_owned(),
        accepted: true,
        burden: "preponderance".to_owned(),
        evidence: vec![EvidenceRef::Boundary(BoundaryId::new(1))],
        at: SimTime::from_minutes(2),
        predecessor: None,
    };
    runtime
        .record_finding(&plan, finding.clone())
        .expect("record finding");
    assert!(runtime.record_finding(&plan, finding).is_err());

    let ruling = LegalRulingVersion {
        id: "ruling:eligibility:1".to_owned(),
        case_id: "case:eligibility".to_owned(),
        institution: "assembly".to_owned(),
        issues: vec!["scope".to_owned()],
        findings: vec![LegalRecordRef {
            kind: "finding".to_owned(),
            id: "finding:scope:1".to_owned(),
        }],
        sources: vec![source],
        resolved_versions: vec![resolved_version.clone()],
        selected_versions: vec![resolved_version],
        scope: vec!["assembly-forum".to_owned()],
        precedent_profile: Some("persuasive".to_owned()),
        effective_from: SimTime::from_minutes(2),
        effective_until: None,
        remedy: Some("declaration".to_owned()),
        predecessors: Vec::new(),
        disposition: OperativeDisposition::Operative,
        evidence: vec![EvidenceRef::Boundary(BoundaryId::new(2))],
    };
    let mut scopeless = ruling.clone();
    scopeless.id = "ruling:eligibility:scopeless".to_owned();
    scopeless.scope.clear();
    assert!(runtime.record_ruling(&plan, scopeless).is_err());
    runtime
        .record_ruling(&plan, ruling.clone())
        .expect("record ruling");
    assert!(runtime.record_ruling(&plan, ruling).is_err());

    let ruling_origin = LegalOriginRef::Ruling {
        ruling: LegalRecordRef {
            kind: "ruling".to_owned(),
            id: "ruling:eligibility:1".to_owned(),
        },
    };
    let mut adjudicated = proposal_change("adjudicated-precedent", LawOperation::Recognize, 3);
    adjudicated.source_profile = "adjudicated".to_owned();
    adjudicated.procedure_profile.clear();
    adjudicated.rule_id = "rule:adjudicated-precedent".to_owned();
    adjudicated.origin = Some(ruling_origin.clone());
    adjudicated.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(2))];
    runtime
        .admit_non_procedural_source(&plan, adjudicated, &[], SimTime::from_minutes(3))
        .expect("materialize adjudicated source from exact ruling");

    let agreement_version = canwu_api::DomainRecordVersionRef {
        record: DomainRecordRef::new("treaty", "instrument", "compact-1"),
        version: 1,
        established_by: canwu_api::DomainRecordVersionSource::InitialScenario,
    };
    let agreement_origin = LegalOriginRef::Agreement {
        instrument: agreement_version.clone(),
        parties: vec![
            EntityRef::Person(PersonId::new(1)),
            EntityRef::Person(PersonId::new(2)),
        ],
        ratifications: vec![EvidenceRef::Boundary(BoundaryId::new(2))],
    };
    let mut agreed = proposal_change("agreed-compact", LawOperation::Recognize, 4);
    agreed.source_profile = "agreed".to_owned();
    agreed.procedure_profile.clear();
    agreed.rule_id = "rule:agreed-compact".to_owned();
    agreed.origin = Some(agreement_origin.clone());
    agreed.expected_versions = vec![agreement_version.clone()];
    agreed.evidence = vec![
        EvidenceRef::Boundary(BoundaryId::new(2)),
        EvidenceRef::DomainRecordVersion(agreement_version),
    ];
    let mut under_partied = agreed.clone();
    under_partied.id = "agreed-compact-under-partied".to_owned();
    under_partied.rule_id = "rule:agreed-compact-under-partied".to_owned();
    if let Some(LegalOriginRef::Agreement { parties, .. }) = &mut under_partied.origin {
        parties.truncate(1);
    }
    assert!(
        runtime
            .admit_non_procedural_source(&plan, under_partied, &[], SimTime::from_minutes(4),)
            .is_err()
    );
    runtime
        .admit_non_procedural_source(&plan, agreed, &[], SimTime::from_minutes(4))
        .expect("materialize agreed source from exact instrument version");

    assert!(runtime.sources.values().any(|source| {
        source.mode == SourceMode::Adjudicated && source.origin.as_ref() == Some(&ruling_origin)
    }));
    assert!(runtime.sources.values().any(|source| {
        source.mode == SourceMode::Agreed && source.origin.as_ref() == Some(&agreement_origin)
    }));
    runtime
        .validate_against_plan(&plan)
        .expect("valid adjudication and agreement provenance");
    let mut missing_case_scope = runtime.clone();
    missing_case_scope
        .cases
        .get_mut("case:eligibility")
        .expect("cold case")
        .subject_matters
        .clear();
    assert!(missing_case_scope.validate_against_plan(&plan).is_err());
    let mut missing_case_remedy = runtime.clone();
    missing_case_remedy
        .cases
        .get_mut("case:eligibility")
        .expect("cold case")
        .remedies
        .clear();
    assert!(missing_case_remedy.validate_against_plan(&plan).is_err());
    let mut missing_ruling_finding = runtime.clone();
    missing_ruling_finding
        .rulings
        .get_mut("ruling:eligibility:1")
        .expect("cold ruling")
        .findings
        .clear();
    assert!(missing_ruling_finding.validate_against_plan(&plan).is_err());
}

#[test]
fn idle_boundaries_do_not_consume_persisted_state_budget() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let reserved = runtime.reserved_state_bytes;

    for minute in 1..=10_000 {
        runtime
            .settle_boundary(&plan, SimTime::from_minutes(minute), &[])
            .expect("idle legal boundary");
    }

    assert_eq!(runtime.reserved_state_bytes, reserved);
}

#[test]
fn open_procedure_idle_boundaries_do_not_consume_state_budget() {
    let plan = compile_law(&definition()).expect("compile legal plan");
    let mut runtime = LegalRuntime::new(&plan);
    let mut waiting = proposal();
    waiting.deadline = SimTime::from_minutes(20_000);
    runtime
        .submit_proposal(&plan, waiting)
        .expect("submit waiting procedure");
    let reserved = runtime.reserved_state_bytes;

    for minute in 1..=10_000 {
        runtime
            .settle_boundary(&plan, SimTime::from_minutes(minute), &[])
            .expect("idle open-procedure boundary");
    }

    assert_eq!(runtime.reserved_state_bytes, reserved);
    assert_eq!(runtime.open_procedures.len(), 1);
}

#[test]
fn applicability_pipeline_enforces_scope_and_jurisdiction() {
    let mut authored = definition();
    authored.jurisdictions.push(LegalJurisdictionDefinition {
        id: "unrelated-forum".to_owned(),
        relations: Vec::new(),
        metadata: BTreeMap::new(),
    });
    let plan = compile_law(&authored).expect("compile scoped law");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("scoped-custom", LawOperation::Recognize, 1);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:scoped-custom".to_owned();
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    claim.clauses[0].subject_matters = vec!["voting".to_owned()];
    claim.clauses[0].territories = vec![TerritoryId::new(7)];
    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit scoped custom");
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("refresh applicability");

    let query = |matter: &str, territory, jurisdiction: &str| ApplicabilityQuery {
        event_at: SimTime::from_minutes(1),
        read_at: SimTime::from_minutes(1),
        subject: None,
        actor: None,
        knowledge_read_cut: None,
        territory,
        subject_matter: Some(matter.to_owned()),
        legal_order: "state-code".to_owned(),
        profile: "state-choice".to_owned(),
        jurisdiction: Some(jurisdiction.to_owned()),
        facts: applicable_facts(),
        fact_evidence: applicable_fact_evidence(),
        fact_knowledge_records: BTreeMap::new(),
    };
    assert_eq!(
        runtime
            .query_applicability_for_plan(
                &plan,
                &query("voting", Some(TerritoryId::new(7)), "assembly-forum"),
            )
            .expect("matching applicability")
            .outcome,
        ApplicabilityOutcome::Applicable
    );
    for mismatch in [
        query("property", Some(TerritoryId::new(7)), "assembly-forum"),
        query("voting", Some(TerritoryId::new(8)), "assembly-forum"),
        query("voting", Some(TerritoryId::new(7)), "unrelated-forum"),
    ] {
        assert_eq!(
            runtime
                .query_applicability_for_plan(&plan, &mismatch)
                .expect("bounded mismatch")
                .outcome,
            ApplicabilityOutcome::NotApplicable
        );
    }
}

#[test]
fn actor_relative_applicability_is_derived_from_one_exact_holder_read_cut() {
    let mut authored = definition();
    for predicate in &mut authored.predicates {
        predicate.knowledge_schema = Some(legal_fact_schema());
        predicate.payload_pointer = Some(format!("/facts/{}", predicate.id));
    }
    let plan = compile_law(&authored).expect("compile actor-bound predicates");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("actor-facts", LawOperation::Recognize, 1);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:actor-facts".to_owned();
    claim.clauses[0].duty_bearers = vec![EntityRef::Person(PersonId::new(1)).to_string()];
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit actor-scoped source");

    let mut canwu = Canwu::demo(7).expect("create knowledge host");
    canwu
        .register_plugin(&ActorLegalFactsPlugin)
        .expect("register actor fact plugin");
    let publication_at = canwu.time();
    canwu
        .settle_boundary(BoundaryRequest::at(publication_at).with_cadence(SystemCadence::Daily))
        .expect("publish actor facts");
    let holder = KnowledgeHolderRef::Person(PersonId::new(1));
    let knowledge_query = KnowledgeQuery {
        view: KnowledgeHistoryView::FullHistory,
        ..KnowledgeQuery::default()
    };
    let read = canwu
        .admin_query_knowledge(holder.clone(), &knowledge_query)
        .expect("read actor facts");
    let record_id = read.records[0].id;
    let facts = applicable_facts();
    let mut query = ApplicabilityQuery {
        event_at: SimTime::from_minutes(1),
        read_at: SimTime::from_minutes(1),
        subject: None,
        actor: Some(holder),
        knowledge_read_cut: Some(read.read_cut),
        territory: None,
        subject_matter: Some("voting".to_owned()),
        legal_order: "state-code".to_owned(),
        profile: "state-choice".to_owned(),
        jurisdiction: Some("assembly-forum".to_owned()),
        fact_evidence: applicable_fact_evidence(),
        fact_knowledge_records: facts
            .keys()
            .map(|predicate| (predicate.clone(), record_id))
            .collect(),
        facts,
    };
    assert!(runtime.query_applicability_for_plan(&plan, &query).is_err());
    assert_eq!(
        runtime
            .query_applicability_with_host(&plan, &canwu, &query, Some(&knowledge_query))
            .expect("host-bound actor applicability")
            .outcome,
        ApplicabilityOutcome::Applicable
    );
    query.facts.insert("citizen".to_owned(), false);
    assert!(
        runtime
            .query_applicability_with_host(&plan, &canwu, &query, Some(&knowledge_query))
            .is_err()
    );
}

#[test]
fn proposal_fanout_fails_before_mutation() {
    let mut authored = definition();
    authored.budgets.max_clauses_per_proposal = 1;
    let plan = compile_law(&authored).expect("compile bounded law");
    let mut runtime = LegalRuntime::new(&plan);
    let mut oversized = proposal();
    oversized.clauses.push(oversized.clauses[0].clone());

    assert!(runtime.submit_proposal(&plan, oversized).is_err());
    assert!(runtime.proposals.is_empty());

    let mut nested_authored = definition();
    nested_authored.budgets.max_nested_items_per_record = 1;
    let nested_plan = compile_law(&nested_authored).expect("compile nested-item budget");
    let mut nested_runtime = LegalRuntime::new(&nested_plan);
    let mut nested_oversized = proposal();
    nested_oversized.clauses[0]
        .holders
        .push("status:additional-holder".to_owned());
    assert!(
        nested_runtime
            .submit_proposal(&nested_plan, nested_oversized)
            .is_err()
    );
    assert!(nested_runtime.proposals.is_empty());
    assert!(runtime.procedures.is_empty());

    let mut no_jurisdiction = proposal();
    no_jurisdiction.jurisdictions.clear();
    assert!(runtime.submit_proposal(&plan, no_jurisdiction).is_err());
    let mut no_subject_matter = proposal();
    no_subject_matter.clauses[0].subject_matters.clear();
    assert!(runtime.submit_proposal(&plan, no_subject_matter).is_err());
    let mut unknown_remedy = proposal();
    unknown_remedy.clauses[0].remedy_profile = Some("unavailable-remedy".to_owned());
    assert!(runtime.submit_proposal(&plan, unknown_remedy).is_err());
    assert!(runtime.proposals.is_empty());
}

#[test]
fn applicability_query_total_work_budget_is_enforced() {
    let mut authored = definition();
    authored.budgets.max_applicability_query_work = 1;
    let plan = compile_law(&authored).expect("compile query work budget");
    let mut runtime = LegalRuntime::new(&plan);
    let mut claim = proposal_change("bounded-query", LawOperation::Recognize, 1);
    claim.source_profile = "custom".to_owned();
    claim.procedure_profile.clear();
    claim.rule_id = "rule:bounded-query".to_owned();
    claim.evidence = vec![EvidenceRef::Boundary(BoundaryId::new(1))];
    runtime
        .admit_non_procedural_source(
            &plan,
            claim,
            &["practice.recognized".to_owned()],
            SimTime::from_minutes(1),
        )
        .expect("admit bounded query source");
    let query = ApplicabilityQuery {
        event_at: SimTime::from_minutes(1),
        read_at: SimTime::from_minutes(1),
        subject: None,
        actor: None,
        knowledge_read_cut: None,
        territory: None,
        subject_matter: Some("voting".to_owned()),
        legal_order: "state-code".to_owned(),
        profile: "state-choice".to_owned(),
        jurisdiction: Some("assembly-forum".to_owned()),
        facts: applicable_facts(),
        fact_evidence: applicable_fact_evidence(),
        fact_knowledge_records: BTreeMap::new(),
    };
    assert!(runtime.query_applicability_for_plan(&plan, &query).is_err());
}

#[test]
fn recovery_rebuilds_latest_participation_index() {
    let mut authored = definition();
    authored.procedures[0].stages[0].allow_replacement = true;
    let plan = compile_law(&authored).expect("compile replacement procedure");
    let mut runtime = LegalRuntime::new(&plan);
    runtime
        .submit_proposal(&plan, proposal())
        .expect("submit proposal");
    stage_required_contexts(&mut runtime, &plan);
    let emitted = runtime
        .settle_boundary(&plan, SimTime::from_minutes(1), &[])
        .expect("emit decisions")
        .emitted_outbox;
    let item = emitted
        .iter()
        .find(|item| item.seat == "east")
        .expect("east decision");
    mark_test_outbox(&mut runtime, item.sequence, IngressId::new(1));
    runtime
        .queue_pending_intent(&plan, intent_from_outbox(item, "for"))
        .expect("first vote");
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(2), &[])
        .expect("settle first vote");
    runtime
        .queue_pending_intent(&plan, intent_from_outbox(item, "against"))
        .expect("replacement vote");
    runtime
        .settle_boundary(&plan, SimTime::from_minutes(3), &[])
        .expect("settle replacement vote");
    assert_eq!(runtime.participations.len(), 2);

    let mut tampered = runtime.snapshot_state();
    let key = tampered
        .latest_participation_by_key
        .keys()
        .next()
        .expect("participation key")
        .clone();
    tampered.latest_participation_by_key.insert(key, 0);
    assert!(LegalRuntime::from_state(&plan, tampered).is_err());
}

#[test]
fn plugin_registers_all_legal_schemas_and_pending_intent_command() {
    assert_eq!(law_record_schemas().len(), 1);
    assert_eq!(law_command_descriptor().name, LAW_COMMAND);
    canwu_api::Canwu::new_with_plugins(7, Scenario::new(SimTime::EPOCH, Vec::new()), &[&LawPlugin])
        .expect("law plugin registration");
}
