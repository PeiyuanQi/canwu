#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, DomainEntityKindClass, DomainRecordDraft,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, DomainValueKindClass, ErrorCode,
    IngressClass, IngressPayload, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginDescriptor, PluginIngressDescriptor, PluginRegistrar, Scenario, SimDuration, SimTime,
    SimulationPlugin, SimulationView, StateKey, StateVisibility, SystemCadence,
    TypedDomainRecordRef, canonical_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MANIFEST_PLUGIN: &str = "fixture-transition-manifest";
const OWNER_A_PLUGIN: &str = "fixture-transition-owner-a";
const OWNER_B_PLUGIN: &str = "fixture-transition-owner-b";
const OWNER_INGRESS: &str = "transition-ready";
const MANIFEST_KEY: &str = "grant-1";
const OWNER_A_KEY: &str = "grant-1-a";
const OWNER_B_KEY: &str = "grant-1-b";
const POST_STATE_HASH_DOMAIN: &str = "fixture.transition.post-state.v1";

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct TransitionManifestPayload {
    transition: String,
    owner_a_system: String,
    owner_a_version: u64,
    owner_a_disposition: String,
    owner_a_hash: String,
    owner_b_system: String,
    owner_b_version: u64,
    owner_b_disposition: String,
    owner_b_hash: String,
}

struct TransitionManifest;

impl DomainRecordType for TransitionManifest {
    type Payload = TransitionManifestPayload;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "fixture.transition";
    const NAME: &'static str = "manifest";
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct OwnerPayload {
    status: String,
}

struct OwnerARecord;

impl DomainRecordType for OwnerARecord {
    type Payload = OwnerPayload;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "fixture.transition.owner_a";
    const NAME: &'static str = "disposition";
}

struct OwnerBRecord;

impl DomainRecordType for OwnerBRecord {
    type Payload = OwnerPayload;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "fixture.transition.owner_b";
    const NAME: &'static str = "disposition";
}

fn manifest_reference() -> TypedDomainRecordRef<TransitionManifest> {
    TypedDomainRecordRef::new(MANIFEST_KEY)
}

fn owner_a_reference() -> TypedDomainRecordRef<OwnerARecord> {
    TypedDomainRecordRef::new(OWNER_A_KEY)
}

fn owner_b_reference() -> TypedDomainRecordRef<OwnerBRecord> {
    TypedDomainRecordRef::new(OWNER_B_KEY)
}

fn object_schema(fields: &[(&str, PayloadValueType)]) -> PayloadSchema {
    PayloadSchema::Object {
        properties: fields
            .iter()
            .map(|(name, value_type)| {
                (
                    (*name).to_owned(),
                    PayloadProperty {
                        value_type: value_type.clone(),
                        required: true,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        allow_additional: false,
    }
}

fn manifest_state() -> StateKey {
    DomainRecordSchema::for_record::<TransitionManifest>().state_key()
}

fn owner_a_state() -> StateKey {
    DomainRecordSchema::for_entity::<OwnerARecord>().state_key()
}

fn owner_b_state() -> StateKey {
    DomainRecordSchema::for_entity::<OwnerBRecord>().state_key()
}

fn owner_payload() -> OwnerPayload {
    OwnerPayload {
        status: "committed".to_owned(),
    }
}

fn owner_payload_value() -> Value {
    serde_json::to_value(owner_payload()).expect("the fixture payload should encode")
}

fn expected_owner_hash() -> Result<String, CanwuError> {
    canonical_hash(POST_STATE_HASH_DOMAIN, &owner_payload_value())
}

fn transition_ingress_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: OWNER_INGRESS.to_owned(),
        description: "Admit a prepared transition owner request".to_owned(),
        class: IngressClass::ScheduledSystem,
        payload_schema: object_schema(&[("transition", PayloadValueType::String)]),
    }
}

fn start_ingress_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: "start".to_owned(),
        description: "Start a prepared transition".to_owned(),
        class: IngressClass::ScheduledSystem,
        payload_schema: object_schema(&[("transition", PayloadValueType::String)]),
    }
}

fn owned_transition_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    owner: &str,
    packet_type: &str,
) -> Result<Option<String>, CanwuError> {
    let mut transition = None;
    for ingress_id in &context.admitted_ingress {
        let Some(record) = view.ingress(*ingress_id)? else {
            continue;
        };
        let IngressPayload::Plugin {
            plugin,
            packet_type: admitted_packet_type,
            payload,
            ..
        } = &record.payload
        else {
            continue;
        };
        if plugin != owner || admitted_packet_type != packet_type {
            continue;
        }
        if transition.is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("{owner} received duplicate {packet_type} ingress"),
            ));
        }
        let value = payload
            .get("transition")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("{owner}.{packet_type} is missing its transition identity"),
                )
            })?;
        transition = Some(value.to_owned());
    }
    Ok(transition)
}

fn manifest_payload(view: &SimulationView<'_>) -> Result<TransitionManifestPayload, CanwuError> {
    view.typed_domain_record(&manifest_reference())?
        .ok_or_else(|| {
            CanwuError::new(ErrorCode::InvalidBoundary, "transition manifest is missing")
        })?
        .decode_payload::<TransitionManifest>()
}

fn validate_manifest_entry(
    manifest: &TransitionManifestPayload,
    owner: &str,
    system: &str,
    hash: &str,
    expected_plugin: &str,
) -> Result<(), CanwuError> {
    let (entry_plugin, entry_system, entry_version, entry_disposition, entry_hash) =
        if owner == "owner A" {
            (
                OWNER_A_PLUGIN,
                manifest.owner_a_system.as_str(),
                manifest.owner_a_version,
                manifest.owner_a_disposition.as_str(),
                manifest.owner_a_hash.as_str(),
            )
        } else {
            (
                OWNER_B_PLUGIN,
                manifest.owner_b_system.as_str(),
                manifest.owner_b_version,
                manifest.owner_b_disposition.as_str(),
                manifest.owner_b_hash.as_str(),
            )
        };
    if manifest.transition != MANIFEST_KEY
        || entry_plugin != expected_plugin
        || entry_system != system
        || entry_version != 1
        || entry_disposition != "committed"
        || entry_hash != hash
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!("{owner} manifest entry does not match the prepared transition"),
        ));
    }
    Ok(())
}

fn ready_manifest(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(transition) = owned_transition_ingress(view, context, MANIFEST_PLUGIN, "start")?
    else {
        return Ok(BoundaryProposal::default());
    };
    if transition != MANIFEST_KEY {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the start ingress names an unexpected transition",
        ));
    }
    if view.typed_domain_record(&manifest_reference())?.is_some() {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the transition manifest already exists; duplicate start ingress is invalid",
        ));
    }
    let hash = expected_owner_hash()?;
    let manifest = DomainRecordDraft::from_typed(
        manifest_reference(),
        &TransitionManifestPayload {
            transition: MANIFEST_KEY.to_owned(),
            owner_a_system: "prepare-owner-a".to_owned(),
            owner_a_version: 1,
            owner_a_disposition: "committed".to_owned(),
            owner_a_hash: hash.clone(),
            owner_b_system: "prepare-owner-b".to_owned(),
            owner_b_version: 1,
            owner_b_disposition: "committed".to_owned(),
            owner_b_hash: hash,
        },
    )?;
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create { record: manifest },
                summary: "Publish a complete owner transition manifest".to_owned(),
            },
            BoundaryDirective::SchedulePluginIngress {
                target_plugin: OWNER_A_PLUGIN.to_owned(),
                after: SimDuration::ZERO,
                packet_type: OWNER_INGRESS.to_owned(),
                priority: 0,
                payload: json!({"transition": MANIFEST_KEY}),
                affected: Vec::new(),
            },
            BoundaryDirective::SchedulePluginIngress {
                target_plugin: OWNER_B_PLUGIN.to_owned(),
                after: SimDuration::ZERO,
                packet_type: OWNER_INGRESS.to_owned(),
                priority: 0,
                payload: json!({"transition": MANIFEST_KEY}),
                affected: Vec::new(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

fn owner_a_proposal(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(transition) = owned_transition_ingress(view, context, OWNER_A_PLUGIN, OWNER_INGRESS)?
    else {
        return Ok(BoundaryProposal::default());
    };
    if transition != MANIFEST_KEY {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "owner A received an ingress for the wrong transition",
        ));
    }
    let manifest = manifest_payload(view)?;
    let hash = expected_owner_hash()?;
    validate_manifest_entry(
        &manifest,
        "owner A",
        "prepare-owner-a",
        &hash,
        OWNER_A_PLUGIN,
    )?;
    let record = DomainRecordDraft::from_typed(owner_a_reference(), &owner_payload())?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record },
            summary: "Stage owner A's guarded transition disposition".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn owner_b_proposal(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(transition) = owned_transition_ingress(view, context, OWNER_B_PLUGIN, OWNER_INGRESS)?
    else {
        return Ok(BoundaryProposal::default());
    };
    if transition != MANIFEST_KEY {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "owner B received an ingress for the wrong transition",
        ));
    }
    let manifest = manifest_payload(view)?;
    let hash = expected_owner_hash()?;
    validate_manifest_entry(
        &manifest,
        "owner B",
        "prepare-owner-b",
        &hash,
        OWNER_B_PLUGIN,
    )?;
    let record = DomainRecordDraft::from_typed(owner_b_reference(), &owner_payload())?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record },
            summary: "Stage owner B's guarded transition disposition".to_owned(),
        }],
        ..BoundaryProposal::default()
    })
}

fn owner_b_omits_proposal(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(transition) = owned_transition_ingress(view, context, OWNER_B_PLUGIN, OWNER_INGRESS)?
    else {
        return Ok(BoundaryProposal::default());
    };
    if transition != MANIFEST_KEY {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "owner B received an ingress for the wrong transition",
        ));
    }
    let manifest = manifest_payload(view)?;
    let hash = expected_owner_hash()?;
    validate_manifest_entry(
        &manifest,
        "owner B",
        "prepare-owner-b",
        &hash,
        OWNER_B_PLUGIN,
    )?;
    Ok(BoundaryProposal::default())
}

fn validate_before_transition(
    view: &SimulationView<'_>,
    _context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if view.typed_domain_record(&manifest_reference())?.is_none() {
        return Ok(BoundaryProposal::default());
    }
    if view
        .domain_record(owner_a_reference().as_untyped())?
        .is_some()
        || view
            .domain_record(owner_b_reference().as_untyped())?
            .is_some()
    {
        return Ok(BoundaryProposal::default());
    }
    for reference in [
        owner_a_reference().as_untyped(),
        owner_b_reference().as_untyped(),
    ] {
        if view.domain_record(reference)?.is_some()
            || view.proposed_domain_record(reference)?.is_some()
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                "transition dispositions became visible before the transition stage",
            ));
        }
    }
    Ok(BoundaryProposal::default())
}

fn audit_transition(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if owned_transition_ingress(view, context, MANIFEST_PLUGIN, "start")?
        .is_some_and(|transition| transition == MANIFEST_KEY)
    {
        return Ok(BoundaryProposal::default());
    }
    let manifest = manifest_payload(view)?;
    let hash = expected_owner_hash()?;
    validate_manifest_entry(
        &manifest,
        "owner A",
        "prepare-owner-a",
        &hash,
        OWNER_A_PLUGIN,
    )?;
    validate_manifest_entry(
        &manifest,
        "owner B",
        "prepare-owner-b",
        &hash,
        OWNER_B_PLUGIN,
    )?;
    let expected_a = (
        manifest.owner_a_hash,
        manifest.owner_a_system,
        manifest.owner_a_version,
        manifest.owner_a_disposition,
    );
    let expected_b = (
        manifest.owner_b_hash,
        manifest.owner_b_system,
        manifest.owner_b_version,
        manifest.owner_b_disposition,
    );
    for (label, reference, expected, expected_owner) in [
        (
            "owner A",
            owner_a_reference().as_untyped(),
            expected_a,
            OWNER_A_PLUGIN,
        ),
        (
            "owner B",
            owner_b_reference().as_untyped(),
            expected_b,
            OWNER_B_PLUGIN,
        ),
    ] {
        let record = view.domain_record(reference)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("transition audit missing {label} disposition"),
            )
        })?;
        let (expected_hash, expected_system, expected_version, expected_disposition) = expected;
        let actual_hash = canonical_hash(POST_STATE_HASH_DOMAIN, &record.payload)?;
        if record.version != 1
            || !record.is_active()
            || record.owner != expected_owner
            || actual_hash != expected_hash
            || expected_version != record.version
            || expected_disposition != "committed"
            || record.payload != json!({"status": expected_disposition})
            || expected_system
                != if label == "owner A" {
                    "prepare-owner-a"
                } else {
                    "prepare-owner-b"
                }
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("transition audit found an invalid {label} post-state"),
            ));
        }
    }
    Ok(BoundaryProposal::default())
}

struct ManifestPlugin;

impl SimulationPlugin for ManifestPlugin {
    fn name(&self) -> &'static str {
        MANIFEST_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000201"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<TransitionManifest>();
        schema.payload_schema = object_schema(&[
            ("transition", PayloadValueType::String),
            ("owner_a_system", PayloadValueType::String),
            ("owner_a_version", PayloadValueType::Integer),
            ("owner_a_disposition", PayloadValueType::String),
            ("owner_a_hash", PayloadValueType::String),
            ("owner_b_system", PayloadValueType::String),
            ("owner_b_version", PayloadValueType::Integer),
            ("owner_b_disposition", PayloadValueType::String),
            ("owner_b_hash", PayloadValueType::String),
        ]);
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(start_ingress_descriptor())?;

        let mut ready = BoundarySystemContract::new(
            "publish-ready",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        ready.reads = vec![manifest_state(), StateKey::core_ingress()];
        ready.writes = vec![manifest_state()];
        ready.plugin_ingress_targets = vec![
            canwu_api::PluginIngressTarget {
                target_plugin: OWNER_A_PLUGIN.to_owned(),
                packet_type: OWNER_INGRESS.to_owned(),
            },
            canwu_api::PluginIngressTarget {
                target_plugin: OWNER_B_PLUGIN.to_owned(),
                packet_type: OWNER_INGRESS.to_owned(),
            },
        ];
        ready.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(ready, ready_manifest)?;

        let mut pre_transition = BoundarySystemContract::new(
            "validate-before-transition",
            BoundaryPhase::InvariantValidation,
            SystemCadence::EventDriven,
        );
        pre_transition.reads = vec![manifest_state(), owner_a_state(), owner_b_state()];
        registrar.register_boundary_system(pre_transition, validate_before_transition)?;

        let mut audit = BoundarySystemContract::new(
            "audit-commit",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        audit.reads = vec![
            manifest_state(),
            owner_a_state(),
            owner_b_state(),
            StateKey::core_ingress(),
        ];
        registrar.register_boundary_system(audit, audit_transition)
    }
}

struct OwnerAPlugin;

impl SimulationPlugin for OwnerAPlugin {
    fn name(&self) -> &'static str {
        OWNER_A_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000202"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_entity::<OwnerARecord>();
        schema.payload_schema = object_schema(&[("status", PayloadValueType::String)]);
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(transition_ingress_descriptor())?;
        let mut participant = BoundarySystemContract::new(
            "prepare-owner-a",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        participant.reads = vec![manifest_state(), StateKey::core_ingress()];
        participant.writes = vec![owner_a_state()];
        participant.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(participant, owner_a_proposal)
    }
}

struct OwnerBPlugin {
    omit: bool,
}

impl SimulationPlugin for OwnerBPlugin {
    fn name(&self) -> &'static str {
        OWNER_B_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        if self.omit {
            "0000000000000000000000000000000000000000000000000000000000000204"
        } else {
            "0000000000000000000000000000000000000000000000000000000000000203"
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_entity::<OwnerBRecord>();
        schema.payload_schema = object_schema(&[("status", PayloadValueType::String)]);
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(transition_ingress_descriptor())?;
        let mut participant = BoundarySystemContract::new(
            "prepare-owner-b",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        participant.reads = vec![manifest_state(), StateKey::core_ingress()];
        participant.writes = vec![owner_b_state()];
        participant.visibility = StateVisibility::SameBoundary;
        if self.omit {
            registrar.register_boundary_system(participant, owner_b_omits_proposal)
        } else {
            registrar.register_boundary_system(participant, owner_b_proposal)
        }
    }
}

fn assert_event_driven_transition_descriptors(descriptors: &[PluginDescriptor]) {
    let required = [
        (
            MANIFEST_PLUGIN,
            "publish-ready",
            BoundaryPhase::DomainDeltaProposal,
        ),
        (
            OWNER_A_PLUGIN,
            "prepare-owner-a",
            BoundaryPhase::HistoricalCandidateEvaluation,
        ),
        (
            OWNER_B_PLUGIN,
            "prepare-owner-b",
            BoundaryPhase::HistoricalCandidateEvaluation,
        ),
        (
            MANIFEST_PLUGIN,
            "validate-before-transition",
            BoundaryPhase::InvariantValidation,
        ),
        (
            MANIFEST_PLUGIN,
            "audit-commit",
            BoundaryPhase::StrategicAggregation,
        ),
    ];
    for (plugin, system, phase) in required {
        let descriptor = descriptors
            .iter()
            .find(|candidate| candidate.name == plugin)
            .unwrap_or_else(|| panic!("missing descriptor for {plugin}"));
        let boundary = descriptor
            .boundary_systems
            .iter()
            .find(|candidate| candidate.name == system)
            .unwrap_or_else(|| panic!("missing system descriptor for {plugin}.{system}"));
        assert_eq!(boundary.phase, phase);
        assert_eq!(boundary.cadence, SystemCadence::EventDriven);
    }
    let manifest = descriptors
        .iter()
        .find(|candidate| candidate.name == MANIFEST_PLUGIN)
        .expect("the manifest descriptor is present");
    let ready = manifest
        .boundary_systems
        .iter()
        .find(|candidate| candidate.name == "publish-ready")
        .expect("the ready writer descriptor is present");
    assert_eq!(ready.visibility, StateVisibility::SameBoundary);
    assert_eq!(ready.plugin_ingress_targets.len(), 2);
    assert_eq!(
        ready.plugin_ingress_targets[0].target_plugin,
        OWNER_A_PLUGIN
    );
    assert_eq!(
        ready.plugin_ingress_targets[1].target_plugin,
        OWNER_B_PLUGIN
    );
    let audit = manifest
        .boundary_systems
        .iter()
        .find(|candidate| candidate.name == "audit-commit")
        .expect("the audit descriptor is present");
    let mut audit_reads = vec![
        manifest_state(),
        owner_a_state(),
        owner_b_state(),
        StateKey::core_ingress(),
    ];
    audit_reads.sort();
    assert_eq!(audit.reads, audit_reads);
    assert!(audit.writes.is_empty());
    assert!(audit.emits.is_empty());
    assert!(audit.plugin_ingress_targets.is_empty());
    assert!(audit.reservation_offers.is_empty());
    assert!(audit.reservation_requests.is_empty());
    assert!(audit.reservation_reads.is_empty());
    assert!(audit.random_streams.is_empty());
    assert!(audit.knowledge_writes.is_empty());
    let mut participant_reads = vec![manifest_state(), StateKey::core_ingress()];
    participant_reads.sort();
    for (plugin, system, state) in [
        (OWNER_A_PLUGIN, "prepare-owner-a", owner_a_state()),
        (OWNER_B_PLUGIN, "prepare-owner-b", owner_b_state()),
    ] {
        let descriptor = descriptors
            .iter()
            .find(|candidate| candidate.name == plugin)
            .expect("the participant descriptor is present");
        let participant = descriptor
            .boundary_systems
            .iter()
            .find(|candidate| candidate.name == system)
            .expect("the participant system descriptor is present");
        assert_eq!(participant.visibility, StateVisibility::SameBoundary);
        assert_eq!(participant.reads, participant_reads);
        assert_eq!(participant.writes, vec![state]);
    }
}

#[test]
fn prepared_multi_owner_transition_is_atomic_audited_and_replayable() {
    let manifest = ManifestPlugin;
    let owner_a = OwnerAPlugin;
    let owner_b = OwnerBPlugin { omit: false };
    let plugins: [&dyn SimulationPlugin; 3] = [&manifest, &owner_a, &owner_b];
    let mut canwu =
        Canwu::new_with_plugins(11, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)
            .expect("the prepared-transition package set should register");
    let descriptors: Vec<_> = canwu.plugin_descriptors().cloned().collect();
    assert_event_driven_transition_descriptors(&descriptors);

    canwu
        .enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
            MANIFEST_PLUGIN,
            "start",
            SimTime::EPOCH,
            json!({"transition": MANIFEST_KEY}),
        ))
        .expect("the prepared transition should start through canonical ingress");

    let first = canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the initial event-driven boundary should settle");
    assert_eq!(first.generated_ingress.len(), 2);
    let journal_after_first = canwu.replay_journal();
    let first_record = journal_after_first
        .boundaries
        .last()
        .expect("the initial boundary should be recorded");
    assert_eq!(first_record.record_changes.len(), 1);
    assert_eq!(
        first_record.record_changes[0].current.reference,
        manifest_reference().as_untyped().clone()
    );
    let mut generated_targets = Vec::new();
    for ingress_id in &first.generated_ingress {
        let ingress = journal_after_first
            .ingress
            .iter()
            .find(|record| record.id == *ingress_id)
            .expect("every generated ingress should be retained in evidence");
        let IngressPayload::Plugin {
            plugin,
            packet_type,
            payload,
            ..
        } = &ingress.payload
        else {
            panic!("generated owner requests must be plugin ingress");
        };
        assert_eq!(packet_type, OWNER_INGRESS);
        assert_eq!(payload, &json!({"transition": MANIFEST_KEY}));
        assert_eq!(
            ingress.cause,
            Some(canwu_api::CauseRef::Boundary(first.boundary_id))
        );
        generated_targets.push(plugin.as_str());
    }
    generated_targets.sort_unstable();
    assert_eq!(generated_targets, vec![OWNER_A_PLUGIN, OWNER_B_PLUGIN]);
    assert!(canwu.typed_domain_record(&manifest_reference()).is_some());
    assert!(canwu.typed_domain_record(&owner_a_reference()).is_none());
    assert!(canwu.typed_domain_record(&owner_b_reference()).is_none());

    let pending_snapshot_json = canwu
        .snapshot_json()
        .expect("pending owner ingress should be serializable");
    let pending_journal = canwu.replay_journal();
    let mut restored_pending =
        Canwu::from_snapshot_json_with_plugins(&pending_snapshot_json, &plugins)
            .expect("pending owner ingress should survive snapshot restore");
    let mut replayed_pending = Canwu::replay_from_journal(&plugins, &pending_journal)
        .expect("pending owner ingress should survive journal replay");

    let second = canwu
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the owner requests should settle at the next boundary cut");
    let journal_after_second = canwu.replay_journal();
    let second_record = journal_after_second
        .boundaries
        .last()
        .expect("the second boundary should be recorded");
    assert_eq!(second_record.admitted_ingress.len(), 2);
    assert_eq!(second.record_change_count, 2);
    assert_eq!(
        second_record
            .record_changes
            .iter()
            .map(|change| (change.plugin.as_str(), change.system.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (OWNER_A_PLUGIN, "prepare-owner-a"),
            (OWNER_B_PLUGIN, "prepare-owner-b"),
        ]
    );
    assert_eq!(second.change_count, 0);
    assert!(second.generated_ingress.is_empty());
    assert!(second.random_draws.is_empty());
    assert_eq!(second.knowledge_batch_count, 0);
    assert_eq!(second.knowledge_record_count, 0);
    assert!(second_record
        .emissions
        .iter()
        .all(|emission| emission.plugin != MANIFEST_PLUGIN || emission.system != "audit-commit"));
    assert_eq!(
        canwu
            .typed_domain_record(&owner_a_reference())
            .expect("owner A should commit its disposition")
            .decode_payload::<OwnerARecord>()
            .expect("owner A payload should decode"),
        owner_payload()
    );
    assert_eq!(
        canwu
            .typed_domain_record(&owner_b_reference())
            .expect("owner B should commit its disposition")
            .decode_payload::<OwnerBRecord>()
            .expect("owner B payload should decode"),
        owner_payload()
    );

    restored_pending
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("restored pending owner ingress should continue");
    replayed_pending
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("replayed pending owner ingress should continue");
    assert_eq!(canwu.snapshot(), restored_pending.snapshot());
    assert_eq!(canwu.snapshot(), replayed_pending.snapshot());

    let snapshot_json = canwu
        .snapshot_json()
        .expect("the audited transition should serialize");
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot_json, &plugins)
        .expect("the audited transition should restore with its exact package set");
    assert_eq!(canwu.snapshot(), restored.snapshot());
    let journal = canwu.replay_journal();
    let replayed = Canwu::replay_from_journal(&plugins, &journal)
        .expect("the audited transition should replay exactly");
    assert_eq!(canwu.snapshot(), replayed.snapshot());

    let manifest = ManifestPlugin;
    let owner_a = OwnerAPlugin;
    let owner_b = OwnerBPlugin { omit: true };
    let failing_plugins: [&dyn SimulationPlugin; 3] = [&manifest, &owner_a, &owner_b];
    let mut failing = Canwu::new_with_plugins(
        11,
        Scenario::new(SimTime::EPOCH, Vec::new()),
        &failing_plugins,
    )
    .expect("the omission package set should register");
    failing
        .enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
            MANIFEST_PLUGIN,
            "start",
            SimTime::EPOCH,
            json!({"transition": MANIFEST_KEY}),
        ))
        .expect("the omission fixture should start through canonical ingress");
    failing
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect("the failing fixture should publish its ready manifest");
    let before_failure = failing.snapshot();
    let error = failing
        .settle_boundary(BoundaryRequest::at(SimTime::EPOCH))
        .expect_err("phase-12 audit must reject an omitted ready participant");
    assert_eq!(error.code, ErrorCode::InvalidBoundary);
    assert_eq!(failing.snapshot(), before_failure);
    assert!(failing.typed_domain_record(&owner_a_reference()).is_none());
    assert!(failing.typed_domain_record(&owner_b_reference()).is_none());
}
