//! Expected legal refusals and full budgets are recorded and never fail a
//! boundary.
//!
//! A legal packet whose content is refused, or that would exceed a lifetime
//! budget, changes nothing and is recorded with
//! `canwu.law.ingress_rejected.v1`; a pending proposal whose host
//! compare-and-set stops holding is rejected and recorded with
//! `canwu.law.proposal_rejected.v1`; work past the per-boundary budget waits
//! for a continuation boundary at the same time. The boundary settles the
//! other work, so later canonical steps keep settling, and exact replay
//! reproduces the run.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, DomainRecord, DomainRecordClass, DomainRecordDraft,
    DomainRecordKind, DomainRecordLifecycle, DomainRecordMutation, DomainRecordRef,
    DomainRecordSchema, DomainRecordVersionRef, DomainRecordVersionSource, EntityRef, IngressClass,
    IngressPayload, KnowledgeHolderRef, KnowledgeQuery, PayloadSchema, PersonId,
    PluginIngressDescriptor, PluginIngressRequest, PluginRegistrar, Scenario, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
};
use canwu_law::*;
use std::collections::{BTreeMap, BTreeSet};

const ORDER: &str = "council-order";
const JURISDICTION: &str = "realm";
const PROCEDURE: &str = "council";
const HOST: &str = "fixture-registry";

fn host_kind() -> DomainRecordKind {
    DomainRecordKind::new(HOST, "entry")
}

fn host_record() -> DomainRecordRef {
    DomainRecordRef {
        kind: host_kind(),
        id: "charter".to_owned(),
    }
}

fn host_state() -> StateKey {
    DomainRecordSchema::new(host_kind(), DomainRecordClass::Record).state_key()
}

/// Bumps the host record once for every `amend` packet it admits.
fn amend_registry(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        if !matches!(
            &ingress.payload,
            IngressPayload::Plugin { plugin, packet_type, .. }
                if plugin == HOST && packet_type == "amend"
        ) {
            continue;
        }
        let current = view
            .domain_record(&host_record())?
            .expect("the host record exists");
        let revision = current.version + 1;
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(
                    host_record(),
                    serde_json::json!({ "revision": revision }),
                ),
                expected_version: current.version,
            },
            summary: "Amend the host charter".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// A host plugin owning one record that legal proposals compare-and-set.
struct RegistryPlugin;

impl SimulationPlugin for RegistryPlugin {
    fn name(&self) -> &'static str {
        HOST
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000b01"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            host_kind(),
            DomainRecordClass::Record,
        ))?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: "amend".to_owned(),
            description: "Amend the host record".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut contract = BoundarySystemContract::new(
            "amend-registry",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        contract.reads = vec![StateKey::core_ingress(), host_state()];
        contract.writes = vec![host_state()];
        contract.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(contract, amend_registry)
    }
}

fn seat(index: u64) -> String {
    format!("seat-{index}")
}

/// One council of two seats with a single deliberation stage.
fn definition() -> LegalDefinition {
    let mut definition = LegalDefinition::new("registry-law");
    definition.orders.push(LegalOrderDefinition {
        id: ORDER.to_owned(),
        precedence_profile: "later-in-time".to_owned(),
    });
    definition.jurisdictions.push(LegalJurisdictionDefinition {
        id: JURISDICTION.to_owned(),
        relations: Vec::new(),
        metadata: BTreeMap::new(),
    });
    definition.institutions.push(LegalInstitutionDefinition {
        id: "council".to_owned(),
        organization: None,
        jurisdictions: vec![JURISDICTION.to_owned()],
        seats: (1..=2)
            .map(|index| AuthoritySeatDefinition {
                id: seat(index),
                holder: Some(KnowledgeHolderRef::Person(PersonId::new(index))),
                permission_profile: "councillor".to_owned(),
            })
            .collect(),
        procedures: vec![PROCEDURE.to_owned()],
        competences: vec![LegalCompetenceDefinition {
            legal_orders: vec![ORDER.to_owned()],
            jurisdictions: vec![JURISDICTION.to_owned()],
            subject_matters: vec!["governance".to_owned()],
            source_modes: vec![SourceMode::Promulgated],
            operations: vec![LawOperation::Establish],
            procedures: vec!["*".to_owned()],
            forums: vec!["*".to_owned()],
            can_adjudicate: false,
        }],
    });
    definition.procedures.push(ProcedureProfileDefinition {
        id: PROCEDURE.to_owned(),
        stages: vec![ProcedureStageDefinition {
            id: "vote".to_owned(),
            kind: ProcedureStageKind::Deliberation,
            seats: (1..=2).map(seat).collect(),
            allowed_ballots: vec![Ballot::Abstain, Ballot::Against, Ballot::For],
            quorum: 0,
            threshold: 0,
            deadline_minutes: 60,
            allow_replacement: false,
            seat_weights: BTreeMap::new(),
            block_of_seat: BTreeMap::new(),
            block_threshold: None,
        }],
        deterministic_tie_break: PROCEDURE_TIE_BREAK_STATUS_QUO.to_owned(),
        reservation_pool: None,
        reservation_quantity: 0,
    });
    definition
        .source_profiles
        .push(LegalSourceProfileDefinition {
            id: PROCEDURE.to_owned(),
            mode: SourceMode::Promulgated,
            procedure: Some(PROCEDURE.to_owned()),
            applicability_profile: "realm-rules".to_owned(),
            origin_policy: SourceOriginPolicy::NoOrigin,
            authority_policy: SourceAuthorityPolicy::ProceduralInstitution,
            publicity_policy: PublicityPolicy::NotRequired,
            publicity_signal_kind: None,
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
    definition.clauses.push(ClauseDefinition {
        id: "resolution".to_owned(),
        schema: "canwu.test.resolution.v1".to_owned(),
        modality: NormativeModality::Status,
        operation_kinds: vec!["status".to_owned()],
    });
    definition
        .applicability_profiles
        .push(ApplicabilityProfileDefinition {
            id: "realm-rules".to_owned(),
            legal_order: ORDER.to_owned(),
            temporal_conflict_rule: "later-in-time".to_owned(),
            pipeline: ["scope", "jurisdiction", "validity", "conflict"]
                .map(str::to_owned)
                .to_vec(),
            jurisdiction_traversal: Vec::new(),
            max_candidates: 64,
        });
    definition
        .precedence_profiles
        .push(PrecedenceProfileDefinition {
            id: "later-in-time".to_owned(),
            ordered_bases: vec![ConflictResolutionBasis::Temporal],
        });
    definition
}

fn proposal(id: &str, expected_versions: Vec<DomainRecordVersionRef>) -> LegalProposal {
    LegalProposal {
        id: id.to_owned(),
        sponsor: None,
        legal_order: ORDER.to_owned(),
        jurisdictions: vec![JURISDICTION.to_owned()],
        subjects: Vec::new(),
        cultural_dependencies: Vec::new(),
        clauses: vec![ClauseOperation {
            clause: "resolution".to_owned(),
            operation: "establish".to_owned(),
            content_hash: "c".repeat(64),
            value: serde_json::json!({ "resolution": id }),
            holders: vec!["status:realm".to_owned()],
            subject_matters: vec!["governance".to_owned()],
            ..ClauseOperation::default()
        }],
        source_profile: PROCEDURE.to_owned(),
        procedure_profile: PROCEDURE.to_owned(),
        procedure_profile_hash: String::new(),
        deadline: SimTime::from_minutes(600),
        effective_at: SimTime::from_minutes(100_000),
        operation: LawOperation::Establish,
        rule_id: format!("rule:{id}"),
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
        expected_versions,
        active_procedure: None,
    }
}

struct Run {
    canwu: Canwu,
    plan: CompiledLawPlan,
}

impl Run {
    fn new(definition: &LegalDefinition) -> Self {
        let plan = compile_law(definition).expect("compile registry plan");
        let mut records = LegalRuntime::new(&plan)
            .to_record_drafts()
            .expect("encode empty legal shards")
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
        records.push(DomainRecord {
            reference: host_record(),
            owner: HOST.to_owned(),
            class: DomainRecordClass::Record,
            version: 1,
            lifecycle: DomainRecordLifecycle::Active,
            payload: serde_json::json!({ "revision": 1 }),
            references: Vec::new(),
        });
        let persons = (1..=2)
            .map(|index| EntityRef::Person(PersonId::new(index)))
            .collect();
        let canwu = Canwu::new_with_plugins(
            7,
            Scenario::new(SimTime::EPOCH, persons).with_domain_records(records),
            &[&LawPlugin, &RegistryPlugin],
        )
        .expect("law and registry host");
        Self { canwu, plan }
    }

    fn law(&self) -> LegalRuntime {
        load_legal_runtime(&self.canwu, &self.plan)
            .expect("load legal runtime")
            .expect("legal runtime")
    }

    fn settle(&mut self, minutes: i64, label: &str) {
        self.canwu
            .settle_boundary(BoundaryRequest::at(SimTime::from_minutes(minutes)))
            .expect(label);
    }

    fn amend_host_record(&mut self) {
        self.canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                HOST,
                "amend",
                self.canwu.time(),
                serde_json::json!({}),
            ))
            .expect("queue host amendment");
        let now = self.canwu.time().as_minutes();
        self.settle(now, "host amendment boundary");
    }

    fn current_host_version(&self) -> DomainRecordVersionRef {
        self.canwu
            .current_domain_record_version(&host_record())
            .expect("host record version read")
            .expect("host record version")
    }

    fn events(&self, event_type: &str) -> Vec<String> {
        self.canwu
            .events()
            .iter()
            .filter(|event| event.kind.plugin_identity() == Some((PLUGIN_NAME, event_type)))
            .map(|event| event.summary.clone())
            .collect()
    }

    fn assert_exact_replay(&self) {
        let replayed = Canwu::replay_from_journal(
            &[&LawPlugin, &RegistryPlugin],
            &self.canwu.replay_journal(),
        )
        .expect("exact replay");
        assert_eq!(replayed.snapshot(), self.canwu.snapshot());
    }
}

/// A host record that changes between a proposal's enqueue and its admission,
/// or while the proposal deliberates, refuses the proposal; the boundary
/// settles and the run continues.
#[test]
fn a_stale_host_compare_and_set_refuses_the_proposal_not_the_boundary() {
    let mut run = Run::new(&definition());
    let initial = DomainRecordVersionRef {
        record: host_record(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };
    // Queued for minute 10 against version 1; the host record changes first.
    enqueue_legal_mutation_at(
        &mut run.canwu,
        SimTime::from_minutes(10),
        &LegalMutation::SubmitProposal {
            proposal: proposal("stale-at-admission", vec![initial]),
        },
    )
    .expect("queue proposal");
    run.amend_host_record();
    run.settle(10, "a stale compare-and-set is a recorded refusal");
    let refusals = run.events(LAW_INGRESS_REJECTED_EVENT);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(refusals[0].contains("compare-and-set"), "{refusals:?}");
    assert!(!run.law().proposals.contains_key("stale-at-admission"));

    // The run continues: a proposal naming the current version opens.
    let current = run.current_host_version();
    enqueue_legal_mutation(
        &mut run.canwu,
        &LegalMutation::SubmitProposal {
            proposal: proposal("stale-in-deliberation", vec![current]),
        },
    )
    .expect("queue current proposal");
    run.settle(10, "proposal boundary");
    assert!(
        run.law()
            .open_procedures
            .contains("procedure:stale-in-deliberation")
    );

    // The host record changes again while the proposal deliberates. The next
    // legal work on its procedure, staging its seat contexts, rejects it.
    run.amend_host_record();
    let law = run.law();
    for requirement in law
        .pending_actor_context_requirements(&run.plan)
        .expect("seat requirements")
    {
        law.enqueue_actor_context(
            &run.plan,
            &mut run.canwu,
            &requirement,
            &KnowledgeQuery::default(),
        )
        .expect("queue seat context");
    }
    run.settle(10, "a stale pending proposal is rejected, not the boundary");
    let law = run.law();
    assert_eq!(
        law.proposals["stale-in-deliberation"].status,
        ProposalStatus::Rejected
    );
    assert!(
        !law.open_procedures
            .contains("procedure:stale-in-deliberation")
    );
    assert!(law.staged_actor_contexts.is_empty());
    let rejected = run.events(LAW_PROPOSAL_REJECTED_EVENT);
    assert_eq!(rejected.len(), 1, "{rejected:?}");
    assert!(rejected[0].contains("stale-in-deliberation"));

    // Later canonical steps keep settling.
    run.settle(20, "a later boundary");
    run.assert_exact_replay();
}

/// Malformed, conflicting, unroutable, early, and over-budget legal packets
/// are refused one by one and change nothing; the packets beside them settle.
#[test]
fn refused_legal_packets_change_nothing_and_their_neighbours_settle() {
    let mut authored = definition();
    authored.budgets.max_mutations_per_boundary = 3;
    let mut run = Run::new(&authored);
    let at = run.canwu.time();
    let mutation = |proposal: LegalProposal| serde_json::json!({ "mutation": LegalMutation::SubmitProposal { proposal } });
    let mut unroutable = proposal("unroutable", Vec::new());
    unroutable.legal_order = "uncompiled-order".to_owned();
    for payload in [
        // Does not decode.
        serde_json::json!({ "mutation": { "type": "unknown_mutation" } }),
        // Admitted.
        mutation(proposal("kept", Vec::new())),
        // The runtime refuses a second proposal with the same ID after the
        // first one changed the working runtime.
        mutation(proposal("kept", Vec::new())),
        // Names a legal order, and so a shard, the plan does not hold.
        mutation(unroutable),
        // Beyond the boundary's legal ingress budget of three.
        mutation(proposal("over-budget", Vec::new())),
    ] {
        run.canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                PLUGIN_NAME,
                LAW_MUTATION_INGRESS,
                at,
                payload,
            ))
            .expect("queue legal packet");
    }
    // A host-authored wake for a time that has not come.
    run.canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            LAW_WAKE_INGRESS,
            at,
            serde_json::json!({ "due_at": 500 }),
        ))
        .expect("queue early wake");
    run.settle(0, "refused legal packets settle as recorded refusals");

    let refusals = run.events(LAW_INGRESS_REJECTED_EVENT);
    assert_eq!(refusals.len(), 5, "{refusals:?}");
    assert!(refusals[0].contains("invalid_payload"), "{refusals:?}");
    assert!(
        refusals[1].contains("duplicate legal proposal"),
        "{refusals:?}"
    );
    assert!(refusals[2].contains("does not hold"), "{refusals:?}");
    assert!(refusals[3].contains("budget exhausted"), "{refusals:?}");
    assert!(refusals[4].contains("before its due time"), "{refusals:?}");
    let law = run.law();
    assert_eq!(
        law.proposals.keys().cloned().collect::<Vec<_>>(),
        vec!["kept".to_owned()]
    );
    assert_eq!(law.open_procedures.len(), 1);

    run.settle(1, "a later boundary");
    run.assert_exact_replay();
}

/// Work past the per-boundary budget waits for a continuation boundary at the
/// same time instead of failing a boundary: four procedures that two
/// boundaries opened reach their deadline together and expire over two
/// boundaries at that instant.
#[test]
fn a_deadline_pile_up_settles_over_continuation_boundaries() {
    let mut authored = definition();
    authored.budgets.max_mutations_per_boundary = 2;
    let mut run = Run::new(&authored);
    for pair in [["first", "second"], ["third", "fourth"]] {
        for id in pair {
            enqueue_legal_mutation(
                &mut run.canwu,
                &LegalMutation::SubmitProposal {
                    proposal: proposal(id, Vec::new()),
                },
            )
            .expect("queue proposal");
        }
        run.settle(0, "two proposals open");
    }
    let law = run.law();
    assert_eq!(law.open_procedures.len(), 4);
    let deadline = law.procedures["procedure:first"].deadline;
    assert!(
        law.procedures
            .values()
            .all(|procedure| procedure.deadline == deadline)
    );

    // The wake after the deadline expires two procedures and defers two.
    let expiry = run
        .canwu
        .step_canonical()
        .expect("the deadline boundary settles")
        .expect("deadline boundary");
    assert!(expiry.settled_at > deadline);
    let law = run.law();
    assert_eq!(law.open_procedures.len(), 2);
    assert_eq!(expiry.generated_ingress.len(), 1, "one continuation wake");
    // A continuation boundary at the same time settles the rest.
    let continuation = run
        .canwu
        .step_canonical()
        .expect("the continuation boundary settles")
        .expect("continuation boundary");
    assert_eq!(continuation.settled_at, expiry.settled_at);
    assert!(continuation.generated_ingress.is_empty());
    let law = run.law();
    assert!(law.open_procedures.is_empty());
    assert!(
        law.proposals
            .values()
            .all(|proposal| proposal.status == ProposalStatus::Expired)
    );
    assert!(law.scheduled_wakes.is_empty());
    assert!(run.events(LAW_INGRESS_REJECTED_EVENT).is_empty());
    run.assert_exact_replay();
}

/// A packet that would exceed a lifetime budget of the plan is refused at
/// admission; the boundary settles.
#[test]
fn a_lifetime_budget_refuses_at_admission() {
    let mut authored = definition();
    authored.budgets.max_procedures = 1;
    let mut run = Run::new(&authored);
    for id in ["first", "second"] {
        enqueue_legal_mutation(
            &mut run.canwu,
            &LegalMutation::SubmitProposal {
                proposal: proposal(id, Vec::new()),
            },
        )
        .expect("queue proposal");
        run.settle(0, "a full proposal budget is a recorded refusal");
    }
    let refusals = run.events(LAW_INGRESS_REJECTED_EVENT);
    assert_eq!(refusals.len(), 1, "{refusals:?}");
    assert!(
        refusals[0].contains("legal proposal budget exhausted"),
        "{refusals:?}"
    );
    assert_eq!(
        run.law().proposals.keys().cloned().collect::<Vec<_>>(),
        vec!["first".to_owned()]
    );
    run.settle(1, "a later boundary");
    run.assert_exact_replay();
}
