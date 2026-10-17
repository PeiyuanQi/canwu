//! Which hot records claim which outcomes and lease receipts when a commit
//! archives them, and the residues: terminal records that no execution or
//! facility project archives, so that nothing a production root settles
//! stays in it for the life of a run.
//!
//! An execution archives the outcomes of its own operations and of the
//! completion lease it bound; a facility project likewise. What nothing
//! claims leaves as a residue once every record it names is gone:
//!
//! - a work order that ended without an execution, with its outcomes;
//! - a completion lease released or expired without binding an execution or
//!   a project, with its grants, certificate, receipts, and outcomes;
//! - any other outcome: a rejection naming nothing hot, a facility
//!   retirement, or an outcome its record did not match.
//!
//! [`ArchiveClaims`] groups the root's outcomes and lease receipts by the
//! records that can claim them, once per preparation or settlement of a
//! commit, so building a commit costs the selected records and their claims
//! rather than the selected records times every hot outcome.

use crate::model::{
    ProductionCompletionLeaseArchiveV1, ProductionDecisionReceiptV1, ProductionOperation,
    ProductionOperationOutcome, ProductionResidueArchiveRecordV1, ProductionResidueBodyV1,
    ProductionState, ProductionTerminalArchiveKeyV1, WorkOrder, WorkOrderLifecycle, invalid,
};
use crate::{FacilityProject, FacilityProjectId, ProductionExecution, ProductionExecutionId};
use canwu_api::{CanwuError, SimTime, canonical_hash};
use canwu_resource::{
    CompletionLeaseAcquisitionId, CompletionLeaseAcquisitionStateV1, CompletionLeaseReceiptV1,
};
use std::collections::{BTreeMap, BTreeSet};

/// The acquisition a lease operation's outcome names.
pub(crate) fn lease_outcome_acquisition(
    outcome: &ProductionOperationOutcome,
) -> Option<&CompletionLeaseAcquisitionId> {
    match &outcome.command.operation {
        ProductionOperation::RequestCompletionLease { request } => Some(&request.id),
        ProductionOperation::AbortCompletionLease { request } => Some(&request.acquisition),
        _ => None,
    }
}

/// Whether `outcome` is one of `order`'s own operations: its creation with
/// the order's content, or an authorization, cancellation, or degraded
/// choice naming it.
pub(crate) fn outcome_matches_work_order(
    outcome: &ProductionOperationOutcome,
    order: &WorkOrder,
) -> bool {
    if outcome.command.holder != order.holder
        || outcome.project.is_some()
        || outcome.execution.is_some()
        || outcome.work_order.as_ref() != Some(&order.id)
    {
        return false;
    }
    match &outcome.command.operation {
        ProductionOperation::CreateWorkOrder { work_order } => {
            work_order.id == order.id
                && work_order.holder == order.holder
                && work_order.process == order.process
                && work_order.site == order.site
        }
        ProductionOperation::AuthorizeWorkOrder { work_order }
        | ProductionOperation::CancelWorkOrder { work_order }
        | ProductionOperation::ResolveDegradedFacility { work_order, .. } => {
            work_order == &order.id
        }
        _ => false,
    }
}

/// Whether `outcome` is an operation on the completion lease `acquisition`
/// of `holder`.
pub(crate) fn outcome_matches_lease(
    outcome: &ProductionOperationOutcome,
    acquisition: &CompletionLeaseAcquisitionId,
    holder: &canwu_api::KnowledgeHolderRef,
) -> bool {
    lease_outcome_acquisition(outcome) == Some(acquisition)
        && &outcome.command.holder == holder
        && outcome.work_order.is_none()
        && outcome.execution.is_none()
        && outcome.project.is_none()
}

/// The decision tickets a set of outcomes resolved.
pub(crate) fn resolved_tickets<'a>(
    outcomes: impl IntoIterator<Item = &'a ProductionOperationOutcome>,
) -> BTreeSet<canwu_api::DecisionTicketId> {
    outcomes
        .into_iter()
        .filter_map(|outcome| match &outcome.command.operation {
            ProductionOperation::ResolveDegradedFacility {
                decision_ticket, ..
            } => Some(*decision_ticket),
            _ => None,
        })
        .collect()
}

/// A root's outcomes and lease receipts grouped by the records that can
/// claim them, and the hot executions and projects that bind each lease.
pub(crate) struct ArchiveClaims<'a> {
    by_execution: BTreeMap<&'a ProductionExecutionId, Vec<&'a ProductionOperationOutcome>>,
    by_work_order: BTreeMap<&'a crate::WorkOrderId, Vec<&'a ProductionOperationOutcome>>,
    by_project: BTreeMap<&'a FacilityProjectId, Vec<&'a ProductionOperationOutcome>>,
    by_lease: BTreeMap<&'a CompletionLeaseAcquisitionId, Vec<&'a ProductionOperationOutcome>>,
    receipts_by_lease:
        BTreeMap<&'a CompletionLeaseAcquisitionId, Vec<&'a CompletionLeaseReceiptV1>>,
    pub(crate) bound_leases: BTreeMap<&'a CompletionLeaseAcquisitionId, usize>,
}

impl<'a> ArchiveClaims<'a> {
    pub(crate) fn of(state: &'a ProductionState) -> Self {
        let mut claims = Self {
            by_execution: BTreeMap::new(),
            by_work_order: BTreeMap::new(),
            by_project: BTreeMap::new(),
            by_lease: BTreeMap::new(),
            receipts_by_lease: BTreeMap::new(),
            bound_leases: BTreeMap::new(),
        };
        for outcome in state.operation_outcomes.values() {
            if let Some(execution) = &outcome.execution {
                claims
                    .by_execution
                    .entry(execution)
                    .or_default()
                    .push(outcome);
            }
            if let Some(order) = &outcome.work_order {
                claims.by_work_order.entry(order).or_default().push(outcome);
            }
            if let Some(project) = &outcome.project {
                claims.by_project.entry(project).or_default().push(outcome);
            }
            if let Some(acquisition) = lease_outcome_acquisition(outcome) {
                claims
                    .by_lease
                    .entry(acquisition)
                    .or_default()
                    .push(outcome);
            }
        }
        for receipt in state.completion_receipts.values() {
            claims
                .receipts_by_lease
                .entry(&receipt.acquisition)
                .or_default()
                .push(receipt);
        }
        for acquisition in state
            .executions
            .values()
            .map(|execution| &execution.completion_certificate.acquisition)
            .chain(
                state
                    .facility_projects
                    .values()
                    .map(|project| &project.completion_certificate.acquisition),
            )
        {
            *claims.bound_leases.entry(acquisition).or_default() += 1;
        }
        claims
    }

    fn collect<'b>(
        candidates: impl IntoIterator<Item = &'b ProductionOperationOutcome>,
        claims: impl Fn(&ProductionOperationOutcome) -> bool,
    ) -> Vec<ProductionOperationOutcome> {
        let mut outcomes = BTreeMap::new();
        for outcome in candidates {
            if claims(outcome) {
                outcomes.insert(outcome.id.clone(), outcome.clone());
            }
        }
        outcomes.into_values().collect()
    }

    /// The outcomes an execution archives: its own operations' and its
    /// lease's, in identity order.
    pub(crate) fn execution_outcomes(
        &self,
        order: &WorkOrder,
        execution: &ProductionExecution,
    ) -> Vec<ProductionOperationOutcome> {
        let acquisition = &execution.completion_certificate.acquisition;
        Self::collect(
            self.by_execution
                .get(&execution.id)
                .into_iter()
                .chain(self.by_work_order.get(&order.id))
                .chain(self.by_lease.get(acquisition))
                .flatten()
                .copied(),
            |outcome| {
                crate::archive::outcome_matches_execution(outcome, order, execution)
                    || outcome_matches_lease(outcome, acquisition, &order.holder)
            },
        )
    }

    /// The outcomes a facility project archives: its own operations' and
    /// its lease's, in identity order.
    pub(crate) fn project_outcomes(
        &self,
        project: &FacilityProject,
    ) -> Vec<ProductionOperationOutcome> {
        let acquisition = &project.completion_certificate.acquisition;
        Self::collect(
            self.by_project
                .get(&project.id)
                .into_iter()
                .chain(self.by_lease.get(acquisition))
                .flatten()
                .copied(),
            |outcome| {
                crate::archive::outcome_matches_project(outcome, project)
                    || outcome_matches_lease(outcome, acquisition, &project.holder)
            },
        )
    }

    /// The lease receipts of `acquisition`, in sequence order.
    pub(crate) fn lease_receipts(
        &self,
        acquisition: &CompletionLeaseAcquisitionId,
    ) -> Vec<CompletionLeaseReceiptV1> {
        self.receipts_by_lease
            .get(acquisition)
            .into_iter()
            .flatten()
            .map(|receipt| (*receipt).clone())
            .collect()
    }

    fn work_order_outcomes(&self, order: &WorkOrder) -> Vec<ProductionOperationOutcome> {
        Self::collect(
            self.by_work_order
                .get(&order.id)
                .into_iter()
                .flatten()
                .copied(),
            |outcome| outcome_matches_work_order(outcome, order),
        )
    }

    fn lease_outcomes(
        &self,
        state: &ProductionState,
        id: &CompletionLeaseAcquisitionId,
    ) -> Vec<ProductionOperationOutcome> {
        let holder = state
            .completion_acquisitions
            .get(id)
            .map(|value| value.holder.clone());
        Self::collect(
            self.by_lease.get(id).into_iter().flatten().copied(),
            |outcome| {
                holder
                    .as_ref()
                    .is_some_and(|holder| outcome_matches_lease(outcome, id, holder))
            },
        )
    }
}

/// The decision receipts of `tickets` the root holds, in ticket order.
pub(crate) fn decision_receipts_of(
    state: &ProductionState,
    tickets: &BTreeSet<canwu_api::DecisionTicketId>,
) -> Vec<ProductionDecisionReceiptV1> {
    tickets
        .iter()
        .filter_map(|ticket| state.decision_receipts.get(ticket).cloned())
        .collect()
}

const fn work_order_ended(lifecycle: WorkOrderLifecycle) -> bool {
    matches!(
        lifecycle,
        WorkOrderLifecycle::Settled | WorkOrderLifecycle::Cancelled | WorkOrderLifecycle::Failed
    )
}

const fn lease_ended(state: CompletionLeaseAcquisitionStateV1) -> bool {
    matches!(
        state,
        CompletionLeaseAcquisitionStateV1::Released | CompletionLeaseAcquisitionStateV1::Expired
    )
}

const fn grant_ended(state: canwu_resource::CompletionGrantStateV1) -> bool {
    matches!(
        state,
        canwu_resource::CompletionGrantStateV1::Completed
            | canwu_resource::CompletionGrantStateV1::Released
            | canwu_resource::CompletionGrantStateV1::Rejected
            | canwu_resource::CompletionGrantStateV1::Expired
    )
}

impl ProductionState {
    /// Whether the lease `acquisition` ended on both sides: released or
    /// expired here, and every participant grant this root recorded for it
    /// ended too, so the participant's reserved units are free before its
    /// record leaves the root.
    fn lease_fully_ended(
        &self,
        acquisition: &canwu_resource::CompletionLeaseAcquisitionV1,
    ) -> bool {
        lease_ended(acquisition.state)
            && self
                .completion_participant_grants
                .get(&acquisition.id)
                .is_none_or(|participants| {
                    participants
                        .values()
                        .all(|participant| grant_ended(participant.grant.state))
                })
    }
}

impl ProductionState {
    /// The residues the next archive commit may archive, in key order: work
    /// orders that ended without an execution, then completion leases that
    /// ended without binding a hot execution or facility project, then
    /// outcomes that name no hot record. A host application commits when
    /// this, `archive_due_index`, or `project_archive_due_index` holds
    /// records.
    #[must_use]
    pub fn residue_archive_candidates(&self) -> Vec<ProductionTerminalArchiveKeyV1> {
        self.residue_candidates().collect()
    }

    /// [`Self::residue_archive_candidates`] as a lazy iterator, so a commit
    /// that takes a few residues builds only those keys.
    pub(crate) fn residue_candidates(
        &self,
    ) -> impl Iterator<Item = ProductionTerminalArchiveKeyV1> + '_ {
        let bound = self
            .executions
            .values()
            .map(|execution| &execution.completion_certificate.acquisition)
            .chain(
                self.facility_projects
                    .values()
                    .map(|project| &project.completion_certificate.acquisition),
            )
            .collect::<BTreeSet<_>>();
        let work_orders = self
            .work_orders
            .values()
            .filter(|order| work_order_ended(order.lifecycle) && order.execution.is_none())
            .map(|order| ProductionTerminalArchiveKeyV1::WorkOrder(order.id.clone()));
        let leases = self
            .completion_acquisitions
            .values()
            .filter(move |acquisition| {
                self.lease_fully_ended(acquisition) && !bound.contains(&acquisition.id)
            })
            .map(|acquisition| {
                ProductionTerminalArchiveKeyV1::CompletionLease(acquisition.id.clone())
            });
        let outcomes = self
            .operation_outcomes
            .values()
            .filter(|outcome| self.outcome_is_residue(outcome))
            .map(|outcome| ProductionTerminalArchiveKeyV1::OperationOutcome(outcome.id.clone()));
        work_orders.chain(leases).chain(outcomes)
    }

    /// Whether no hot record can claim `outcome` any longer: it names no hot
    /// execution, work order, facility project, or completion lease.
    fn outcome_is_residue(&self, outcome: &ProductionOperationOutcome) -> bool {
        outcome
            .execution
            .as_ref()
            .is_none_or(|id| !self.executions.contains_key(id))
            && outcome
                .work_order
                .as_ref()
                .is_none_or(|id| !self.work_orders.contains_key(id))
            && outcome
                .project
                .as_ref()
                .is_none_or(|id| !self.facility_projects.contains_key(id))
            && lease_outcome_acquisition(outcome)
                .is_none_or(|id| !self.completion_acquisitions.contains_key(id))
    }

    /// The archive record of one residue candidate.
    pub(crate) fn residue_archive_record(
        &self,
        key: &ProductionTerminalArchiveKeyV1,
        claims: &ArchiveClaims<'_>,
    ) -> Result<ProductionResidueArchiveRecordV1, CanwuError> {
        let (body, operation_outcomes, terminal_floor) = match key {
            ProductionTerminalArchiveKeyV1::WorkOrder(id) => {
                let order = self
                    .work_orders
                    .get(id)
                    .filter(|order| work_order_ended(order.lifecycle) && order.execution.is_none())
                    .ok_or_else(|| invalid("work order residue is not an ended work order"))?;
                (
                    ProductionResidueBodyV1::WorkOrder {
                        work_order: order.clone(),
                    },
                    claims.work_order_outcomes(order),
                    order.created_at,
                )
            }
            ProductionTerminalArchiveKeyV1::CompletionLease(id) => {
                let acquisition = self
                    .completion_acquisitions
                    .get(id)
                    .filter(|acquisition| {
                        self.lease_fully_ended(acquisition) && !claims.bound_leases.contains_key(id)
                    })
                    .ok_or_else(|| invalid("completion lease residue is not an ended lease"))?;
                let production_grants = self
                    .production_completion_grants
                    .values()
                    .filter(|grant| grant.acquisition == *id)
                    .cloned()
                    .collect();
                let participant_grants = self
                    .completion_participant_grants
                    .get(id)
                    .into_iter()
                    .flat_map(BTreeMap::values)
                    .cloned()
                    .collect();
                (
                    ProductionResidueBodyV1::CompletionLease {
                        lease: ProductionCompletionLeaseArchiveV1 {
                            acquisition: acquisition.clone(),
                            production_grants,
                            certificate: self.production_completion_certificates.get(id).cloned(),
                            participant_grants,
                            completion_receipts: claims.lease_receipts(id),
                        },
                    },
                    claims.lease_outcomes(self, id),
                    acquisition.eligibility_time,
                )
            }
            ProductionTerminalArchiveKeyV1::OperationOutcome(id) => {
                let outcome = self
                    .operation_outcomes
                    .get(id)
                    .filter(|outcome| self.outcome_is_residue(outcome))
                    .ok_or_else(|| invalid("outcome residue is not a standalone hot outcome"))?;
                (
                    ProductionResidueBodyV1::OperationOutcome,
                    vec![outcome.clone()],
                    outcome.settled_at,
                )
            }
            _ => return Err(invalid("archive key is not a residue")),
        };
        let decision_receipts = decision_receipts_of(self, &resolved_tickets(&operation_outcomes));
        let mut record = ProductionResidueArchiveRecordV1 {
            key: key.clone(),
            body,
            terminal_at: residue_terminal_at(terminal_floor, &operation_outcomes),
            operation_outcomes,
            decision_receipts,
            canonical_digest: String::new(),
        };
        record.canonical_digest =
            canonical_hash("canwu.production.residue-archive-record.v1", &record)?;
        Ok(record)
    }

    /// Removes the residue `record` archives from the root.
    pub(crate) fn remove_residue(
        &mut self,
        record: &ProductionResidueArchiveRecordV1,
    ) -> Result<(), CanwuError> {
        match &record.body {
            ProductionResidueBodyV1::WorkOrder { work_order } => {
                self.work_orders
                    .remove(&work_order.id)
                    .ok_or_else(|| invalid("archived work order residue is not hot"))?;
            }
            ProductionResidueBodyV1::CompletionLease { lease } => {
                let id = &lease.acquisition.id;
                self.completion_acquisitions
                    .remove(id)
                    .ok_or_else(|| invalid("archived completion lease residue is not hot"))?;
                for grant in &lease.production_grants {
                    self.production_completion_grants.remove(&grant.id);
                }
                self.production_completion_certificates.remove(id);
                self.completion_participant_grants.remove(id);
                for receipt in &lease.completion_receipts {
                    self.completion_receipts.remove(&receipt.sequence);
                }
                for acquisitions in self.completion_expiry_due.values_mut() {
                    acquisitions.remove(id);
                }
                self.completion_expiry_due
                    .retain(|_, acquisitions| !acquisitions.is_empty());
            }
            ProductionResidueBodyV1::OperationOutcome => {}
        }
        for outcome in &record.operation_outcomes {
            self.operation_outcomes
                .remove(&outcome.id)
                .ok_or_else(|| invalid("archived residue outcome is not hot"))?;
        }
        for receipt in &record.decision_receipts {
            self.decision_receipts.remove(&receipt.ticket_id);
        }
        Ok(())
    }
}

fn residue_terminal_at(floor: SimTime, outcomes: &[ProductionOperationOutcome]) -> SimTime {
    outcomes
        .iter()
        .map(|outcome| outcome.settled_at)
        .chain(std::iter::once(floor))
        .max()
        .unwrap_or(floor)
}

/// Checks one archived residue: its key against its body, its outcomes
/// against what that body claims, its decision receipts against its
/// outcomes, and its terminal time.
pub(crate) fn validate_residue_archive_record(
    record: &ProductionResidueArchiveRecordV1,
) -> Result<(), CanwuError> {
    let outcomes = &record.operation_outcomes;
    let canonical = outcomes.windows(2).all(|pair| pair[0].id < pair[1].id)
        && outcomes.iter().all(|outcome| {
            crate::archive::validate_archived_operation_outcome(outcome).is_ok()
                && outcome.settled_at <= record.terminal_at
        });
    let tickets = resolved_tickets(outcomes);
    let receipts_cited = record
        .decision_receipts
        .windows(2)
        .all(|pair| pair[0].ticket_id < pair[1].ticket_id)
        && record.decision_receipts.iter().all(|receipt| {
            let mut detached = receipt.clone();
            let recorded = std::mem::take(&mut detached.canonical_digest);
            tickets.contains(&receipt.ticket_id)
                && canonical_hash("canwu.production.decision-receipt.v1", &detached)
                    .is_ok_and(|digest| digest == recorded)
        });
    let (matches, floor) = match (&record.key, &record.body) {
        (
            ProductionTerminalArchiveKeyV1::WorkOrder(id),
            ProductionResidueBodyV1::WorkOrder { work_order },
        ) => (
            &work_order.id == id
                && work_order_ended(work_order.lifecycle)
                && work_order.execution.is_none()
                && outcomes
                    .iter()
                    .all(|outcome| outcome_matches_work_order(outcome, work_order)),
            work_order.created_at,
        ),
        (
            ProductionTerminalArchiveKeyV1::CompletionLease(id),
            ProductionResidueBodyV1::CompletionLease { lease },
        ) => {
            let acquisition = &lease.acquisition;
            (
                &acquisition.id == id
                    && lease_ended(acquisition.state)
                    && lease.production_grants.iter().all(|grant| {
                        grant.acquisition == *id && grant.owner_plugin == crate::PLUGIN_NAME
                    })
                    && lease
                        .certificate
                        .as_ref()
                        .is_none_or(|certificate| certificate.acquisition == *id)
                    && lease.participant_grants.iter().all(|participant| {
                        participant.grant.acquisition == *id && grant_ended(participant.grant.state)
                    })
                    && lease
                        .completion_receipts
                        .windows(2)
                        .all(|pair| pair[0].sequence < pair[1].sequence)
                    && lease.completion_receipts.iter().all(|receipt| {
                        let mut detached = receipt.clone();
                        let recorded = std::mem::take(&mut detached.semantic_digest);
                        receipt.acquisition == *id
                            && canwu_resource::canonical_digest(
                                "canwu.resource.completion-lease-receipt.v1",
                                &detached,
                            )
                            .is_ok_and(|digest| digest == recorded)
                    })
                    && outcomes
                        .iter()
                        .all(|outcome| outcome_matches_lease(outcome, id, &acquisition.holder)),
                acquisition.eligibility_time,
            )
        }
        (
            ProductionTerminalArchiveKeyV1::OperationOutcome(id),
            ProductionResidueBodyV1::OperationOutcome,
        ) => match outcomes.as_slice() {
            [outcome] => (&outcome.id == id, outcome.settled_at),
            _ => (false, SimTime::EPOCH),
        },
        _ => (false, SimTime::EPOCH),
    };
    if !matches
        || !canonical
        || !receipts_cited
        || record.terminal_at != residue_terminal_at(floor, outcomes)
    {
        return Err(invalid(
            "production residue archive record does not bind its exact closure",
        ));
    }
    Ok(())
}
