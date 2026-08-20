use crate::{PLUGIN_NAME, PolicyDecision};
use canwu_api::{
    CanwuError, Command, DecisionAction, DecisionContext, DecisionOption, DecisionTicketDraft,
    DecisionTicketId, EntityRef, ErrorCode, SimTime,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyChoice {
    pub id: String,
    pub label: String,
    pub decision: PolicyDecision,
    pub utility_inputs: BTreeMap<String, i64>,
}

#[allow(clippy::too_many_arguments)]
/// Builds one controller-neutral institutional policy ticket.
///
/// # Errors
///
/// Returns an error when the choices are empty, have duplicate identifiers,
/// disagree about their alignment or version, or cannot be serialized into a
/// Canwu command option.
pub fn institutional_policy_ticket(
    id: DecisionTicketId,
    definition: impl Into<String>,
    decision_maker: EntityRef,
    assigned_controller: impl Into<String>,
    summary: impl Into<String>,
    choices: Vec<PolicyChoice>,
    deadline: Option<SimTime>,
) -> Result<DecisionTicketDraft, CanwuError> {
    let Some(first) = choices.first() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "institutional policy ticket requires at least one choice",
        ));
    };
    let alignment_id = first.decision.alignment_id.clone();
    let decision_version = first.decision.decision_version;
    let mut option_ids = BTreeSet::new();
    let mut options = Vec::with_capacity(choices.len());
    for choice in choices {
        if choice.id.is_empty() || !option_ids.insert(choice.id.clone()) {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "institutional policy choice IDs must be non-empty and unique",
            ));
        }
        if choice.decision.alignment_id != alignment_id
            || choice.decision.decision_version != decision_version
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "all institutional policy choices must target one alignment and version",
            ));
        }
        let command = Command::Plugin {
            plugin: PLUGIN_NAME.to_owned(),
            command: "set_institutional_policy".to_owned(),
            payload: serde_json::to_value(&choice.decision).map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidDecision,
                    format!("institutional policy choice could not be encoded: {error}"),
                )
            })?,
        };
        options.push(DecisionOption {
            action: DecisionAction::Command {
                command: serde_json::to_value(command).map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidDecision,
                        format!("institutional policy command could not be encoded: {error}"),
                    )
                })?,
            },
            utility_inputs: choice.utility_inputs,
            ..DecisionOption::new(choice.id, choice.label)
        });
    }

    Ok(DecisionTicketDraft {
        id,
        definition: definition.into(),
        decision_maker,
        assigned_controller: assigned_controller.into(),
        summary: summary.into(),
        context: DecisionContext::new(
            "canwu.society.institutional-policy.v1",
            json!({
                "alignment_id": alignment_id,
                "decision_version": decision_version,
            }),
        ),
        options,
        deadline,
    })
}
