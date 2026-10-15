//! Observer bindings can be held without a person, re-pointed, and removed,
//! and every binding keeps its report lines whatever happens to the others.
//!
//! A binding without an actor is held by a government. A binding with a
//! controlling institution can be re-pointed to another holder, who gets the
//! whole report at the next publication, while the old holder gets one last
//! summary marked withdrawn and keeps what it was told; or removed, after
//! which nothing is published to it. A binding's lines are numbered from the
//! slot the scenario gave it, once: re-pointing or removing one binding moves
//! no other binding's lines, and two bindings never share a slot.

mod support;

use canwu_api::{
    Canwu, EntityRef, ErrorCode, GovernmentId, KnowledgeHolderRef, KnowledgeSubject, PersonId,
};
use canwu_fiscal::{
    FISCAL_OBSERVER_REMOVED, FiscalAction, FiscalActionDisposition, FiscalActionTarget,
    FiscalObserverBinding, FiscalReportRecord, FiscalSettledAction, fiscal_reports,
    fiscal_reports_of_holder,
};
use std::collections::BTreeSet;
use support::{
    AUDITOR, DEPUTY, Draft, Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, PRINCIPAL,
    SECOND_AUDITOR, SUCCESSOR, TREASURY_AUTHORITY, admission_rejection, assessment, draft,
    enqueue_life, fiscal_state, observer_binding, published_report_records, scenario, settle,
    submit, treasury, treasury_scope,
};

const CONTROLLED: &str = "observer.controlled";
const GOVERNMENT: &str = "observer.government";
const OTHER: &str = "observer.other";
const REVIVED: &str = "observer.revived";

fn government() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(1)))
}

fn controlled(id: &str, slot: u16, actor: PersonId) -> FiscalObserverBinding {
    FiscalObserverBinding {
        controlled_by: Some(treasury()),
        ..observer_binding(id, slot, actor, BTreeSet::from([treasury()]))
    }
}

fn bindings_scenario() -> canwu_api::Scenario {
    scenario(|state| {
        let held_by_government = FiscalObserverBinding {
            actor: None,
            knowledge_holder: government(),
            ..observer_binding(GOVERNMENT, 2, PRINCIPAL, BTreeSet::from([treasury()]))
        };
        for binding in [
            controlled(CONTROLLED, 1, DEPUTY),
            held_by_government,
            observer_binding(OTHER, 3, AUDITOR, BTreeSet::from([treasury()])),
            controlled(REVIVED, 4, GRANARY_PRINCIPAL),
        ] {
            state.observer_bindings.insert(binding.id.clone(), binding);
        }
    })
}

fn rebind(
    action_id: &str,
    observer: &str,
    actor: Option<PersonId>,
    holder: KnowledgeHolderRef,
) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::RebindObserver {
            observer_binding_id: observer.to_owned(),
            actor,
            knowledge_holder: holder,
        },
    )
}

fn remove(action_id: &str, observer: &str) -> Draft {
    draft(
        action_id,
        TREASURY_AUTHORITY,
        FiscalAction::RemoveObserver {
            observer_binding_id: observer.to_owned(),
        },
    )
}

fn reports(
    canwu: &Canwu,
    actor: PersonId,
) -> std::collections::BTreeMap<String, canwu_fiscal::FiscalReport> {
    fiscal_reports(&canwu.viewer_for_actor(actor).expect("viewer")).expect("reports")
}

fn applied(canwu: &Canwu, action_id: &str) -> bool {
    FiscalSettledAction::from_events(canwu.events())
        .iter()
        .any(|settled| {
            settled.action_id == action_id
                && settled.disposition == FiscalActionDisposition::Applied
        })
}

/// The lines a holder's records of one binding were published on: the
/// subjects of each, by its summary or part number.
fn lines(
    canwu: &Canwu,
    actor: PersonId,
    observer: &str,
) -> BTreeSet<(Option<u32>, Vec<KnowledgeSubject>)> {
    published_report_records(canwu, actor)
        .into_iter()
        .filter_map(|(record, payload)| match payload {
            FiscalReportRecord::Summary(summary) if summary.observer_id == observer => {
                Some((None, record.subjects))
            }
            FiscalReportRecord::Part(part) if part.observer_id == observer => {
                Some((Some(part.part), record.subjects))
            }
            _ => None,
        })
        .collect()
}

fn assess(action_id: &str, scope: usize, cycle: &str) -> Vec<(PersonId, Draft)> {
    vec![(
        PRINCIPAL,
        assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(scope), cycle),
    )]
}

#[test]
#[allow(clippy::too_many_lines)]
fn bindings_are_held_by_governments_re_pointed_and_removed_keeping_every_line() {
    let fixture = Fixture::new();
    let mut canwu = fixture.runtime(bindings_scenario());

    // The first change reaches every binding, the government's included.
    submit(&mut canwu, assess("first", 0, "2026"));
    let held = fiscal_reports_of_holder(&canwu, &government()).expect("the government's reports");
    assert_eq!(held[GOVERNMENT].projection.facts.len(), 1);
    assert_eq!(held[GOVERNMENT].projection.actor, None);
    assert!(reports(&canwu, DEPUTY).contains_key(CONTROLLED));
    let other_lines = lines(&canwu, AUDITOR, OTHER);
    let deputy_lines = lines(&canwu, DEPUTY, CONTROLLED);

    // Re-pointed to the successor: he gets the whole report on the binding's
    // own lines; the deputy gets one withdrawn summary and no longer reads the
    // report; no other binding is published to or moves.
    let auditor_records = published_report_records(&canwu, AUDITOR).len();
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind(
                "rebind",
                CONTROLLED,
                Some(SUCCESSOR),
                KnowledgeHolderRef::Person(SUCCESSOR),
            ),
        )],
    );
    assert!(applied(&canwu, "rebind"));
    let successor = reports(&canwu, SUCCESSOR);
    assert!(successor[CONTROLLED].is_complete());
    assert_eq!(successor[CONTROLLED].projection.facts.len(), 1);
    assert_eq!(successor[CONTROLLED].projection.actor, Some(SUCCESSOR));
    assert_eq!(lines(&canwu, SUCCESSOR, CONTROLLED), deputy_lines);
    assert!(!reports(&canwu, DEPUTY).contains_key(CONTROLLED));
    let withdrawn = published_report_records(&canwu, DEPUTY)
        .into_iter()
        .filter_map(|(_, payload)| match payload {
            FiscalReportRecord::Summary(summary) if summary.withdrawn => Some(summary),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(withdrawn.len(), 1);
    assert_eq!(
        published_report_records(&canwu, AUDITOR).len(),
        auditor_records
    );
    let binding = &fiscal_state(&canwu).observer_bindings[CONTROLLED];
    assert_eq!(binding.actor, Some(SUCCESSOR));
    assert_eq!(binding.line_slot, 1);
    assert_eq!(
        binding.rebound_by.as_ref().map(|by| by.action_id.as_str()),
        Some("rebind")
    );

    // A later change reaches the successor and the others on their own
    // lines, and the deputy no more.
    let deputy_records = published_report_records(&canwu, DEPUTY).len();
    submit(&mut canwu, assess("second", 1, "2026"));
    assert_eq!(
        reports(&canwu, SUCCESSOR)[CONTROLLED]
            .projection
            .facts
            .len(),
        2
    );
    assert_eq!(
        published_report_records(&canwu, DEPUTY).len(),
        deputy_records
    );
    assert_eq!(lines(&canwu, AUDITOR, OTHER), other_lines);
    assert_eq!(reports(&canwu, AUDITOR)[OTHER].projection.facts.len(), 2);

    // Removed: the successor gets a withdrawn summary and nothing after it;
    // the binding stays, keeping its slot, and cannot be re-pointed again.
    submit(&mut canwu, vec![(PRINCIPAL, remove("remove", CONTROLLED))]);
    assert!(applied(&canwu, "remove"));
    assert!(!reports(&canwu, SUCCESSOR).contains_key(CONTROLLED));
    let successor_records = published_report_records(&canwu, SUCCESSOR).len();
    submit(&mut canwu, assess("third", 2, "2026"));
    assert_eq!(
        published_report_records(&canwu, SUCCESSOR).len(),
        successor_records
    );
    let state = fiscal_state(&canwu);
    assert!(!state.observer_bindings[CONTROLLED].is_active());
    assert_eq!(state.observer_bindings[CONTROLLED].line_slot, 1);
    assert!(
        state
            .recent_outcome(
                &FiscalActionTarget::Observer(CONTROLLED.to_owned()),
                "remove"
            )
            .is_some()
    );
    assert_eq!(lines(&canwu, AUDITOR, OTHER), other_lines);
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind(
                "rebind.removed",
                CONTROLLED,
                Some(DEPUTY),
                KnowledgeHolderRef::Person(DEPUTY),
            ),
        )],
    );
    let error = admission_rejection(&canwu, "rebind.removed").expect("refused");
    assert_eq!(error.code, ErrorCode::InvalidPayload);
    assert_eq!(error.message, FISCAL_OBSERVER_REMOVED);

    // Only the controlling institution re-points a binding, and a binding
    // without one changes only with the scenario.
    submit(
        &mut canwu,
        vec![
            (
                PRINCIPAL,
                rebind(
                    "rebind.uncontrolled",
                    OTHER,
                    Some(DEPUTY),
                    KnowledgeHolderRef::Person(DEPUTY),
                ),
            ),
            (
                GRANARY_PRINCIPAL,
                draft(
                    "rebind.granary",
                    GRANARY_AUTHORITY,
                    FiscalAction::RebindObserver {
                        observer_binding_id: REVIVED.to_owned(),
                        actor: Some(DEPUTY),
                        knowledge_holder: KnowledgeHolderRef::Person(DEPUTY),
                    },
                ),
            ),
            (
                PRINCIPAL,
                rebind(
                    "rebind.mismatch",
                    REVIVED,
                    Some(DEPUTY),
                    KnowledgeHolderRef::Person(AUDITOR),
                ),
            ),
            (
                PRINCIPAL,
                rebind(
                    "rebind.nobody",
                    REVIVED,
                    Some(PersonId::new(777)),
                    KnowledgeHolderRef::Person(PersonId::new(777)),
                ),
            ),
            (
                PRINCIPAL,
                rebind(
                    "rebind.no-government",
                    REVIVED,
                    None,
                    KnowledgeHolderRef::Entity(EntityRef::Government(GovernmentId::new(9))),
                ),
            ),
        ],
    );
    for (action_id, code) in [
        ("rebind.uncontrolled", ErrorCode::InvalidAuthority),
        ("rebind.granary", ErrorCode::InvalidAuthority),
        ("rebind.mismatch", ErrorCode::InvalidPayload),
        ("rebind.nobody", ErrorCode::EntityNotFound),
        ("rebind.no-government", ErrorCode::EntityNotFound),
    ] {
        assert_eq!(
            admission_rejection(&canwu, action_id)
                .unwrap_or_else(|| panic!("{action_id} is refused"))
                .code,
            code,
            "{action_id}"
        );
    }

    // Re-pointed to a holder who is dead, the whole report is owed, and the
    // boundary that revives him publishes it.
    enqueue_life(&mut canwu, SECOND_AUDITOR, false);
    settle(&mut canwu);
    submit(
        &mut canwu,
        vec![(
            PRINCIPAL,
            rebind(
                "rebind.dead",
                REVIVED,
                Some(SECOND_AUDITOR),
                KnowledgeHolderRef::Person(SECOND_AUDITOR),
            ),
        )],
    );
    assert!(applied(&canwu, "rebind.dead"));
    assert!(published_report_records(&canwu, SECOND_AUDITOR).is_empty());
    assert!(
        fiscal_state(&canwu)
            .report_plan
            .deferred
            .contains_key(REVIVED)
    );
    enqueue_life(&mut canwu, SECOND_AUDITOR, true);
    settle(&mut canwu);
    let revived = reports(&canwu, SECOND_AUDITOR);
    assert!(revived[REVIVED].is_complete());
    assert_eq!(revived[REVIVED].projection.facts.len(), 3);

    fixture.assert_restore_and_replay(&canwu);
}

/// Line slots are the scenario's, once per binding: a state in which two
/// bindings share a slot, or a slot lies past the last one, is refused when
/// the run is created.
#[test]
fn two_bindings_never_share_a_line_slot() {
    let fixture = Fixture::new();
    let valid = scenario(|state| {
        let binding = observer_binding("observer.second", 1, DEPUTY, BTreeSet::from([treasury()]));
        state.observer_bindings.insert(binding.id.clone(), binding);
    });
    assert!(fixture.try_runtime(valid.clone()).is_ok());
    for slot in [0, 64] {
        let mut scenario = valid.clone();
        let record = scenario
            .domain_records
            .iter_mut()
            .find(|record| {
                record.reference == canwu_fiscal::fiscal_state_reference().into_untyped()
            })
            .expect("the fiscal state record");
        record.payload["observer_bindings"]["observer.second"]["line_slot"] = slot.into();
        let error = fixture
            .try_runtime(scenario)
            .err()
            .unwrap_or_else(|| panic!("slot {slot} is refused"));
        assert!(error.message.contains("line slot"), "{error}");
    }
}
