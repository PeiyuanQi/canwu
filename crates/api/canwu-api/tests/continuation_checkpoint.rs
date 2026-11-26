//! A run restored from a continuation checkpoint continues exactly as the
//! run it was written from.
//!
//! The fixture keeps work in flight at every boundary: commands whose
//! handlers queue packets and schedule an event, boundary systems that
//! schedule packets up to 30 days ahead for their own plugin and for another,
//! and cancel some of them, keyed draws and their retries, a boundary-ledger
//! read, superseding knowledge, and a record that declares a payload-required
//! event, an identity-only record version, and identity-only ingress whose
//! provenance it reads. An unsealed run, a run that seals every five
//! boundaries with that work pending, a run restored from a full save, and
//! runs restored from a continuation checkpoint take the same inputs and must
//! agree on every receipt, checkpoint hash, and state hash, and write
//! byte-equal continuation checkpoints. Each answers `sealed_evidence_cursor`
//! as its own seals say: `None` until it seals after it is created or
//! restored, a continuation restore included, then the end of its last
//! sealed segment. The first lockstep runs on a 1 MiB thread, so a debug
//! build proves the runtime fits the smallest common thread stack.
//!
//! A run that releases its evidence instead of sealing it keeps the same
//! closure and joins a lockstep of its own; a release, like a seal, is
//! refused where the closure cannot be built.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryId, BoundaryPhase, BoundaryProposal,
    BoundaryReceipt, BoundaryRequest, BoundarySystemContract, Canwu, CanwuError, CanwuRun,
    CauseRef, Command, CommandContext, CommandEnvelope, CommandRequest, CommandRequestId,
    CompactedCanwu, ContinuationCheckpoint, DomainRecordClass, DomainRecordDraft, DomainRecordKind,
    DomainRecordMutation, DomainRecordRef, DomainRecordSchema, EntityRef, ErrorCode, EvidenceRef,
    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD, IdentityEvidenceDependenciesV1, IngressClass, IngressId,
    IngressPayload, IngressRecord, Issuer, KnowledgeHolderRef, KnowledgeOrigin,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeSchemaId, KnowledgeSubject,
    KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeWriteGrant, OrganizationId, PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD,
    PayloadRequiredEvidenceContinuationV1, PayloadSchema, PersonId, PluginActionDescriptor,
    PluginIngressDescriptor, PluginIngressRequest, PluginIngressTarget, PluginKnowledgeSchema,
    PluginRegistrar, RandomOperationTarget, RandomStreamKey, Scenario, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence, SystemDirective,
    identity_evidence_dependencies_property_v1, payload_required_evidence_continuation_property_v1,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-continuation";
/// A plugin that only receives packets the fixture's boundaries schedule.
const RECEIVER: &str = "fixture-continuation-receiver";
/// The most packets the holder watches at once.
const WATCHED: usize = 6;

fn tally_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "tally")
}

fn holder_kind() -> DomainRecordKind {
    DomainRecordKind::new(PLUGIN, "holder")
}

fn tally() -> DomainRecordRef {
    DomainRecordRef {
        kind: tally_kind(),
        id: "tally".to_owned(),
    }
}

fn holder() -> DomainRecordRef {
    DomainRecordRef {
        kind: holder_kind(),
        id: "holder".to_owned(),
    }
}

fn record_state(kind: DomainRecordKind) -> StateKey {
    DomainRecordSchema::new(kind, DomainRecordClass::Record).state_key()
}

fn stream() -> RandomStreamKey {
    RandomStreamKey::new(PLUGIN, "draws", 1)
}

fn knowledge_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(
        KnowledgeRecordKind::new("fixture.continuation", "standing"),
        1,
    )
}

const fn knowledge_holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

const fn subject() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(10))
}

fn about() -> KnowledgeSubject {
    KnowledgeSubject {
        role: "about".to_owned(),
        target: KnowledgeSubjectTarget::Entity(subject()),
    }
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

fn failed(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidPayload, message)
}

/// A command whose handler queues a packet now and schedules an event.
fn dispatch(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let n = payload["n"].as_i64().unwrap_or_default();
    Ok(vec![
        SystemDirective::EnqueuePluginIngress {
            after: SimDuration::days(1 + n % 3),
            packet_type: "work".to_owned(),
            priority: 0,
            payload: json!({ "n": n, "from": "command" }),
            affected: Vec::new(),
        },
        SystemDirective::Schedule {
            after: SimDuration::days(2),
            directive: Box::new(SystemDirective::Emit {
                event_type: "relayed".to_owned(),
                summary: format!("relay of dispatch {n}"),
                affected: Vec::new(),
            }),
        },
    ])
}

/// One pulse event per boundary, which the next boundary admits.
fn pulse(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::Emit {
            event_type: "pulse".to_owned(),
            summary: format!("pulse of boundary {}", context.boundary_id.get()),
            affected: Vec::new(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Admits packets, reading each one and the command that caused it, moves
/// the tally to its next version, and keeps the holder's declarations: an
/// identity-only dependency on the tally version this boundary replaces, a
/// payload-required dependency on an event this boundary admits, which it
/// reads at every later boundary until it rotates twelve boundaries on, and
/// identity-only dependencies on the last packets it watches, whose
/// provenance it reads at every later boundary while it watches them.
fn admit(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut admitted = 0_i64;
    // The first admitted packet of each kind: boundary work, command work,
    // and the delivery the fixture scheduled for the receiver.
    let mut newly_watched: Vec<(&str, IngressId)> = Vec::new();
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { packet_type, .. } = &record.payload else {
            continue;
        };
        let kind = match (packet_type.as_str(), &record.cause) {
            ("work", Some(CauseRef::Command(_))) => "command",
            ("work", _) => "boundary",
            ("delivery", _) => "delivery",
            _ => continue,
        };
        if !newly_watched.iter().any(|(watched, _)| *watched == kind) {
            newly_watched.push((kind, *id));
        }
        if packet_type != "work" {
            continue;
        }
        if let Some(CauseRef::Command(command)) = record.cause {
            view.command(command)?
                .ok_or_else(|| failed("the cause command of a packet is readable at admission"))?;
        }
        admitted += 1;
    }

    let mut directives = Vec::new();
    let current_tally = view.domain_record(&tally())?;
    let replaced = view.current_domain_record_version(&tally())?;
    let total = current_tally
        .as_ref()
        .and_then(|record| record.payload["total"].as_i64())
        .unwrap_or_default()
        + admitted;
    let draft = DomainRecordDraft::new(
        tally(),
        json!({ "total": total, "boundary": context.boundary_id.get() }),
    );
    directives.push(BoundaryDirective::MutateRecord {
        mutation: match &current_tally {
            None => DomainRecordMutation::Create { record: draft },
            Some(record) => DomainRecordMutation::Update {
                record: draft,
                expected_version: record.version,
            },
        },
        summary: "The tally moves on".to_owned(),
    });

    let current_holder = view.domain_record(&holder())?;
    let previous = current_holder
        .as_ref()
        .map_or(Value::Null, |record| record.payload.clone());
    let continuation: PayloadRequiredEvidenceContinuationV1 =
        serde_json::from_value(previous[PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD].clone())
            .unwrap_or_else(|_| PayloadRequiredEvidenceContinuationV1::completed());
    let since = previous["since"].as_u64().unwrap_or_default();
    let mut seen = previous["seen"].as_u64().unwrap_or_default();
    // The provenance of every watched packet, sealed or not: only a packet
    // the fixture both scheduled and received matches.
    let mut watched: Vec<u64> = previous["watched"]
        .as_array()
        .map(|ids| ids.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    let mut matched = previous["matched"].as_u64().unwrap_or_default();
    for id in &watched {
        for (plugin, packet_type) in [
            (PLUGIN, "work"),
            (PLUGIN, "delivery"),
            (RECEIVER, "delivery"),
        ] {
            if view.plugin_ingress_matches(IngressId::new(*id), plugin, packet_type)? {
                matched += 1;
            }
        }
    }
    watched.extend(newly_watched.iter().map(|(_, id)| id.get()));
    let excess = watched.len().saturating_sub(WATCHED);
    watched.drain(..excess);
    for dependency in &continuation.dependencies {
        if let EvidenceRef::Event(event) = dependency {
            let payload = view
                .event(*event)?
                .ok_or_else(|| failed("a payload-required event is readable"))?;
            seen += u64::try_from(payload.summary.len()).unwrap_or_default();
        }
    }
    let boundary = context.boundary_id.get();
    let (continuation, since) = if !continuation.active || boundary >= since + 12 {
        match context.admitted_events.first() {
            Some(event) => (
                PayloadRequiredEvidenceContinuationV1::active(vec![EvidenceRef::Event(*event)]),
                boundary,
            ),
            None => (PayloadRequiredEvidenceContinuationV1::completed(), boundary),
        }
    } else {
        (continuation, since)
    };
    // Declarations are sorted and unique.
    let identity = IdentityEvidenceDependenciesV1::new(
        replaced
            .into_iter()
            .map(EvidenceRef::DomainRecordVersion)
            .chain(
                watched
                    .iter()
                    .map(|id| EvidenceRef::Ingress(IngressId::new(*id))),
            )
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    );
    let draft = DomainRecordDraft::new(
        holder(),
        json!({
            PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD: continuation,
            IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: identity,
            "since": since,
            "seen": seen,
            "watched": watched,
            "matched": matched,
        }),
    );
    directives.push(BoundaryDirective::MutateRecord {
        mutation: match &current_holder {
            None => DomainRecordMutation::Create { record: draft },
            Some(record) => DomainRecordMutation::Update {
                record: draft,
                expected_version: record.version,
            },
        },
        summary: "The holder keeps its declarations".to_owned(),
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Schedules two packets, one for tomorrow and one up to 30 days ahead, and
/// a delivery for the receiver up to 17 days ahead.
fn schedule(
    _view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = i64::try_from(context.boundary_id.get()).unwrap_or_default();
    let mut directives: Vec<_> = [1, (boundary * 7) % 29 + 1]
        .into_iter()
        .enumerate()
        .map(|(index, days)| BoundaryDirective::ScheduleIngress {
            after: SimDuration::days(days),
            packet_type: "work".to_owned(),
            priority: 0,
            payload: json!({ "n": boundary * 10 + i64::try_from(index).unwrap_or_default(), "from": "boundary" }),
            affected: Vec::new(),
        })
        .collect();
    directives.push(BoundaryDirective::SchedulePluginIngress {
        target_plugin: RECEIVER.to_owned(),
        after: SimDuration::days((boundary * 5) % 17 + 1),
        packet_type: "delivery".to_owned(),
        priority: 0,
        payload: json!({ "n": boundary * 10 + 2, "from": "boundary" }),
        affected: Vec::new(),
    });
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// Cancels the first of its own pending packets whose number leaves one
/// when divided by three: a long-scheduled packet or a delivery for the
/// receiver.
fn cancel(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let target = view.cancellable_plugin_ingress()?.into_iter().find(|record| {
        matches!(
            &record.payload,
            IngressPayload::Plugin { payload, .. } if payload["n"].as_i64().unwrap_or_default() % 3 == 1
        )
    });
    Ok(BoundaryProposal {
        directives: target
            .map(|record| BoundaryDirective::CancelPluginIngress {
                ingress_id: record.id,
                reason: "fixture cancellation".to_owned(),
            })
            .into_iter()
            .collect(),
        ..BoundaryProposal::default()
    })
}

/// Retries a keyed draw made at the first boundaries and makes a fresh one
/// citing the previous boundary.
fn draw(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let boundary = context.boundary_id.get();
    if boundary == 1 {
        return Ok(BoundaryProposal::default());
    }
    let mut summaries = Vec::new();
    let fresh = format!("b{boundary}");
    for (operation, evidence) in [
        ("standing", EvidenceRef::Boundary(BoundaryId::new(1))),
        (
            fresh.as_str(),
            EvidenceRef::Boundary(BoundaryId::new(boundary - 1)),
        ),
    ] {
        let drawn = view.random_draw_for_operation(
            &stream(),
            evidence,
            "draw",
            operation,
            RandomOperationTarget::CanonicalKey(operation.to_owned()),
            0,
            1_000,
            "continuation fixture draw",
        )?;
        summaries.push(format!("{operation}={}", drawn.sample.value));
    }
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::Emit {
            event_type: "drew".to_owned(),
            summary: summaries.join(" "),
            affected: Vec::new(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Reads the boundary ledger over its declared week.
fn ledger(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let from = SimTime::from_minutes(
        context
            .at
            .as_minutes()
            .saturating_sub(SimDuration::days(7).as_minutes())
            .max(0),
    );
    let window = view.boundaries_between(from, context.at + SimDuration::days(3))?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::Emit {
            event_type: "ledger".to_owned(),
            summary: format!(
                "settled={} queued={}",
                window.settled.len(),
                window.queued.len()
            ),
            affected: Vec::new(),
        }],
        ..BoundaryProposal::default()
    })
}

/// Publishes a standing record that supersedes the holder's current ones.
fn know(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let heads = view
        .knowledge_heads(&knowledge_holder(), &knowledge_schema().kind, &[about()])?
        .records;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::PublishKnowledge {
            visibility: StateVisibility::SameBoundary,
            producer_correlation: None,
            supersession: canwu_api::KnowledgeSupersession::Explicit,
            groups: vec![canwu_api::KnowledgePublicationGroup {
                holders: vec![knowledge_holder()],
                records: vec![KnowledgeRecordDraft {
                    schema: knowledge_schema(),
                    subjects: vec![about()],
                    payload: json!({ "boundary": context.boundary_id.get() }),
                    as_of: Some(context.at),
                    confidence_per_mille: 1_000,
                    origin: KnowledgeOrigin {
                        method: "fixture.continuation".to_owned(),
                        // Every standing record cites the first boundary, which
                        // the seals archive early on.
                        evidence: (context.boundary_id.get() > 1)
                            .then(|| EvidenceRef::Boundary(BoundaryId::new(1)))
                            .into_iter()
                            .collect(),
                    },
                    supersedes: heads,
                    contradicts: Vec::new(),
                }],
            }],
            summary: "The holder's standing record".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

struct ContinuationFixture;

impl SimulationPlugin for ContinuationFixture {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000016c1"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            tally_kind(),
            DomainRecordClass::Record,
        ))?;
        let mut holder_schema = DomainRecordSchema::new(holder_kind(), DomainRecordClass::Record);
        holder_schema.payload_schema = PayloadSchema::Object {
            properties: std::collections::BTreeMap::from([
                (
                    PAYLOAD_REQUIRED_EVIDENCE_CONTINUATION_FIELD.to_owned(),
                    payload_required_evidence_continuation_property_v1(),
                ),
                (
                    IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(),
                    identity_evidence_dependencies_property_v1(),
                ),
            ]),
            allow_additional: true,
        };
        registrar.register_record_schema(holder_schema)?;
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: knowledge_schema(),
            schema_hash: "00000000000000000000000000000000000000000000000000000000000016c2"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "about".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
            retention: canwu_api::KnowledgeRetention::Keep,
        })?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: "work".to_owned(),
            description: "fixture work packet".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "dispatch".to_owned(),
                description: "Queue work now and schedule a relay event".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            dispatch,
        )?;

        let mut pulsing = BoundarySystemContract::new(
            "a-pulse",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        pulsing.emits = vec!["pulse".to_owned()];
        registrar.register_boundary_system(pulsing, pulse)?;

        // A later phase than the pulse, so the pulse is among the events the
        // boundary has emitted when the holder declares it.
        let mut admitting = BoundarySystemContract::new(
            "b-admit",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::Daily,
        );
        admitting.reads = vec![
            StateKey::core_ingress(),
            StateKey::core_commands(),
            StateKey::core_events(),
            record_state(tally_kind()),
            record_state(holder_kind()),
        ];
        admitting.writes = vec![record_state(tally_kind()), record_state(holder_kind())];
        registrar.register_boundary_system(admitting, admit)?;

        let mut scheduling = BoundarySystemContract::new(
            "c-schedule",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        scheduling.reads = vec![StateKey::core_ingress()];
        scheduling.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: RECEIVER.to_owned(),
            packet_type: "delivery".to_owned(),
        }];
        registrar.register_boundary_system(scheduling, schedule)?;

        let mut cancelling = BoundarySystemContract::new(
            "d-cancel",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        cancelling.reads = vec![StateKey::core_ingress()];
        registrar.register_boundary_system(cancelling, cancel)?;

        let mut drawing = BoundarySystemContract::new(
            "e-draw",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        drawing.reads = vec![StateKey::core_evidence()];
        drawing.random_streams = vec![stream()];
        drawing.emits = vec!["drew".to_owned()];
        registrar.register_boundary_system(drawing, draw)?;

        let mut reading = BoundarySystemContract::new(
            "f-ledger",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        reading.reads = vec![StateKey::core_boundary_schedule()];
        reading.boundary_schedule_horizon = SimDuration::days(7);
        reading.emits = vec!["ledger".to_owned()];
        registrar.register_boundary_system(reading, ledger)?;

        let mut knowing = BoundarySystemContract::new(
            "g-know",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Daily,
        );
        knowing.reads = vec![StateKey::core_knowledge()];
        knowing.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: knowledge_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(knowing, know)
    }
}

struct ContinuationReceiver;

impl SimulationPlugin for ContinuationReceiver {
    fn name(&self) -> &'static str {
        RECEIVER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000016c3"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: "delivery".to_owned(),
            description: "fixture delivery from another plugin".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })
    }
}

fn command(request: u64, revision: u64, n: i64) -> CommandRequest {
    CommandRequest::new(
        CommandRequestId::new(request),
        revision,
        CommandEnvelope::new(
            Issuer::Debug,
            Command::Plugin {
                plugin: PLUGIN.to_owned(),
                command: "dispatch".to_owned(),
                payload: json!({ "n": n }),
            },
        ),
    )
}

/// One run of the fixture, whichever handle it is.
enum Run {
    Unsealed(Canwu),
    Compact(CompactedCanwu),
}

impl Run {
    fn revision(&self) -> u64 {
        match self {
            Self::Unsealed(run) => run.revision(),
            Self::Compact(run) => run.revision(),
        }
    }

    fn step(&mut self, at: SimTime, request: Option<CommandRequest>) -> BoundaryReceipt {
        match self {
            Self::Unsealed(run) => {
                if let Some(request) = request {
                    run.enqueue_command(at, 0, request)
                        .expect("dispatch queues");
                }
                run.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))
                    .expect("boundary settles")
            }
            Self::Compact(run) => {
                if let Some(request) = request {
                    run.enqueue_command(at, 0, request)
                        .expect("dispatch queues");
                }
                run.settle_boundary(BoundaryRequest::at(at).with_cadence(SystemCadence::Daily))
                    .expect("boundary settles")
            }
        }
    }

    /// Queues `request` again at `at`, as a host retrying it would.
    fn enqueue(
        &mut self,
        at: SimTime,
        request: CommandRequest,
    ) -> Result<canwu_api::IngressReceipt, ErrorCode> {
        match self {
            Self::Unsealed(run) => run.enqueue_command(at, 0, request),
            Self::Compact(run) => run.enqueue_command(at, 0, request),
        }
        .map_err(|error| error.code)
    }

    /// Queues a decision request, as a host would.
    fn enqueue_decision(
        &mut self,
        at: SimTime,
        request: canwu_api::DecisionIngressRequest,
    ) -> Result<canwu_api::IngressReceipt, ErrorCode> {
        match self {
            Self::Unsealed(run) => run.enqueue_decision(at, 0, request),
            Self::Compact(run) => run.enqueue_decision(at, 0, request),
        }
        .map_err(|error| error.code)
    }

    fn hashes(&self) -> (String, String) {
        match self {
            Self::Unsealed(run) => (
                run.checkpoint_hash().to_owned(),
                run.authoritative_state_hash().expect("state hash"),
            ),
            Self::Compact(run) => (
                run.checkpoint_hash().to_owned(),
                run.authoritative_state_hash().expect("state hash"),
            ),
        }
    }

    /// Where the run's last seal ended.
    fn sealed_cut(&self) -> Option<canwu_api::EvidenceCursor> {
        match self {
            Self::Unsealed(run) => run.sealed_evidence_cursor(),
            Self::Compact(run) => run.sealed_evidence_cursor(),
        }
    }

    fn continuation(&self) -> (ContinuationCheckpoint, String) {
        match self {
            Self::Unsealed(run) => run.continuation_checkpoint(),
            Self::Compact(run) => run.continuation_checkpoint(),
        }
        .expect("the continuation checkpoint builds")
    }

    /// The run's handle contract, as a host helper would take it.
    fn handle(&self) -> &dyn CanwuRun {
        match self {
            Self::Unsealed(run) => run,
            Self::Compact(run) => run,
        }
    }

    fn ingress_log(&self) -> &[IngressRecord] {
        match self {
            Self::Unsealed(run) => run.ingress_log(),
            Self::Compact(run) => run.ingress_log(),
        }
    }
}

struct EmptyArchive;

impl canwu_api::ArchiveProvider for EmptyArchive {
    fn load_evidence_segment(
        &self,
        _segment_id: &str,
    ) -> Result<Option<canwu_api::EvidenceJournalSegment>, CanwuError> {
        Ok(None)
    }
}

fn plugins() -> [&'static dyn SimulationPlugin; 2] {
    [&ContinuationFixture, &ContinuationReceiver]
}

fn scenario() -> Scenario {
    Scenario::new(
        SimTime::EPOCH,
        vec![EntityRef::Person(PersonId::new(1)), subject()],
    )
}

/// The host input of day `d`: a dispatch every third day, with consecutive
/// request IDs, so retired requests collapse into one range across seals and
/// restores.
const fn dispatch_on(day: i64) -> Option<(u64, i64)> {
    if day % 3 == 0 {
        Some((day.unsigned_abs() / 3, day))
    } else {
        None
    }
}

/// Runs on a 1 MiB thread: a debug build must keep room on the stack for the
/// runtime and its transaction checkpoints.
#[test]
fn a_continuation_restored_run_continues_in_lockstep() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(lockstep)
        .expect("the thread starts")
        .join()
        .expect("the lockstep runs on a 1 MiB stack");
}

fn lockstep() {
    const RESTORE_AT: i64 = 10;
    let plugins = plugins();
    let unsealed = Canwu::new_with_plugins(1601, scenario(), &plugins).expect("fixture starts");
    let sealed = unsealed.fork().into_compacted().expect("compact copy");
    let mut runs = vec![Run::Unsealed(unsealed), Run::Compact(sealed)];
    // What each run answers for `sealed_evidence_cursor`: `None` until it
    // seals after it is created or restored, then the end of its last sealed
    // segment. A continuation restore's base is not a seal.
    let mut cuts: Vec<Option<canwu_api::EvidenceCursor>> = vec![None, None];
    let mut sealed_pinned_cancellation = false;
    let mut known_versions_seen = false;
    let mut pinned_payload_seen = false;
    let mut cross_plugin_fact_seen = false;
    let mut ungenerated_fact_seen = false;

    // Every queued dispatch, with its due time and first receipt, for the
    // retries below.
    let mut sent: Vec<(SimTime, CommandRequest, canwu_api::IngressReceipt)> = Vec::new();
    let mut retired_retry_seen = false;
    for d in 1..=RESTORE_AT + 40 {
        let at = day(d);
        let revision = runs[0].revision();
        let request = dispatch_on(d).map(|(id, n)| command(id, revision, n));
        let receipts: Vec<_> = runs
            .iter_mut()
            .map(|run| run.step(at, request.clone()))
            .collect();
        assert!(
            receipts.windows(2).all(|pair| pair[0] == pair[1]),
            "receipts differ at day {d}"
        );
        let hashes: Vec<_> = runs.iter().map(Run::hashes).collect();
        assert!(
            hashes.windows(2).all(|pair| pair[0] == pair[1]),
            "hashes differ at day {d}"
        );
        if let Some(request) = request {
            let receipt = runs[0]
                .enqueue(at, request.clone())
                .expect("an exact retry at its own instant returns its receipt");
            sent.push((at, request, receipt));
        }
        // Every run answers a retry of every dispatch alike: its receipt
        // while its attempt is inside the 30-day retention, then
        // `RequestOutcomeRetired`, without changing the run. A retired ID with
        // new content is refused the same way.
        let kept_from = SimTime::from_minutes(
            at.as_minutes() - canwu_api::DEFAULT_REQUEST_OUTCOME_RETENTION.as_minutes(),
        );
        for (due, request, receipt) in &sent {
            let expected = if *due < kept_from {
                Err(ErrorCode::RequestOutcomeRetired)
            } else {
                Ok(receipt.clone())
            };
            retired_retry_seen |= expected.is_err();
            for run in &mut runs {
                assert_eq!(
                    run.enqueue(*due, request.clone()),
                    expected,
                    "a retry of request {} at day {d}",
                    request.request_id
                );
            }
        }
        if let Some((due, oldest, _)) = sent.first()
            && *due < kept_from
        {
            let reused = command(oldest.request_id.get(), runs[0].revision(), -1);
            let mut nested = reused.clone();
            nested.envelope = nested.envelope.at_time(at);
            let decision = canwu_api::DecisionIngressRequest::new(
                canwu_api::DecisionRequestId::new(d.unsigned_abs()),
                runs[0].revision(),
                canwu_api::DecisionMutation::Cancel {
                    ticket_id: canwu_api::DecisionTicketId::new(1),
                    expected_version: 0,
                    reason: "fixture".to_owned(),
                },
            )
            .with_command(nested);
            for run in &mut runs {
                assert_eq!(
                    run.enqueue(at, reused.clone()),
                    Err(ErrorCode::RequestOutcomeRetired),
                    "a retired ID with new content at day {d}"
                );
                assert_eq!(
                    run.enqueue_decision(at, decision.clone()),
                    Err(ErrorCode::RequestOutcomeRetired),
                    "a decision that nests a retired ID at day {d}"
                );
            }
        }
        let hashes_after: Vec<_> = runs.iter().map(Run::hashes).collect();
        assert_eq!(hashes_after, hashes, "retries change nothing at day {d}");

        // The sealed run: a cancellation of a packet that an earlier seal
        // pinned, whose producing boundary is sealed (edge case b).
        if let Run::Compact(sealed) = &runs[1] {
            let first_retained = sealed.ingress_log().first().map(|record| record.id);
            sealed_pinned_cancellation |= sealed.ingress_log().iter().any(|record| {
                matches!(
                    &record.payload,
                    IngressPayload::PluginCancellation { cancelled, .. }
                        if first_retained.is_some_and(|first| *cancelled < first)
                )
            });
        }
        // The sealed run seals every five boundaries with work in flight;
        // the restored runs seal on the same days.
        if d % 5 == 0 {
            for (run, cut) in runs[1..].iter_mut().zip(&mut cuts[1..]) {
                if let Run::Compact(compact) = run {
                    let segment = compact
                        .seal_evidence()
                        .expect("a run with work in flight seals")
                        .expect("the settled tail seals");
                    *cut = Some(segment.end);
                    // An idle reseal moves nothing, and a fork answers alike.
                    assert!(
                        compact.seal_evidence().expect("an idle reseal").is_none(),
                        "nothing is left to seal at day {d}"
                    );
                    assert_eq!(compact.sealed_evidence_cursor(), *cut, "day {d}");
                    assert_eq!(compact.fork().sealed_evidence_cursor(), *cut, "day {d}");
                }
            }
        }

        if d == RESTORE_AT {
            let (checkpoint, hash) = runs[0].continuation();
            assert!(!checkpoint.pending_ingress.is_empty(), "pending ingress");
            assert!(
                !checkpoint.unadmitted.events.is_empty(),
                "unadmitted events"
            );
            let state = serde_json::to_value(&checkpoint.state).expect("state JSON");
            assert!(
                state["scheduled"]
                    .as_array()
                    .is_some_and(|scheduled| !scheduled.is_empty()),
                "scheduled actions"
            );
            let json = checkpoint.to_json().expect("checkpoint JSON");
            let restored =
                CompactedCanwu::from_continuation_checkpoint_json_verified(&json, &hash, &plugins)
                    .expect("the first restore");
            assert_eq!(
                restored
                    .checkpoint()
                    .expect_err("no compact checkpoint")
                    .code,
                ErrorCode::HistoryUnavailable
            );
            assert_eq!(
                restored
                    .load_archived_evidence_segment(
                        &EvidenceRef::Event(canwu_api::EventId::new(1)),
                        &EmptyArchive,
                    )
                    .expect_err("no archived receipt before the base")
                    .code,
                ErrorCode::EvidenceUnavailable
            );
            // A continuation restore no longer retains what precedes its
            // base, but it sealed none of it: it answers `None` until its
            // own first seal.
            assert_eq!(
                restored.continuation_base().map(|base| base.cut),
                Some(checkpoint.settled_cut)
            );
            runs.push(Run::Compact(restored));
            cuts.push(None);
            // A full save of the same run restores and joins the lockstep.
            let Run::Unsealed(unsealed) = &runs[0] else {
                unreachable!("the first run is unsealed");
            };
            let save = unsealed.checkpoint_journal_json().expect("the full save");
            let restored = Canwu::from_checkpoint_journal_json_with_plugins(&save, &plugins)
                .expect("the full save restores");
            runs.push(Run::Unsealed(restored));
            cuts.push(None);
        }
        if d == RESTORE_AT + 20 {
            let (checkpoint, hash) = runs[2].continuation();
            let settled = checkpoint.settled_cut;
            let restored =
                CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
                    .expect("the second restore");
            assert_eq!(
                restored.continuation_base().map(|base| base.cut),
                Some(settled)
            );
            runs.push(Run::Compact(restored));
            cuts.push(None);
        }
        let answered: Vec<_> = runs.iter().map(Run::sealed_cut).collect();
        assert_eq!(answered, cuts, "sealed evidence cursors at day {d}");
        // Every fifth boundary from the first restore, and the boundary after
        // each seal, when the holder declares a version that its establishing
        // boundary's seal left only in the version this boundary replaced
        // (edge case a).
        if d >= RESTORE_AT && ((d - RESTORE_AT) % 5 == 0 || d % 5 == 1) {
            let checkpoints: Vec<_> = runs
                .iter()
                .map(|run| run.continuation().0.to_json().expect("JSON"))
                .collect();
            assert!(
                checkpoints.windows(2).all(|pair| pair[0] == pair[1]),
                "continuation checkpoints differ at day {d}"
            );
            let (checkpoint, _) = runs[0].continuation();
            known_versions_seen |= !checkpoint.known_record_versions.is_empty();
            pinned_payload_seen |= !checkpoint.pinned.events.is_empty();
            cross_plugin_fact_seen |= checkpoint.ingress_provenance.iter().any(|fact| {
                fact.generation
                    .as_ref()
                    .is_some_and(|generation| generation.issuer != generation.target_plugin)
            });
            ungenerated_fact_seen |= checkpoint
                .ingress_provenance
                .iter()
                .any(|fact| fact.generation.is_none());
            // The index keeps the outcomes inside the retention and names
            // every retired dispatch.
            let kept_from = SimTime::from_minutes(
                at.as_minutes() - canwu_api::DEFAULT_REQUEST_OUTCOME_RETENTION.as_minutes(),
            );
            let retired: Vec<u64> = sent
                .iter()
                .filter(|(due, _, _)| *due < kept_from)
                .map(|(_, request, _)| request.request_id.get())
                .collect();
            let listed: Vec<u64> = checkpoint
                .requests
                .retired_commands
                .iter()
                .flat_map(|range| range.first.get()..=range.last.get())
                .collect();
            assert_eq!(listed, retired, "retired requests at day {d}");
            assert!(
                checkpoint.requests.retired_commands.len() <= 1,
                "consecutive retired IDs form one range at day {d}"
            );
            assert!(
                checkpoint
                    .requests
                    .commands
                    .iter()
                    .all(|entry| !retired.contains(&entry.request_id.get())),
                "no retired outcome is kept at day {d}"
            );
        }
    }
    assert!(retired_retry_seen, "requests retired during the run");
    assert!(sealed_pinned_cancellation, "edge case b was exercised");
    assert!(known_versions_seen, "edge case a was exercised");
    assert!(pinned_payload_seen, "edge case c was exercised");
    assert!(
        cross_plugin_fact_seen,
        "a sealed cross-plugin packet kept its issuer"
    );
    assert!(
        ungenerated_fact_seen,
        "a declared packet no boundary generated kept a fact"
    );
    let Run::Unsealed(unsealed) = &runs[0] else {
        unreachable!("the first run is unsealed");
    };
    let holder = unsealed
        .domain_record(&holder())
        .expect("the holder exists");
    assert!(
        holder.payload["matched"].as_u64().unwrap_or_default() > 0,
        "a watched packet matched its provenance"
    );

    // The unsealed run still replays.
    let replayed = Canwu::replay_from_journal(&plugins, &unsealed.replay_journal())
        .expect("the unsealed run replays");
    assert_eq!(replayed.checkpoint_hash(), unsealed.checkpoint_hash());
}

/// A host lists the ingress still queued with `pending_ingress`, whatever its
/// run has sealed or was restored from: an unsealed run, a run that seals
/// every five boundaries, and a run restored from a continuation checkpoint
/// list the same records in the same queue order every day, including those
/// a seal or the restore pinned out of the retained `ingress_log`, and a
/// record leaves the list in the boundary that admits it.
#[test]
fn pending_ingress_lists_the_same_queue_in_every_run() {
    const RESTORE_AT: i64 = 7;
    let plugins = plugins();
    let unsealed = Canwu::new_with_plugins(1602, scenario(), &plugins).expect("fixture starts");
    let sealed = unsealed.fork().into_compacted().expect("compact copy");
    let mut runs = vec![Run::Unsealed(unsealed), Run::Compact(sealed)];
    let mut pinned_listed = false;
    let mut admitted_left = false;
    let mut previous: Vec<IngressRecord> = Vec::new();
    for d in 1..=RESTORE_AT + 13 {
        let at = day(d);
        let revision = runs[0].revision();
        let request = dispatch_on(d).map(|(id, n)| command(id, revision, n));
        for run in &mut runs {
            run.step(at, request.clone());
        }
        if d % 5 == 0 {
            for run in &mut runs[1..] {
                if let Run::Compact(compact) = run {
                    compact
                        .seal_evidence()
                        .expect("a run with work in flight seals")
                        .expect("the settled tail seals");
                }
            }
        }
        if d == RESTORE_AT {
            let (checkpoint, hash) = runs[0].continuation();
            runs.push(Run::Compact(
                CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
                    .expect("the restore"),
            ));
        }

        let pending: Vec<IngressRecord> = runs[0].handle().pending_ingress().cloned().collect();
        assert!(!pending.is_empty(), "work is queued at day {d}");
        for run in &runs[1..] {
            assert_eq!(
                run.handle().pending_ingress().cloned().collect::<Vec<_>>(),
                pending,
                "pending ingress differs at day {d}"
            );
        }
        // Queue order: the order boundaries admit the records in.
        let order = |record: &IngressRecord| {
            (
                record.due_at,
                record.class,
                std::cmp::Reverse(record.priority),
                record.issued_at,
                record.id,
            )
        };
        assert!(
            pending
                .windows(2)
                .all(|pair| order(&pair[0]) < order(&pair[1])),
            "pending ingress is in queue order at day {d}"
        );
        // The unsealed run retains every record it lists; a sealed or
        // restored run lists records its retained tail no longer holds.
        assert!(
            pending
                .iter()
                .all(|record| runs[0].ingress_log().contains(record))
        );
        pinned_listed |= runs[1..].iter().any(|run| {
            pending
                .iter()
                .any(|record| !run.ingress_log().contains(record))
        });
        // What the boundary admitted has left the queue.
        let Run::Unsealed(unsealed) = &runs[0] else {
            unreachable!("the first run is unsealed");
        };
        let admitted = &unsealed
            .boundaries()
            .last()
            .expect("a settled boundary")
            .admitted_ingress;
        assert!(
            pending.iter().all(|record| !admitted.contains(&record.id)),
            "admitted ingress is still listed at day {d}"
        );
        admitted_left |= previous.iter().any(|record| admitted.contains(&record.id));
        previous = pending;
    }
    assert_eq!(runs.len(), 3, "the restored run joined");
    assert!(
        pinned_listed,
        "a sealed or restored run listed pinned ingress"
    );
    assert!(
        admitted_left,
        "a listed record was admitted and left the list"
    );
}

/// What a compact run of the release lockstep does every fifth day.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Keep {
    Seal,
    Release,
    /// Releases on odd fifths and seals on even ones.
    Alternate,
}

/// Releases a compact run's settled evidence and checks what the release
/// leaves (G-119): the retained journals hold only what settled after the
/// cut, no segment or receipt is kept, the history begins at the cut, the
/// continuation checkpoint is unchanged, and every released tally version
/// is known only if the closure pins it. Returns the release and whether
/// the closure pinned an event's payload, kept a released version, dropped
/// one, and whether the release dropped a receipt an earlier seal kept.
fn release_and_check(
    run: &mut CompactedCanwu,
    unsealed: &Canwu,
    tally_versions: &[canwu_api::DomainRecordVersionRef],
    previous_end: canwu_api::EvidenceCursor,
    d: i64,
) -> (canwu_api::EvidenceRelease, (bool, bool, bool, bool)) {
    let (before, before_hash) = run
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds");
    let journal_end = run.evidence_cursor().expect("the journal end");
    // The receipts an earlier seal kept: one for each current dependency it
    // archived (the declared event, a record version, watched ingress).
    let receipted: Vec<EvidenceRef> = (1..=journal_end.event_count)
        .map(|id| EvidenceRef::Event(canwu_api::EventId::new(id)))
        .chain(
            (1..=journal_end.ingress_count)
                .map(|id| EvidenceRef::Ingress(canwu_api::IngressId::new(id))),
        )
        .chain(
            (1..=journal_end.command_count)
                .map(|id| EvidenceRef::Command(canwu_api::CommandId::new(id))),
        )
        .chain(
            tally_versions
                .iter()
                .cloned()
                .map(EvidenceRef::DomainRecordVersion),
        )
        .filter(|reference| run.archived_evidence_receipt(reference).is_some())
        .collect();
    let release = run
        .release_evidence()
        .expect("a run with work in flight releases")
        .expect("the settled tail releases");
    assert_eq!(release.start, previous_end, "day {d}");
    assert_eq!(release.end, before.settled_cut, "day {d}");
    assert_eq!(run.evidence_cursor().expect("the journal end"), journal_end);

    // A release changes no continuation checkpoint: the closure, the
    // unadmitted tail, and the indexes are what the seal would keep.
    let (after, after_hash) = run
        .continuation_checkpoint()
        .expect("the continuation checkpoint builds after a release");
    assert_eq!(after_hash, before_hash, "day {d}");
    assert_eq!(after, before, "day {d}");
    assert!(
        !after.pending_ingress.is_empty(),
        "pending ingress, day {d}"
    );
    assert_eq!(run.pending_ingress().count(), after.pending_ingress.len());

    // Only what settled after the cut is retained.
    let cut = release.end;
    assert!(run.boundaries().is_empty(), "day {d}");
    assert!(run.random_draws().is_empty(), "day {d}");
    assert!(run.ingress_log().is_empty(), "day {d}");
    assert!(
        run.events()
            .iter()
            .all(|event| event.id.get() > cut.event_count)
    );
    assert_eq!(run.events().len(), after.unadmitted.events.len());
    assert!(
        run.commands()
            .iter()
            .all(|command| command.id.get() > cut.command_count)
    );
    assert!(
        run.command_attempts()
            .iter()
            .all(|attempt| attempt.id.get() > cut.command_attempt_count)
    );

    // No segment, index, or receipt: the history begins at the cut.
    assert_eq!(
        run.sealed_evidence_cursor(),
        None,
        "a release is not a seal"
    );
    let base = run.continuation_base().expect("a released run has a base");
    assert_eq!(base.cut, cut);
    assert_eq!(base.continuation_hash, None);
    assert_eq!(base.checkpoint_hash, run.checkpoint_hash());
    for reference in &receipted {
        assert!(
            run.archived_evidence_receipt(reference).is_none(),
            "{reference:?} keeps its receipt at day {d}"
        );
    }
    assert_eq!(
        run.load_archived_evidence_segment(
            &EvidenceRef::Event(canwu_api::EventId::new(1)),
            &EmptyArchive,
        )
        .expect_err("no receipt is kept")
        .code,
        ErrorCode::EvidenceUnavailable
    );
    for code in [
        run.checkpoint().map(|_| ()),
        run.snapshot_with_segments(Vec::new()).map(|_| ()),
        run.replay_journal_with_segments(Vec::new()).map(|_| ()),
    ]
    .map(|result| result.expect_err("no history before the base").code)
    {
        assert_eq!(code, ErrorCode::HistoryUnavailable, "day {d}");
    }
    assert!(
        run.release_evidence().expect("an idle release").is_none(),
        "nothing is left to release at day {d}"
    );

    // A released tally version is known only through the closure, and its
    // body only if the closure keeps it; the unsealed run keeps them all.
    let current = run
        .current_domain_record_version(&tally())
        .expect("the current version");
    let (mut kept_seen, mut dropped_seen) = (false, false);
    for version in tally_versions
        .iter()
        .filter(|version| Some(*version) != current.as_ref())
    {
        let body_kept = after
            .pinned
            .record_versions
            .iter()
            .any(|pin| pin.version == *version);
        let kept = body_kept || after.known_record_versions.contains(version);
        assert!(unsealed.domain_record_version_evidence_exists(version));
        assert!(unsealed.domain_record_version(version).is_some());
        assert_eq!(
            run.domain_record_version_evidence_exists(version),
            kept,
            "{version:?} at day {d}"
        );
        assert_eq!(run.domain_record_version(version).is_some(), body_kept);
        kept_seen |= kept;
        dropped_seen |= !kept;
    }
    (
        release,
        (
            !after.pinned.events.is_empty(),
            kept_seen,
            dropped_seen,
            !receipted.is_empty(),
        ),
    )
}

/// A run that releases its evidence instead of sealing it (downstream gap
/// G-119) continues exactly as the unsealed run, a sealed run, a run that
/// alternates releases and seals, and a run restored from the releasing
/// run's continuation checkpoint: every receipt, checkpoint hash, state
/// hash, and pending queue agrees, and every run writes byte-equal
/// continuation checkpoints. Each answers `sealed_evidence_cursor` and
/// `continuation_base` as its own seals and releases say: a release ends the
/// history at its cut, and a later seal starts a new chain there.
#[test]
fn a_released_run_continues_in_lockstep() {
    const DAYS: i64 = 30;
    const RESTORE_AT: i64 = 12;
    let plugins = plugins();
    let unsealed = Canwu::new_with_plugins(1605, scenario(), &plugins).expect("fixture starts");
    let mut keeps = vec![None];
    let mut runs = vec![];
    for keep in [Keep::Seal, Keep::Release, Keep::Alternate] {
        keeps.push(Some(keep));
        runs.push(Run::Compact(
            unsealed.fork().into_compacted().expect("compact copy"),
        ));
    }
    runs.insert(0, Run::Unsealed(unsealed));
    // What each run answers: its last seal's end, its base, and where its
    // retained evidence begins.
    let mut cuts: Vec<Option<canwu_api::EvidenceCursor>> = vec![None; runs.len()];
    let mut bases: Vec<Option<canwu_api::EvidenceCursor>> = vec![None; runs.len()];
    let mut starts = vec![canwu_api::EvidenceCursor::default(); runs.len()];
    let mut tally_versions = Vec::new();
    let (mut payload_pinned, mut version_kept, mut version_dropped) = (false, false, false);
    let mut chained_seal = false;
    let mut receipt_dropped = false;

    for d in 1..=DAYS {
        let at = day(d);
        let revision = runs[0].revision();
        let request = dispatch_on(d).map(|(id, n)| command(id, revision, n));
        let receipts: Vec<_> = runs
            .iter_mut()
            .map(|run| run.step(at, request.clone()))
            .collect();
        assert!(
            receipts.windows(2).all(|pair| pair[0] == pair[1]),
            "receipts differ at day {d}"
        );
        let hashes: Vec<_> = runs.iter().map(Run::hashes).collect();
        assert!(
            hashes.windows(2).all(|pair| pair[0] == pair[1]),
            "hashes differ at day {d}"
        );
        if let Some(version) = runs[0]
            .handle()
            .current_domain_record_version(&tally())
            .expect("the tally's version")
        {
            tally_versions.push(version);
        }

        if d % 5 == 0 {
            let (first, rest) = runs.split_at_mut(1);
            let Run::Unsealed(unsealed) = &first[0] else {
                unreachable!("the first run is unsealed");
            };
            for (index, run) in rest.iter_mut().enumerate() {
                let index = index + 1;
                let Run::Compact(compact) = run else {
                    unreachable!("the other runs are compact");
                };
                let release = match keeps[index].expect("a compact run keeps") {
                    Keep::Seal => false,
                    Keep::Release => true,
                    Keep::Alternate => (d / 5) % 2 == 1,
                };
                if release {
                    let (released, (payload, kept, dropped, receipts)) =
                        release_and_check(compact, unsealed, &tally_versions, starts[index], d);
                    receipt_dropped |= receipts;
                    payload_pinned |= payload;
                    version_kept |= kept;
                    version_dropped |= dropped;
                    cuts[index] = None;
                    bases[index] = Some(released.end);
                    starts[index] = released.end;
                } else {
                    let segment = compact
                        .seal_evidence()
                        .expect("a run with work in flight seals")
                        .expect("the settled tail seals");
                    // After a release the next seal starts a new chain at
                    // the release's cut, and the history still begins there.
                    assert_eq!(segment.start, starts[index], "day {d}");
                    if bases[index].is_some() {
                        chained_seal = true;
                        assert_eq!(
                            compact
                                .snapshot_with_segments(vec![segment.clone()])
                                .expect_err("no history before the base")
                                .code,
                            ErrorCode::HistoryUnavailable
                        );
                    }
                    cuts[index] = Some(segment.end);
                    starts[index] = segment.end;
                    // A release straight after a seal has nothing settled to
                    // drop: it returns `None` and keeps the run's history and
                    // segment chain.
                    let base = compact.continuation_base().cloned();
                    assert_eq!(
                        compact.release_evidence().expect("a release after a seal"),
                        None,
                        "day {d}"
                    );
                    assert_eq!(compact.sealed_evidence_cursor(), Some(segment.end));
                    assert_eq!(compact.continuation_base().cloned(), base, "day {d}");
                }
            }
        }

        if d == RESTORE_AT {
            let (checkpoint, hash) = runs[2].continuation();
            let json = checkpoint.to_json().expect("checkpoint JSON");
            let restored =
                CompactedCanwu::from_continuation_checkpoint_json_verified(&json, &hash, &plugins)
                    .expect("a released run's checkpoint restores");
            runs.push(Run::Compact(restored));
            keeps.push(Some(Keep::Release));
            cuts.push(None);
            bases.push(Some(checkpoint.settled_cut));
            starts.push(checkpoint.settled_cut);
        }

        let answered: Vec<_> = runs.iter().map(Run::sealed_cut).collect();
        assert_eq!(answered, cuts, "sealed evidence cursors at day {d}");
        let answered_bases: Vec<_> = runs
            .iter()
            .map(|run| match run {
                Run::Unsealed(_) => None,
                Run::Compact(compact) => compact.continuation_base().map(|base| base.cut),
            })
            .collect();
        assert_eq!(answered_bases, bases, "continuation bases at day {d}");
        let pending: Vec<IngressRecord> = runs[0].handle().pending_ingress().cloned().collect();
        for run in &runs[1..] {
            assert_eq!(
                run.handle().pending_ingress().cloned().collect::<Vec<_>>(),
                pending,
                "pending ingress differs at day {d}"
            );
        }
        if d % 5 == 0 || d % 5 == 1 {
            let checkpoints: Vec<_> = runs
                .iter()
                .map(|run| {
                    let (checkpoint, hash) = run.continuation();
                    (checkpoint.to_json().expect("JSON"), hash)
                })
                .collect();
            assert!(
                checkpoints.windows(2).all(|pair| pair[0] == pair[1]),
                "continuation checkpoints differ at day {d}"
            );
        }
    }
    assert_eq!(runs.len(), 5, "the restored run joined");
    assert!(payload_pinned, "a release pinned a payload-required event");
    assert!(version_kept, "a released version stayed known");
    assert!(version_dropped, "a released version became unreachable");
    assert!(chained_seal, "a seal followed a release");
    assert!(receipt_dropped, "a release dropped a receipt a seal kept");
}

/// Changes the first leaf of a JSON subtree, if it has one.
fn alter_first_leaf(value: &mut Value) -> bool {
    match value {
        Value::Bool(flag) => {
            *flag = !*flag;
            true
        }
        Value::Number(number) => {
            *number = (number.as_u64().unwrap_or_default() + 1).into();
            true
        }
        Value::String(text) => {
            text.push('0');
            true
        }
        Value::Array(items) => items.iter_mut().any(alter_first_leaf),
        Value::Object(fields) => fields.values_mut().any(alter_first_leaf),
        Value::Null => {
            *value = json!(1);
            true
        }
    }
}

fn rehashed(mut checkpoint: ContinuationCheckpoint) -> ContinuationCheckpoint {
    checkpoint.continuation_hash = checkpoint
        .compute_continuation_hash()
        .expect("the material hashes");
    checkpoint
}

#[test]
fn a_tampered_continuation_checkpoint_is_refused() {
    let plugins = plugins();
    let mut run =
        Run::Unsealed(Canwu::new_with_plugins(1602, scenario(), &plugins).expect("fixture starts"));
    for d in 1..=12 {
        let revision = run.revision();
        run.step(
            day(d),
            dispatch_on(d).map(|(id, n)| command(id, revision, n)),
        );
    }
    let (checkpoint, hash) = run.continuation();
    let restore = |checkpoint: ContinuationCheckpoint| {
        CompactedCanwu::from_continuation_checkpoint_with_plugins(checkpoint, &plugins)
    };
    assert!(
        restore(checkpoint.clone()).is_ok(),
        "the untouched checkpoint restores"
    );

    // One altered value in any part fails the checkpoint's own hash, or a
    // check before it, without a panic.
    let wire = serde_json::to_value(&checkpoint).expect("checkpoint JSON");
    let fields: Vec<String> = wire
        .as_object()
        .expect("an object")
        .keys()
        .filter(|field| *field != "continuation_hash")
        .cloned()
        .collect();
    for field in fields {
        let mut altered = wire.clone();
        if !alter_first_leaf(&mut altered[&field]) {
            continue;
        }
        let refused = ContinuationCheckpoint::from_json(&altered.to_string())
            .map_or(true, |altered| restore(altered).is_err());
        assert!(refused, "an altered {field} is refused");
    }

    // A tampered keyed value is refused even with a recomputed hash.
    let mut keyed = checkpoint.clone();
    let entry = keyed.keyed_draws.first_mut().expect("the run drew");
    entry.value = (entry.value + 1) % entry.upper_exclusive;
    let Err(error) = restore(rehashed(keyed)) else {
        panic!("a tampered keyed value is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A changed state field fails the checkpoint hash.
    let mut state = checkpoint.clone();
    state.state.now += SimDuration::minutes(1);
    let Err(error) = restore(rehashed(state)) else {
        panic!("a changed state field is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A missing or an extra closure item fails the round trip.
    let mut missing = checkpoint.clone();
    assert!(!missing.pinned.commands.is_empty() || !missing.pinned.events.is_empty());
    if missing.pinned.events.pop().is_none() {
        missing.pinned.commands.pop();
    }
    assert!(
        restore(rehashed(missing)).is_err(),
        "a missing closure item is refused"
    );
    let mut extra = checkpoint.clone();
    let mut spare = extra
        .unadmitted
        .events
        .first()
        .cloned()
        .expect("an unadmitted event");
    let unpinned = (1..=extra.settled_cut.event_count)
        .find(|id| {
            !extra
                .pinned
                .events
                .iter()
                .any(|event| event.id.get() == *id)
        })
        .expect("most sealed events are not pinned");
    spare.id = canwu_api::EventId::new(unpinned);
    let position = extra
        .pinned
        .events
        .partition_point(|event| event.id.get() < unpinned);
    extra.pinned.events.insert(position, spare);
    assert!(
        restore(rehashed(extra)).is_err(),
        "an extra closure item is refused"
    );

    // A malformed frontier is refused without a panic.
    let mut frontier = checkpoint.clone();
    frontier.journal_frontiers.events.tail.clear();
    let Err(error) = restore(rehashed(frontier)) else {
        panic!("a malformed frontier is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    let mut overflow = checkpoint.clone();
    overflow.journal_frontiers.events.length = u64::MAX;
    assert!(
        restore(rehashed(overflow)).is_err(),
        "an overlong frontier is refused"
    );

    // A boundary ledger that does not end at the last boundary, or reaches
    // back past the horizon, is refused even with a recomputed hash.
    let mut short = checkpoint.clone();
    short
        .settled_boundaries
        .pop()
        .expect("the run settled boundaries");
    let Err(error) = restore(rehashed(short)) else {
        panic!("a ledger without the last boundary is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    let mut long = checkpoint.clone();
    let mut earlier = long
        .settled_boundaries
        .first()
        .cloned()
        .expect("the run settled boundaries");
    earlier.boundary_id = earlier.boundary_id.map(|id| BoundaryId::new(id.get() - 1));
    earlier.at = SimTime::from_minutes(earlier.at.as_minutes() - SimDuration::days(1).as_minutes());
    long.settled_boundaries.insert(0, earlier);
    let Err(error) = restore(rehashed(long)) else {
        panic!("a ledger past the horizon is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A pending packet whose cause lies beyond the journal end is refused
    // even with a recomputed hash.
    let mut cause = checkpoint.clone();
    let beyond = canwu_api::CommandId::new(cause.journal_end.command_count + 1);
    let pending = cause
        .pinned
        .ingress
        .iter_mut()
        .find(|record| matches!(record.cause, Some(CauseRef::Command(_))))
        .expect("a command-caused packet is pending");
    pending.cause = Some(CauseRef::Command(beyond));
    let Err(error) = restore(rehashed(cause)) else {
        panic!("a cause beyond the journal end is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A request index that is not the canonical one for its state is refused
    // even with a recomputed hash: retired ranges that are empty, unsorted,
    // or adjacent; a kept outcome or receipt inside a range; an outcome
    // outside the retention.
    let kept = checkpoint
        .requests
        .commands
        .first()
        .expect("the run settled requests")
        .request_id;
    let range = |first: u64, last: u64| canwu_api::RetiredRequestRange {
        first: CommandRequestId::new(first),
        last: CommandRequestId::new(last),
    };
    let spare = checkpoint
        .requests
        .commands
        .iter()
        .map(|entry| entry.request_id.get())
        .max()
        .unwrap_or_default()
        + 10;
    let forged_indexes = [
        vec![range(spare + 1, spare)],
        vec![range(spare + 3, spare + 4), range(spare, spare + 1)],
        vec![range(spare, spare), range(spare + 1, spare + 1)],
        vec![range(kept.get(), kept.get())],
    ];
    for retired in forged_indexes {
        let mut forged = checkpoint.clone();
        forged.requests.retired_commands = retired.clone();
        let Err(error) = restore(rehashed(forged)) else {
            panic!("a forged retired range {retired:?} is refused");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }
    let mut old_outcome = checkpoint.clone();
    let entry = old_outcome
        .requests
        .commands
        .first_mut()
        .expect("the run settled requests");
    if let canwu_api::CommandOutcome::Accepted { receipt } = &mut entry.outcome {
        receipt.accepted_at = SimTime::from_minutes(
            old_outcome.state.now.as_minutes()
                - canwu_api::DEFAULT_REQUEST_OUTCOME_RETENTION.as_minutes()
                - 1,
        );
    }
    let Err(error) = restore(rehashed(old_outcome)) else {
        panic!("an outcome outside the retention is refused");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // A pending queued command keeps its receipt and is never retired: a
    // forged index that drops the receipt, or retires the pending ID, would
    // let its boundary admit it without an attempt.
    let mut pending_run = run;
    let revision = pending_run.revision();
    let pending = command(900, revision, 900);
    pending_run
        .enqueue(day(20), pending.clone())
        .expect("a command queued for later");
    let (with_pending, _) = pending_run.continuation();
    assert!(restore(with_pending.clone()).is_ok());
    let mut dropped = with_pending.clone();
    dropped
        .requests
        .queued_commands
        .retain(|entry| entry.request_id != pending.request_id);
    let mut retired_pending = with_pending.clone();
    retired_pending.requests.queued_commands = dropped.requests.queued_commands.clone();
    retired_pending.requests.retired_commands = vec![range(900, 900)];
    for (forged, case) in [
        (dropped, "dropped receipt"),
        (retired_pending, "retired pending ID"),
    ] {
        let Err(error) = restore(rehashed(forged)) else {
            panic!("a pending command with a {case} is refused");
        };
        assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    }

    // A pinned payload is in no commitment: with a recomputed hash the
    // unverified restore accepts it, and only the stored hash refuses it.
    let mut payload = checkpoint.clone();
    let pinned = payload
        .pinned
        .ingress
        .first_mut()
        .expect("pending ingress is pinned");
    if let IngressPayload::Plugin { payload, .. } = &mut pinned.payload {
        payload["forged"] = json!(true);
    }
    let payload = rehashed(payload);
    assert!(
        restore(payload.clone()).is_ok(),
        "the unverified restore cannot see a forged pinned payload"
    );
    let Err(error) =
        CompactedCanwu::from_continuation_checkpoint_verified(payload, &hash, &plugins)
    else {
        panic!("the stored continuation hash refuses a forged pinned payload");
    };
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
}

const LATE: &str = "fixture-late-declaration";

fn late_kind() -> DomainRecordKind {
    DomainRecordKind::new(LATE, "late")
}

/// Declares, identity-only, the ingress that an admitted note names.
fn declare_late(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for id in &context.admitted_ingress {
        let Some(record) = view.ingress(*id)? else {
            continue;
        };
        let IngressPayload::Plugin { payload, .. } = &record.payload else {
            continue;
        };
        let Some(named) = payload["declare"].as_u64() else {
            continue;
        };
        let declaration =
            IdentityEvidenceDependenciesV1::new(vec![EvidenceRef::Ingress(IngressId::new(named))]);
        directives.push(BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create {
                record: DomainRecordDraft::new(
                    DomainRecordRef {
                        kind: late_kind(),
                        id: format!("late-{named}"),
                    },
                    json!({ IDENTITY_EVIDENCE_DEPENDENCIES_FIELD: declaration }),
                ),
            },
            summary: "A late declaration".to_owned(),
        });
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

/// A command whose handler schedules a directive that enqueues plugin
/// ingress.
fn nest(
    _view: &SimulationView<'_>,
    _context: &CommandContext,
    _payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    Ok(vec![SystemDirective::Schedule {
        after: SimDuration::days(1),
        directive: Box::new(SystemDirective::EnqueuePluginIngress {
            after: SimDuration::days(1),
            packet_type: "note".to_owned(),
            priority: 0,
            payload: Value::Null,
            affected: Vec::new(),
        }),
    }])
}

struct LateDeclaration;

impl SimulationPlugin for LateDeclaration {
    fn name(&self) -> &'static str {
        LATE
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000016c4"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::new(late_kind(), DomainRecordClass::Record);
        schema.payload_schema = PayloadSchema::Object {
            properties: std::collections::BTreeMap::from([(
                IDENTITY_EVIDENCE_DEPENDENCIES_FIELD.to_owned(),
                identity_evidence_dependencies_property_v1(),
            )]),
            allow_additional: false,
        };
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(PluginIngressDescriptor {
            name: "note".to_owned(),
            description: "fixture note".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        registrar.register_command(
            PluginActionDescriptor {
                name: "nest".to_owned(),
                description: "Schedule a directive that enqueues a note".to_owned(),
                payload_schema: PayloadSchema::Any,
                reads: Vec::new(),
                writes: Vec::new(),
            },
            nest,
        )?;
        let mut declaring = BoundarySystemContract::new(
            "declare",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Daily,
        );
        declaring.reads = vec![StateKey::core_ingress(), record_state(late_kind())];
        declaring.writes = vec![record_state(late_kind())];
        registrar.register_boundary_system(declaring, declare_late)
    }
}

/// An ingress record first declared after a seal moved it out, and no longer
/// held, leaves the sealed run without its provenance: the continuation
/// checkpoint and the next seal report that the archive is not ready, as
/// they do for any evidence declared after its seal, rather than writing a
/// checkpoint that differs from the unsealed run's. Under the continuation
/// gate, the gate's own sealed shadow of `unsealed` meets the same refusal,
/// so the gate must panic.
#[test]
#[cfg_attr(
    feature = "continuation-gate",
    should_panic(
        expected = "continuation gate: the sealed run writes no continuation checkpoint after settle_boundary at revision 2: CanwuError { code: ArchiveNotReady"
    )
)]
fn a_packet_first_declared_after_its_seal_is_not_ready() {
    let plugins: [&dyn SimulationPlugin; 1] = [&LateDeclaration];
    let note = |at: SimTime, declare: Value| {
        PluginIngressRequest::new(LATE, "note", at, json!({ "declare": declare }))
    };
    let daily = |at: SimTime| BoundaryRequest::at(at).with_cadence(SystemCadence::Daily);
    let mut unsealed = Canwu::new_with_plugins(1603, scenario(), &plugins).expect("fixture starts");
    let first = unsealed
        .enqueue_plugin_ingress(note(day(1), Value::Null))
        .expect("the first note queues")
        .ingress_id;
    unsealed.settle_boundary(daily(day(1))).expect("day 1");
    let mut sealed = unsealed.fork().into_compacted().expect("compact copy");
    sealed
        .seal_evidence()
        .expect("the run seals")
        .expect("the tail seals");

    let declare = note(day(2), json!(first.get()));
    unsealed
        .enqueue_plugin_ingress(declare.clone())
        .expect("the second note queues");
    unsealed.settle_boundary(daily(day(2))).expect("day 2");
    sealed
        .enqueue_plugin_ingress(declare)
        .expect("the second note queues");
    sealed.settle_boundary(daily(day(2))).expect("day 2");
    assert_eq!(sealed.checkpoint_hash(), unsealed.checkpoint_hash());

    unsealed
        .continuation_checkpoint()
        .expect("the unsealed run holds the declared note");
    let Err(error) = sealed.continuation_checkpoint() else {
        panic!("the sealed run no longer holds the declared note");
    };
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
    let Err(error) = sealed.seal_evidence() else {
        panic!("the next seal needs the declared note");
    };
    assert_eq!(error.code, ErrorCode::ArchiveNotReady);
}

/// A release is refused exactly where a seal is (G-119): an ingress record
/// first declared after a release or a seal moved it out, and no longer
/// held, leaves both runs unable to write a continuation checkpoint, seal,
/// or release, each with `ArchiveNotReady`, and the refused release leaves
/// the run unchanged. Both runs are compact from the start, so the
/// continuation gate shadows neither.
#[test]
fn a_release_is_refused_where_a_seal_is() {
    let plugins: [&dyn SimulationPlugin; 1] = [&LateDeclaration];
    let note = |at: SimTime, declare: Value| {
        PluginIngressRequest::new(LATE, "note", at, json!({ "declare": declare }))
    };
    let daily = |at: SimTime| BoundaryRequest::at(at).with_cadence(SystemCadence::Daily);
    let mut released = Canwu::new_with_plugins(1606, scenario(), &plugins)
        .expect("fixture starts")
        .into_compacted()
        .expect("compact interface");
    let first = released
        .enqueue_plugin_ingress(note(day(1), Value::Null))
        .expect("the first note queues")
        .ingress_id;
    released.settle_boundary(daily(day(1))).expect("day 1");
    let mut sealed = released.fork();
    let release = released
        .release_evidence()
        .expect("the run releases")
        .expect("the tail releases");
    let segment = sealed
        .seal_evidence()
        .expect("the run seals")
        .expect("the tail seals");
    assert_eq!(release.end, segment.end);
    assert!(
        released.ingress_log().is_empty() && sealed.ingress_log().is_empty(),
        "both runs moved the first note out"
    );

    let declare = note(day(2), json!(first.get()));
    for run in [&mut released, &mut sealed] {
        run.enqueue_plugin_ingress(declare.clone())
            .expect("the second note queues");
        run.settle_boundary(daily(day(2))).expect("day 2");
    }
    assert_eq!(released.checkpoint_hash(), sealed.checkpoint_hash());
    for run in [&mut released, &mut sealed] {
        let cursor = run.evidence_cursor().expect("the journal end");
        let base = run.continuation_base().cloned();
        let sealed_cut = run.sealed_evidence_cursor();
        let boundaries = run.boundaries().len();
        let hash = run.checkpoint_hash().to_owned();
        let codes = [
            run.continuation_checkpoint().map(|_| ()),
            run.release_evidence().map(|_| ()),
            run.seal_evidence().map(|_| ()),
        ]
        .map(|result| result.expect_err("the declared note is gone").code);
        assert_eq!(codes, [const { ErrorCode::ArchiveNotReady }; 3]);
        assert_eq!(run.evidence_cursor().expect("the journal end"), cursor);
        assert_eq!(run.continuation_base().cloned(), base);
        assert_eq!(run.sealed_evidence_cursor(), sealed_cut);
        assert_eq!(run.boundaries().len(), boundaries);
        assert_eq!(run.checkpoint_hash(), hash);
    }
    assert_eq!(released.sealed_evidence_cursor(), None);
    assert_eq!(sealed.sealed_evidence_cursor(), Some(segment.end));
}

/// Ingress that a scheduled directive issued would carry its cause but not
/// its issue time, so a full save could not check it; a handler gives
/// `EnqueuePluginIngress` its own delay instead.
#[test]
fn a_scheduled_directive_cannot_enqueue_plugin_ingress() {
    let plugins: [&dyn SimulationPlugin; 1] = [&LateDeclaration];
    let mut run = Canwu::new_with_plugins(1604, scenario(), &plugins).expect("fixture starts");
    let before = run.checkpoint_hash().to_owned();
    let error = run
        .submit(CommandEnvelope::new(
            Issuer::Debug,
            Command::Plugin {
                plugin: LATE.to_owned(),
                command: "nest".to_owned(),
                payload: Value::Null,
            },
        ))
        .expect_err("a scheduled ingress directive is refused");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert_eq!(run.checkpoint_hash(), before);
}
