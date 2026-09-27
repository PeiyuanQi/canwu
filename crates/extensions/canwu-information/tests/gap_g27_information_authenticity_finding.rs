//! Public-API fixture for gap G-27: an interpretation records the interpreting
//! holder's authenticity finding on a representation's claimed source, bound
//! to the exact representation version it interpreted.

#![allow(clippy::too_many_lines)]

use canwu_api::{
    BoundaryId, BoundaryRequest, Canwu, Command, CommandEnvelope, CommandRequest, CommandRequestId,
    DomainRecord, DomainRecordClass, DomainRecordDraft, DomainRecordLifecycle,
    DomainRecordVersionRef, DomainRecordVersionSource, DomainReference, DomainReferenceTarget,
    EntityRef, Government, GovernmentId, Issuer, KnowledgeHolderRef, KnowledgeQuery,
    KnowledgeSnapshot, MapPoint, Person, PersonId, Scenario, SimTime, SimulationPlugin, Territory,
    TerritoryId, TypedDomainRecordRef, WorldSnapshot,
};
use canwu_information::{
    Access, AccessPayload, AuthenticityFinding, ClaimedSourceRef, Content, ContentPayload,
    ContentRelation, INFORMATION_COMMAND, InformationBody, InformationOperation,
    InformationOperationEnvelope, InformationOperationId, InformationOperationPayload,
    InformationOperationRecord, InformationOperationStatus, InformationOutputKind,
    InformationOutputSlot, InformationPlugin, Interpretation, InterpretationAuthority,
    InterpretationPayload, InterpretationStatus, LifecycleRequest, PLUGIN_NAME, RecordBinding,
    Representation, RepresentationPayload, derive_operation_record_ref, derive_output_record_ref,
};
use serde::Serialize;
use serde_json::json;

const NAMESPACE: &str = "fixture.authenticity";

fn interpreter() -> KnowledgeHolderRef {
    KnowledgeHolderRef::Person(PersonId::new(2))
}

fn holder_reference(role: &str, holder: &KnowledgeHolderRef) -> DomainReference {
    let entity = match holder {
        KnowledgeHolderRef::Person(id) => EntityRef::Person(*id),
        KnowledgeHolderRef::Entity(entity) => entity.clone(),
    };
    DomainReference {
        role: role.to_owned(),
        target: DomainReferenceTarget::Core(entity),
    }
}

fn initial_record<T>(
    reference: TypedDomainRecordRef<T>,
    payload: &T::Payload,
    mut references: Vec<DomainReference>,
) -> DomainRecord
where
    T: canwu_api::DomainRecordType,
    T::Payload: Serialize,
{
    references.sort();
    let mut draft = DomainRecordDraft::from_typed(reference, payload).unwrap();
    draft.references = references;
    DomainRecord {
        reference: draft.reference,
        owner: PLUGIN_NAME.to_owned(),
        class: DomainRecordClass::Record,
        version: 1,
        lifecycle: DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: draft.references,
    }
}

struct Fixture {
    scenario: Scenario,
    access: TypedDomainRecordRef<Access>,
    representation: TypedDomainRecordRef<Representation>,
    result: TypedDomainRecordRef<Content>,
}

/// A representation that claims a source, read by the interpreter.
fn fixture() -> Fixture {
    let world = WorldSnapshot {
        people: (1..=2)
            .map(|index| Person {
                id: PersonId::new(index),
                name: format!("Holder {index}"),
                government: GovernmentId::new(1),
                current_location: TerritoryId::new(1),
                roles: vec!["observer".to_owned()],
                transit: None,
            })
            .collect(),
        governments: vec![Government {
            id: GovernmentId::new(1),
            name: "Fixture Polity".to_owned(),
            capital: TerritoryId::new(1),
        }],
        territories: vec![Territory {
            id: TerritoryId::new(1),
            name: "Fixture Place".to_owned(),
            controller: GovernmentId::new(1),
            position: MapPoint { x: 0.0, y: 0.0 },
        }],
        routes: Vec::new(),
        armies: Vec::new(),
        letters: Vec::new(),
    };
    let content = TypedDomainRecordRef::<Content>::new("claimed-letter-content");
    let result = TypedDomainRecordRef::<Content>::new("claimed-letter-reading");
    let representation = TypedDomainRecordRef::<Representation>::new("claimed-letter");
    let access = TypedDomainRecordRef::<Access>::new("claimed-letter-access");
    let body = |code: &str| InformationBody::InlineJson {
        value: json!({ "code": code }),
    };
    let records = vec![
        initial_record(
            content.clone(),
            &ContentPayload {
                content_type: "letter".to_owned(),
                body: body("letter"),
                created_at: SimTime::EPOCH,
                derivation: None,
            },
            vec![holder_reference(
                "creator",
                &KnowledgeHolderRef::Person(PersonId::new(1)),
            )],
        ),
        initial_record(
            result.clone(),
            &ContentPayload {
                content_type: "reading".to_owned(),
                body: body("reading"),
                created_at: SimTime::EPOCH,
                derivation: None,
            },
            Vec::new(),
        ),
        initial_record(
            representation.clone(),
            &RepresentationPayload {
                format: "sealed_letter".to_owned(),
                created_at: SimTime::EPOCH,
                operation: "write".to_owned(),
                content_relation: ContentRelation::SameContent,
                sources: Vec::new(),
                claimed_source: Some(ClaimedSourceRef {
                    namespace: "fixture.claim".to_owned(),
                    value: "distant-commander".to_owned(),
                }),
                interpretation_capability: None,
            },
            vec![DomainReference::from_typed("content", content)],
        ),
        initial_record(
            access.clone(),
            &AccessPayload {
                accessed_at: SimTime::EPOCH,
                method: "delivered_reading".to_owned(),
                extent_per_mille: 1_000,
            },
            vec![
                holder_reference("holder", &interpreter()),
                DomainReference::from_typed("representation", representation.clone()),
            ],
        ),
    ];
    Fixture {
        scenario: Scenario {
            start_time: SimTime::EPOCH,
            entities: world.entities(),
            world,
            knowledge: KnowledgeSnapshot::default(),
            domain_records: records,
        },
        access,
        representation,
        result,
    }
}

fn interpretation(
    fixture: &Fixture,
    suffix: &str,
    finding: Option<AuthenticityFinding>,
) -> (
    InformationOperationEnvelope,
    TypedDomainRecordRef<Interpretation>,
) {
    let id = InformationOperationId::new(NAMESPACE, format!("interpret-{suffix}"));
    let slot = InformationOutputSlot {
        index: 0,
        name: "result".to_owned(),
        kind: InformationOutputKind::Interpretation,
    };
    let reference =
        TypedDomainRecordRef::<Interpretation>::from_untyped(derive_output_record_ref(&id, &slot))
            .unwrap();
    let envelope = InformationOperationEnvelope {
        id,
        operation_version: 1,
        operation_kind: "record_interpretation".to_owned(),
        output_slots: vec![slot],
        lineage: Vec::new(),
        operation: InformationOperation {
            request: LifecycleRequest::RecordInterpretation {
                binding: RecordBinding::new(
                    reference.clone(),
                    vec![
                        DomainReference::from_typed("input_access", fixture.access.clone()),
                        DomainReference::from_typed(
                            "input_representation",
                            fixture.representation.clone(),
                        ),
                        holder_reference("performed_by", &interpreter()),
                        holder_reference("performed_for", &interpreter()),
                        DomainReference::from_typed("result_content", fixture.result.clone()),
                    ],
                ),
                payload: InterpretationPayload {
                    interpreted_at: SimTime::EPOCH,
                    status: InterpretationStatus::Succeeded,
                    capability: "read_letter".to_owned(),
                    confidence_per_mille: 900,
                    authenticity: finding,
                },
                authority: InterpretationAuthority::HolderSelf,
            },
        },
    };
    (envelope, reference)
}

fn finding(representation: DomainRecordVersionRef) -> AuthenticityFinding {
    AuthenticityFinding {
        representation,
        claimed_source_accepted: false,
        basis: "seal_mismatch".to_owned(),
        confidence_per_mille: 700,
    }
}

fn interpret(canwu: &mut Canwu, request: u64, envelope: &InformationOperationEnvelope) {
    canwu
        .enqueue_command(
            canwu.time(),
            0,
            CommandRequest::new(
                CommandRequestId::new(request),
                canwu.revision(),
                CommandEnvelope::new(
                    Issuer::System("fixture-interpreter".to_owned()),
                    Command::Plugin {
                        plugin: PLUGIN_NAME.to_owned(),
                        command: INFORMATION_COMMAND.to_owned(),
                        payload: serde_json::to_value(envelope).unwrap(),
                    },
                )
                .at_time(canwu.time()),
            ),
        )
        .unwrap();
    for _ in 0..6 {
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .unwrap();
        if canwu
            .typed_domain_record(&derive_operation_record_ref(&envelope.id))
            .is_some_and(|record| {
                record
                    .decode_payload::<InformationOperationRecord>()
                    .is_ok_and(|payload| payload.status.is_terminal())
            })
        {
            return;
        }
    }
    panic!("interpretation operation did not terminate");
}

fn operation(canwu: &Canwu, id: &InformationOperationId) -> InformationOperationPayload {
    canwu
        .typed_domain_record(&derive_operation_record_ref(id))
        .unwrap()
        .decode_payload::<InformationOperationRecord>()
        .unwrap()
}

#[test]
fn gap_g27_information_authenticity_finding() {
    let fixture = fixture();
    let plugins: &[&dyn SimulationPlugin] = &[&InformationPlugin];
    let mut canwu = Canwu::new_with_plugins(27, fixture.scenario.clone(), plugins).unwrap();
    let interpreted = DomainRecordVersionRef {
        record: fixture.representation.as_untyped().clone(),
        version: 1,
        established_by: DomainRecordVersionSource::InitialScenario,
    };

    // The interpreter rejects the claimed source of the exact version it read.
    let (accepted, accepted_record) =
        interpretation(&fixture, "bound", Some(finding(interpreted.clone())));
    interpret(&mut canwu, 1, &accepted);
    assert_eq!(
        operation(&canwu, &accepted.id).status,
        InformationOperationStatus::Completed
    );
    let stored = canwu
        .typed_domain_record(&accepted_record)
        .unwrap()
        .decode_payload::<Interpretation>()
        .unwrap();
    assert_eq!(stored.authenticity, Some(finding(interpreted.clone())));
    let ledger = canwu
        .admin_query_knowledge(interpreter(), &KnowledgeQuery::default())
        .unwrap();
    assert!(
        ledger
            .records
            .iter()
            .any(|record| record.schema.kind.name == "interpretation_recorded")
    );

    // A finding citing another version of the representation, or the right
    // version number under other establishing evidence, is rejected and
    // records no interpretation.
    let later_version = DomainRecordVersionRef {
        version: 2,
        ..interpreted.clone()
    };
    let other_evidence = DomainRecordVersionRef {
        established_by: DomainRecordVersionSource::BoundaryChange {
            boundary: BoundaryId::new(1),
            change_index: 0,
        },
        ..interpreted.clone()
    };
    for (index, cited) in [later_version, other_evidence].into_iter().enumerate() {
        let (rejected, rejected_record) =
            interpretation(&fixture, &format!("unbound-{index}"), Some(finding(cited)));
        interpret(&mut canwu, 2 + index as u64, &rejected);
        let payload = operation(&canwu, &rejected.id);
        assert_eq!(payload.status, InformationOperationStatus::Rejected);
        assert_eq!(payload.rejection_code.as_deref(), Some("invalid_lifecycle"));
        assert!(canwu.typed_domain_record(&rejected_record).is_none());
    }

    // Interpretations without a finding keep their exact encoding, and the
    // finding itself loads strictly.
    let (plain, _) = interpretation(&fixture, "plain", None);
    let LifecycleRequest::RecordInterpretation { payload, .. } = &plain.operation.request else {
        unreachable!("fixture builds an interpretation");
    };
    assert!(
        serde_json::to_value(payload)
            .unwrap()
            .get("authenticity")
            .is_none()
    );
    let mut unknown = serde_json::to_value(finding(interpreted)).unwrap();
    unknown["forger"] = json!("unexpected");
    assert!(serde_json::from_value::<AuthenticityFinding>(unknown).is_err());

    let restored =
        Canwu::from_snapshot_json_with_plugins(&canwu.snapshot_json().unwrap(), plugins).unwrap();
    assert_eq!(restored.snapshot(), canwu.snapshot());
    let replayed = Canwu::replay_from_journal(plugins, &canwu.replay_journal()).unwrap();
    assert_eq!(replayed.snapshot(), canwu.snapshot());
}
