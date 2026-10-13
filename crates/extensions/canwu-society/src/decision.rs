use crate::revision::{PRESSURE_REVISION_COMMAND, PressureRevisionDecision};
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
        parent_ticket: None,
    })
}

/// One selectable pressure revision of a [`pressure_revision_ticket`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PressureRevisionChoice {
    pub id: String,
    pub label: String,
    pub decision: PressureRevisionDecision,
    pub utility_inputs: BTreeMap<String, i64>,
}

#[allow(clippy::too_many_arguments)]
/// Builds one controller-neutral pressure revision ticket. Each option
/// carries a `revise_society_pressure` command, so the selected revision is
/// admitted with the ticket controller's validated authority.
///
/// # Errors
///
/// Returns an error when the choices are empty, have duplicate identifiers,
/// are malformed, disagree about their alignment, decision version, or
/// expected version, or cannot be serialized into a Canwu command option.
pub fn pressure_revision_ticket(
    id: DecisionTicketId,
    definition: impl Into<String>,
    decision_maker: EntityRef,
    assigned_controller: impl Into<String>,
    summary: impl Into<String>,
    choices: Vec<PressureRevisionChoice>,
    deadline: Option<SimTime>,
) -> Result<DecisionTicketDraft, CanwuError> {
    let Some(first) = choices.first() else {
        return Err(CanwuError::new(
            ErrorCode::InvalidDecision,
            "pressure revision ticket requires at least one choice",
        ));
    };
    let alignment_id = first.decision.alignment_id.clone();
    let decision_version = first.decision.decision_version;
    let expected_version = first.decision.expected_version;
    let mut option_ids = BTreeSet::new();
    let mut options = Vec::with_capacity(choices.len());
    for choice in choices {
        if choice.id.is_empty() || !option_ids.insert(choice.id.clone()) {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "pressure revision choice IDs must be non-empty and unique",
            ));
        }
        choice.decision.validate()?;
        if choice.decision.alignment_id != alignment_id
            || choice.decision.decision_version != decision_version
            || choice.decision.expected_version != expected_version
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidDecision,
                "all pressure revision choices must target one alignment, version, and \
                 expected version",
            ));
        }
        let command = Command::Plugin {
            plugin: PLUGIN_NAME.to_owned(),
            command: PRESSURE_REVISION_COMMAND.to_owned(),
            payload: serde_json::to_value(&choice.decision).map_err(|error| {
                CanwuError::new(
                    ErrorCode::InvalidDecision,
                    format!("pressure revision choice could not be encoded: {error}"),
                )
            })?,
        };
        options.push(DecisionOption {
            action: DecisionAction::Command {
                command: serde_json::to_value(command).map_err(|error| {
                    CanwuError::new(
                        ErrorCode::InvalidDecision,
                        format!("pressure revision command could not be encoded: {error}"),
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
            "canwu.society.pressure-revision.v1",
            json!({
                "alignment_id": alignment_id,
                "decision_version": decision_version,
                "expected_version": expected_version,
            }),
        ),
        options,
        deadline,
        parent_ticket: None,
    })
}
