use canwu_api::{
    ArmyId, Canwu, CanwuError, Command, CommandContext, CommandEnvelope, EntityRef, ErrorCode,
    Issuer, PayloadProperty, PayloadSchema, PayloadValueType, PluginActionDescriptor,
    PluginRegistrar, SimulationPlugin, SimulationView, StateKey, SystemDirective,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

struct StancePlugin;

fn set_stance(
    view: &SimulationView<'_>,
    context: &CommandContext,
    payload: &Value,
) -> Result<Vec<SystemDirective>, CanwuError> {
    let army = ArmyId::new(
        payload
            .get("army")
            .and_then(Value::as_u64)
            .expect("payload was validated before the handler ran"),
    );
    let Some(army_state) = view.army(army)? else {
        return Err(CanwuError::new(
            ErrorCode::ArmyNotFound,
            format!("army {army} was not found"),
        ));
    };
    if context.issuer != Issuer::Actor(army_state.commander) {
        return Err(CanwuError::new(
            ErrorCode::InvalidAuthority,
            "only the army commander may set its stance",
        ));
    }
    Ok(vec![SystemDirective::SetComponent {
        state: StateKey::new("military", "stance"),
        entity: EntityRef::Army(army),
        component: "stance".to_owned(),
        value: payload["stance"].clone(),
        summary: format!("Army {army} changed stance"),
    }])
}

impl SimulationPlugin for StancePlugin {
    fn name(&self) -> &'static str {
        "example-stance"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_command(
            PluginActionDescriptor {
                name: "set_stance".to_owned(),
                description: "Set an army stance through an issuer-aware command".to_owned(),
                payload_schema: PayloadSchema::Object {
                    properties: BTreeMap::from([
                        (
                            "army".to_owned(),
                            PayloadProperty {
                                value_type: PayloadValueType::Integer,
                                required: true,
                            },
                        ),
                        (
                            "stance".to_owned(),
                            PayloadProperty {
                                value_type: PayloadValueType::String,
                                required: true,
                            },
                        ),
                    ]),
                    allow_additional: false,
                },
                reads: vec![StateKey::core_armies()],
                writes: vec![StateKey::new("military", "stance")],
            },
            set_stance,
        )
    }
}

fn main() -> Result<(), CanwuError> {
    let mut canwu = Canwu::demo(35)?;
    let ids = Canwu::demo_ids();
    canwu.register_plugin(&StancePlugin)?;
    canwu.submit(CommandEnvelope::new(
        Issuer::Actor(ids.commander),
        Command::Plugin {
            plugin: "example-stance".to_owned(),
            command: "set_stance".to_owned(),
            payload: json!({ "army": ids.army, "stance": "hold" }),
        },
    ))?;
    Ok(())
}
