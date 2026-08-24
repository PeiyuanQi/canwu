use super::{BoundaryRecord, CanwuError, invalid_snapshot, invalid_snapshot_error};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PersistedAdmissionCursors {
    pub(super) attempts: u64,
    pub(super) commands: u64,
    pub(super) events: u64,
}

pub(super) fn boundaries_before_attempts(
    attempt_count: usize,
    boundaries: &[BoundaryRecord],
) -> Result<Vec<u64>, CanwuError> {
    let boundary_count = u64::try_from(boundaries.len())
        .map_err(|_| invalid_snapshot_error("boundary count exceeds revision space"))?;
    let mut values = vec![boundary_count; attempt_count];
    for (boundary_index, boundary) in boundaries.iter().enumerate() {
        let prior_boundaries = u64::try_from(boundary_index)
            .map_err(|_| invalid_snapshot_error("boundary index exceeds revision space"))?;
        for attempt_id in &boundary.admitted_attempts {
            let attempt_index =
                usize::try_from(attempt_id.get().saturating_sub(1)).map_err(|_| {
                    invalid_snapshot_error("boundary attempt ID exceeds the journal index range")
                })?;
            let Some(value) = values.get_mut(attempt_index) else {
                return invalid_snapshot("boundary admits an unknown command attempt");
            };
            *value = prior_boundaries;
        }
    }
    Ok(values)
}

pub(super) fn authoritative_revision_count(
    command_count: usize,
    attempt_count: usize,
    boundary_count: usize,
) -> Result<u64, CanwuError> {
    let command_transactions = if attempt_count == 0 {
        command_count
    } else {
        attempt_count
    };
    u64::try_from(command_transactions)
        .ok()
        .and_then(|commands| {
            u64::try_from(boundary_count)
                .ok()
                .and_then(|boundaries| commands.checked_add(boundaries))
        })
        .ok_or_else(|| invalid_snapshot_error("authoritative revision space is exhausted"))
}
