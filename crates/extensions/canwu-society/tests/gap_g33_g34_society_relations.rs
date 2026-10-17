//! Downstream gaps G-33 and G-34: a vocabulary that bounds organization
//! relation kinds, and intergroup relations between cohort classes that a
//! scenario authors and canonical ingress revises.

use canwu_api::{
    BoundaryRequest, Canwu, DomainRecord, EntityRef, ErrorCode, EvidenceRef, IngressId,
    PluginIngressRequest, Scenario, SimDuration, SimTime, SimulationPlugin, SystemCadence,
    TerritoryId,
};
use canwu_society::{
    AffiliationTarget, CohortClass, DispositionBucket, DispositionDistribution, DispositionProfile,
    INTERGROUP_RELATION_CLASS_REJECTION, INTERGROUP_RELATION_IDENTITY_REJECTION,
    INTERGROUP_RELATION_INGRESS, INTERGROUP_RELATION_KIND_REJECTION,
    INTERGROUP_RELATION_REGION_REJECTION, INTERGROUP_RELATION_REJECTED_EVENT,
    INTERGROUP_RELATION_STALE_REJECTION, IntergroupRelationChange, IntergroupRelationEdge,
    IntergroupRelationRevisionV1, IntergroupRelations, OrganizationNode, OrganizationRelation,
    SocietyCohort, SocietyPlugin, SocietyState, distribution_id, intergroup_relations_reference,
    load_intergroup_relations, society_state_reference, validate_society_runtime,
};
use std::collections::{BTreeMap, BTreeSet};

fn day(days: i64) -> SimTime {
    SimTime::EPOCH + SimDuration::days(days)
}

/// Three cohorts in two classes of one dimension, with two organizations of
/// one target and a patronage relation between them.
fn society() -> SocietyState {
    let ids = Canwu::demo_ids();
    let neutral = DispositionProfile::neutral();
    let mut state = SocietyState::default();
    state.targets.insert(
        "idea".to_owned(),
        AffiliationTarget {
            id: "idea".to_owned(),
            parent: None,
            neutral_profile: neutral,
            metadata: BTreeMap::new(),
        },
    );
    for (id, territory, people) in [
        ("hill-town", ids.western_territory, "hill"),
        ("hill-farms", ids.central_territory, "hill"),
        ("river-port", ids.western_territory, RIVER_FOLK),
    ] {
        state.cohorts.insert(
            id.to_owned(),
            SocietyCohort {
                id: id.to_owned(),
                territory,
                headcount: 1_000,
                classification: BTreeMap::from([("people".to_owned(), people.to_owned())]),
            },
        );
        state.distributions.insert(
            distribution_id(id, "idea"),
            DispositionDistribution {
                id: distribution_id(id, "idea"),
                cohort_id: id.to_owned(),
                target_id: "idea".to_owned(),
                buckets: vec![DispositionBucket {
                    profile: neutral,
                    headcount: 1_000,
                }],
            },
        );
    }
    for id in ["guild", "temple"] {
        state.organizations.insert(
            id.to_owned(),
            OrganizationNode {
                id: id.to_owned(),
                target_id: "idea".to_owned(),
                base_reach_per_mille: 100,
                concealment_per_mille: 0,
                active: true,
            },
        );
    }
    state.organization_relations.insert(
        "guild-temple".to_owned(),
        OrganizationRelation {
            id: "guild-temple".to_owned(),
            source_organization_id: "guild".to_owned(),
            target_organization_id: "temple".to_owned(),
            relation: "patronage".to_owned(),
            strength_per_mille: 200,
        },
    );
    state
}

fn hill() -> CohortClass {
    CohortClass::new("people", "hill")
}

/// A classification value with spaces and non-ASCII text: a class is the
/// authored text, not an identifier.
const RIVER_FOLK: &str = "River folk \u{6c5f}\u{5357}";

fn river() -> CohortClass {
    CohortClass::new("people", RIVER_FOLK)
}

fn authored() -> IntergroupRelations {
    let ids = Canwu::demo_ids();
    let mut relations = IntergroupRelations::empty();
    relations.kinds = BTreeSet::from(["communal".to_owned(), "commerce".to_owned()]);
    relations.edges.insert(
        "hill-river".to_owned(),
        IntergroupRelationEdge {
            id: "hill-river".to_owned(),
            kind: "communal".to_owned(),
            source_class: hill(),
            target_class: river(),
            region: Some(EntityRef::Territory(ids.western_territory)),
            acceptance_per_mille: 400,
            tension_per_mille: 300,
            trust_per_mille: 500,
            evidence: Vec::new(),
            revision: 1,
            revised_by: None,
            revised_at: None,
        },
    );
    relations
}

fn scenario(records: Vec<DomainRecord>) -> Scenario {
    let snapshot = Canwu::demo(42).expect("demo scenario").snapshot();
    Scenario {
        start_time: snapshot.initial_time,
        entities: snapshot.entities,
        world: snapshot.world,
        knowledge: snapshot.knowledge,
        domain_records: records,
    }
}

fn change(
    id: &str,
    expected_revision: u64,
    kind: &str,
    classes: (CohortClass, CohortClass),
    region: Option<EntityRef>,
    tension_per_mille: u16,
) -> IntergroupRelationChange {
    IntergroupRelationChange {
        id: id.to_owned(),
        expected_revision,
        kind: kind.to_owned(),
        source_class: classes.0,
        target_class: classes.1,
        region,
        acceptance_per_mille: 350,
        tension_per_mille,
        trust_per_mille: 450,
        evidence: Vec::new(),
    }
}

fn revise(canwu: &mut Canwu, changes: Vec<IntergroupRelationChange>) -> IngressId {
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            "canwu-society",
            INTERGROUP_RELATION_INGRESS,
            canwu.time(),
            serde_json::to_value(IntergroupRelationRevisionV1 { changes }).expect("revision"),
        ))
        .expect("an admissible relation revision")
        .ingress_id
}

/// Settles a boundary at the current time that carries no cadence, so only
/// the event-driven systems run, and returns the summaries of the relation
/// rejections it emitted.
fn settle(canwu: &mut Canwu) -> Vec<String> {
    let before = canwu.events().len();
    canwu
        .settle_boundary(BoundaryRequest::at(canwu.time()))
        .expect("a refused relation revision is recorded, not a failure");
    canwu.events()[before..]
        .iter()
        .filter(|event| {
            event.kind.plugin_identity()
                == Some(("canwu-society", INTERGROUP_RELATION_REJECTED_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect()
}

#[test]
fn gap_g33_relation_vocabulary_bounds_organization_relation_kinds() {
    // Without a vocabulary any canonical kind is admitted, and the root
    // encodes as it did before the field existed.
    let unrestricted = society().into_record().expect("an open vocabulary");
    assert!(unrestricted.payload.get("relation_vocabulary").is_none());

    let mut bounded = society();
    bounded.relation_vocabulary = BTreeSet::from(["alliance".to_owned(), "rivalry".to_owned()]);
    let error = bounded
        .clone()
        .into_record()
        .expect_err("a kind outside the vocabulary is refused");
    assert_eq!(error.code, ErrorCode::InvalidDomainRecord);
    assert!(error.message.contains("guild-temple"), "{}", error.message);

    bounded.relation_vocabulary.insert("patronage".to_owned());
    let record = bounded.into_record().expect("a kind inside the vocabulary");
    let plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin::DAILY];
    let mut canwu =
        Canwu::new_with_plugins(33, scenario(vec![record]), &plugins).expect("bounded run");
    canwu
        .settle_boundary(BoundaryRequest::at(day(1)).with_cadence(SystemCadence::Daily))
        .expect("the bounded root settles");
    let restored =
        canwu_society::from_society_snapshot_json(&canwu.snapshot_json().expect("snapshot"))
            .expect("the vocabulary survives a restore");
    assert_eq!(
        canwu_society::load_society_state(&restored)
            .expect("restored root")
            .relation_vocabulary
            .len(),
        3
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn gap_g34_intergroup_relations_are_authored_and_revised_through_ingress() {
    let ids = Canwu::demo_ids();
    let plugins: [&dyn SimulationPlugin; 1] = [&SocietyPlugin::DAILY];
    let root = society().into_record().expect("society root");

    // The run's creation checks that some cohort carries every class an
    // authored edge names.
    let mut unknown = authored();
    unknown
        .edges
        .get_mut("hill-river")
        .expect("edge")
        .target_class = CohortClass::new("people", "coast");
    assert_eq!(
        Canwu::new_with_plugins(
            34,
            scenario(vec![root.clone(), unknown.into_record().expect("shape")]),
            &plugins,
        )
        .err()
        .expect("an edge naming a class no cohort carries is refused")
        .code,
        ErrorCode::InvalidDomainRecord
    );

    let mut canwu = Canwu::new_with_plugins(
        34,
        scenario(vec![root, authored().into_record().expect("relations")]),
        &plugins,
    )
    .expect("a run with authored relations");
    assert_eq!(
        load_intergroup_relations(&canwu).expect("authored relations"),
        authored()
    );
    let root_version = |canwu: &Canwu| {
        canwu
            .typed_domain_record(&society_state_reference())
            .expect("society root")
            .version
    };
    let settled_root = root_version(&canwu);

    // One boundary admits three revisions, applied in admission order: the
    // first revises the authored edge and creates the opposite one, the
    // second still expects the authored revision and is stale, and the
    // third would create a second communal edge between the same classes in
    // the same region. Neither refusal fails the boundary.
    let evidence = vec![EvidenceRef::Ingress(IngressId::new(1))];
    let mut revised = change(
        "hill-river",
        1,
        "communal",
        (hill(), river()),
        Some(EntityRef::Territory(ids.western_territory)),
        600,
    );
    revised.evidence.clone_from(&evidence);
    let applied = revise(
        &mut canwu,
        vec![
            revised,
            change("river-hill", 0, "communal", (river(), hill()), None, 200),
        ],
    );
    revise(
        &mut canwu,
        vec![change(
            "hill-river",
            1,
            "communal",
            (hill(), river()),
            Some(EntityRef::Territory(ids.western_territory)),
            900,
        )],
    );
    revise(
        &mut canwu,
        vec![change(
            "hill-river-again",
            0,
            "communal",
            (hill(), river()),
            Some(EntityRef::Territory(ids.western_territory)),
            100,
        )],
    );
    // A boundary that carries no cadence: the society settlement does not
    // run, and the relations change anyway.
    let refused = settle(&mut canwu);
    let relations = load_intergroup_relations(&canwu).expect("revised relations");
    let edge = &relations.edges["hill-river"];
    assert_eq!(
        (edge.revision, edge.tension_per_mille, edge.revised_by),
        (2, 600, Some(applied))
    );
    assert_eq!(edge.revised_at, Some(canwu.time()));
    assert_eq!(edge.evidence, evidence);
    let opposite = &relations.edges["river-hill"];
    assert_eq!((opposite.revision, opposite.region.clone()), (1, None));
    assert!(!relations.edges.contains_key("hill-river-again"));
    assert_eq!(refused.len(), 2);
    assert!(
        refused[0].ends_with(INTERGROUP_RELATION_STALE_REJECTION),
        "{}",
        refused[0]
    );
    assert!(
        refused[1].ends_with(INTERGROUP_RELATION_IDENTITY_REJECTION),
        "{}",
        refused[1]
    );
    // A revision never writes the society root.
    assert_eq!(root_version(&canwu), settled_root);

    // Each refusal changes nothing, and a revision applies as a whole: the
    // valid first change of each refused packet does not land, though an
    // applied revision of another edge in the same boundary writes the
    // record.
    let mut relations = relations;
    let good = change(
        "hill-river",
        2,
        "communal",
        (hill(), river()),
        Some(EntityRef::Territory(ids.western_territory)),
        700,
    );
    for (bad, reason) in [
        (
            change(
                "hill-river",
                2,
                "communal",
                (hill(), river()),
                Some(EntityRef::Territory(ids.central_territory)),
                700,
            ),
            INTERGROUP_RELATION_IDENTITY_REJECTION,
        ),
        (
            change("river-trade", 0, "trade", (river(), hill()), None, 0),
            INTERGROUP_RELATION_KIND_REJECTION,
        ),
        (
            change(
                "river-coast",
                0,
                "commerce",
                (river(), CohortClass::new("people", "coast")),
                None,
                0,
            ),
            INTERGROUP_RELATION_CLASS_REJECTION,
        ),
        (
            change(
                "river-trade",
                0,
                "commerce",
                (river(), hill()),
                Some(EntityRef::Territory(TerritoryId::new(999))),
                0,
            ),
            INTERGROUP_RELATION_REGION_REJECTION,
        ),
    ] {
        let mut changes = vec![good.clone(), bad];
        changes.sort_by(|left, right| left.id.cmp(&right.id));
        if changes[0].id == changes[1].id {
            changes.remove(0);
        }
        revise(&mut canwu, changes);
        let opposite = relations.edges["river-hill"].revision;
        revise(
            &mut canwu,
            vec![change(
                "river-hill",
                opposite,
                "communal",
                (river(), hill()),
                None,
                200,
            )],
        );
        let summaries = settle(&mut canwu);
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].ends_with(reason), "{}", summaries[0]);
        let revised = load_intergroup_relations(&canwu).expect("revised relations");
        assert_eq!(revised.edges["hill-river"], relations.edges["hill-river"]);
        assert_eq!(revised.edges["river-hill"].revision, opposite + 1);
        assert_eq!(revised.edges.len(), relations.edges.len());
        relations = revised;
    }
    // A packet that is not a well-formed revision (here its changes are not
    // in ascending ID order) is refused as malformed.
    revise(
        &mut canwu,
        vec![
            good,
            change("a-first", 0, "commerce", (river(), hill()), None, 0),
        ],
    );
    assert!(settle(&mut canwu)[0].ends_with("malformed_payload"));
    assert_eq!(
        load_intergroup_relations(&canwu).expect("unchanged"),
        relations
    );

    // The relations restore and replay exactly.
    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    validate_society_runtime(&restored).expect("restored relations validate");
    assert_eq!(
        load_intergroup_relations(&restored).expect("restored relations"),
        relations
    );
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}

const REGION_PLUGIN: &str = "fixture-regions";
const REGION_LIFECYCLE: &str = "region_lifecycle";

/// An application-owned region entity.
#[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq, serde::Serialize)]
struct Area {
    name: String,
}

struct AreaRecord;

impl canwu_api::DomainRecordType for AreaRecord {
    type Payload = Area;
    type Class = canwu_api::DomainEntityKindClass;

    const NAMESPACE: &'static str = "fixture.region";
    const NAME: &'static str = "area";
}

fn area_key() -> canwu_api::StateKey {
    canwu_api::StateKey::new("fixture.region", "area")
}

fn area_reference() -> canwu_api::TypedDomainRecordRef<AreaRecord> {
    canwu_api::TypedDomainRecordRef::new("county")
}

/// Owns the area records and retires or deletes the area when an admitted
/// `region_lifecycle` packet says so.
struct RegionPlugin;

impl SimulationPlugin for RegionPlugin {
    fn name(&self) -> &'static str {
        REGION_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000e4"
    }

    fn register(
        &self,
        registrar: &mut canwu_api::PluginRegistrar<'_>,
    ) -> Result<(), canwu_api::CanwuError> {
        registrar
            .register_record_schema(canwu_api::DomainRecordSchema::for_entity::<AreaRecord>())?;
        registrar.register_ingress(canwu_api::PluginIngressDescriptor {
            name: REGION_LIFECYCLE.to_owned(),
            description: "Retire or delete the area".to_owned(),
            class: canwu_api::IngressClass::Information,
            payload_schema: canwu_api::PayloadSchema::Any,
        })?;
        let mut system = canwu_api::BoundarySystemContract::new(
            "settle-region-lifecycle",
            canwu_api::BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        system.reads = vec![area_key(), canwu_api::StateKey::core_ingress()];
        system.reads.sort();
        system.writes = vec![area_key()];
        system.visibility = canwu_api::StateVisibility::SameBoundary;
        registrar.register_boundary_system(system, settle_region_lifecycle)
    }
}

fn settle_region_lifecycle(
    view: &canwu_api::SimulationView<'_>,
    context: &canwu_api::BoundaryContext,
) -> Result<canwu_api::BoundaryProposal, canwu_api::CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        let Some(ingress) = view.ingress(*ingress_id)? else {
            continue;
        };
        let canwu_api::IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            continue;
        };
        if plugin != REGION_PLUGIN || packet_type != REGION_LIFECYCLE {
            continue;
        }
        let record = view
            .typed_domain_record(&area_reference())?
            .expect("the area record");
        let mutation = if payload["operation"] == "retire" {
            canwu_api::DomainRecordMutation::Retire {
                record: record.reference.clone(),
                expected_version: record.version,
                successor: None,
            }
        } else {
            canwu_api::DomainRecordMutation::Delete {
                record: record.reference.clone(),
                expected_version: record.version,
            }
        };
        directives.push(canwu_api::BoundaryDirective::MutateRecord {
            mutation,
            summary: "Changed the lifecycle of the area".to_owned(),
        });
    }
    Ok(canwu_api::BoundaryProposal {
        directives,
        ..canwu_api::BoundaryProposal::default()
    })
}

/// A region an edge names stays with its owner to retire and delete: the
/// relations record binds no reference to it, so the boundaries of the
/// owner commit. The edge keeps the region as history and can still be
/// revised, and a new edge cannot name the deleted region. An edge at the
/// last revision is refused as stale rather than overflowing.
#[test]
#[allow(clippy::too_many_lines)]
fn gap_g34_a_region_an_edge_names_stays_with_its_owner() {
    let regions = RegionPlugin;
    let plugins: [&dyn SimulationPlugin; 2] = [&SocietyPlugin::DAILY, &regions];
    let draft = canwu_api::DomainRecordDraft::from_typed(
        area_reference(),
        &Area {
            name: "county".to_owned(),
        },
    )
    .expect("area draft");
    let region = EntityRef::Domain(draft.reference.clone());
    let area = DomainRecord {
        reference: draft.reference,
        owner: REGION_PLUGIN.to_owned(),
        class: canwu_api::DomainRecordClass::Entity,
        version: 1,
        lifecycle: canwu_api::DomainRecordLifecycle::Active,
        payload: draft.payload,
        references: Vec::new(),
    };
    let mut last = authored();
    last.edges.get_mut("hill-river").expect("edge").revision = u64::MAX;
    let mut canwu = Canwu::new_with_plugins(
        34,
        scenario(vec![
            society().into_record().expect("society root"),
            last.into_record().expect("relations"),
            area,
        ]),
        &plugins,
    )
    .expect("a run with a region entity");

    // An edge at the last revision cannot be revised again; the boundary
    // commits.
    let ids = Canwu::demo_ids();
    revise(
        &mut canwu,
        vec![change(
            "hill-river",
            u64::MAX,
            "communal",
            (hill(), river()),
            Some(EntityRef::Territory(ids.western_territory)),
            100,
        )],
    );
    let refused = settle(&mut canwu);
    assert_eq!(refused.len(), 1);
    assert!(
        refused[0].ends_with(INTERGROUP_RELATION_STALE_REJECTION),
        "{}",
        refused[0]
    );

    revise(
        &mut canwu,
        vec![change(
            "river-hill",
            0,
            "communal",
            (river(), hill()),
            Some(region.clone()),
            200,
        )],
    );
    assert!(settle(&mut canwu).is_empty());
    assert!(
        canwu
            .typed_domain_record(&intergroup_relations_reference())
            .expect("relations record")
            .references
            .is_empty()
    );

    // The owner retires and then deletes the region; both boundaries commit.
    for operation in ["retire", "delete"] {
        canwu
            .enqueue_plugin_ingress(PluginIngressRequest::new(
                REGION_PLUGIN,
                REGION_LIFECYCLE,
                canwu.time(),
                serde_json::json!({ "operation": operation }),
            ))
            .expect("a region lifecycle packet");
        canwu
            .settle_boundary(BoundaryRequest::at(canwu.time()))
            .expect("the boundary of the owner commits");
    }
    assert!(
        canwu
            .typed_domain_record(&area_reference())
            .is_none_or(DomainRecord::is_deleted)
    );

    // The edge keeps the region as history and can still be revised; a new
    // edge cannot name it.
    let relations = load_intergroup_relations(&canwu).expect("relations");
    assert_eq!(relations.edges["river-hill"].region, Some(region.clone()));
    revise(
        &mut canwu,
        vec![change(
            "river-hill",
            1,
            "communal",
            (river(), hill()),
            Some(region.clone()),
            250,
        )],
    );
    revise(
        &mut canwu,
        vec![change(
            "river-hill-trade",
            0,
            "commerce",
            (river(), hill()),
            Some(region),
            0,
        )],
    );
    let refused = settle(&mut canwu);
    assert_eq!(refused.len(), 1);
    assert!(
        refused[0].ends_with(INTERGROUP_RELATION_REGION_REJECTION),
        "{}",
        refused[0]
    );
    let relations = load_intergroup_relations(&canwu).expect("revised relations");
    assert_eq!(
        (
            relations.edges["river-hill"].revision,
            relations.edges["river-hill"].tension_per_mille
        ),
        (2, 250)
    );
    assert!(!relations.edges.contains_key("river-hill-trade"));

    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    validate_society_runtime(&restored).expect("restored relations validate");
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}
