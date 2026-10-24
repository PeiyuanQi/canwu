//! Host calendar marks on calendar boundaries (downstream gap G-69).
//!
//! A host that runs two calendars marks each calendar boundary with the
//! calendars whose period starts there. A boundary system keyed to a
//! calendar wakes only on its own calendar's marks of its cadence; a system
//! without a key wakes on every boundary of its cadence and reads the marks
//! on its view. The marks are on the boundary ledger before and after the
//! boundary settles; a snapshot restore (with marked ingress still queued and
//! after the last boundary), a continuation restore, and exact replay wake
//! the same systems, and a snapshot whose boundary lost a mark is refused.

#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, CalendarMark, Canwu, CanwuError, CompactedCanwu, DomainRecordClass,
    DomainRecordDraft, DomainRecordKind, DomainRecordMutation, DomainRecordRef, DomainRecordSchema,
    EntityRef, ErrorCode, KnowledgeHolderRef, KnowledgeOrigin, KnowledgePublicationGroup,
    KnowledgeRecordDraft, KnowledgeRecordKind, KnowledgeRetention, KnowledgeSchemaId,
    KnowledgeSubject, KnowledgeSubjectSchema, KnowledgeSubjectTarget, KnowledgeSubjectTargetKind,
    KnowledgeSupersession, KnowledgeWriteGrant, OrganizationId, PayloadSchema, PersonId,
    PluginKnowledgeSchema, PluginRegistrar, Scenario, SimDuration, SimEvent, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
};
use serde_json::{Value, json};

const PLUGIN: &str = "fixture-calendar-marks";

fn tally() -> DomainRecordRef {
    DomainRecordRef {
        kind: DomainRecordKind::new(PLUGIN, "tally"),
        id: "southern".to_owned(),
    }
}

fn tally_state() -> StateKey {
    DomainRecordSchema::new(tally().kind, DomainRecordClass::Record).state_key()
}

fn season_schema() -> KnowledgeSchemaId {
    KnowledgeSchemaId::new(KnowledgeRecordKind::new(PLUGIN, "season"), 1)
}

const fn holder() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(1))
}

const fn polity() -> EntityRef {
    EntityRef::Organization(OrganizationId::new(10))
}

/// The emission that reports that the system woke and the marks its view
/// shows.
fn woke_directive(view: &SimulationView<'_>, context: &BoundaryContext) -> BoundaryDirective {
    let marks: Vec<String> = view
        .calendar_marks()
        .iter()
        .map(|mark| format!("{:?}/{}", mark.cadence, mark.calendar))
        .collect();
    BoundaryDirective::Emit {
        event_type: "woke".to_owned(),
        summary: format!("{} [{}]", context.system, marks.join(",")),
        affected: Vec::new(),
    }
}

fn wake(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![woke_directive(view, context)],
        ..BoundaryProposal::default()
    })
}

/// Wakes and counts its wakes in a record, so load validation checks a
/// keyed writer's record change against its marks.
fn wake_and_count(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let count = view
        .domain_record(&tally())?
        .map_or(0, |record| record.version);
    let record = DomainRecordDraft::new(tally(), json!({ "wakes": count + 1 }));
    let mutation = if count == 0 {
        DomainRecordMutation::Create { record }
    } else {
        DomainRecordMutation::Update {
            record,
            expected_version: count,
        }
    };
    Ok(BoundaryProposal {
        directives: vec![
            woke_directive(view, context),
            BoundaryDirective::MutateRecord {
                mutation,
                summary: "The southern month is counted".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

/// Wakes and publishes the season to the holder, so load validation checks
/// a keyed publisher's knowledge change against its marks.
fn wake_and_publish(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    Ok(BoundaryProposal {
        directives: vec![
            woke_directive(view, context),
            BoundaryDirective::PublishKnowledge {
                visibility: StateVisibility::SameBoundary,
                producer_correlation: None,
                supersession: KnowledgeSupersession::Explicit,
                groups: vec![KnowledgePublicationGroup {
                    holders: vec![holder()],
                    records: vec![KnowledgeRecordDraft {
                        schema: season_schema(),
                        subjects: vec![KnowledgeSubject {
                            role: "polity".to_owned(),
                            target: KnowledgeSubjectTarget::Entity(polity()),
                        }],
                        payload: json!({ "season": context.at }),
                        as_of: Some(context.at),
                        confidence_per_mille: 1_000,
                        origin: KnowledgeOrigin {
                            method: "fixture.calendar".to_owned(),
                            evidence: Vec::new(),
                        },
                        supersedes: Vec::new(),
                        contradicts: Vec::new(),
                    }],
                }],
                summary: "The northern season starts".to_owned(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

struct CalendarPlugin;

impl SimulationPlugin for CalendarPlugin {
    fn name(&self) -> &'static str {
        PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000006900"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_record_schema(DomainRecordSchema::new(
            tally().kind,
            DomainRecordClass::Record,
        ))?;
        registrar.register_knowledge_schema(PluginKnowledgeSchema {
            id: season_schema(),
            schema_hash: "0000000000000000000000000000000000000000000000000000000000006901"
                .to_owned(),
            writable: true,
            payload_schema: PayloadSchema::Any,
            subjects: vec![KnowledgeSubjectSchema {
                role: "polity".to_owned(),
                targets: vec![KnowledgeSubjectTargetKind::AnyEntity],
                required: true,
                multiple: false,
            }],
            retention: KnowledgeRetention::Keep,
        })?;
        let mut any = BoundarySystemContract::new(
            "any",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Monthly,
        );
        any.emits = vec!["woke".to_owned()];
        registrar.register_boundary_system(any, wake)?;
        let mut northern = BoundarySystemContract::new(
            "northern",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Monthly,
        );
        northern.calendar = Some("northern".to_owned());
        northern.emits = vec!["woke".to_owned()];
        registrar.register_boundary_system(northern, wake)?;
        let mut southern = BoundarySystemContract::new(
            "southern",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::Monthly,
        );
        southern.calendar = Some("southern".to_owned());
        southern.reads = vec![tally_state()];
        southern.writes = vec![tally_state()];
        southern.emits = vec!["woke".to_owned()];
        registrar.register_boundary_system(southern, wake_and_count)?;
        let mut season = BoundarySystemContract::new(
            "season-north",
            BoundaryPhase::PerspectiveAndReportMaterialization,
            SystemCadence::Seasonal,
        );
        season.calendar = Some("northern".to_owned());
        season.emits = vec!["woke".to_owned()];
        season.knowledge_writes = vec![KnowledgeWriteGrant {
            schema: season_schema(),
            visibilities: vec![StateVisibility::SameBoundary],
        }];
        registrar.register_boundary_system(season, wake_and_publish)
    }
}

fn day(day: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(day)
}

fn monthly(calendar: &str) -> CalendarMark {
    CalendarMark::new(SystemCadence::Monthly, calendar)
}

/// The systems that woke at `at`, with the marks each saw, in name order.
fn woke(events: &[SimEvent], at: SimTime) -> Vec<String> {
    let mut woke: Vec<String> = events
        .iter()
        .filter(|event| {
            event.timestamp == at && event.kind.field("event_type") == Some(json!("woke"))
        })
        .map(|event| event.summary.clone())
        .collect();
    woke.sort();
    woke
}

/// Queues the calendar input of day `d`.
fn schedule(run: &mut dyn FnMut(SimTime, Vec<CalendarMark>, Vec<SystemCadence>), d: i64) {
    match d {
        0 => run(
            day(0),
            vec![monthly("southern"), monthly("northern")],
            Vec::new(),
        ),
        1 => run(
            day(1),
            vec![monthly("southern")],
            vec![SystemCadence::Daily],
        ),
        2 => run(
            day(2),
            vec![CalendarMark::new(SystemCadence::Seasonal, "northern")],
            Vec::new(),
        ),
        3 => run(
            day(3),
            Vec::new(),
            vec![SystemCadence::Monthly, SystemCadence::Seasonal],
        ),
        _ => unreachable!("the fixture runs four days"),
    }
}

fn expected(d: i64) -> Vec<String> {
    let both = "[Monthly/northern,Monthly/southern]";
    match d {
        0 => vec![
            format!("any {both}"),
            format!("northern {both}"),
            format!("southern {both}"),
        ],
        1 => vec![
            "any [Monthly/southern]".to_owned(),
            "southern [Monthly/southern]".to_owned(),
        ],
        2 => vec!["season-north [Seasonal/northern]".to_owned()],
        // An unmarked boundary wakes only the systems without a key.
        3 => vec!["any []".to_owned()],
        _ => unreachable!("the fixture runs four days"),
    }
}

#[test]
fn a_keyed_system_wakes_only_on_its_own_calendar_marks() {
    let plugin = CalendarPlugin;
    let plugins: [&dyn SimulationPlugin; 1] = [&plugin];
    let scenario = Scenario::new(
        SimTime::EPOCH,
        vec![EntityRef::Person(PersonId::new(1)), polity()],
    );
    let mut canwu =
        Canwu::new_with_plugins(69, scenario, &plugins).expect("the fixture should start");

    // Refusals before anything is queued.
    for marks in [
        Vec::new(),
        vec![CalendarMark::new(SystemCadence::EventDriven, "northern")],
        vec![monthly(" northern")],
        vec![monthly("north\u{7}ern")],
        vec![monthly(&"x".repeat(canwu_api::MAX_CALENDAR_KEY_BYTES + 1))],
    ] {
        let error = canwu
            .schedule_calendar_marks(day(0), marks)
            .expect_err("a malformed mark list is refused");
        assert_eq!(error.code, ErrorCode::InvalidBoundary);
    }

    for d in 0..4 {
        schedule(
            &mut |at, marks, cadences| {
                if !marks.is_empty() {
                    canwu.schedule_calendar_marks(at, marks).expect("marks");
                }
                if !cadences.is_empty() {
                    canwu
                        .schedule_calendar_boundary(at, cadences)
                        .expect("cadences");
                }
            },
            d,
        );
    }

    // The boundary ledger lists the queued marks, merged per instant, with
    // the marks' cadences among the boundary's cadences.
    let window = canwu
        .boundaries_between(day(0), day(4))
        .expect("the queued window");
    assert_eq!(window.queued.len(), 4);
    assert_eq!(
        window.queued[0].calendar_marks,
        vec![monthly("northern"), monthly("southern")]
    );
    assert_eq!(
        window.queued[1].cadences,
        vec![SystemCadence::Daily, SystemCadence::Monthly]
    );
    assert_eq!(window.queued[1].calendar_marks, vec![monthly("southern")]);
    assert!(window.queued[3].calendar_marks.is_empty());

    for d in 0..2 {
        canwu
            .settle_boundary(BoundaryRequest::at(day(d)))
            .expect("a calendar boundary");
        assert_eq!(woke(canwu.events(), day(d)), expected(d), "day {d}");
    }
    assert_eq!(
        canwu
            .boundaries()
            .last()
            .map(|record| &record.calendar_marks),
        Some(&vec![monthly("southern")]),
        "the boundary record keeps its marks"
    );

    // A snapshot restore and a continuation restore taken between the
    // boundaries, with marked calendar ingress still queued, wake the same
    // systems.
    let mut from_snapshot =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().expect("snapshot"), &plugins)
            .expect("the snapshot with queued marks loads");
    assert_eq!(from_snapshot.checkpoint_hash(), canwu.checkpoint_hash());
    let (checkpoint, hash) = canwu.continuation_checkpoint().expect("checkpoint");
    let mut restored =
        CompactedCanwu::from_continuation_checkpoint_verified(checkpoint, &hash, &plugins)
            .expect("the continuation restores");
    for d in 2..4 {
        let live = canwu
            .settle_boundary(BoundaryRequest::at(day(d)))
            .expect("a calendar boundary");
        let loaded = from_snapshot
            .settle_boundary(BoundaryRequest::at(day(d)))
            .expect("a calendar boundary");
        let continued = restored
            .settle_boundary(BoundaryRequest::at(day(d)))
            .expect("a calendar boundary");
        assert_eq!(loaded, live);
        assert_eq!(continued, live);
        assert_eq!(woke(canwu.events(), day(d)), expected(d), "day {d}");
        assert_eq!(woke(from_snapshot.events(), day(d)), expected(d), "day {d}");
        assert_eq!(woke(restored.events(), day(d)), expected(d), "day {d}");
    }
    assert_eq!(from_snapshot.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(restored.checkpoint_hash(), canwu.checkpoint_hash());

    // After the last boundary a snapshot loads, its keyed writer's record
    // change, keyed publisher's knowledge, and keyed emitters checked
    // against the marks; one whose boundary lost a mark is refused.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let loaded =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("the snapshot loads");
    assert_eq!(loaded.checkpoint_hash(), canwu.checkpoint_hash());
    let mut tampered: Value = serde_json::from_str(&snapshot).expect("snapshot JSON");
    let marks = tampered["boundaries"][0]["calendar_marks"]
        .as_array_mut()
        .expect("the first boundary carries marks");
    assert_eq!(marks.len(), 2);
    marks.pop();
    let error = Canwu::from_snapshot_json_with_plugins(&tampered.to_string(), &plugins)
        .err()
        .expect("a boundary that lost a mark is refused");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);
    // So is a mark no admitted calendar ingress set and no system depends
    // on (the boundary hash covers the marks).
    let mut tampered: Value = serde_json::from_str(&snapshot).expect("snapshot JSON");
    tampered["boundaries"][3]["calendar_marks"] =
        json!([{"cadence": "monthly", "calendar": "eastern"}]);
    let error = Canwu::from_snapshot_json_with_plugins(&tampered.to_string(), &plugins)
        .err()
        .expect("a boundary with a mark its ingress did not set is refused");
    assert_eq!(error.code, ErrorCode::InvalidSnapshot);

    // Exact replay re-queues the marked calendar ingress and reaches the
    // same state.
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("exact replay");
    assert_eq!(replayed.checkpoint_hash(), canwu.checkpoint_hash());
    assert_eq!(replayed.time(), day(3));
}
