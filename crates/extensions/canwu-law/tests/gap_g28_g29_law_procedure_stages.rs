//! Public-API evidence for weighted and unit-block procedure stages (G-28) and
//! the advisory consultation stage kind (G-29).
//!
//! Both fixtures run the real host adapter: seat contexts, outbox preparation,
//! ticket dispatch, acknowledgement, per-seat ticket resolution, and the
//! command-bound legal intent, followed by save/load and exact replay.

use canwu_api::{
    BoundaryRequest, Canwu, CommandRequestId, DecisionRequestId, DecisionTicketId, DomainRecord,
    DomainRecordClass, DomainRecordLifecycle, EntityRef, HumanDecisionResponse, KnowledgeHolderRef,
    KnowledgeQuery, PersonId, QueuedHumanPolicy, Scenario, SimTime, SimulationPlugin,
};
use canwu_law::*;
use std::collections::{BTreeMap, BTreeSet};

const ORDER: &str = "council-order";
const JURISDICTION: &str = "realm";
const INSTITUTION: &str = "council";

fn seat(index: u64) -> String {
    format!("seat-{index}")
}

fn stage(id: &str, kind: ProcedureStageKind, seats: u64) -> ProcedureStageDefinition {
    ProcedureStageDefinition {
        id: id.to_owned(),
        kind,
        seats: (1..=seats).map(seat).collect(),
        allowed_ballots: vec![Ballot::Abstain, Ballot::Against, Ballot::For],
        quorum: 0,
        threshold: 0,
        deadline_minutes: 60,
        allow_replacement: false,
        seat_weights: BTreeMap::new(),
        block_of_seat: BTreeMap::new(),
        block_threshold: None,
    }
}

fn procedure(
    id: &str,
    tie_break: &str,
    stages: Vec<ProcedureStageDefinition>,
) -> ProcedureProfileDefinition {
    ProcedureProfileDefinition {
        id: id.to_owned(),
        stages,
        deterministic_tie_break: tie_break.to_owned(),
        reservation_pool: None,
        reservation_quantity: 0,
    }
}

/// One institution whose seats `seat-1..=seat-N` are held by persons `1..=N`.
fn definition(seats: u64, procedures: Vec<ProcedureProfileDefinition>) -> LegalDefinition {
    let mut definition = LegalDefinition::new("council-law");
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
        id: INSTITUTION.to_owned(),
        organization: None,
        jurisdictions: vec![JURISDICTION.to_owned()],
        seats: (1..=seats)
            .map(|index| AuthoritySeatDefinition {
                id: seat(index),
                holder: Some(KnowledgeHolderRef::Person(PersonId::new(index))),
                permission_profile: "councillor".to_owned(),
            })
            .collect(),
        procedures: procedures
            .iter()
            .map(|profile| profile.id.clone())
            .collect(),
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
    for profile in &procedures {
        definition
            .source_profiles
            .push(LegalSourceProfileDefinition {
                id: profile.id.clone(),
                mode: SourceMode::Promulgated,
                procedure: Some(profile.id.clone()),
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
    }
    definition.procedures = procedures;
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

fn proposal(id: &str, procedure: &str, deadline: i64) -> LegalProposal {
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
        source_profile: procedure.to_owned(),
        procedure_profile: procedure.to_owned(),
        procedure_profile_hash: String::new(),
        deadline: SimTime::from_minutes(deadline),
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
        expected_versions: Vec::new(),
        active_procedure: None,
    }
}

/// A Canwu run hosting the law plugin for one compiled council plan.
struct Council {
    canwu: Canwu,
    plan: CompiledLawPlan,
}

impl Council {
    fn new(definition: &LegalDefinition, seats: u64) -> Self {
        let plan = compile_law(definition).expect("compile council plan");
        let records = LegalRuntime::new(&plan)
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
            .collect();
        let persons = (1..=seats)
            .map(|index| EntityRef::Person(PersonId::new(index)))
            .collect();
        let canwu = Canwu::new_with_plugins(
            7,
            Scenario::new(SimTime::EPOCH, persons).with_domain_records(records),
            &[&LawPlugin],
        )
        .expect("law host");
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

    fn submit(&mut self, proposals: Vec<LegalProposal>) {
        for proposal in proposals {
            enqueue_legal_mutation(&mut self.canwu, &LegalMutation::SubmitProposal { proposal })
                .expect("queue proposal");
        }
        let now = self.canwu.time().as_minutes();
        self.settle(now, "proposal boundary");
    }

    /// Stages every pending seat context, then dispatches and acknowledges the
    /// resulting tickets through the three persisted adapter stages.
    fn open_tickets(&mut self) {
        let now = self.canwu.time().as_minutes();
        let law = self.law();
        for requirement in law
            .pending_actor_context_requirements(&self.plan)
            .expect("seat requirements")
        {
            law.enqueue_actor_context(
                &self.plan,
                &mut self.canwu,
                &requirement,
                &KnowledgeQuery::default(),
            )
            .expect("queue seat context");
        }
        self.settle(now, "seat context boundary");
        self.law()
            .prepare_pending_decision_enqueues(&mut self.canwu)
            .expect("prepare tickets");
        self.settle(now, "preparation boundary");
        self.law()
            .enqueue_pending_decisions(&mut self.canwu)
            .expect("enqueue tickets");
        self.settle(now, "ticket boundary");
        self.law()
            .acknowledge_enqueued_decisions(&mut self.canwu)
            .expect("acknowledge tickets");
        self.settle(now, "acknowledgement boundary");
        assert!(self.law().pending_outbox().next().is_none());
    }

    /// Resolves each listed seat's open ticket for `proposal` with `option`.
    fn vote(&mut self, proposal: &str, ballots: &[(u64, &str)]) {
        let law = self.law();
        let procedure = &law.procedures[&format!("procedure:{proposal}")];
        for (index, option) in ballots {
            let item = law
                .outbox
                .values()
                .find(|item| {
                    item.proposal.id == proposal
                        && item.seat == seat(*index)
                        && item.stage == procedure.active_stage
                        && item.round == procedure.round
                })
                .expect("seat ticket in the active stage")
                .clone();
            let ticket_id = DecisionTicketId::new(item.ticket_id);
            let ticket_version = self
                .canwu
                .decision_ticket(ticket_id)
                .expect("open seat ticket")
                .version;
            let mut policy = QueuedHumanPolicy::new("canwu-law-human-seat", "1");
            policy
                .submit(
                    ticket_id,
                    HumanDecisionResponse {
                        ticket_version,
                        option_id: (*option).to_owned(),
                        operator_id: seat(*index),
                    },
                )
                .expect("queue seat ballot");
            self.canwu
                .drive_decision(
                    self.canwu.time(),
                    0,
                    DecisionRequestId::new(item.resolution_request_id),
                    Some(CommandRequestId::new(item.nested_command_request_id)),
                    ticket_id,
                    &policy,
                )
                .expect("drive seat ballot");
            // Each resolution expects the current revision, so ballots settle
            // one boundary at a time; the resulting intent joins the next one.
            let now = self.canwu.time().as_minutes();
            self.settle(now, "ballot decision boundary");
        }
        let now = self.canwu.time().as_minutes();
        self.settle(now, "legal intent boundary");
    }

    /// Save/load and exact replay reproduce the run and its legal ledger.
    fn assert_save_load_and_exact_replay(&self) {
        let plugins: [&dyn SimulationPlugin; 1] = [&LawPlugin];
        let snapshot = self.canwu.snapshot_json().expect("snapshot");
        let restored =
            Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
        assert_eq!(
            restored.snapshot_json().expect("restored snapshot"),
            snapshot
        );
        let restored_law = load_legal_runtime(&restored, &self.plan)
            .expect("load restored legal runtime")
            .expect("restored legal runtime");
        assert_eq!(restored_law, self.law());
        let replayed = Canwu::replay_from_journal(&plugins, &self.canwu.replay_journal())
            .expect("exact replay");
        assert_eq!(
            replayed.snapshot_json().expect("replayed snapshot"),
            snapshot
        );
    }
}

fn status(law: &LegalRuntime, proposal: &str) -> ProposalStatus {
    law.proposals[proposal].status
}

fn ballots_of(law: &LegalRuntime, proposal: &str, stage: &str) -> Vec<(String, Ballot)> {
    law.participations
        .iter()
        .filter(|participation| {
            participation.procedure.id == format!("procedure:{proposal}")
                && participation.stage == stage
        })
        .map(|participation| (participation.seat.clone(), participation.ballot))
        .collect()
}

fn rejects(definition: &LegalDefinition) -> bool {
    compile_law(definition).is_err()
}

/// Eight seats in eight unit blocks with a block threshold of five and a chair
/// casting seat, a divided two-seat block, a rank-weighted stage whose outcome
/// differs from equal seat counting, and late seat responses to a stage that
/// already passed.
#[test]
#[allow(clippy::too_many_lines)]
fn gap_g28_law_weighted_block_stage() {
    let mut unit = stage("council", ProcedureStageKind::Deliberation, 8);
    unit.quorum = 8;
    unit.block_of_seat = (1..=8)
        .map(|index| (seat(index), format!("block-{index}")))
        .collect();
    unit.block_threshold = Some(5);
    // Seats 1 and 2 share one block; seat 3, the casting seat, and seat 4 each
    // form their own. Two of three blocks carry the stage.
    let mut banner = stage("banner", ProcedureStageKind::Deliberation, 4);
    banner.quorum = 4;
    banner.block_of_seat = BTreeMap::from([
        (seat(1), "block-a".to_owned()),
        (seat(2), "block-a".to_owned()),
        (seat(3), "block-b".to_owned()),
        (seat(4), "block-c".to_owned()),
    ]);
    banner.block_threshold = Some(2);
    // The senior seat weighs 3 and the two junior seats 1 each; a quorum of 3
    // lets the senior seat alone carry a weighted stage.
    let mut ranked = stage("nominate", ProcedureStageKind::Deliberation, 3);
    ranked.quorum = 3;
    ranked.threshold = 501;
    ranked.seat_weights = BTreeMap::from([(seat(1), 3)]);
    let mut equal = ranked.clone();
    equal.seat_weights.clear();
    let authored = definition(
        8,
        vec![
            procedure(
                "chaired-council",
                &format!("{PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX}{}", seat(1)),
                vec![unit],
            ),
            procedure(
                "banner-council",
                &format!("{PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX}{}", seat(3)),
                vec![banner],
            ),
            procedure(
                "ranked-nomination",
                PROCEDURE_TIE_BREAK_STATUS_QUO,
                vec![ranked],
            ),
            procedure(
                "equal-nomination",
                PROCEDURE_TIE_BREAK_STATUS_QUO,
                vec![equal],
            ),
        ],
    );

    // Unused weight and block fields stay out of the canonical encoding, and an
    // explicit unit weight compiles to the same plan as an omitted one.
    let encoded = serde_json::to_value(stage("plain", ProcedureStageKind::Review, 1))
        .expect("encode plain stage");
    for field in ["seat_weights", "block_of_seat", "block_threshold"] {
        assert!(encoded.get(field).is_none(), "{field} must be omitted");
    }
    let mut unit_weight = authored.clone();
    unit_weight.procedures[2].stages[0]
        .seat_weights
        .insert(seat(2), 1);
    assert_eq!(
        compile_law(&unit_weight).expect("unit weight").content_hash,
        compile_law(&authored).expect("plan").content_hash
    );

    // Invalid weight, block, and tie-break contracts fail when the plan compiles.
    let with_chaired_stage = |edit: &dyn Fn(&mut ProcedureStageDefinition)| {
        let mut invalid = authored.clone();
        edit(&mut invalid.procedures[0].stages[0]);
        invalid
    };
    for invalid in [
        with_chaired_stage(&|stage| {
            stage.seat_weights.insert(seat(2), 0);
        }),
        with_chaired_stage(&|stage| {
            stage.seat_weights.insert(seat(9), 2);
        }),
        with_chaired_stage(&|stage| {
            stage.block_of_seat.remove(&seat(8));
        }),
        with_chaired_stage(&|stage| {
            stage.block_of_seat.insert(seat(9), "block-9".to_owned());
        }),
        with_chaired_stage(&|stage| stage.block_threshold = Some(9)),
        with_chaired_stage(&|stage| stage.block_threshold = Some(0)),
        with_chaired_stage(&|stage| stage.block_threshold = None),
        // A casting seat needs a majority block threshold to always matter.
        with_chaired_stage(&|stage| stage.block_threshold = Some(4)),
        with_chaired_stage(&|stage| stage.quorum = 9),
        with_chaired_stage(&|stage| {
            stage.block_of_seat.clear();
        }),
    ] {
        assert!(rejects(&invalid));
    }
    let mut unknown_tie_break = authored.clone();
    "seat-id".clone_into(&mut unknown_tie_break.procedures[0].deterministic_tie_break);
    assert!(rejects(&unknown_tie_break));
    let mut outside_chair = authored.clone();
    outside_chair.procedures[0].deterministic_tie_break =
        format!("{PROCEDURE_TIE_BREAK_CASTING_SEAT_PREFIX}{}", seat(9));
    assert!(rejects(&outside_chair));

    let mut council = Council::new(&authored, 8);
    council.submit(vec![
        proposal("chair-for", "chaired-council", 60),
        proposal("chair-against", "chaired-council", 60),
        proposal("banner-for", "banner-council", 60),
        proposal("banner-against", "banner-council", 60),
        proposal("ranked", "ranked-nomination", 60),
        proposal("equal", "equal-nomination", 60),
    ]);
    council.open_tickets();

    // Both councils split four blocks to four; only the chair's side differs.
    let first_half_for = [
        (1, "for"),
        (2, "for"),
        (3, "for"),
        (4, "for"),
        (5, "against"),
        (6, "against"),
        (7, "against"),
        (8, "against"),
    ];
    let first_half_against = first_half_for
        .map(|(index, option)| (index, if option == "for" { "against" } else { "for" }));
    council.vote("chair-for", &first_half_for);
    council.vote("chair-against", &first_half_against);
    // Block A splits one to one and takes no position, leaving block B against
    // block C: the casting seat of block B decides. Were the divided block For,
    // "banner-against" would pass; were it Against, "banner-for" would fail.
    council.vote(
        "banner-for",
        &[(1, "for"), (2, "against"), (3, "for"), (4, "against")],
    );
    council.vote(
        "banner-against",
        &[(1, "for"), (2, "against"), (3, "against"), (4, "for")],
    );
    // The same ballots nominate only when the senior seat is weighted: its
    // weight alone meets quorum and threshold, and its stage passes at once.
    council.vote("ranked", &[(1, "for")]);
    council.vote("equal", &[(1, "for"), (2, "against"), (3, "against")]);

    let law = council.law();
    for adopted in ["chair-for", "banner-for", "ranked"] {
        assert_eq!(status(&law, adopted), ProposalStatus::Adopted);
    }
    for undecided in ["chair-against", "banner-against", "equal"] {
        assert_eq!(status(&law, undecided), ProposalStatus::Submitted);
    }
    assert_eq!(ballots_of(&law, "chair-for", "council").len(), 8);
    assert_eq!(law.sources.len(), 3);

    // The passed stage's remaining seat work expired, so the junior seats'
    // late answers to their still-open tickets are recorded as rejected
    // intent outcomes and the boundary succeeds.
    let seat_work = law
        .outbox
        .values()
        .filter(|item| item.proposal.id == "ranked")
        .map(|item| (item.seat.clone(), item.dispatch))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        seat_work,
        BTreeMap::from([
            (seat(1), DispatchState::Acknowledged),
            (seat(2), DispatchState::Expired),
            (seat(3), DispatchState::Expired),
        ])
    );
    assert!(law.pending_outbox().next().is_none());
    council.vote("ranked", &[(2, "against"), (3, "against")]);
    let law = council.law();
    assert_eq!(
        ballots_of(&law, "ranked", "nominate"),
        [(seat(1), Ballot::For)]
    );
    for late_seat in [seat(2), seat(3)] {
        let prefix = format!("intent:procedure:ranked:0:0:{late_seat}:");
        let late = law
            .intent_outcomes
            .iter()
            .find(|outcome| outcome.intent.starts_with(&prefix))
            .expect("late seat response outcome");
        assert_eq!(late.status, LegalIntentStatus::Rejected);
    }
    assert_eq!(status(&law, "ranked"), ProposalStatus::Adopted);

    // The undecided stages cannot pass later; their deadline expires them.
    council.settle(61, "deadline boundary");
    let law = council.law();
    for undecided in ["chair-against", "banner-against", "equal"] {
        assert_eq!(status(&law, undecided), ProposalStatus::Expired);
    }
    assert!(law.open_procedures.is_empty());

    council.assert_save_load_and_exact_replay();
}

/// An advisory consultation records ballots as evidence, never decides, and
/// completes at its deadline by opening the deciding stage.
#[test]
#[allow(clippy::too_many_lines)]
fn gap_g29_law_consultation_stage() {
    let consult = stage("consult", ProcedureStageKind::Consultation, 3);
    let mut decide = stage("decide", ProcedureStageKind::Deliberation, 3);
    decide.quorum = 3;
    decide.threshold = 500;
    let consulted = procedure(
        "consulted-decision",
        PROCEDURE_TIE_BREAK_STATUS_QUO,
        vec![consult.clone(), decide.clone()],
    );
    let authored = definition(3, vec![consulted.clone()]);

    // A consultation needs a deadline, never counts ballots, and never decides last.
    let with_procedure = |edit: &dyn Fn(&mut ProcedureProfileDefinition)| {
        let mut invalid = authored.clone();
        edit(&mut invalid.procedures[0]);
        invalid
    };
    for invalid in [
        with_procedure(&|profile| profile.stages[0].deadline_minutes = 0),
        with_procedure(&|profile| profile.stages[0].quorum = 1),
        with_procedure(&|profile| profile.stages[0].threshold = 500),
        with_procedure(&|profile| {
            profile.stages[0].seat_weights.insert(seat(1), 2);
        }),
        with_procedure(&|profile| profile.stages.reverse()),
        with_procedure(&|profile| profile.stages.truncate(1)),
    ] {
        assert!(rejects(&invalid));
    }

    let mut council = Council::new(&authored, 3);
    council.submit(vec![
        proposal("silent", "consulted-decision", 60),
        proposal("advised", "consulted-decision", 60),
    ]);
    council.open_tickets();

    // Unanimous support would pass a deciding stage; a consultation only records it.
    council.vote("advised", &[(1, "for"), (2, "for"), (3, "against")]);
    let law = council.law();
    for id in ["silent", "advised"] {
        let procedure = &law.procedures[&format!("procedure:{id}")];
        assert_eq!(procedure.active_stage, 0);
        assert!(!procedure.closed);
        assert_ne!(status(&law, id), ProposalStatus::Adopted);
    }
    assert_eq!(
        ballots_of(&law, "advised", "consult"),
        vec![
            (seat(1), Ballot::For),
            (seat(2), Ballot::For),
            (seat(3), Ballot::Against)
        ]
    );
    assert!(ballots_of(&law, "silent", "consult").is_empty());

    // Consultation tickets tell their controllers that the ballot is advisory.
    for item in law.outbox.values() {
        let ticket = council
            .canwu
            .decision_ticket(DecisionTicketId::new(item.ticket_id))
            .expect("consultation ticket");
        assert_eq!(ticket.context.payload["advisory"], serde_json::json!(true));
    }

    // The deadline wake completes both consultations, with or without ballots.
    council.settle(61, "consultation deadline boundary");
    // Open procedures holding expired consultation work persist and replay.
    council.assert_save_load_and_exact_replay();
    let law = council.law();
    for id in ["silent", "advised"] {
        let procedure = &law.procedures[&format!("procedure:{id}")];
        assert_eq!(procedure.active_stage, 1);
        assert_eq!(procedure.round, 1);
        assert_eq!(procedure.deadline, SimTime::from_minutes(121));
        assert!(!procedure.closed);
        assert_ne!(status(&law, id), ProposalStatus::Expired);
    }
    // Unanswered consultation seat work expired with the stage; answered work
    // stays acknowledged, and the advisory ballots remain as evidence.
    let consultation_dispatch = |id: &str| {
        law.outbox
            .values()
            .filter(|item| item.proposal.id == id && item.stage == 0)
            .map(|item| item.dispatch)
            .collect::<Vec<_>>()
    };
    assert_eq!(consultation_dispatch("silent"), [DispatchState::Expired; 3]);
    assert_eq!(
        consultation_dispatch("advised"),
        [DispatchState::Acknowledged; 3]
    );
    assert_eq!(ballots_of(&law, "advised", "consult").len(), 3);

    // Identical deciding ballots give identical outcomes whatever was advised.
    council.open_tickets();
    let law = council.law();
    for item in law.outbox.values().filter(|item| item.stage == 1) {
        let ticket = council
            .canwu
            .decision_ticket(DecisionTicketId::new(item.ticket_id))
            .expect("deciding ticket");
        assert!(ticket.context.payload.get("advisory").is_none());
    }
    for id in ["silent", "advised"] {
        council.vote(id, &[(1, "against"), (2, "for"), (3, "for")]);
    }
    let law = council.law();
    for id in ["silent", "advised"] {
        assert_eq!(status(&law, id), ProposalStatus::Adopted);
        assert_eq!(ballots_of(&law, id, "decide").len(), 3);
    }
    assert_eq!(ballots_of(&law, "advised", "consult").len(), 3);

    council.assert_save_load_and_exact_replay();
}
