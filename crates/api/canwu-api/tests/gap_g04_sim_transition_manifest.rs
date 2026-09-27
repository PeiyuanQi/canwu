//! Gap G-04: a transition manifest makes an omitted participant fail closed.
//!
//! A coordinating plugin registers a two-participant manifest one boundary
//! ahead. At the ready boundary both participants stage their writes in
//! phase 10, and a phase-12 auditor reads the committed audit. A silent
//! participant, a stale expected pre-version, staging by an unlisted plugin,
//! or a wrong expected post-version fails the whole boundary with nothing
//! written, including a same-boundary write already applied at phase 11.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProposal,
    BoundaryRequest, BoundarySystemContract, BoundarySystemHandler, Canwu, CanwuError,
    DomainRecordClass, DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef,
    DomainRecordSchema, EntityRef, ErrorCode, PendingTransitionManifest, PluginRegistrar,
    SimDuration, SimTime, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence, TransitionAuditOutcome, TransitionAuditRecord, TransitionManifest,
    TransitionManifestId, TransitionParticipant, TransitionParticipantAudit,
    TransitionRecordVersion,
};
use serde_json::{Value, json};

const COURT: &str = "fixture-court";
const ESTATES: &str = "fixture-estates";
const OUTSIDER: &str = "fixture-outsider";
const LINEAGE: &str = "succession";

fn title_kind() -> DomainRecordKind {
    DomainRecordKind::new(COURT, "title")
}

fn title() -> DomainRecordRef {
    DomainRecordRef {
        kind: title_kind(),
        id: "crown".to_owned(),
    }
}

fn title_state() -> StateKey {
    DomainRecordSchema::new(title_kind(), DomainRecordClass::Record).state_key()
}

fn audit_state() -> StateKey {
    StateKey::new(COURT, "audits")
}

fn holdings_state() -> StateKey {
    StateKey::new(ESTATES, "holdings")
}

fn government() -> EntityRef {
    EntityRef::Government(Canwu::demo_ids().government)
}

fn manifest_id() -> TransitionManifestId {
    TransitionManifestId {
        coordinator: COURT.to_owned(),
        lineage_id: LINEAGE.to_owned(),
        attempt: 1,
    }
}

/// Manifests ready at this boundary that list `plugin`.
fn ready_manifests(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    plugin: &str,
) -> Result<Vec<TransitionManifestId>, CanwuError> {
    Ok(view
        .transition_manifests()?
        .into_iter()
        .filter(|pending| pending.manifest.ready_at == context.boundary_id && pending.lists(plugin))
        .map(PendingTransitionManifest::id)
        .collect())
}

fn create_title(context: &BoundaryContext) -> Vec<BoundaryDirective> {
    if context.boundary_id.get() != 1 {
        return Vec::new();
    }
    vec![BoundaryDirective::MutateRecord {
        mutation: DomainRecordMutation::Create {
            record: DomainRecordDraft::new(title(), json!({"holder": "elder"})),
        },
        summary: "The crown is held by the elder".to_owned(),
    }]
}

fn chancery(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: create_title(context),
        ..BoundaryProposal::default()
    })
}

/// Also appoints a regent at the ready boundary, before phase 10, so the
/// manifest's expected pre-version is stale when the transition settles.
fn reforming_chancery(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = create_title(context);
    if context.boundary_id.get() == 3 {
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Update {
                record: DomainRecordDraft::new(title(), json!({"holder": "regent"})),
                expected_version: 1,
            },
            summary: "A regent holds the crown".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Registers the succession one boundary ahead of its ready boundary,
/// expecting the court's write to advance the title by `post_advance`.
fn register_succession(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    post_advance: u64,
) -> Result<BoundaryProposal, CanwuError> {
    if context.boundary_id.get() != 2 {
        return Ok(BoundaryProposal::default());
    }
    let current = view
        .current_domain_record_version(&title())?
        .expect("the title exists before the succession is declared");
    let post = TransitionRecordVersion {
        record: title(),
        version: current.version + post_advance,
    };
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::RegisterTransitionManifest {
            manifest: TransitionManifest {
                lineage_id: LINEAGE.to_owned(),
                attempt: 1,
                participants: vec![
                    TransitionParticipant {
                        plugin: COURT.to_owned(),
                        expected_pre: vec![current],
                        expected_post: vec![post],
                    },
                    TransitionParticipant::new(ESTATES),
                ],
                ready_at: BoundaryId::new(context.boundary_id.get() + 1),
            },
        }],
        ..BoundaryProposal::default()
    })
}

fn clerk(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    register_succession(view, context, 1)
}

/// Declares a post-version the court's single update cannot produce.
fn misdeclaring_clerk(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    register_succession(view, context, 2)
}

fn court_transition(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for manifest_id in ready_manifests(view, context, COURT)? {
        let version = view
            .domain_record(&title())?
            .expect("the title exists at the ready boundary")
            .version;
        directives.push(BoundaryDirective::StageTransitionWrite {
            manifest_id,
            writes: vec![BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Update {
                    record: DomainRecordDraft::new(title(), json!({"holder": "heir"})),
                    expected_version: version,
                },
                summary: "The heir succeeds to the crown".to_owned(),
            }],
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// A phase-12 system that reports what the kernel audited.
fn auditor(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let directives = view
        .transition_audits()?
        .iter()
        .map(|audit| BoundaryDirective::SetComponent {
            state: audit_state(),
            entity: government(),
            component: "transition_audit".to_owned(),
            value: json!({
                "lineage": audit.manifest_id.lineage_id,
                "outcome": audit.outcome,
                "staged": audit
                    .participants
                    .iter()
                    .map(|participant| json!([participant.plugin, participant.staged_writes]))
                    .collect::<Vec<_>>(),
            }),
            summary: "The court recorded the transition audit".to_owned(),
        })
        .collect();
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn estates_transition(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: ready_manifests(view, context, ESTATES)?
            .into_iter()
            .map(|manifest_id| BoundaryDirective::StageTransitionWrite {
                manifest_id,
                writes: vec![BoundaryDirective::SetComponent {
                    state: holdings_state(),
                    entity: government(),
                    component: "succession".to_owned(),
                    value: json!({"estates_pass_to": "heir"}),
                    summary: "The estates pass to the heir".to_owned(),
                }],
            })
            .collect(),
        ..BoundaryProposal::default()
    })
}

fn silent_estates(
    _view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal::default())
}

/// Stages into the succession although the manifest does not list it. The
/// manifest is invisible to this plugin, so it names the ID directly.
fn meddle(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    assert!(view.transition_manifests()?.is_empty());
    if context.boundary_id.get() != 3 {
        return Ok(BoundaryProposal::default());
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::StageTransitionWrite {
            manifest_id: manifest_id(),
            writes: Vec::new(),
        }],
        ..BoundaryProposal::default()
    })
}

fn contract(
    name: &str,
    phase: BoundaryPhase,
    reads: Vec<StateKey>,
    writes: Vec<StateKey>,
    visibility: StateVisibility,
) -> BoundarySystemContract {
    let mut contract = BoundarySystemContract::new(name, phase, SystemCadence::Daily);
    contract.reads = reads;
    contract.writes = writes;
    contract.visibility = visibility;
    contract
}

#[derive(Clone, Copy)]
enum CourtMode {
    Faithful,
    Reforming,
    Misdeclaring,
}

struct CourtPlugin(CourtMode);

impl SimulationPlugin for CourtPlugin {
    fn name(&self) -> &'static str {
        COURT
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        match self.0 {
            CourtMode::Faithful => {
                "0000000000000000000000000000000000000000000000000000000000000401"
            }
            CourtMode::Reforming => {
                "0000000000000000000000000000000000000000000000000000000000000402"
            }
            CourtMode::Misdeclaring => {
                "0000000000000000000000000000000000000000000000000000000000000406"
            }
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            title_kind(),
            DomainRecordClass::Record,
        ))?;
        let (chancery_handler, clerk_handler): (BoundarySystemHandler, BoundarySystemHandler) =
            match self.0 {
                CourtMode::Faithful => (chancery, clerk),
                CourtMode::Reforming => (reforming_chancery, clerk),
                CourtMode::Misdeclaring => (chancery, misdeclaring_clerk),
            };
        registrar.register_boundary_system(
            contract(
                "chancery",
                BoundaryPhase::DomainDeltaProposal,
                Vec::new(),
                vec![title_state()],
                StateVisibility::SameBoundary,
            ),
            chancery_handler,
        )?;
        // Next-boundary visibility: the post-version check reads the staged
        // write before it commits at the end of the boundary.
        registrar.register_boundary_system(
            contract(
                "transition",
                BoundaryPhase::HistoricalCandidateEvaluation,
                vec![title_state(), StateKey::core_transitions()],
                vec![title_state(), StateKey::core_transitions()],
                StateVisibility::NextBoundary,
            ),
            court_transition,
        )?;
        registrar.register_boundary_system(
            contract(
                "clerk",
                BoundaryPhase::StrategicAggregation,
                vec![title_state(), StateKey::core_transitions()],
                vec![StateKey::core_transitions()],
                StateVisibility::NextBoundary,
            ),
            clerk_handler,
        )?;
        registrar.register_boundary_system(
            contract(
                "auditor",
                BoundaryPhase::StrategicAggregation,
                vec![StateKey::core_transitions()],
                vec![audit_state()],
                StateVisibility::SameBoundary,
            ),
            auditor,
        )
    }
}

#[derive(Clone, Copy)]
enum EstatesMode {
    Staging,
    Silent,
}

struct EstatesPlugin(EstatesMode);

impl SimulationPlugin for EstatesPlugin {
    fn name(&self) -> &'static str {
        ESTATES
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        match self.0 {
            EstatesMode::Staging => {
                "0000000000000000000000000000000000000000000000000000000000000403"
            }
            EstatesMode::Silent => {
                "0000000000000000000000000000000000000000000000000000000000000404"
            }
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let handler = match self.0 {
            EstatesMode::Staging => estates_transition,
            EstatesMode::Silent => silent_estates,
        };
        registrar.register_boundary_system(
            contract(
                "transition",
                BoundaryPhase::HistoricalCandidateEvaluation,
                vec![StateKey::core_transitions()],
                vec![holdings_state(), StateKey::core_transitions()],
                StateVisibility::SameBoundary,
            ),
            handler,
        )
    }
}

struct OutsiderPlugin;

impl SimulationPlugin for OutsiderPlugin {
    fn name(&self) -> &'static str {
        OUTSIDER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000405"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_boundary_system(
            contract(
                "meddle",
                BoundaryPhase::HistoricalCandidateEvaluation,
                vec![StateKey::core_transitions()],
                vec![StateKey::core_transitions()],
                StateVisibility::SameBoundary,
            ),
            meddle,
        )
    }
}

fn daily(days: i64) -> BoundaryRequest {
    BoundaryRequest::at(SimTime::EPOCH + SimDuration::days(days)).with_cadence(SystemCadence::Daily)
}

/// Founds the title at boundary 1 and registers the manifest, ready at
/// boundary 3, at boundary 2.
fn registered_run(plugins: &[&dyn SimulationPlugin]) -> Canwu {
    let mut canwu = Canwu::demo(4040).expect("demo");
    for plugin in plugins {
        canwu.register_plugin(*plugin).expect("fixture plugin");
    }
    canwu.settle_boundary(daily(1)).expect("founding boundary");
    canwu
        .settle_boundary(daily(2))
        .expect("registration boundary");
    canwu
}

fn audit(outcome: TransitionAuditOutcome, court: u64, estates: u64) -> TransitionAuditRecord {
    TransitionAuditRecord {
        manifest_id: manifest_id(),
        ready_at: BoundaryId::new(3),
        outcome,
        participants: vec![
            TransitionParticipantAudit {
                plugin: COURT.to_owned(),
                staged_writes: court,
            },
            TransitionParticipantAudit {
                plugin: ESTATES.to_owned(),
                staged_writes: estates,
            },
        ],
    }
}

fn component_change(canwu: &Canwu, boundary: usize, component: &str) -> Option<Value> {
    canwu.boundaries()[boundary]
        .changes
        .iter()
        .find(|change| change.component == component)
        .map(|change| change.value.clone())
}

/// Settles the ready boundary and requires it to fail closed with nothing
/// written: the snapshot, including the pending manifest, is unchanged.
fn assert_fails_closed(mut canwu: Canwu, code: &ErrorCode) -> CanwuError {
    let before = canwu.snapshot();
    let error = canwu
        .settle_boundary(daily(3))
        .expect_err("the transition must fail closed");
    assert_eq!(&error.code, code, "{error}");
    assert_eq!(canwu.snapshot(), before);
    assert_eq!(
        canwu.domain_record(&title()).map(|record| record.version),
        Some(1)
    );
    assert_eq!(
        canwu
            .pending_transition_manifests()
            .map(PendingTransitionManifest::id)
            .collect::<Vec<_>>(),
        vec![manifest_id()]
    );
    error
}

#[test]
fn gap_g04_sim_transition_manifest() {
    let court = CourtPlugin(CourtMode::Faithful);
    let estates = EstatesPlugin(EstatesMode::Staging);
    let plugins: [&dyn SimulationPlugin; 2] = [&court, &estates];

    // The manifest is registered ahead of its ready boundary and persisted.
    let mut canwu = registered_run(&plugins);
    let pending: Vec<_> = canwu.pending_transition_manifests().cloned().collect();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id(), manifest_id());
    assert_eq!(pending[0].manifest.ready_at, BoundaryId::new(3));
    assert_eq!(canwu.boundaries()[1].transition_manifests, pending);
    let before_ready = canwu.fork();
    let saved = canwu
        .snapshot_json()
        .expect("snapshot with a pending manifest");
    let mut stripped: Value = serde_json::from_str(&saved).expect("snapshot value");
    stripped
        .as_object_mut()
        .expect("snapshot object")
        .remove("pending_transition_manifests");
    assert_eq!(
        Canwu::from_snapshot_json_with_plugins(&stripped.to_string(), &plugins)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::InvalidSnapshot),
        "a snapshot must not drop a pending manifest its boundaries registered"
    );

    // (a) Both participants stage: the transition commits and a phase-12
    // system reads the committed audit.
    let receipt = canwu.settle_boundary(daily(3)).expect("ready boundary");
    let committed = audit(TransitionAuditOutcome::Committed, 1, 1);
    assert_eq!(receipt.transition_audits, vec![committed.clone()]);
    assert_eq!(canwu.boundaries()[2].transition_audits, vec![committed]);
    assert_eq!(canwu.pending_transition_manifests().count(), 0);
    let crown = canwu.domain_record(&title()).expect("title");
    assert_eq!(
        (crown.version, crown.payload.clone()),
        (2, json!({"holder": "heir"}))
    );
    assert_eq!(
        component_change(&canwu, 2, "succession"),
        Some(json!({"estates_pass_to": "heir"}))
    );
    assert_eq!(
        component_change(&canwu, 2, "transition_audit"),
        Some(json!({
            "lineage": LINEAGE,
            "outcome": "committed",
            "staged": [[COURT, 1], [ESTATES, 1]],
        }))
    );

    // A fork and a restored save settle the ready boundary identically, and
    // exact replay regenerates the registration and audit evidence.
    let mut fork = before_ready.fork();
    assert_eq!(fork.settle_boundary(daily(3)).expect("fork"), receipt);
    let mut restored =
        Canwu::from_snapshot_json_with_plugins(&saved, &plugins).expect("restore pending manifest");
    assert_eq!(
        restored
            .pending_transition_manifests()
            .cloned()
            .collect::<Vec<_>>(),
        pending
    );
    assert_eq!(
        restored.settle_boundary(daily(3)).expect("restored"),
        receipt
    );
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let json = canwu.snapshot_json().expect("final snapshot");
    let reloaded = Canwu::from_snapshot_json_with_plugins(&json, &plugins).expect("reload");
    assert_eq!(reloaded.snapshot(), canwu.snapshot());
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.snapshot(), canwu.snapshot());
    assert_eq!(replayed.boundaries(), canwu.boundaries());

    // A ready boundary that runs no participant expires the manifest.
    let mut expired = before_ready;
    let receipt = expired
        .settle_boundary(BoundaryRequest::at(expired.time()))
        .expect("cadence-free ready boundary");
    assert_eq!(
        receipt.transition_audits,
        vec![audit(TransitionAuditOutcome::Expired, 0, 0)]
    );
    assert_eq!(expired.pending_transition_manifests().count(), 0);
    assert_eq!(
        expired.domain_record(&title()).map(|record| record.version),
        Some(1)
    );

    // (b) A silent participant fails the whole boundary before any write is
    // applied, and the error names the missing participant.
    let silent = EstatesPlugin(EstatesMode::Silent);
    let error = assert_fails_closed(
        registered_run(&[&court, &silent]),
        &ErrorCode::TransitionParticipantMissing,
    );
    assert!(error.message.contains(ESTATES) && error.message.contains(LINEAGE));

    // (c) A pre-version that changed after registration is rejected.
    let reforming = CourtPlugin(CourtMode::Reforming);
    let error = assert_fails_closed(
        registered_run(&[&reforming, &estates]),
        &ErrorCode::TransitionVersionMismatch,
    );
    assert_eq!(error.related_entities, vec![EntityRef::Domain(title())]);

    // (d) A plugin the manifest does not list cannot stage into it.
    let outsider = OutsiderPlugin;
    let error = assert_fails_closed(
        registered_run(&[&court, &estates, &outsider]),
        &ErrorCode::InvalidAuthority,
    );
    assert!(error.message.contains(OUTSIDER));

    // (e) A wrong post-version fails after phase 11 committed the estates'
    // same-boundary write, which rolls back with the rest of the boundary.
    let misdeclaring = CourtPlugin(CourtMode::Misdeclaring);
    let error = assert_fails_closed(
        registered_run(&[&misdeclaring, &estates]),
        &ErrorCode::TransitionVersionMismatch,
    );
    assert!(error.message.contains("expected_post"), "{error}");
    assert_eq!(error.related_entities, vec![EntityRef::Domain(title())]);
}
