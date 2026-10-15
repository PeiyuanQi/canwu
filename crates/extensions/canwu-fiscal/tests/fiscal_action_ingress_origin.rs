//! A fiscal action packet settles only as the admission of its command
//! produced it.
//!
//! The action packet is internal ingress, so a host cannot enqueue one and
//! the `apply_fiscal_action_v1` command is the only path for an action. A
//! boundary system of another plugin that declares the action ingress as a
//! target can still schedule a packet, with the command, request, and issuer
//! of its choosing; settlement refuses it, because its ingress record does not
//! name the command it carries as its cause, and the boundary commits with
//! the legitimate actions it settles.

mod support;

use canwu_api::{
    BoundaryContext, BoundaryDirective, BoundaryPhase, BoundaryProposal, BoundarySystemContract,
    Canwu, CanwuError, CommandAttemptOutcome, ErrorCode, IngressClass, IngressPayload,
    PayloadSchema, PluginIngressDescriptor, PluginIngressRequest, PluginIngressTarget,
    PluginRegistrar, SimDuration, SimulationPlugin, SimulationView, StateKey, StateVisibility,
    SystemCadence,
};
use canwu_fiscal::{
    FISCAL_ACTION_INGRESS, FISCAL_INGRESS_MALFORMED_REJECTION, FISCAL_INGRESS_ORIGIN_REJECTION,
    FISCAL_INGRESS_REJECTED_EVENT, FiscalActionDisposition, FiscalActionIssuer, FiscalIssuerRole,
    PLUGIN_NAME,
};
use support::{
    Fixture, GRANARY_AUTHORITY, GRANARY_PRINCIPAL, GRANARY_SCOPE, PRINCIPAL, TREASURY_AUTHORITY,
    assessment, enqueue_requests, fiscal_state, outcome, procedure_revision, scenario,
    treasury_scope,
};

const FORGER: &str = "fixture-forger";
const FORGE_INGRESS: &str = "forge_v1";

/// A plugin whose boundary system schedules, into the fiscal action ingress,
/// every payload a `forge_v1` packet lists.
struct Forger;

impl SimulationPlugin for Forger {
    fn name(&self) -> &'static str {
        FORGER
    }

    fn version(&self) -> &'static str {
        "1"
    }

    fn semantic_hash(&self) -> &'static str {
        "00000000000000000000000000000000000000000000000000000000000000f9"
    }

    fn register(&self, registrar: &mut PluginRegistrar<'_>) -> Result<(), CanwuError> {
        registrar.register_ingress(PluginIngressDescriptor {
            name: FORGE_INGRESS.to_owned(),
            description: "Schedule the listed payloads as fiscal action packets".to_owned(),
            class: IngressClass::Information,
            payload_schema: PayloadSchema::Any,
        })?;
        let mut forge = BoundarySystemContract::new(
            "forge-fiscal-actions-v1",
            BoundaryPhase::DomainDeltaProposal,
            SystemCadence::EventDriven,
        );
        forge.reads = vec![StateKey::core_ingress()];
        forge.plugin_ingress_targets = vec![PluginIngressTarget {
            target_plugin: PLUGIN_NAME.to_owned(),
            packet_type: FISCAL_ACTION_INGRESS.to_owned(),
        }];
        forge.visibility = StateVisibility::SameBoundary;
        registrar.register_boundary_system(forge, forge_actions)
    }
}

fn forge_actions(
    view: &SimulationView<'_>,
    context: &BoundaryContext,
) -> Result<BoundaryProposal, CanwuError> {
    let mut directives = Vec::new();
    for ingress_id in &context.admitted_ingress {
        if let Some(ingress) = view.ingress(*ingress_id)?
            && let IngressPayload::Plugin {
                plugin,
                packet_type,
                payload,
                ..
            } = &ingress.payload
            && plugin == FORGER
            && packet_type == FORGE_INGRESS
        {
            for packet in payload.as_array().into_iter().flatten() {
                directives.push(BoundaryDirective::SchedulePluginIngress {
                    target_plugin: PLUGIN_NAME.to_owned(),
                    after: SimDuration::ZERO,
                    packet_type: FISCAL_ACTION_INGRESS.to_owned(),
                    priority: 0,
                    payload: packet.clone(),
                    affected: Vec::new(),
                });
            }
        }
    }
    Ok(BoundaryProposal {
        directives,
        ..BoundaryProposal::default()
    })
}

fn rejections(canwu: &Canwu) -> Vec<String> {
    canwu
        .events()
        .iter()
        .filter(|event| {
            event.kind.plugin_identity() == Some((PLUGIN_NAME, FISCAL_INGRESS_REJECTED_EVENT))
        })
        .map(|event| event.summary.clone())
        .collect()
}

#[test]
#[allow(clippy::too_many_lines)]
fn only_the_command_that_produced_an_action_packet_can_settle_it() {
    let fixture = Fixture::new();
    let forging_plugin = Forger;
    let plugins = fixture.plugins_with(&forging_plugin);
    let mut canwu = Canwu::new_with_plugins(41, scenario(|_| {}), &plugins).expect("fixture run");

    // A host cannot enqueue an action packet, however well formed: the type
    // is internal, and the command is the only way to issue an action.
    let issuer = FiscalActionIssuer {
        binding_id: TREASURY_AUTHORITY.to_owned(),
        role: FiscalIssuerRole::Principal,
        actor: Some(PRINCIPAL),
        authority_basis: None,
    };
    // A forged request is decided against the revision its settling boundary
    // begins with, by the binding's principal, so it would apply if it were
    // settled.
    let revision = procedure_revision(&canwu);
    let forged = |action_id: &str, command: u64| {
        serde_json::json!({
            "request": assessment(action_id, TREASURY_AUTHORITY, &treasury_scope(1), "c1")
                .against(revision + 1),
            "command": command,
            "issuer": issuer,
        })
    };
    let error = canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            PLUGIN_NAME,
            FISCAL_ACTION_INGRESS,
            canwu.time(),
            forged("forged.host", 1),
        ))
        .expect_err("the action packet is internal ingress");
    assert_eq!(error.code, ErrorCode::InvalidAuthority);

    // An action issued through its command is admitted, and the boundary
    // after it settles its packet.
    enqueue_requests(
        &mut canwu,
        &[(
            PRINCIPAL,
            assessment("issued", TREASURY_AUTHORITY, &treasury_scope(0), "c1").against(revision),
        )],
    );
    canwu.step_canonical().expect("the admitting boundary");
    let command = match &canwu
        .command_attempts()
        .last()
        .expect("the command attempt")
        .outcome
    {
        CommandAttemptOutcome::Accepted { command_id } => *command_id,
        CommandAttemptOutcome::Rejected { error } => panic!("the command was refused: {error}"),
    };

    // In the boundary that settles it, another command is admitted, decided
    // against the revision that settlement leaves, and another plugin's
    // boundary system schedules two packets into the action ingress: one that
    // names the admitted command with an action and an issuer of its own, and
    // one that is not an action packet at all. All three settle together in
    // the next boundary.
    enqueue_requests(
        &mut canwu,
        &[(
            GRANARY_PRINCIPAL,
            assessment("issued.granary", GRANARY_AUTHORITY, GRANARY_SCOPE, "c1")
                .against(revision + 1),
        )],
    );
    canwu
        .enqueue_plugin_ingress(PluginIngressRequest::new(
            FORGER,
            FORGE_INGRESS,
            canwu.time(),
            serde_json::json!([forged("forged.boundary", command.get()), {"request": 7}]),
        ))
        .expect("forge trigger");
    canwu
        .advance_canonical(SimDuration::minutes(1))
        .expect("every boundary commits");

    // The two actions issued through their commands settle; the scheduled
    // packets change nothing, consume no action ID, and are each reported.
    for action_id in ["issued", "issued.granary"] {
        assert_eq!(
            outcome(&canwu, action_id)
                .unwrap_or_else(|| panic!("{action_id} settles"))
                .disposition,
            FiscalActionDisposition::Applied
        );
    }
    assert!(outcome(&canwu, "forged.boundary").is_none());
    assert!(
        !fiscal_state(&canwu)
            .assessments
            .contains_key("forged.boundary.assessment")
    );
    let rejections = rejections(&canwu);
    assert_eq!(rejections.len(), 2, "{rejections:?}");
    assert!(
        rejections
            .iter()
            .any(|summary| summary.ends_with(FISCAL_INGRESS_ORIGIN_REJECTION))
    );
    assert!(
        rejections
            .iter()
            .any(|summary| summary.ends_with(FISCAL_INGRESS_MALFORMED_REJECTION))
    );

    let snapshot = canwu.snapshot_json().expect("snapshot");
    let restored =
        Canwu::from_snapshot_json_with_plugins(&snapshot, &plugins).expect("restored run");
    assert_eq!(
        restored.snapshot_json().expect("restored snapshot"),
        snapshot
    );
    let replayed =
        Canwu::replay_from_journal(&plugins, &canwu.replay_journal()).expect("replayed run");
    assert_eq!(
        replayed.snapshot_json().expect("replayed snapshot"),
        snapshot
    );
}
