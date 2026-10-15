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
//! The rest of a boundary's time is the simulation core's own work, which the
//! profile splits into [`BoundaryEnginePhase`]s. The profiler keeps a current
//! engine phase; settlement switches it where its work changes kind, and a
//! handler call pauses it at the clock reading the handler's time starts
//! from. The handlers and the engine phases therefore partition each
//! boundary's wall time.
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

/// A part of the simulation core's own work in a boundary, outside every
/// plugin handler. Each instant of a profiled settlement belongs either to a
/// handler call or to exactly one engine phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BoundaryEnginePhase {
    /// Checking the request, capturing the rollback point, and admitting the
    /// boundary's input: due ingress, queued commands, decisions,
    /// maintenance, and the scheduled actions due at the boundary, with the
    /// effects of command handlers and event systems (the handler calls
    /// themselves are handler time).
    Admission,
    /// Preparing each due boundary system's view and context, and checking,
    /// staging, and recording what it proposed.
    SystemDispatch,
    /// The work between settlement phases: record and component overlays,
    /// reservation allocation, and transition audits.
    Staging,
    /// Applying staged directives to the state: record mutations,
    /// components, events, scheduled ingress, and persons.
    Commit,
    /// Staging and committing knowledge publications.
    Knowledge,
    /// Committing random draws, random decisions, and transition manifests,
    /// and appending the boundary record and its evidence.
    Journal,
    /// Computing the state hash, the boundary hash, and the checkpoint hash.
    Hashing,
}

impl BoundaryEnginePhase {
    /// Every engine phase, in the order [`BoundaryProfile::engine`] lists
    /// them. A slice, so that a later release can add a phase without
    /// changing the constant's type.
    pub const ALL: &'static [Self] = &[
        Self::Admission,
        Self::SystemDispatch,
        Self::Staging,
        Self::Commit,
        Self::Knowledge,
        Self::Journal,
        Self::Hashing,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Admission => 0,
            Self::SystemDispatch => 1,
            Self::Staging => 2,
            Self::Commit => 3,
            Self::Knowledge => 4,
            Self::Journal => 5,
            Self::Hashing => 6,
        }
    }
}

/// The wall time the simulation core spent in one engine phase of one
/// boundary, summed over every stretch of that phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BoundaryEngineTiming {
    /// The engine phase.
    pub phase: BoundaryEnginePhase,
    /// Wall time in the phase, in microseconds, handler calls excluded.
    pub elapsed_micros: u64,
}

/// The wall-time profile of one settled boundary.
///
/// A profile is a host diagnostic. It differs between two runs of the same
/// input and must not be fed back into the simulation.
///
/// The handler calls in [`Self::handlers`] and the engine phases in
/// [`Self::engine`] partition the settlement: their exact durations add up
/// to the exact duration of [`Self::total_micros`]. Each value is rounded
/// down to whole microseconds, so the sum of the listed values is at most
/// `total_micros` and falls short of it by less than one microsecond per
/// listed value.
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
    /// The simulation core's own time, one entry per
    /// [`BoundaryEnginePhase`] in the order of [`BoundaryEnginePhase::ALL`],
    /// a phase that did no work included.
    pub engine: Vec<BoundaryEngineTiming>,
}

impl BoundaryProfile {
    /// Wall time the simulation core spent in `phase`, in microseconds.
    #[must_use]
    pub fn engine_micros(&self, phase: BoundaryEnginePhase) -> u64 {
        self.engine
            .iter()
            .filter(|timing| timing.phase == phase)
            .fold(0, |total, timing| {
                total.saturating_add(timing.elapsed_micros)
            })
    }

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
    /// work, which [`Self::engine`] splits by phase.
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
    /// Starts timing a boundary when profiling is on. The boundary starts in
    /// [`BoundaryEnginePhase::Admission`].
    pub(super) fn begin(&mut self) {
        self.recorder = self.enabled.then(|| {
            let started = Instant::now();
            BoundaryProfileRecorder {
                started,
                handlers: Vec::new(),
                phase: BoundaryEnginePhase::Admission,
                phase_started: started,
                engine: [Duration::ZERO; BoundaryEnginePhase::ALL.len()],
            }
        });
    }

    /// Ends the boundary being timed: keeps its profile when it settled, and
    /// discards its timings when it failed.
    pub(super) fn end(&mut self, receipt: Option<&BoundaryReceipt>) {
        let (Some(mut recorder), Some(receipt)) = (self.recorder.take(), receipt) else {
            return;
        };
        // The last engine phase ends where the settlement ends, at the same
        // clock reading, so the phases and the handlers partition the total.
        let ended = Instant::now();
        recorder.close_phase(ended);
        if self.profiles.len() == MAX_RETAINED_BOUNDARY_PROFILES {
            self.profiles.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.profiles.push_back(BoundaryProfile {
            boundary_id: receipt.boundary_id,
            settled_at: receipt.settled_at,
            total_micros: micros(ended.duration_since(recorder.started)),
            handlers: recorder.handlers,
            engine: BoundaryEnginePhase::ALL
                .iter()
                .map(|&phase| BoundaryEngineTiming {
                    phase,
                    elapsed_micros: micros(recorder.engine[phase.index()]),
                })
                .collect(),
        });
    }

    /// Moves the boundary being timed into the engine phase `phase` and
    /// returns the phase it was in, so a step that runs from several phases
    /// can put it back. Does nothing, and reads no clock, when no boundary
    /// is being timed.
    pub(super) fn enter(&mut self, phase: BoundaryEnginePhase) -> BoundaryEnginePhase {
        enter(self.recorder.as_mut(), phase)
    }

    /// The clock reading before a handler runs, or `None` when no boundary
    /// is being timed, in which case no clock is read. The current engine
    /// phase pauses at that reading until [`Self::record_handler`].
    pub(super) fn handler_started(&mut self) -> Option<Instant> {
        handler_started(self.recorder.as_mut())
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
    /// The engine phase the settlement is in, and since when; a handler call
    /// pauses it.
    phase: BoundaryEnginePhase,
    phase_started: Instant,
    /// Time spent in each engine phase so far, by [`BoundaryEnginePhase`]
    /// index.
    engine: [Duration; BoundaryEnginePhase::ALL.len()],
}

impl BoundaryProfileRecorder {
    /// Adds the time since the current phase (re)started to that phase.
    fn close_phase(&mut self, at: Instant) {
        let spent = at.saturating_duration_since(self.phase_started);
        let total = &mut self.engine[self.phase.index()];
        *total = total.saturating_add(spent);
        self.phase_started = at;
    }
}

/// Moves a timed boundary into the engine phase `phase` and returns the
/// phase it was in. Without a recorder it returns `phase` and reads no
/// clock.
pub(super) fn enter(
    recorder: Option<&mut BoundaryProfileRecorder>,
    phase: BoundaryEnginePhase,
) -> BoundaryEnginePhase {
    let Some(recorder) = recorder else {
        return phase;
    };
    let previous = recorder.phase;
    if previous != phase {
        recorder.close_phase(Instant::now());
        recorder.phase = phase;
    }
    previous
}

/// The clock reading before a handler runs, when a boundary is being timed.
/// The current engine phase pauses at that reading.
pub(super) fn handler_started(recorder: Option<&mut BoundaryProfileRecorder>) -> Option<Instant> {
    recorder.map(|recorder| {
        let started = Instant::now();
        recorder.close_phase(started);
        started
    })
}

/// Records a handler call that began at `started` and just returned, and
/// resumes the current engine phase at the same clock reading.
pub(super) fn record_handler(
    recorder: Option<&mut BoundaryProfileRecorder>,
    started: Option<Instant>,
    kind: BoundaryHandlerKind,
    phase: Option<BoundaryPhase>,
    plugin: &str,
    name: &str,
) {
    if let (Some(recorder), Some(started)) = (recorder, started) {
        let returned = Instant::now();
        recorder.handlers.push(BoundaryHandlerTiming {
            kind,
            phase,
            plugin: plugin.to_owned(),
            name: name.to_owned(),
            elapsed_micros: micros(returned.saturating_duration_since(started)),
        });
        recorder.phase_started = returned;
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
    /// runtime that starts with profiling off, so an ordinary replay is not
    /// profiled; the runtime the replay returns can be profiled from then on.
    /// [`Self::replay_from_journal_with_boundary_profiles`] profiles a replay
    /// from its first boundary.
    ///
    /// Profiling reads the wall clock around each plugin handler and at each
    /// engine-phase switch and changes nothing else: receipts, evidence,
    /// hashes, snapshots, and exact replay are the same with it on or off.
    /// While it is off, settlement reads no clock.
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
