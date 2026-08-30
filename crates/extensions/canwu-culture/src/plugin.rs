use crate::{CultureStateRecord, PLUGIN_NAME, culture_state_reference};
use canwu_api::{
    CanwuError, DomainRecordMutation, DomainRecordSchema, ErrorCode,
    OwnerAuthorizedMaintenanceRequest, OwnerAuthorizedMutation, OwnerAuthorizedParticipantDraft,
    OwnerAuthorizedParticipantRole, PayloadProperty, PayloadSchema, PayloadValueType,
    PluginRegistrar, SimulationPlugin, SimulationView, StateVisibility,
};
use std::collections::BTreeMap;

pub const SEMANTIC_HASH: &str = "182928304e00c319d7d01c52dffc9df8f27d57f4fdd039d2adb4df8627c256b5";

/// Registers the persisted culture lifecycle record.
///
/// Settlement remains host-driven through [`crate::CultureRuntime`]. A future
/// command/ingress layer can add lifecycle mutations without changing the
/// record identity or compiled-plan contract.
#[derive(Clone, Copy, Debug, Default)]
pub struct CulturePlugin;

impl SimulationPlugin for CulturePlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn semantic_hash(&self) -> &'static str {
        SEMANTIC_HASH
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        let mut schema = DomainRecordSchema::for_record::<CultureStateRecord>();
        schema.payload_schema = PayloadSchema::Object {
            properties: BTreeMap::from([
                ("plan_hash".to_owned(), required(PayloadValueType::String)),
                (
                    "schema_version".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                (
                    "boundary_index".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("dirty_pairs".to_owned(), required(PayloadValueType::Object)),
                ("dormant_due".to_owned(), required(PayloadValueType::Object)),
                (
                    "effect_emissions".to_owned(),
                    required(PayloadValueType::Object),
                ),
                ("hot_targets".to_owned(), required(PayloadValueType::Array)),
                (
                    "last_boundary_at".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                (
                    "latest_activity_at".to_owned(),
                    required(PayloadValueType::Integer),
                ),
                ("targets".to_owned(), required(PayloadValueType::Object)),
                ("tombstones".to_owned(), required(PayloadValueType::Array)),
            ]),
            allow_additional: false,
        };
        registrar.register_record_schema(schema)?;
        registrar.register_owner_authorized_maintenance_participant(culture_maintenance_participant)
    }
}

fn culture_maintenance_participant(
    view: &SimulationView<'_>,
    request: &OwnerAuthorizedMaintenanceRequest,
    role: OwnerAuthorizedParticipantRole,
) -> Result<OwnerAuthorizedParticipantDraft, CanwuError> {
    if role != OwnerAuthorizedParticipantRole::TargetOwner
        || request.target.record != culture_state_reference().into_untyped()
        || request
            .payload
            .get("operation")
            .and_then(serde_json::Value::as_str)
            != Some("retire_plugin_state")
    {
        return Err(CanwuError::new(
            ErrorCode::InvalidDomainRecord,
            "culture maintenance authorizes only explicit retirement of its persisted plugin state",
        ));
    }
    let record = view.domain_record(&request.target.record)?.ok_or_else(|| {
        CanwuError::new(ErrorCode::InvalidDomainRecord, "culture state is missing")
    })?;
    Ok(OwnerAuthorizedParticipantDraft {
        plugin: PLUGIN_NAME.to_owned(),
        role,
        accepted: true,
        rejection_reason: None,
        expected_records: vec![request.target.clone()],
        mutations: vec![OwnerAuthorizedMutation {
            mutation: DomainRecordMutation::Retire {
                record: record.reference.clone(),
                expected_version: record.version,
                successor: None,
            },
            visibility: StateVisibility::NextBoundary,
            summary: "Culture owner authorizes retirement of the persisted plugin state".to_owned(),
        }],
    })
}

/// Loads and validates the persisted lifecycle index against an exact plan.
pub fn load_culture_state_for_plan(
    canwu: &canwu_api::Canwu,
    plan: &crate::CompiledCulturePlan,
) -> Result<Option<crate::CultureState>, CanwuError> {
    let Some(record) = canwu.typed_domain_record(&culture_state_reference()) else {
        return Ok(None);
    };
    let state = record.decode_payload::<CultureStateRecord>()?;
    state.validate_against_plan(plan)?;
    Ok(Some(state))
}

/// Loads and hydrates a complete culture runtime from the plugin record.
pub fn load_culture_runtime(
    canwu: &canwu_api::Canwu,
    plan: &crate::CompiledCulturePlan,
) -> Result<Option<crate::CultureRuntime>, CanwuError> {
    load_culture_state_for_plan(canwu, plan)?
        .map(|state| crate::CultureRuntime::from_state(plan, state))
        .transpose()
}

const fn required(value_type: PayloadValueType) -> PayloadProperty {
    PayloadProperty {
        value_type,
        required: true,
    }
}
