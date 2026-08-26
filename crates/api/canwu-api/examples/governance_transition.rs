#![allow(clippy::too_many_lines, clippy::unnecessary_wraps)]

//! A public-API-only governance case.
//!
//! The case models a Southern Ming relief order without putting court,
//! treasury, county, or granary vocabulary into Canwu itself:
//!
//! 1. The central relief office publishes a manifest.
//! 2. The manifest schedules one request for the treasury and one for a county.
//! 3. Both owners prepare their own records in the same boundary.
//! 4. A read-only central audit commits the order only when both records match
//!    the manifest.
//!
//! A zero-delay plugin ingress is intentionally deferred to the next boundary.
//! The final snapshot is then restored and replayed to demonstrate that the
//! governance transition is deterministic and persistence-safe.

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundaryRequest,
    BoundarySystemContract, Canwu, CanwuError, DomainEntityKindClass, DomainRecordDraft,
    DomainRecordMutation, DomainRecordSchema, DomainRecordType, DomainValueKindClass, ErrorCode,
    IngressClass, IngressPayload, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginIngressDescriptor, PluginRegistrar, Scenario, SimDuration, SimTime, SimulationPlugin,
    SimulationView, StateKey, StateVisibility, SystemCadence, TypedDomainRecordRef, canonical_hash,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const CENTRAL_PLUGIN: &str = "case-relief-central";
const TREASURY_PLUGIN: &str = "case-relief-treasury";
const COUNTY_PLUGIN: &str = "case-relief-county";
const ISSUE_INGRESS: &str = "issue-relief-order";
const EXECUTE_INGRESS: &str = "execute-relief-order";
const ORDER_ID: &str = "relief-order-1646";
const TREASURY_RECORD_ID: &str = "relief-order-1646-treasury";
const COUNTY_RECORD_ID: &str = "relief-order-1646-county";
const ACTION_HASH_DOMAIN: &str = "case.relief-action.v1";

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct ReliefOrder {
    order_id: String,
    issued_by: String,
    treasury_system: String,
    treasury_version: u64,
    treasury_disposition: String,
    treasury_hash: String,
    county_system: String,
    county_version: u64,
    county_disposition: String,
    county_hash: String,
}

struct ReliefOrderRecord;

impl DomainRecordType for ReliefOrderRecord {
    type Payload = ReliefOrder;
    type Class = DomainValueKindClass;

    const NAMESPACE: &'static str = "case.relief";
    const NAME: &'static str = "order";
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct ReliefAction {
    status: String,
    grain_units: u64,
}

struct TreasuryActionRecord;

impl DomainRecordType for TreasuryActionRecord {
    type Payload = ReliefAction;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "case.relief.treasury";
    const NAME: &'static str = "action";
}

struct CountyActionRecord;

impl DomainRecordType for CountyActionRecord {
    type Payload = ReliefAction;
    type Class = DomainEntityKindClass;

    const NAMESPACE: &'static str = "case.relief.county";
    const NAME: &'static str = "action";
}

fn order_reference() -> TypedDomainRecordRef<ReliefOrderRecord> {
    TypedDomainRecordRef::new(ORDER_ID)
}

fn treasury_reference() -> TypedDomainRecordRef<TreasuryActionRecord> {
    TypedDomainRecordRef::new(TREASURY_RECORD_ID)
}

fn county_reference() -> TypedDomainRecordRef<CountyActionRecord> {
    TypedDomainRecordRef::new(COUNTY_RECORD_ID)
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

fn order_state() -> StateKey {
    DomainRecordSchema::for_record::<ReliefOrderRecord>().state_key()
}

fn treasury_state() -> StateKey {
    DomainRecordSchema::for_entity::<TreasuryActionRecord>().state_key()
}

fn county_state() -> StateKey {
    DomainRecordSchema::for_entity::<CountyActionRecord>().state_key()
}

fn action_payload(grain_units: u64) -> ReliefAction {
    ReliefAction {
        status: "committed".to_owned(),
        grain_units,
    }
}

fn action_hash(grain_units: u64) -> Result<String, CanwuError> {
    let payload = serde_json::to_value(action_payload(grain_units)).map_err(|error| {
        CanwuError::new(
            ErrorCode::InvalidPayload,
            format!("relief action payload cannot be encoded: {error}"),
        )
    })?;
    canonical_hash(ACTION_HASH_DOMAIN, &payload)
}

fn issue_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: ISSUE_INGRESS.to_owned(),
        description: "Issue a central relief order".to_owned(),
        class: IngressClass::ScheduledSystem,
        payload_schema: object_schema(&[("order_id", PayloadValueType::String)]),
    }
}

fn execute_descriptor() -> PluginIngressDescriptor {
    PluginIngressDescriptor {
        name: EXECUTE_INGRESS.to_owned(),
        description: "Admit an owner execution of a relief order".to_owned(),
        class: IngressClass::ScheduledSystem,
        payload_schema: object_schema(&[("order_id", PayloadValueType::String)]),
    }
}

fn owned_order_ingress(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    owner: &str,
    packet_type: &str,
) -> Result<Option<String>, CanwuError> {
    let mut order_id = None;
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
        if order_id.is_some() {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("{owner} received duplicate {packet_type} ingress"),
            ));
        }
        let value = payload
            .get("order_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CanwuError::new(
                    ErrorCode::InvalidPayload,
                    format!("{owner}.{packet_type} is missing order_id"),
                )
            })?;
        order_id = Some(value.to_owned());
    }
    Ok(order_id)
}

fn order_payload(view: &SimulationView<'_>) -> Result<ReliefOrder, CanwuError> {
    view.typed_domain_record(&order_reference())?
        .ok_or_else(|| CanwuError::new(ErrorCode::InvalidBoundary, "relief order is missing"))?
        .decode_payload::<ReliefOrderRecord>()
}

fn validate_order(
    order: &ReliefOrder,
    expected_system: &str,
    expected_plugin: &str,
    expected_hash: &str,
) -> Result<(), CanwuError> {
    let valid = order.order_id == ORDER_ID
        && order.issued_by == CENTRAL_PLUGIN
        && order.treasury_system == "prepare-treasury"
        && order.treasury_version == 1
        && order.treasury_disposition == "committed"
        && order.county_system == "prepare-county"
        && order.county_version == 1
        && order.county_disposition == "committed"
        && ((expected_plugin == TREASURY_PLUGIN
            && expected_system == "prepare-treasury"
            && order.treasury_hash == expected_hash)
            || (expected_plugin == COUNTY_PLUGIN
                && expected_system == "prepare-county"
                && order.county_hash == expected_hash));
    if !valid {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!("{expected_plugin} received an invalid relief manifest"),
        ));
    }
    Ok(())
}

fn publish_order(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(order_id) = owned_order_ingress(view, context, CENTRAL_PLUGIN, ISSUE_INGRESS)? else {
        return Ok(BoundaryProposal::default());
    };
    if order_id != ORDER_ID {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the issue ingress names an unexpected relief order",
        ));
    }
    if view.typed_domain_record(&order_reference())?.is_some() {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            "the relief order already exists; duplicate issue ingress is invalid",
        ));
    }
    let treasury_hash = action_hash(600)?;
    let county_hash = action_hash(600)?;
    let manifest = DomainRecordDraft::from_typed(
        order_reference(),
        &ReliefOrder {
            order_id: ORDER_ID.to_owned(),
            issued_by: CENTRAL_PLUGIN.to_owned(),
            treasury_system: "prepare-treasury".to_owned(),
            treasury_version: 1,
            treasury_disposition: "committed".to_owned(),
            treasury_hash,
            county_system: "prepare-county".to_owned(),
            county_version: 1,
            county_disposition: "committed".to_owned(),
            county_hash,
        },
    )?;
    Ok(BoundaryProposal {
        directives: vec![
            BoundaryDirective::MutateRecord {
                mutation: DomainRecordMutation::Create { record: manifest },
                summary: "Publish the central relief order manifest".to_owned(),
            },
            BoundaryDirective::SchedulePluginIngress {
                target_plugin: TREASURY_PLUGIN.to_owned(),
                after: SimDuration::ZERO,
                packet_type: EXECUTE_INGRESS.to_owned(),
                priority: 0,
                payload: json!({"order_id": ORDER_ID}),
                affected: Vec::new(),
            },
            BoundaryDirective::SchedulePluginIngress {
                target_plugin: COUNTY_PLUGIN.to_owned(),
                after: SimDuration::ZERO,
                packet_type: EXECUTE_INGRESS.to_owned(),
                priority: 0,
                payload: json!({"order_id": ORDER_ID}),
                affected: Vec::new(),
            },
        ],
        ..BoundaryProposal::default()
    })
}

fn prepare_treasury(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    prepare_owner(
        view,
        context,
        TREASURY_PLUGIN,
        "prepare-treasury",
        treasury_reference(),
        &action_payload(600),
    )
}

fn prepare_county(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    prepare_owner(
        view,
        context,
        COUNTY_PLUGIN,
        "prepare-county",
        county_reference(),
        &action_payload(600),
    )
}

fn prepare_owner<T: DomainRecordType<Payload = ReliefAction>>(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
    plugin: &str,
    system: &str,
    reference: TypedDomainRecordRef<T>,
    payload: &ReliefAction,
) -> Result<BoundaryProposal, CanwuError> {
    let Some(order_id) = owned_order_ingress(view, context, plugin, EXECUTE_INGRESS)? else {
        return Ok(BoundaryProposal::default());
    };
    if order_id != ORDER_ID {
        return Err(CanwuError::new(
            ErrorCode::InvalidBoundary,
            format!("{plugin} received an ingress for the wrong order"),
        ));
    }
    let order = order_payload(view)?;
    let hash = action_hash(payload.grain_units)?;
    validate_order(&order, system, plugin, &hash)?;
    let record = DomainRecordDraft::from_typed(reference, payload)?;
    Ok(BoundaryProposal {
        directives: vec![BoundaryDirective::MutateRecord {
            mutation: DomainRecordMutation::Create { record },
            summary: format!("{plugin} prepares its guarded relief disposition"),
        }],
        ..BoundaryProposal::default()
    })
}

fn audit_order(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    if owned_order_ingress(view, context, CENTRAL_PLUGIN, ISSUE_INGRESS)?.is_some() {
        return Ok(BoundaryProposal::default());
    }
    let order = order_payload(view)?;
    for (label, reference, expected_plugin, expected_system) in [
        (
            "treasury",
            treasury_reference().as_untyped(),
            TREASURY_PLUGIN,
            "prepare-treasury",
        ),
        (
            "county",
            county_reference().as_untyped(),
            COUNTY_PLUGIN,
            "prepare-county",
        ),
    ] {
        let record = view.domain_record(reference)?.ok_or_else(|| {
            CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("central audit is missing the {label} relief disposition"),
            )
        })?;
        let expected_hash = action_hash(600)?;
        validate_order(&order, expected_system, expected_plugin, &expected_hash)?;
        let actual_hash = canonical_hash(ACTION_HASH_DOMAIN, &record.payload)?;
        if record.version != 1
            || !record.is_active()
            || record.owner != expected_plugin
            || actual_hash != expected_hash
            || record.payload != json!({"status": "committed", "grain_units": 600})
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidBoundary,
                format!("central audit found an invalid {label} disposition"),
            ));
        }
    }
    Ok(BoundaryProposal::default())
}

struct CentralPlugin;

impl SimulationPlugin for CentralPlugin {
    fn name(&self) -> &'static str {
        CENTRAL_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000301"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<ReliefOrderRecord>();
        schema.payload_schema = object_schema(&[
            ("order_id", PayloadValueType::String),
            ("issued_by", PayloadValueType::String),
            ("treasury_system", PayloadValueType::String),
            ("treasury_version", PayloadValueType::Integer),
            ("treasury_disposition", PayloadValueType::String),
            ("treasury_hash", PayloadValueType::String),
            ("county_system", PayloadValueType::String),
            ("county_version", PayloadValueType::Integer),
            ("county_disposition", PayloadValueType::String),
            ("county_hash", PayloadValueType::String),
        ]);
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(issue_descriptor())?;

        let mut publish = BoundarySystemContract::new(
            "publish-order",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        publish.reads = vec![order_state(), StateKey::core_ingress()];
        publish.writes = vec![order_state()];
        publish.plugin_ingress_targets = vec![
            canwu_api::PluginIngressTarget {
                target_plugin: TREASURY_PLUGIN.to_owned(),
                packet_type: EXECUTE_INGRESS.to_owned(),
            },
            canwu_api::PluginIngressTarget {
                target_plugin: COUNTY_PLUGIN.to_owned(),
                packet_type: EXECUTE_INGRESS.to_owned(),
            },
        ];
        publish.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(publish, publish_order)?;

        let mut audit = BoundarySystemContract::new(
            "audit-order",
            BoundaryPhase::StrategicAggregation,
            SystemCadence::EventDriven,
        );
        audit.reads = vec![
            order_state(),
            treasury_state(),
            county_state(),
            StateKey::core_ingress(),
        ];
        registrar.register_boundary_system(audit, audit_order)
    }
}

struct TreasuryPlugin;

impl SimulationPlugin for TreasuryPlugin {
    fn name(&self) -> &'static str {
        TREASURY_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000302"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_entity::<TreasuryActionRecord>();
        schema.payload_schema = object_schema(&[
            ("status", PayloadValueType::String),
            ("grain_units", PayloadValueType::Integer),
        ]);
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(execute_descriptor())?;
        let mut prepare = BoundarySystemContract::new(
            "prepare-treasury",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        prepare.reads = vec![order_state(), StateKey::core_ingress()];
        prepare.writes = vec![treasury_state()];
        prepare.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(prepare, prepare_treasury)
    }
}

struct CountyPlugin;

impl SimulationPlugin for CountyPlugin {
    fn name(&self) -> &'static str {
        COUNTY_PLUGIN
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "0000000000000000000000000000000000000000000000000000000000000303"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_entity::<CountyActionRecord>();
        schema.payload_schema = object_schema(&[
            ("status", PayloadValueType::String),
            ("grain_units", PayloadValueType::Integer),
        ]);
        registrar.register_record_schema(schema)?;
        registrar.register_ingress(execute_descriptor())?;
        let mut prepare = BoundarySystemContract::new(
            "prepare-county",
            BoundaryPhase::HistoricalCandidateEvaluation,
            SystemCadence::EventDriven,
        );
        prepare.reads = vec![order_state(), StateKey::core_ingress()];
        prepare.writes = vec![county_state()];
        prepare.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(prepare, prepare_county)
    }
}

fn main() -> Result<(), CanwuError> {
    let central = CentralPlugin;
    let treasury = TreasuryPlugin;
    let county = CountyPlugin;
    let plugins: [&dyn SimulationPlugin; 3] = [&central, &treasury, &county];
    let mut canwu =
        Canwu::new_with_plugins(11, Scenario::new(SimTime::EPOCH, Vec::new()), &plugins)?;

    canwu.enqueue_plugin_ingress(canwu_api::PluginIngressRequest::new(
        CENTRAL_PLUGIN,
        ISSUE_INGRESS,
        SimTime::EPOCH,
        json!({"order_id": ORDER_ID}),
    ))?;

    let first = canwu.settle_boundary(BoundaryRequest::at(SimTime::EPOCH))?;
    assert_eq!(first.generated_ingress.len(), 2);
    assert!(canwu.typed_domain_record(&order_reference()).is_some());
    assert!(canwu.typed_domain_record(&treasury_reference()).is_none());
    assert!(canwu.typed_domain_record(&county_reference()).is_none());

    let second = canwu.settle_boundary(BoundaryRequest::at(SimTime::EPOCH))?;
    assert_eq!(second.record_change_count, 2);
    assert!(canwu.typed_domain_record(&treasury_reference()).is_some());
    assert!(canwu.typed_domain_record(&county_reference()).is_some());

    let snapshot = canwu.snapshot_json()?;
    let restored = Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins)?;
    let replayed = Canwu::replay_from_journal(&plugins, &canwu.replay_journal())?;
    assert_eq!(restored.snapshot(), canwu.snapshot());
    assert_eq!(replayed.snapshot(), canwu.snapshot());

    println!(
        "relief_order={} treasury_grain={} county_grain={} exact_replay=ok",
        ORDER_ID, 600, 600
    );
    Ok(())
}
