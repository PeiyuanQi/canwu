//! Opt-in wall-time profile of settled boundaries.
//!
//! A host application can time a whole `settle_boundary` call, but not the
//! share of each plugin in it. When the host turns profiling on, the runtime
//! times every plugin handler a boundary runs and keeps one
//! [`BoundaryProfile`] per settled boundary for the host to take.
//!
//! A boundary runs three kinds of plugin handler, and all three are timed:
//! boundary systems in their phases, plugin command handlers for the queued
//! commands the boundary admits, and synchronous event-driven systems that
//! react to the events those commands and the scheduled actions due at the
//! boundary emit. Owner-authorized maintenance participants are not part of
//! any profile: they run when the host schedules the maintenance, outside
//! settlement, and the boundary only applies the commit they produced.
//!
//! Wall time never enters the simulation. The profiler lives beside the
//! runtime state, like the plugin archive provider: it is not part of
//! authoritative state, the commitment, evidence, a snapshot, a checkpoint, or
//! a replay journal, no handler can read it, and settlement makes no decision
//! from it. A profiled run and an unprofiled run of the same input therefore
//! produce the same receipts, evidence, and hashes. The profile is
//! deliberately not part of [`crate::BoundaryReceipt`], which stays a
//! deterministic value that equal runs can compare.

use super::{BoundaryId, BoundaryPhase, BoundaryReceipt, SimTime, Simulation};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many settled boundaries the runtime keeps profiles for until the host
/// takes them. When more boundaries settle, the oldest profiles are dropped
/// and counted in [`BoundaryProfileBatch::dropped`].
pub const MAX_RETAINED_BOUNDARY_PROFILES: usize = 1_024;

/// The kind of plugin handler a [`BoundaryHandlerTiming`] times.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryHandlerKind {
    /// A boundary system, run in its settlement phase.
    BoundarySystem,
    /// A plugin command handler, run while the boundary admits a queued
    /// command.
    CommandHandler,
    /// A synchronous event-driven plugin system, run when a command or a
    /// scheduled action the boundary admits emits an event.
    EventSystem,
}

/// The wall time one plugin handler call took in one boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryHandlerTiming {
    /// What kind of handler ran.
    pub kind: BoundaryHandlerKind,
    /// The settlement phase a boundary system ran in. `None` for a command
    /// handler or an event system, which run while the boundary admits its
    /// input, before the first phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<BoundaryPhase>,
    /// The plugin that registered the handler.
    pub plugin: String,
    /// The name of the boundary system, command, or event system within its
    /// plugin.
    pub name: String,
    /// Wall time inside the handler, in microseconds. The work the simulation
    /// core does for the handler, such as validating and applying what it
    /// returned, is not included.
    pub elapsed_micros: u64,
}

/// The wall-time profile of one settled boundary.
///
/// A profile is a host diagnostic. It differs between two runs of the same
/// input and must not be fed back into the simulation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryProfile {
    /// The settled boundary, as in its [`BoundaryReceipt`].
    pub boundary_id: BoundaryId,
    /// The boundary's simulation time.
    pub settled_at: SimTime,
    /// Wall time of the whole settlement, from admission to commit, in
    /// microseconds.
    pub total_micros: u64,
    /// One entry for each plugin handler call the boundary made, in the
    /// order the calls were made: command handlers and event systems while
    /// the boundary admits its input, then the boundary systems that were
    /// due, by phase, plugin name, and system name.
    pub handlers: Vec<BoundaryHandlerTiming>,
}

impl BoundaryProfile {
    /// Wall time inside every handler of `plugin`, of any kind, in
    /// microseconds.
    #[must_use]
    pub fn plugin_micros(&self, plugin: &str) -> u64 {
        self.handlers
            .iter()
            .filter(|timing| timing.plugin == plugin)
            .fold(0, |total, timing| {
                total.saturating_add(timing.elapsed_micros)
            })
    }

    /// Wall time outside every plugin handler, in microseconds: the
    /// simulation core's own admission, validation, commit, and hashing
    /// work.
    #[must_use]
    pub fn remaining_micros(&self) -> u64 {
        self.handlers
            .iter()
            .fold(self.total_micros, |rest, timing| {
                rest.saturating_sub(timing.elapsed_micros)
            })
    }
}

/// The profiles [`Simulation::take_boundary_profiles`] hands over.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryProfileBatch {
    /// The profiles of the boundaries settled since the last take, in
    /// settlement order.
    pub profiles: Vec<BoundaryProfile>,
    /// How many older profiles were dropped since the last take because more
    /// than [`MAX_RETAINED_BOUNDARY_PROFILES`] boundaries settled before the
    /// host took them.
    pub dropped: u64,
}

/// The host's profiling switch, the boundary being timed, and the profiles
/// the host has not taken yet.
#[derive(Default)]
pub(super) struct BoundaryProfiler {
    enabled: bool,
    /// The timings of the boundary being settled; `None` between boundaries
    /// and while profiling is off.
    recorder: Option<BoundaryProfileRecorder>,
    profiles: VecDeque<BoundaryProfile>,
    dropped: u64,
}

impl BoundaryProfiler {
    /// Starts timing a boundary when profiling is on.
    pub(super) fn begin(&mut self) {
        self.recorder = self.enabled.then(|| BoundaryProfileRecorder {
            started: Instant::now(),
            handlers: Vec::new(),
        });
    }

    /// Ends the boundary being timed: keeps its profile when it settled, and
    /// discards its timings when it failed.
    pub(super) fn end(&mut self, receipt: Option<&BoundaryReceipt>) {
        let (Some(recorder), Some(receipt)) = (self.recorder.take(), receipt) else {
            return;
        };
        if self.profiles.len() == MAX_RETAINED_BOUNDARY_PROFILES {
            self.profiles.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.profiles.push_back(BoundaryProfile {
            boundary_id: receipt.boundary_id,
            settled_at: receipt.settled_at,
            total_micros: micros(recorder.started.elapsed()),
            handlers: recorder.handlers,
        });
    }

    /// The clock reading before a handler runs, or `None` when no boundary
    /// is being timed, in which case no clock is read.
    pub(super) fn handler_started(&self) -> Option<Instant> {
        handler_started(self.recorder.as_ref())
    }

    /// Records a handler call that began at `started` and just returned.
    pub(super) fn record_handler(
        &mut self,
        started: Option<Instant>,
        kind: BoundaryHandlerKind,
        phase: Option<BoundaryPhase>,
        plugin: &str,
        name: &str,
    ) {
        record_handler(self.recorder.as_mut(), started, kind, phase, plugin, name);
    }

    /// Lends the recorder to a step that borrows the runtime immutably; the
    /// caller returns it with [`Self::return_recorder`].
    pub(super) fn lend_recorder(&mut self) -> Option<BoundaryProfileRecorder> {
        self.recorder.take()
    }

    pub(super) fn return_recorder(&mut self, recorder: Option<BoundaryProfileRecorder>) {
        self.recorder = recorder;
    }
}

/// The timings of the boundary being settled.
pub(super) struct BoundaryProfileRecorder {
    started: Instant,
    handlers: Vec<BoundaryHandlerTiming>,
}

/// The clock reading before a handler runs, when a boundary is being timed.
pub(super) fn handler_started(recorder: Option<&BoundaryProfileRecorder>) -> Option<Instant> {
    recorder.map(|_| Instant::now())
}

/// Records a handler call that began at `started` and just returned.
pub(super) fn record_handler(
    recorder: Option<&mut BoundaryProfileRecorder>,
    started: Option<Instant>,
    kind: BoundaryHandlerKind,
    phase: Option<BoundaryPhase>,
    plugin: &str,
    name: &str,
) {
    if let (Some(recorder), Some(started)) = (recorder, started) {
        recorder.handlers.push(BoundaryHandlerTiming {
            kind,
            phase,
            plugin: plugin.to_owned(),
            name: name.to_owned(),
            elapsed_micros: micros(started.elapsed()),
        });
    }
}

fn micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

impl Simulation {
    /// Turns the wall-time profile of settled boundaries on or off. It is off
    /// in a new, restored, replayed, or forked runtime.
    ///
    /// While it is on, every boundary that settles leaves one
    /// [`BoundaryProfile`] for [`Self::take_boundary_profiles`], and the
    /// runtime keeps at most [`MAX_RETAINED_BOUNDARY_PROFILES`] of them. A
    /// boundary that fails leaves none. Turning it off keeps the profiles
    /// that were not taken yet.
    ///
    /// An exact replay settles its boundaries inside the replay call, on a
    /// runtime that starts with profiling off, so a replay is not profiled;
    /// the runtime the replay returns can be profiled from then on.
    ///
    /// Profiling reads the wall clock around each plugin handler and changes
    /// nothing else: receipts, evidence, hashes, snapshots, and exact replay
    /// are the same with it on or off. While it is off, settlement reads no
    /// clock.
    pub fn set_boundary_profiling(&mut self, enabled: bool) {
        self.boundary_profiler.enabled = enabled;
    }

    /// Whether settled boundaries are being profiled.
    #[must_use]
    pub const fn boundary_profiling(&self) -> bool {
        self.boundary_profiler.enabled
    }

    /// Takes the profiles of the boundaries settled since the last call, in
    /// settlement order, with the number of older profiles that were dropped
    /// at the cap since then. Both are empty and zero when profiling was off.
    pub fn take_boundary_profiles(&mut self) -> BoundaryProfileBatch {
        BoundaryProfileBatch {
            profiles: self.boundary_profiler.profiles.drain(..).collect(),
            dropped: std::mem::take(&mut self.boundary_profiler.dropped),
        }
    }

    /// Puts back the runtime a failed maintenance step captured with
    /// [`Self::fork`]. Host diagnostics, which the step cannot have changed
    /// and a fork does not carry, stay as they are.
    pub(super) fn restore_from_fork(&mut self, before: Self) {
        let profiler = std::mem::take(&mut self.boundary_profiler);
        *self = before;
        self.boundary_profiler = profiler;
    }
}
