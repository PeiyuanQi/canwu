//! Load-time refusal of initial-scenario evidence declarations no seal can
//! accept, knowledge publication reservations, and the request ID a queued
//! command must carry.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const DECLARING: &str = "fixture-initial-declarations";

fn continuation_kind() -> DomainRecordKind {
    DomainRecordKind::new("fixture.initial", "continuation")
}

fn identity_kind() -> DomainRecordKind {
    DomainRecordKind::new("fixture.initial", "identity")
}

/// Registers one kind declaring a payload-required continuation and one
/// declaring identity-only dependencies.
struct InitialDeclarationPlugin;

impl SimulationPlugin for InitialDeclarationPlugin {
    fn name(&self) -> &'static str {
        DECLARING
    }

    test_plugin_identity!("00000000000000000000000000000000000000000000000000000000000e2701");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        for (kind, field, property) in [
            (
                continuation_kind(),
                PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
                payload_required_evidence_continuation_property_v1(),
            ),
            (
                identity_kind(),
                IDENTITY_EVIDENCE_DEPENDENCIES_FIELD,
                identity_evidence_dependencies_property_v1(),
            ),
        ] {
            let mut schema = DomainRecordSchema::new(kind, DomainRecordClass::Record);
            schema.payload_schema = PayloadSchema::Object {
                properties: BTreeMap::from([(field.to_owned(), property)]),
                allow_additional: false,
            };
            registrar.register_record_schema(schema)?;
        }
        Ok(())
    }
}

fn initial_record(kind: DomainRecordKind, id: &str, payload: Value) -> DomainRecord {
    DomainRecord {
        reference: DomainRecordRef {
            kind,
            id: id.to_owned(),
        },
        owner: DECLARING.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload,
        references: Vec::new(),
    }
}

fn continuation(value: &PayloadRequiredEvidenceContinuationV1) -> Value {
    serde_json::json!({ PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD: value })
}

fn identity(dependencies: Vec<EvidenceRef>) -> Value {
    serde_json::json!({
        IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: IdentityEvidenceDependenciesV1::new(dependencies),
    })
}

/// The initial-scenario version of the completed continuation record
/// `anchor`, which every bad declaration below names.
fn initial_anchor_version() -> EvidenceRef {
    EvidenceRef::DomainRecordVersion(DomainRecordVersionRef {
        record: DomainRecordRef {
            kind: continuation_kind(),
            id: "anchor".to_owned(),
        },
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    })
}

fn declaring_scenario(declaring: DomainRecord) -> Scenario {
    let (mut scenario, _) = demo_scenario();
    scenario.domain_records = vec![
        initial_record(
            continuation_kind(),
            "anchor",
            continuation(&PayloadRequiredEvidenceContinuationV1::completed()),
        ),
        declaring,
    ];
    scenario
}

/// A seal refuses a current record whose payload-required continuation or
/// identity dependencies name an initial-scenario version, so an initial
/// record declaring one would block every seal and continuation checkpoint
/// until a boundary rewrote it. Scenario creation and every restore refuse
/// it instead, host-side; a declaration a seal accepts still loads.
#[test]
fn an_initial_record_declaring_initial_scenario_evidence_is_refused_at_load() {
    let bad = [
        initial_record(
            continuation_kind(),
            "pending",
            continuation(&PayloadRequiredEvidenceContinuationV1::active(vec![
                initial_anchor_version(),
            ])),
        ),
        initial_record(
            identity_kind(),
            "cited",
            identity(vec![initial_anchor_version()]),
        ),
    ];
    for declaring in &bad {
        let error = Simulation::new_with_plugins(
            2_701,
            declaring_scenario(declaring.clone()),
            &[&InitialDeclarationPlugin],
        )
        .err()
        .unwrap_or_else(|| panic!("{} must be refused at creation", declaring.reference));
        assert_eq!(error.code, ErrorCode::InvalidDomainRecord, "{error}");
        assert!(
            error
                .message
                .contains("declares evidence no seal can accept"),
            "{error}"
        );
    }

    // Declarations a seal accepts load, and the run seals.
    let mut scenario = declaring_scenario(initial_record(
        identity_kind(),
        "cited",
        identity(Vec::new()),
    ));
    scenario.domain_records.push(initial_record(
        continuation_kind(),
        "pending",
        continuation(&PayloadRequiredEvidenceContinuationV1::completed()),
    ));
    let simulation = Simulation::new_with_plugins(2_701, scenario, &[&InitialDeclarationPlugin])
        .expect("declarations a seal accepts load");
    let snapshot = simulation.snapshot();
    let compacted = simulation
        .into_compacted()
        .expect("the fixture enters compact mode");
    compacted
        .continuation_checkpoint()
        .expect("a run of declarations a seal accepts writes a continuation checkpoint");

    // A save whose initial scenario carries a bad declaration is refused
    // on restore, even with every identity bound to it.
    for declaring in bad {
        let mut forged = snapshot.clone();
        for records in [
            &mut forged
                .initial_scenario
                .as_mut()
                .expect("the save binds its initial scenario")
                .domain_records,
            &mut forged.domain_records,
        ] {
            let slot = records
                .iter_mut()
                .find(|record| record.reference == declaring.reference)
                .expect("the save holds the declaring record");
            *slot = declaring.clone();
        }
        let initial = forged
            .initial_scenario
            .clone()
            .expect("the save binds its initial scenario");
        let run_manifest = RunManifest::for_scenario("canwu.inline", "scenario", "1", &initial)
            .expect("the forged scenario binds a manifest");
        forged.run_manifest_hash = manifest::hash(&run_manifest).expect("the manifest hashes");
        forged.run_manifest = Some(run_manifest);
        let (_, authority_manifest_hash) = authoritative_run_identity(
            forged.run_manifest.as_ref().expect("forged manifest"),
            &forged.run_manifest_hash,
            forged
                .run_configuration
                .as_ref()
                .expect("forged configuration"),
        )
        .expect("the forged authority identity hashes");
        forged.authority_root_seed =
            fresh_authority_root_seed(forged.root_seed, &authority_manifest_hash)
                .expect("the forged authority root derives");
        rehash_tampered_snapshot(&mut forged);
        let error = Simulation::from_snapshot(forged)
            .err()
            .unwrap_or_else(|| panic!("{} must be refused on restore", declaring.reference));
        assert_eq!(error.code, ErrorCode::InvalidSnapshot, "{error}");
        assert!(
            error
                .message
                .contains("declares evidence no seal can accept"),
            "{error}"
        );
    }
}

const HOG: &str = "fixture-budget-hog";
const RESERVED: &str = "fixture-budget-reserved";
/// The largest payload one draft may carry, as a JSON string.
const MAX_DRAFT_BYTES: usize = KnowledgeLimitsV1::CURRENT.payload_bytes_per_record;

/// What the hog's system and the reserved plugin's system read, in payload
/// bytes; only the reservation test runs these plugins.
static HOG_READ: AtomicUsize = AtomicUsize::new(0);
static RESERVED_READ: AtomicUsize = AtomicUsize::new(0);
/// Whether the hog spends what it reads, or only reads it.
static HOG_SPENDS: AtomicBool = AtomicBool::new(true);

fn budget_schema(namespace: &str) -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(namespace, "line"), 1)
}

fn budget_draft(namespace: &str, payload: Value) -> KnowledgeRecordDraft {
    KnowledgeRecordDraft {
        schema: budget_schema(namespace),
        subjects: Vec::new(),
        payload,
        as_of: None,
        confidence_per_mille: 1_000,
        origin: KnowledgeOrigin {
            method: "fixture".to_owned(),
            evidence: Vec::new(),
        },
        supersedes: Vec::new(),
        contradicts: Vec::new(),
    }
}

fn publish_to_commander(namespace: &str, records: Vec<KnowledgeRecordDraft>) -> BoundaryDirective {
    BoundaryDirective::PublishKnowledge {
        visibility: StateVisibility::SameBoundary,
        producer_correlation: None,
        supersession: crate::KnowledgeSupersession::Explicit,
        groups: vec![crate::KnowledgePublicationGroup {
            holders: vec![KnowledgeHolderRef::Person(PersonId::new(1))],
            records,
        }],
        summary: format!("{namespace} publication"),
    }
}

/// Phase 4: spends every payload byte its budget shows, in distinct drafts
/// of at most [`MAX_DRAFT_BYTES`] and directives of at most the
/// per-directive bound.
fn hog(view: &SimulationView<'_>, _: &BoundaryContext) -> Result<BoundaryProposal, CanwuError> {
    let budget = view.knowledge_publication_budget();
    HOG_READ.store(budget.payload_bytes, Ordering::Relaxed);
    if !HOG_SPENDS.load(Ordering::Relaxed) {
        return Ok(BoundaryProposal::default());
    }
    let count = budget.payload_bytes.div_ceil(MAX_DRAFT_BYTES);
    let mut drafts = Vec::with_capacity(count);
    for index in 0..count {
        // Bytes spread evenly, each draft's JSON string tagged with its index.
        let bytes =
            budget.payload_bytes / count + usize::from(index < budget.payload_bytes % count);
        let mut text = format!("{index:06}");
        text.push_str(&"h".repeat(bytes - 2 - text.len()));
        drafts.push(budget_draft(HOG, Value::String(text)));
    }
    let mut directives = Vec::new();
    while !drafts.is_empty() {
        let rest = drafts.split_off(drafts.len().min(budget.records_per_batch));
        directives.push(publish_to_commander(HOG, drafts));
        drafts = rest;
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Phase 13: publishes one small record when its budget admits it, and
/// defers it otherwise.
fn reserved(
    view: &SimulationView<'_>,
    _: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let budget = view.knowledge_publication_budget();
    RESERVED_READ.store(budget.payload_bytes, Ordering::Relaxed);
    let draft = budget_draft(RESERVED, serde_json::json!({ "line": 1 }));
    let directives = if budget.admits(1, 1, draft.payload_bytes()?) {
        vec![publish_to_commander(RESERVED, vec![draft])]
    } else {
        Vec::new()
    };
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

struct BudgetPlugin {
    name: &'static str,
    phase: BoundaryPhase,
    cadence: SystemCadence,
    handler: BoundarySystemHandler,
}

impl SimulationPlugin for BudgetPlugin {
    fn name(&self) -> &'static str {
        self.name
    }

    test_plugin_identity!("00000000000000000000000000000000000000000000000000000000000e3301");

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: budget_schema(self.name),
            schema_hash: "00000000000000000000000000000000000000000000000000000000000e3302"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: Vec::new(),
            retention: crate::KnowledgeRetention::Keep,
        })?;
        let mut contract = BoundarySystemContract::new("publish", self.phase, self.cadence.clone());
        contract.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: budget_schema(self.name),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(contract, self.handler)
    }
}

const HOG_PLUGIN: BudgetPlugin = BudgetPlugin {
    name: HOG,
    phase: BoundaryPhase::PerceptionAndAttentionRefresh,
    cadence: SystemCadence::Daily,
    handler: hog,
};
const RESERVED_PLUGIN: BudgetPlugin = BudgetPlugin {
    name: RESERVED,
    phase: BoundaryPhase::PerspectiveAndReportMaterialization,
    cadence: SystemCadence::Daily,
    handler: reserved,
};

fn budget_run(plugins: &[&dyn SimulationPlugin]) -> Result<Simulation, CanwuError> {
    let (scenario, _) = demo_scenario();
    Simulation::new_with_plugins(3_301, scenario, plugins)
}

/// A publisher that runs earlier in dispatch order and spends everything it
/// is shown would take the whole budget at every boundary, and a later
/// publisher would defer forever. A reservation the host declares when it
/// registers the later plugin holds a share the earlier publisher is not
/// shown and cannot take; the later publisher reads it through the same
/// budget call, with unchanged code, and publishes. The share is held only
/// in a boundary where the reserving plugin's publisher is due, and only for
/// a plugin that has one. The reservation is part of the plugin's
/// descriptor: a restore validates the boundary against it, and a restore
/// that registers the plugin without it is refused.
#[test]
fn a_reserved_share_outlasts_an_earlier_publisher_that_spends_all_it_is_shown() {
    let limit = KnowledgeLimitsV1::CURRENT.payload_bytes_per_boundary;
    let daily = || BoundaryRequest::at(SimTime::EPOCH).with_cadence(SystemCadence::Daily);
    let published = |simulation: &Simulation, namespace: &str| {
        simulation
            .knowledge()
            .records
            .record_count_in_namespace(namespace)
    };

    // The share leaves the earlier publisher one full draft, so that the
    // boundary stays small; without the reservation it would publish the
    // whole budget and the later publisher would read nothing.
    let share = limit - MAX_DRAFT_BYTES;
    let reserved_plugin = KnowledgeReservedPlugin::new(
        &RESERVED_PLUGIN,
        KnowledgePublicationReservation::new(1, 1, share),
    );
    let mut run = budget_run(&[&HOG_PLUGIN, &reserved_plugin]).expect("the reserved run starts");
    run.settle_boundary(daily())
        .expect("the reserved boundary settles");
    assert_eq!(HOG_READ.load(Ordering::Relaxed), MAX_DRAFT_BYTES);
    assert_eq!(
        published(&run, HOG),
        1,
        "the earlier publisher spent all it read"
    );
    assert_eq!(RESERVED_READ.load(Ordering::Relaxed), share);
    assert_eq!(published(&run, RESERVED), 1, "the reservation publishes");

    let snapshot = run.snapshot();
    let restored =
        Simulation::from_snapshot_with_plugins(snapshot.clone(), &[&HOG_PLUGIN, &reserved_plugin])
            .expect("a restore validates the boundary against the reservation");
    assert_eq!(restored.checkpoint_hash(), run.checkpoint_hash());
    let error = Simulation::from_snapshot_with_plugins(snapshot, &[&HOG_PLUGIN, &RESERVED_PLUGIN])
        .err()
        .expect("a restore without the reservation is refused");
    assert_eq!(error.code, ErrorCode::PluginManifestMismatch, "{error}");
    drop((run, restored));

    // Reservations that together exceed the budget are refused when the
    // later plugin registers.
    let greedy_hog = KnowledgeReservedPlugin::new(
        &HOG_PLUGIN,
        KnowledgePublicationReservation::new(1, 1, limit),
    );
    let small_reserved = KnowledgeReservedPlugin::new(
        &RESERVED_PLUGIN,
        KnowledgePublicationReservation::new(1, 1, 1),
    );
    let error = budget_run(&[&greedy_hog, &small_reserved])
        .err()
        .expect("reservations past the budget are refused");
    assert_eq!(error.code, ErrorCode::InvalidPluginRegistration, "{error}");

    // A plugin with no knowledge-writing system cannot hold a share.
    let idle = KnowledgeReservedPlugin::new(
        &InitialDeclarationPlugin,
        KnowledgePublicationReservation::new(1, 1, 1),
    );
    let error = budget_run(&[&HOG_PLUGIN, &idle])
        .err()
        .expect("a reservation without a publisher is refused");
    assert_eq!(error.code, ErrorCode::InvalidPluginRegistration, "{error}");

    // A boundary in which the reserving plugin's publisher is not due holds
    // nothing for it.
    let monthly = BudgetPlugin {
        cadence: SystemCadence::Monthly,
        ..RESERVED_PLUGIN
    };
    let monthly_reserved =
        KnowledgeReservedPlugin::new(&monthly, KnowledgePublicationReservation::new(1, 1, share));
    let mut run = budget_run(&[&HOG_PLUGIN, &monthly_reserved]).expect("the monthly run starts");
    HOG_SPENDS.store(false, Ordering::Relaxed);
    let settled = run.settle_boundary(daily());
    HOG_SPENDS.store(true, Ordering::Relaxed);
    settled.expect("the daily boundary settles");
    assert_eq!(HOG_READ.load(Ordering::Relaxed), limit);
}

/// Command admission refuses request ID 0, and a save refuses a queued
/// command that carries it, so `enqueue_command` refuses it too, as a host
/// error that queues nothing: a host that used ID 0 could otherwise settle
/// on and never save. A command submitted directly with ID 0 stays a
/// recorded rejection, which a save holds.
#[test]
fn a_queued_command_request_needs_a_nonzero_id() {
    let (scenario, ids) = demo_scenario();
    let mut simulation = Simulation::new(47, scenario).expect("fixture scenario should load");
    let before = simulation.checkpoint_hash().to_owned();
    let error = simulation
        .enqueue_command(
            SimTime::EPOCH,
            0,
            CommandRequest::new(CommandRequestId::new(0), 0, move_order(&ids)),
        )
        .expect_err("request ID 0 is refused at enqueue");
    assert_eq!(error.code, ErrorCode::InvalidPayload, "{error}");
    assert_eq!(simulation.checkpoint_hash(), before);
    assert!(simulation.ingress_log().is_empty());

    let outcome = simulation
        .process_command(CommandRequest::new(
            CommandRequestId::new(0),
            0,
            move_order(&ids),
        ))
        .expect("a direct command with ID 0 is a recorded rejection");
    assert!(matches!(
        outcome,
        CommandOutcome::Rejected { rejection } if rejection.error.code == ErrorCode::InvalidPayload
    ));
    Simulation::from_snapshot(simulation.snapshot()).expect("the run still saves and restores");
}
