use crate::{
    CORRESPONDENCE_COMMAND, InitiateCorrespondenceRequest, PLUGIN_NAME,
    RESOLVE_CORRESPONDENCE_COMMAND, ResolveCorrespondenceRequest,
};
use canwu_api::{
    Command, DecisionContext, DecisionOption, DecisionTicketDraft, DecisionTicketId, EntityRef,
    SimTime,
};

pub fn correspondence_command(
    request: &InitiateCorrespondenceRequest,
) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: CORRESPONDENCE_COMMAND.to_owned(),
        payload: serde_json::to_value(request)?,
    })
}

pub fn resolve_correspondence_command(
    request: &ResolveCorrespondenceRequest,
) -> Result<Command, serde_json::Error> {
    Ok(Command::Plugin {
        plugin: PLUGIN_NAME.to_owned(),
        command: RESOLVE_CORRESPONDENCE_COMMAND.to_owned(),
        payload: serde_json::to_value(request)?,
    })
}

pub fn correspondence_decision_ticket(
    id: DecisionTicketId,
    decision_maker: EntityRef,
    controller: impl Into<String>,
    summary: impl Into<String>,
    deadline: Option<SimTime>,
    request: &InitiateCorrespondenceRequest,
) -> Result<DecisionTicketDraft, serde_json::Error> {
    let mut send = DecisionOption::new("send", "Send correspondence");
    "Commit the prepared dispatch to addressed delivery.".clone_into(&mut send.description);
    send.action = canwu_api::DecisionAction::Command {
        command: serde_json::to_value(correspondence_command(request)?)?,
    };
    send.metadata = serde_json::json!({
        "operation_key": request.operation_key,
        "recipient": request.recipient,
        "channel_profile": request.channel_profile,
    });
    let mut decline = DecisionOption::new("do_not_send", "Do not send");
    "Leave the prepared dispatch inactive.".clone_into(&mut decline.description);
    Ok(DecisionTicketDraft {
        id,
        definition: "canwu.correspondence.send.v1".to_owned(),
        decision_maker,
        assigned_controller: controller.into(),
        summary: summary.into(),
        context: DecisionContext::new(
            "canwu.correspondence.prepared-dispatch.v1",
            serde_json::json!({
                "operation_key": request.operation_key,
                "prepared_dispatch": request.prepared_dispatch,
            }),
        ),
        options: vec![decline, send],
        deadline,
    })
}

pub fn correspondence_recovery_decision_ticket(
    id: DecisionTicketId,
    decision_maker: EntityRef,
    controller: impl Into<String>,
    summary: impl Into<String>,
    deadline: Option<SimTime>,
    request: &ResolveCorrespondenceRequest,
) -> Result<DecisionTicketDraft, serde_json::Error> {
    let mut apply = DecisionOption::new("apply_recovery", "Apply correspondence recovery");
    "Apply the selected replan, retry, or finalization action.".clone_into(&mut apply.description);
    apply.action = canwu_api::DecisionAction::Command {
        command: serde_json::to_value(resolve_correspondence_command(request)?)?,
    };
    apply.metadata = serde_json::json!({
        "operation_key": request.operation_key,
        "action": request.action,
    });
    let mut defer = DecisionOption::new("defer", "Defer recovery");
    "Leave the failed correspondence unchanged.".clone_into(&mut defer.description);
    Ok(DecisionTicketDraft {
        id,
        definition: "canwu.correspondence.recovery.v1".to_owned(),
        decision_maker,
        assigned_controller: controller.into(),
        summary: summary.into(),
        context: DecisionContext::new(
            "canwu.correspondence.failed-operation.v1",
            serde_json::json!({"operation_key": request.operation_key}),
        ),
        options: vec![defer, apply],
        deadline,
    })
}
