//! Rule-evaluation trace evidence.
//!
//! A phase-7 or phase-12 boundary system may explain an application rule it
//! evaluated with [`super::BoundaryDirective::RecordEvaluationTrace`]. The
//! kernel records each trace, with its producing system, on the boundary
//! record, so the trace is part of the hash-chained boundary evidence and is
//! sealed and archived with that record. Traces are never simulation state:
//! no boundary system, command handler, or decision policy can read one back
//! to decide an outcome.
//!
//! The per-boundary bounds come from [`super::RunConfiguration`]. A proposal
//! set that exceeds them fails the boundary with
//! [`ErrorCode::EvaluationTraceLimitExceeded`]; the engine never samples or
//! drops traces. A host that does not want traces does not emit them.

use super::validation::{
    EvidenceAvailability, SnapshotValidationContext, core_world_entity_exists,
    resolve_evidence_reference, snapshot_boundary_contract,
};
use super::{
    BTreeMap, BoundaryDirective, BoundaryDomainEntityCuts, BoundaryPhase, BoundaryRecord,
    BoundarySystemContract, CanwuError, DomainRecord, DomainRecordClass, DomainRecordRef,
    EntityRef, ErrorCode, PluginRegistry, SimulationSnapshot, boundary_has_event_ingress,
    boundary_system_due, canonical_text, domain_record_commit_stage, invalid_snapshot,
    invalid_snapshot_error,
};
use canwu_core::{BoundaryId, EvaluationTraceRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Per-boundary bounds on rule-evaluation traces, declared in
/// [`super::RunConfiguration::evaluation_limits`].
///
/// Both bounds are enforced fail-closed and deterministically: the boundary
/// whose proposals exceed either bound fails with
/// [`ErrorCode::EvaluationTraceLimitExceeded`] and commits nothing. Zero is a
/// valid bound; `traces_per_boundary: 0` forbids traces for the run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EvaluationLimitsV1 {
    /// Traces recorded by all systems of one boundary together.
    pub traces_per_boundary: u32,
    /// Terms in one trace.
    pub terms_per_trace: u32,
}

impl EvaluationLimitsV1 {
    /// The bounds used when a run configuration declares none: 4,096 traces
    /// of up to 32 terms per boundary.
    pub const DEFAULT: Self = Self {
        traces_per_boundary: 4_096,
        terms_per_trace: 32,
    };
    /// The largest bounds a run configuration may declare.
    pub const MAX: Self = Self {
        traces_per_boundary: 65_536,
        terms_per_trace: 256,
    };
    /// Fixed bound on the evidence references of one term.
    pub const EVIDENCE_PER_TERM: usize = 16;
    /// Fixed byte bound on a rule ID, rule version, or term ID.
    pub const TEXT_BYTES: usize = 256;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::DEFAULT
    }

    pub(crate) fn validate(self) -> Result<(), CanwuError> {
        if self.traces_per_boundary > Self::MAX.traces_per_boundary
            || self.terms_per_trace > Self::MAX.terms_per_trace
        {
            return Err(CanwuError::new(
                ErrorCode::InvalidRunConfiguration,
                "evaluation trace limits exceed the engine maximum",
            ));
        }
        Ok(())
    }
}

impl Default for EvaluationLimitsV1 {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Committed rule-evaluation trace evidence in a boundary record, with its
/// producing system. Entries follow system execution order: phase, then
/// plugin and system name, then proposal order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryEvaluationTrace {
    pub plugin: String,
    pub system: String,
    pub phase: BoundaryPhase,
    pub trace: EvaluationTraceRecord,
}

const fn phase_records_traces(phase: BoundaryPhase) -> bool {
    matches!(
        phase,
        BoundaryPhase::DomainDeltaProposal | BoundaryPhase::StrategicAggregation
    )
}

fn limit_exceeded(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::EvaluationTraceLimitExceeded, message)
}

fn invalid_trace(message: &str) -> CanwuError {
    CanwuError::new(ErrorCode::InvalidBoundary, message)
}

fn validate_text(value: &str, label: &str) -> Result<(), CanwuError> {
    if !canonical_text(value) {
        return Err(invalid_trace(&format!(
            "evaluation trace {label} must be non-empty canonical text"
        )));
    }
    if value.len() > EvaluationLimitsV1::TEXT_BYTES {
        return Err(limit_exceeded(&format!(
            "evaluation trace {label} exceeds its byte limit"
        )));
    }
    Ok(())
}

/// Checks the producing phase, shape, and per-trace bounds shared by live
/// admission and snapshot loading.
pub(super) fn validate_trace_shape(
    phase: BoundaryPhase,
    trace: &EvaluationTraceRecord,
    boundary: BoundaryId,
    limits: EvaluationLimitsV1,
) -> Result<(), CanwuError> {
    if !phase_records_traces(phase) {
        return Err(invalid_trace(
            "evaluation traces are recorded only by phase-7 and phase-12 systems",
        ));
    }
    if trace.boundary != boundary {
        return Err(invalid_trace(
            "an evaluation trace must name the boundary that records it",
        ));
    }
    validate_text(&trace.rule_id, "rule ID")?;
    validate_text(&trace.rule_version, "rule version")?;
    if trace.terms.len() > limits.terms_per_trace as usize {
        return Err(limit_exceeded(
            "evaluation trace exceeds the run's terms-per-trace limit",
        ));
    }
    let mut term_ids = BTreeSet::new();
    for term in &trace.terms {
        validate_text(&term.term_id, "term ID")?;
        if !term_ids.insert(term.term_id.as_str()) {
            return Err(invalid_trace(
                "evaluation trace term IDs must be unique within the trace",
            ));
        }
        if term.evidence.len() > EvaluationLimitsV1::EVIDENCE_PER_TERM {
            return Err(limit_exceeded(
                "evaluation term exceeds its evidence-reference limit",
            ));
        }
        if term.evidence.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(invalid_trace(
                "evaluation term evidence must be strictly sorted and unique",
            ));
        }
    }
    Ok(())
}

/// Fails before any trace of a proposal is validated when the proposal would
/// take the boundary total past the run's limit.
pub(super) fn check_trace_budget(
    staged: usize,
    directives: &[BoundaryDirective],
    limits: EvaluationLimitsV1,
) -> Result<(), CanwuError> {
    let proposed = directives
        .iter()
        .filter(|directive| matches!(directive, BoundaryDirective::RecordEvaluationTrace { .. }))
        .count();
    if staged.saturating_add(proposed) > limits.traces_per_boundary as usize {
        return Err(limit_exceeded(
            "boundary proposals exceed the run's traces-per-boundary limit",
        ));
    }
    Ok(())
}

/// Appends one system's validated, budget-checked traces to the boundary's
/// pending evidence.
pub(super) fn stage_traces(
    staged: &mut Vec<BoundaryEvaluationTrace>,
    plugin: &str,
    contract: &BoundarySystemContract,
    traces: impl IntoIterator<Item = EvaluationTraceRecord>,
) {
    staged.extend(traces.into_iter().map(|trace| BoundaryEvaluationTrace {
        plugin: plugin.to_owned(),
        system: contract.name.clone(),
        phase: contract.phase,
        trace,
    }));
}

/// Validates one persisted boundary's trace evidence against its producing
/// contracts, the run's limits, subject identities, and retained evidence.
pub(super) fn validate_snapshot_boundary_traces(
    record: &BoundaryRecord,
    snapshot: &SimulationSnapshot,
    plugins: &PluginRegistry,
    final_records: &BTreeMap<DomainRecordRef, DomainRecord>,
    cuts: &BoundaryDomainEntityCuts,
    limits: EvaluationLimitsV1,
) -> Result<(), CanwuError> {
    if record.evaluation_traces.len() > limits.traces_per_boundary as usize {
        return invalid_snapshot("boundary evaluation traces exceed the run's limit");
    }
    let evidence = SnapshotValidationContext::new(snapshot);
    let mut previous: Option<(BoundaryPhase, &str, &str)> = None;
    for entry in &record.evaluation_traces {
        let Some(contract) = snapshot_boundary_contract(plugins, &entry.plugin, &entry.system)
        else {
            return invalid_snapshot("evaluation trace references an unknown boundary system");
        };
        let Some(commit_stage) = domain_record_commit_stage(contract.phase, contract.visibility)
        else {
            return invalid_snapshot("evaluation trace producer has no settlement stage");
        };
        let order = (entry.phase, entry.plugin.as_str(), entry.system.as_str());
        if entry.phase != contract.phase
            || previous.is_some_and(|previous| previous > order)
            || !boundary_system_due(
                contract,
                &record.cadences,
                boundary_has_event_ingress(record),
            )
        {
            return invalid_snapshot(
                "evaluation trace producer, phase, or execution order is inconsistent",
            );
        }
        previous = Some(order);
        validate_trace_shape(entry.phase, &entry.trace, record.id, limits).map_err(|error| {
            invalid_snapshot_error(format!(
                "evaluation trace evidence is invalid: {}",
                error.message
            ))
        })?;
        let subject_exists = match &entry.trace.subject {
            EntityRef::Domain(reference) => {
                final_records
                    .get(reference)
                    .is_some_and(|record| record.class == DomainRecordClass::Entity)
                    && cuts.identity_exists_for_proposal(
                        final_records,
                        reference,
                        contract.phase,
                        commit_stage,
                        &entry.plugin,
                        &entry.system,
                    )
            }
            subject => {
                snapshot.entities.binary_search(subject).is_ok()
                    || core_world_entity_exists(&snapshot.world, subject)
            }
        };
        if !subject_exists {
            return invalid_snapshot("evaluation trace names an unknown subject");
        }
        if entry
            .trace
            .terms
            .iter()
            .flat_map(|term| &term.evidence)
            .any(|reference| {
                resolve_evidence_reference(&evidence, reference) != EvidenceAvailability::Retained
            })
        {
            return invalid_snapshot("evaluation trace cites missing or wrong-version evidence");
        }
    }
    Ok(())
}
