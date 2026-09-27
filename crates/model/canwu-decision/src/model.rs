use canwu_core::{
    CommandRequestId, DecisionRequestId, DecisionTicketId, DecisionTraceId, EntityRef, PersonId,
    RandomDrawId,
};
use canwu_time::SimTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionErrorCode {
    ClosedTicket,
    DuplicateController,
    DuplicateResponse,
    DuplicateTicket,
    DecisionHistoryUnavailable,
    InvalidController,
    InvalidDecision,
    InvalidOption,
    PolicyMismatch,
    TicketNotFound,
    QueryBudgetExceeded,
    VersionConflict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAttemptErrorCode {
    SimulationRevisionConflict,
    CommandRequestConflict,
    EntityUnavailable,
    ClosedTicket,
    DuplicateController,
    DuplicateResponse,
    DuplicateTicket,
    DecisionHistoryUnavailable,
    InvalidController,
    InvalidDecision,
    InvalidOption,
    PolicyMismatch,
    TicketNotFound,
    QueryBudgetExceeded,
    VersionConflict,
    /// The resolving controller acts for a person who is not alive or is
    /// detained or captive.
    IssuerUnavailable,
    /// The decision maker is a person who is not alive or is detained or
    /// captive.
    DecisionMakerUnavailable,
}

impl From<DecisionErrorCode> for DecisionAttemptErrorCode {
    fn from(value: DecisionErrorCode) -> Self {
        match value {
            DecisionErrorCode::ClosedTicket => Self::ClosedTicket,
            DecisionErrorCode::DuplicateController => Self::DuplicateController,
            DecisionErrorCode::DuplicateResponse => Self::DuplicateResponse,
            DecisionErrorCode::DuplicateTicket => Self::DuplicateTicket,
            DecisionErrorCode::DecisionHistoryUnavailable => Self::DecisionHistoryUnavailable,
            DecisionErrorCode::InvalidController => Self::InvalidController,
            DecisionErrorCode::InvalidDecision => Self::InvalidDecision,
            DecisionErrorCode::InvalidOption => Self::InvalidOption,
            DecisionErrorCode::PolicyMismatch => Self::PolicyMismatch,
            DecisionErrorCode::TicketNotFound => Self::TicketNotFound,
            DecisionErrorCode::QueryBudgetExceeded => Self::QueryBudgetExceeded,
            DecisionErrorCode::VersionConflict => Self::VersionConflict,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionError {
    pub code: DecisionErrorCode,
    pub message: String,
}

impl DecisionError {
    #[must_use]
    pub fn new(code: DecisionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl Display for DecisionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl Error for DecisionError {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPolicyKind {
    Utility,
    Rule,
    Random,
    Human,
    External,
    Llm,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DecisionPolicyIdentity {
    pub kind: DecisionPolicyKind,
    pub id: String,
    pub version: String,
    /// Canonical lower-case BLAKE3 digest of the policy's semantic
    /// configuration, for SDK adapters whose configuration is part of their
    /// identity (for example [`crate::GuardedUtilityPolicy`]). A controller
    /// binding then rejects a policy whose configuration drifted without a
    /// version change. `None` keeps the historical identity shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_hash: Option<String>,
}

impl DecisionPolicyIdentity {
    #[must_use]
    pub fn new(
        kind: DecisionPolicyKind,
        id: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            id: id.into(),
            version: version.into(),
            semantic_hash: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.id, "policy ID")?;
        require_text(&self.version, "policy version")?;
        if self.semantic_hash.as_deref().is_some_and(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        }) {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidController,
                "policy semantic hash must be lower-case 32-byte hexadecimal",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAuthority {
    Actor {
        actor: PersonId,
    },
    Institution {
        institution: EntityRef,
        responsible_actor: Option<PersonId>,
    },
    Council {
        council_id: String,
    },
    NoResponsibleActor {
        reason: String,
    },
}

impl DecisionAuthority {
    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        match self {
            Self::Actor { .. } | Self::Institution { .. } => Ok(()),
            Self::Council { council_id } => require_identifier(council_id, "council ID"),
            Self::NoResponsibleActor { reason } => require_text(reason, "authority reason"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionControllerBinding {
    pub id: String,
    pub policy: DecisionPolicyIdentity,
    pub authority: DecisionAuthority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_subject: Option<EntityRef>,
    /// Opts a utility-policy controller into random tie-breaks: its tickets
    /// may then be resolved by `ResolveDecisionRandomly` over the candidates
    /// of a pending [`DecisionOutcome::PendingRandom`] decision. Without this
    /// opt-in, only random-policy controllers accept draw evidence.
    #[serde(default, skip_serializing_if = "is_false")]
    pub random_tie_break: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}

impl DecisionControllerBinding {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        policy: DecisionPolicyIdentity,
        authority: DecisionAuthority,
    ) -> Self {
        Self {
            id: id.into(),
            policy,
            authority,
            seat_id: None,
            permission_profile_id: None,
            command_subject: None,
            random_tie_break: false,
        }
    }

    /// Permits random tie-breaks for this utility-policy controller.
    #[must_use]
    pub const fn with_random_tie_break(mut self) -> Self {
        self.random_tie_break = true;
        self
    }

    #[must_use]
    pub fn with_seat(
        mut self,
        seat_id: impl Into<String>,
        permission_profile_id: impl Into<String>,
    ) -> Self {
        self.seat_id = Some(seat_id.into());
        self.permission_profile_id = Some(permission_profile_id.into());
        self
    }

    #[must_use]
    pub fn with_command_subject(mut self, command_subject: EntityRef) -> Self {
        self.command_subject = Some(command_subject);
        self
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.id, "controller ID")?;
        self.policy.validate()?;
        self.authority.validate()?;
        if self.seat_id.is_some() != self.permission_profile_id.is_some() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidController,
                "seat ID and permission-profile ID must be supplied together",
            ));
        }
        if let Some(seat_id) = &self.seat_id {
            require_identifier(seat_id, "seat ID")?;
        }
        if let Some(profile) = &self.permission_profile_id {
            require_identifier(profile, "permission-profile ID")?;
        }
        if self.random_tie_break && self.policy.kind != DecisionPolicyKind::Utility {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidController,
                "only utility-policy controllers can opt into random tie-breaks",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionContext {
    pub schema: String,
    pub payload: Value,
}

impl DecisionContext {
    #[must_use]
    pub fn new(schema: impl Into<String>, payload: Value) -> Self {
        Self {
            schema: schema.into(),
            payload,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.schema, "decision context schema")
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionAction {
    #[default]
    None,
    /// A serialized `canwu_sim::Command`. The controller supplies issuer and
    /// authority; a policy can only select this existing envelope.
    Command { command: Value },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionOption {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub action: DecisionAction,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub utility_inputs: BTreeMap<String, i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

impl DecisionOption {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: String::new(),
            action: DecisionAction::None,
            utility_inputs: BTreeMap::new(),
            blockers: Vec::new(),
            metadata: Value::Null,
        }
    }

    #[must_use]
    pub fn with_command(mut self, command: Value) -> Self {
        self.action = DecisionAction::Command { command };
        self
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.blockers.is_empty()
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        require_identifier(&self.id, "option ID")?;
        require_text(&self.label, "option label")?;
        for key in self.utility_inputs.keys() {
            require_identifier(key, "utility factor")?;
        }
        if self.blockers.iter().any(|value| !is_canonical_text(value)) {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidOption,
                "option blockers must be non-empty canonical text",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTicketDraft {
    pub id: DecisionTicketId,
    pub definition: String,
    pub decision_maker: EntityRef,
    pub assigned_controller: String,
    pub summary: String,
    pub context: DecisionContext,
    pub options: Vec<DecisionOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<SimTime>,
    /// Earlier ticket this decision follows up. At admission the parent must
    /// be a terminal ticket in hot decision history with the same
    /// `decision_maker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ticket: Option<DecisionTicketId>,
}

impl DecisionTicketDraft {
    pub(crate) fn validate(&mut self) -> Result<(), DecisionError> {
        if self.id.get() == 0 {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision ticket IDs must be nonzero",
            ));
        }
        validate_parent_reference(self.id, self.parent_ticket)?;
        require_identifier(&self.definition, "decision definition")?;
        require_identifier(&self.assigned_controller, "assigned controller")?;
        require_text(&self.summary, "decision summary")?;
        self.context.validate()?;
        canonicalize_options(&mut self.options)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DecisionTicketState {
    Open,
    Resolved {
        option_id: String,
        trace_id: DecisionTraceId,
    },
    Cancelled {
        reason: String,
    },
    Expired,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTicket {
    pub id: DecisionTicketId,
    pub definition: String,
    pub decision_maker: EntityRef,
    pub assigned_controller: String,
    pub summary: String,
    pub context: DecisionContext,
    pub options: Vec<DecisionOption>,
    pub opened_at: SimTime,
    pub updated_at: SimTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<SimTime>,
    pub version: u64,
    pub state: DecisionTicketState,
    /// Terminal ticket, of the same decision maker, that this ticket follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ticket: Option<DecisionTicketId>,
}

impl DecisionTicket {
    #[must_use]
    pub fn option(&self, id: &str) -> Option<&DecisionOption> {
        self.options
            .binary_search_by(|option| option.id.as_str().cmp(id))
            .ok()
            .and_then(|index| self.options.get(index))
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self.state, DecisionTicketState::Open)
    }

    pub(crate) fn validate(&self) -> Result<(), DecisionError> {
        validate_parent_reference(self.id, self.parent_ticket)?;
        require_identifier(&self.definition, "decision definition")?;
        require_identifier(&self.assigned_controller, "assigned controller")?;
        require_text(&self.summary, "decision summary")?;
        self.context.validate()?;
        let mut options = self.options.clone();
        canonicalize_options(&mut options)?;
        if options != self.options || self.version == 0 || self.updated_at < self.opened_at {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision ticket ordering, version, or timestamps are invalid",
            ));
        }
        if self
            .deadline
            .is_some_and(|deadline| deadline < self.opened_at)
        {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "decision deadline precedes the ticket opening time",
            ));
        }
        match &self.state {
            DecisionTicketState::Resolved { option_id, .. } if self.option(option_id).is_none() => {
                Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "resolved decision references an unknown option",
                ))
            }
            DecisionTicketState::Cancelled { reason } => {
                require_text(reason, "decision cancellation reason")
            }
            DecisionTicketState::Open
            | DecisionTicketState::Resolved { .. }
            | DecisionTicketState::Expired => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionFactorContribution {
    pub factor: String,
    pub value: i64,
    pub weight: i64,
    pub contribution: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionOptionEvaluation {
    pub option_id: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub factors: Vec<DecisionFactorContribution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionExternalEvidence {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct DecisionOptionWeight {
    pub option_id: String,
    pub weight: u64,
}

impl DecisionOptionWeight {
    #[must_use]
    pub fn new(option_id: impl Into<String>, weight: u64) -> Self {
        Self {
            option_id: option_id.into(),
            weight,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionRandomEvidence {
    pub draw_id: RandomDrawId,
    pub value: u64,
    pub upper_exclusive: u64,
    pub option_weights: Vec<DecisionOptionWeight>,
}

impl DecisionRandomEvidence {
    /// Selects the option of a random-policy draw. The weights must cover
    /// every available ticket option exactly once, in canonical order.
    pub fn selected_option(
        ticket: &DecisionTicket,
        option_weights: &[DecisionOptionWeight],
        value: u64,
    ) -> Result<String, DecisionError> {
        validate_option_weights(ticket, option_weights)?;
        let upper_exclusive = checked_option_weight_total(option_weights)?;
        Self::selected_option_from_weights(option_weights, value, upper_exclusive)
    }

    /// Selects the option of a random tie-break draw. The weights name only
    /// the near-equivalent candidates a policy left pending: at least two
    /// distinct available ticket options, each with a positive weight, in
    /// canonical order.
    pub fn selected_candidate(
        ticket: &DecisionTicket,
        candidates: &[DecisionOptionWeight],
        value: u64,
    ) -> Result<String, DecisionError> {
        validate_candidate_weights(ticket, candidates)?;
        let upper_exclusive = checked_option_weight_total(candidates)?;
        Self::selected_option_from_weights(candidates, value, upper_exclusive)
    }

    pub fn selected_option_from_weights(
        option_weights: &[DecisionOptionWeight],
        value: u64,
        upper_exclusive: u64,
    ) -> Result<String, DecisionError> {
        let observed_total = checked_option_weight_total(option_weights)?;
        if observed_total != upper_exclusive {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "random decision option weights disagree with the draw bound",
            ));
        }
        if value >= upper_exclusive {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "random decision value is outside its positive total weight",
            ));
        }
        let mut cursor = 0_u64;
        for option in option_weights {
            cursor = cursor.checked_add(option.weight).ok_or_else(|| {
                DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "random decision option weights overflow the supported range",
                )
            })?;
            if value < cursor {
                return Ok(option.option_id.clone());
            }
        }
        Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "random decision weights did not select an option",
        ))
    }

    fn validate(
        &self,
        ticket: &DecisionTicket,
        selected_option: &str,
        tie_break: bool,
    ) -> Result<(), DecisionError> {
        if self.draw_id.get() == 0 {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "random decision evidence requires a nonzero draw ID",
            ));
        }
        let observed = if tie_break {
            Self::selected_candidate(ticket, &self.option_weights, self.value)?
        } else {
            Self::selected_option(ticket, &self.option_weights, self.value)?
        };
        if checked_option_weight_total(&self.option_weights)? != self.upper_exclusive {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "random decision total weight disagrees with its draw bound",
            ));
        }
        if observed != selected_option {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "random decision evidence does not select the recorded option",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionOutcome {
    Selected {
        option_id: String,
    },
    Deferred {
        reason: String,
    },
    Pending {
        reason: String,
    },
    /// A non-authoritative outcome: the policy found near-equivalent
    /// candidates and asks the boundary to choose among only these options
    /// with `ResolveDecisionRandomly`. It is never persisted as a resolution.
    PendingRandom {
        candidates: Vec<DecisionOptionWeight>,
    },
}

impl DecisionOutcome {
    /// Returns whether the outcome waits for later input instead of resolving
    /// or deferring the ticket.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. } | Self::PendingRandom { .. })
    }
}

/// The stage of a composite policy that produced a decision. Decisions from
/// single-stage policies, and every historical trace, carry no stage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStage {
    /// An ordered guard selected or deferred before utility scoring.
    Guard,
    /// Weighted utility scoring selected or deferred.
    Utility,
    /// A bounded random tie-break among near-equivalent utility candidates.
    Random,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyDecision {
    pub outcome: DecisionOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluations: Vec<DecisionOptionEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<DecisionExternalEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random: Option<DecisionRandomEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<DecisionStage>,
    /// Guard IDs that returned a choice other than no-match, in evaluation
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fired_guards: Vec<String>,
}

impl PolicyDecision {
    #[must_use]
    pub fn selected(option_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            outcome: DecisionOutcome::Selected {
                option_id: option_id.into(),
            },
            summary: summary.into(),
            evaluations: Vec::new(),
            external: None,
            random: None,
            stage: None,
            fired_guards: Vec::new(),
        }
    }

    #[must_use]
    pub fn pending(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        Self {
            outcome: DecisionOutcome::Pending {
                reason: reason.clone(),
            },
            summary: reason,
            evaluations: Vec::new(),
            external: None,
            random: None,
            stage: None,
            fired_guards: Vec::new(),
        }
    }

    /// Returns whether this decision is a random tie-break among candidates
    /// rather than a random-policy draw over every available option.
    #[must_use]
    pub fn is_random_tie_break(&self) -> bool {
        self.stage == Some(DecisionStage::Random)
    }

    /// Validates this decision against the ticket it answers: a selection
    /// names an available option, a pending tie-break names only available
    /// candidates, evaluations reference ticket options, and draw evidence
    /// selects the recorded option.
    pub fn validate(&self, ticket: &DecisionTicket) -> Result<(), DecisionError> {
        require_text(&self.summary, "policy decision summary")?;
        match &self.outcome {
            DecisionOutcome::Selected { option_id } => {
                let option = ticket.option(option_id).ok_or_else(|| {
                    DecisionError::new(
                        DecisionErrorCode::InvalidOption,
                        format!("policy selected unknown option {option_id}"),
                    )
                })?;
                if !option.is_available() {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidOption,
                        format!("policy selected blocked option {option_id}"),
                    ));
                }
            }
            DecisionOutcome::Deferred { reason } | DecisionOutcome::Pending { reason } => {
                require_text(reason, "decision outcome reason")?;
            }
            DecisionOutcome::PendingRandom { candidates } => {
                validate_candidate_weights(ticket, candidates)?;
                validate_candidates_top_scored(ticket, candidates, &self.evaluations)?;
                if !self.is_random_tie_break() || self.random.is_some() {
                    return Err(DecisionError::new(
                        DecisionErrorCode::InvalidDecision,
                        "a pending random tie-break requires the random stage and no draw evidence",
                    ));
                }
            }
        }
        for evaluation in &self.evaluations {
            if ticket.option(&evaluation.option_id).is_none() {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "policy evaluation references an unknown option",
                ));
            }
        }
        for guard in &self.fired_guards {
            require_identifier(guard, "fired guard ID")?;
        }
        if self.external.is_some() && self.random.is_some() {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "a decision cannot carry both external and random evidence",
            ));
        }
        if let Some(random) = &self.random {
            let DecisionOutcome::Selected { option_id } = &self.outcome else {
                return Err(DecisionError::new(
                    DecisionErrorCode::InvalidDecision,
                    "random decision evidence requires a selected outcome",
                ));
            };
            random.validate(ticket, option_id, self.is_random_tie_break())?;
            if self.is_random_tie_break() {
                validate_candidates_top_scored(ticket, &random.option_weights, &self.evaluations)?;
            }
        } else if self.is_random_tie_break()
            && !matches!(self.outcome, DecisionOutcome::PendingRandom { .. })
        {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "the random stage either awaits a tie-break draw or selects with draw evidence",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionTrace {
    pub id: DecisionTraceId,
    pub ticket_id: DecisionTicketId,
    pub ticket_version: u64,
    pub controller_id: String,
    pub policy: DecisionPolicyIdentity,
    pub decided_at: SimTime,
    pub outcome: DecisionOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evaluations: Vec<DecisionOptionEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<DecisionExternalEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random: Option<DecisionRandomEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_request_id: Option<CommandRequestId>,
    /// Composite-policy stage that produced the outcome; `None` for
    /// single-stage policies and historical traces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<DecisionStage>,
    /// Guard IDs that fired, in evaluation order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fired_guards: Vec<String>,
    /// The resolved ticket's parent, copied from the ticket so a trace read
    /// from history carries its decision lineage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ticket: Option<DecisionTicketId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DecisionAttemptOutcome {
    Accepted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace_id: Option<DecisionTraceId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_request_id: Option<CommandRequestId>,
    },
    Rejected {
        code: DecisionAttemptErrorCode,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DecisionAttemptRecord {
    pub request_id: DecisionRequestId,
    /// Canonical commitment to the complete admitted decision ingress request.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_commitment: String,
    pub at: SimTime,
    /// Authoritative revision immediately before decision admission.
    pub revision_before: u64,
    pub expected_revision: u64,
    pub outcome: DecisionAttemptOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionMutation {
    RegisterController {
        controller: DecisionControllerBinding,
    },
    Open {
        ticket: DecisionTicketDraft,
    },
    ReplaceOptions {
        ticket_id: DecisionTicketId,
        expected_version: u64,
        context: DecisionContext,
        options: Vec<DecisionOption>,
    },
    Resolve {
        ticket_id: DecisionTicketId,
        expected_version: u64,
        controller_id: String,
        policy: DecisionPolicyIdentity,
        decision: PolicyDecision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_request_id: Option<CommandRequestId>,
    },
    Cancel {
        ticket_id: DecisionTicketId,
        expected_version: u64,
        reason: String,
    },
}

pub(crate) fn canonicalize_options(options: &mut Vec<DecisionOption>) -> Result<(), DecisionError> {
    for option in &mut *options {
        option.blockers.sort();
        option.blockers.dedup();
        option.validate()?;
    }
    options.sort_by(|left, right| left.id.cmp(&right.id));
    if options.is_empty() || options.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidOption,
            "decision options must contain at least one unique option",
        ));
    }
    Ok(())
}

fn validate_option_weights(
    ticket: &DecisionTicket,
    option_weights: &[DecisionOptionWeight],
) -> Result<(), DecisionError> {
    checked_option_weight_total(option_weights)?;
    let available = ticket
        .options
        .iter()
        .filter(|option| option.is_available())
        .map(|option| option.id.as_str())
        .collect::<Vec<_>>();
    if available.len() != option_weights.len()
        || available
            .iter()
            .zip(option_weights)
            .any(|(option_id, weight)| *option_id != weight.option_id)
    {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "random decision weights must cover every available option exactly once",
        ));
    }
    Ok(())
}

fn validate_candidate_weights(
    ticket: &DecisionTicket,
    candidates: &[DecisionOptionWeight],
) -> Result<(), DecisionError> {
    checked_option_weight_total(candidates)?;
    if candidates.len() < 2 {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "a random tie-break requires at least two candidates",
        ));
    }
    for candidate in candidates {
        if candidate.weight == 0
            || !ticket
                .option(&candidate.option_id)
                .is_some_and(DecisionOption::is_available)
        {
            return Err(DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                format!(
                    "random tie-break candidate {} must be an available option with a positive weight",
                    candidate.option_id
                ),
            ));
        }
    }
    Ok(())
}

/// A tie-break is evidence about scores: the evaluations cover every available
/// ticket option exactly once, every candidate carries an available scored
/// evaluation, and no scored non-candidate reaches the lowest candidate score.
fn validate_candidates_top_scored(
    ticket: &DecisionTicket,
    candidates: &[DecisionOptionWeight],
    evaluations: &[DecisionOptionEvaluation],
) -> Result<(), DecisionError> {
    let mut evaluated = std::collections::BTreeSet::new();
    let covered = evaluations
        .iter()
        .all(|evaluation| evaluated.insert(evaluation.option_id.as_str()))
        && ticket
            .options
            .iter()
            .filter(|option| option.is_available())
            .all(|option| evaluated.contains(option.id.as_str()));
    if !covered {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "random tie-break evaluations must cover every available option exactly once",
        ));
    }
    let score = |option_id: &str| {
        evaluations
            .iter()
            .find(|evaluation| evaluation.option_id == option_id)
            .filter(|evaluation| evaluation.available)
            .and_then(|evaluation| evaluation.score)
    };
    let lowest_candidate = candidates
        .iter()
        .map(|candidate| score(&candidate.option_id))
        .try_fold(i64::MAX, |lowest, score| {
            score.map(|score| lowest.min(score))
        });
    let valid = lowest_candidate.is_some_and(|lowest| {
        evaluations.iter().all(|evaluation| {
            candidates
                .iter()
                .any(|candidate| candidate.option_id == evaluation.option_id)
                || !evaluation.available
                || evaluation.score.is_none_or(|score| score < lowest)
        })
    });
    if !valid {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "random tie-break candidates must be exactly the top-scored available evaluations",
        ));
    }
    Ok(())
}

fn validate_parent_reference(
    id: DecisionTicketId,
    parent: Option<DecisionTicketId>,
) -> Result<(), DecisionError> {
    if parent.is_some_and(|parent| parent.get() == 0 || parent == id) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "a decision ticket parent must be a different nonzero ticket ID",
        ));
    }
    Ok(())
}

fn checked_option_weight_total(
    option_weights: &[DecisionOptionWeight],
) -> Result<u64, DecisionError> {
    if option_weights
        .windows(2)
        .any(|pair| pair[0].option_id >= pair[1].option_id)
    {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "random decision option weights must be in canonical option-ID order",
        ));
    }
    for option in option_weights {
        require_identifier(&option.option_id, "random decision option ID")?;
    }
    let total = option_weights
        .iter()
        .try_fold(0_u64, |sum, option| sum.checked_add(option.weight))
        .ok_or_else(|| {
            DecisionError::new(
                DecisionErrorCode::InvalidDecision,
                "random decision option weights overflow the supported range",
            )
        })?;
    if total == 0 {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            "random decision option weights require a positive total",
        ));
    }
    Ok(total)
}

pub(crate) fn require_identifier(value: &str, label: &str) -> Result<(), DecisionError> {
    if !is_canonical_text(value) || value.chars().any(char::is_whitespace) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            format!("{label} must be non-empty canonical text without whitespace"),
        ));
    }
    Ok(())
}

pub(crate) fn require_text(value: &str, label: &str) -> Result<(), DecisionError> {
    if !is_canonical_text(value) {
        return Err(DecisionError::new(
            DecisionErrorCode::InvalidDecision,
            format!("{label} must be non-empty canonical text"),
        ));
    }
    Ok(())
}

fn is_canonical_text(value: &str) -> bool {
    !value.is_empty() && value == value.trim()
}
