//! Reference host loop for continuously rendered, proportional-time games.
//!
//! The host owns wall time, game speed, the sub-minute accumulator, and all
//! presentation state. Canwu only receives representable simulation durations
//! and remains the authoritative source of world state and events.

use canwu_api::{
    ArmyId, Canwu, CanwuError, Command, CommandEnvelope, CommandRecord, CommandRequest,
    CommandRequestId, DemoIds, Issuer, SimDuration, SimEvent, SimTime, TerritoryId, TransitState,
    WorldSnapshot,
};
use std::{
    fmt::{Display, Formatter},
    time::Duration,
};

const SIMULATION_QUANTUM: SimDuration = SimDuration::minutes(1);
const SIMULATION_MINUTE_NANOS: u128 = 60_000_000_000;

// This game's host policy says that 1x advances the calendar by one simulation
// minute per wall second. Canwu does not prescribe this rate. After converting
// wall time into this base game-time unit, the proportional rule is simply:
//
//     simulation_elapsed = converted_wall_elapsed * game_speed
const BASE_SIM_SECONDS_PER_WALL_SECOND: u128 = 60;

const SIXTY_FPS_ISH: FrameProfile = FrameProfile {
    name: "60 FPS-ish",
    frame_millis: &[16, 17],
};
const THIRTY_FPS_ISH: FrameProfile = FrameProfile {
    name: "30 FPS-ish",
    frame_millis: &[33, 34],
};

// The scripted wall-time schedule is shared by both FPS runs. Input is captured
// halfway through the first simulation minute and quantized upward to minute 1.
// The 5x phase reaches about 25% visual progress with a half-minute remainder,
// pause preserves that remainder, and 20x carries the army through arrival.
const SPEED_SCHEDULE: [SpeedPhase; 5] = [
    SpeedPhase::new(GameSpeed::X1, 500),
    SpeedPhase::new(GameSpeed::X1, 500),
    SpeedPhase::new(GameSpeed::X5, 53_900),
    SpeedPhase::new(GameSpeed::Paused, 1_000),
    SpeedPhase::new(GameSpeed::X20, 40_575),
];

#[derive(Clone, Copy, Debug)]
enum GameSpeed {
    Paused,
    X1,
    X5,
    X20,
}

impl GameSpeed {
    const fn multiplier(self) -> u128 {
        match self {
            Self::Paused => 0,
            Self::X1 => 1,
            Self::X5 => 5,
            Self::X20 => 20,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Paused => "Paused",
            Self::X1 => "1x",
            Self::X5 => "5x",
            Self::X20 => "20x",
        }
    }
}

#[derive(Clone, Copy)]
struct SpeedPhase {
    speed: GameSpeed,
    wall_millis: u64,
}

impl SpeedPhase {
    const fn new(speed: GameSpeed, wall_millis: u64) -> Self {
        Self { speed, wall_millis }
    }
}

#[derive(Clone, Copy)]
struct FrameProfile {
    name: &'static str,
    frame_millis: &'static [u64],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PresentationTime {
    canonical: SimTime,
    sub_minute: Duration,
}

impl Display for PresentationTime {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} + {:.3}m",
            self.canonical,
            self.sub_minute.as_secs_f64() / 60.0,
        )
    }
}

/// Renderer-owned state. It is refreshed from detached public snapshots and is
/// never written back into Canwu.
struct PresentationState {
    authoritative_location: TerritoryId,
    last_authoritative_transit: Option<TransitState>,
    visual_progress: Option<f64>,
}

impl PresentationState {
    fn from_world(world: &WorldSnapshot, army: ArmyId) -> Self {
        let army = world.army(army).expect("the demo army must exist");
        Self {
            authoritative_location: army.location,
            last_authoritative_transit: army.transit.clone(),
            visual_progress: None,
        }
    }

    fn refresh_from_authority(&mut self, world: &WorldSnapshot, army: ArmyId) {
        let army = world.army(army).expect("the demo army must exist");
        self.authoritative_location = army.location;
        if let Some(transit) = &army.transit {
            self.last_authoritative_transit = Some(transit.clone());
        }
    }

    fn render(&mut self, presentation_time: PresentationTime) -> Option<f64> {
        self.visual_progress = self.last_authoritative_transit.as_ref().map(|transit| {
            if presentation_time.canonical < transit.departed_at {
                return 0.0;
            }
            let elapsed = std_duration(presentation_time.canonical - transit.departed_at)
                .checked_add(presentation_time.sub_minute)
                .expect("presentation elapsed time must remain representable");
            let travel = std_duration(transit.arrives_at - transit.departed_at);
            (elapsed.as_secs_f64() / travel.as_secs_f64()).clamp(0.0, 1.0)
        });
        self.visual_progress
    }

    fn describe(&self) -> String {
        match (&self.last_authoritative_transit, self.visual_progress) {
            (Some(transit), Some(progress)) => format!(
                "army visual progress={:.1}% ({} -> {}, authoritative_location={})",
                progress * 100.0,
                transit.from,
                transit.to,
                self.authoritative_location,
            ),
            _ => format!("army stationary at {}", self.authoritative_location),
        }
    }
}

/// The authoritative simulation clock and the render clock deliberately live
/// in different fields. Only whole simulation minutes cross the Canwu API.
struct GameHost {
    authority: Canwu,
    ids: DemoIds,
    presentation: PresentationState,
    accumulated_sim_nanos: u128,
    wall_elapsed: Duration,
    frame: u64,
    reported_events: usize,
    next_progress_milestone: u8,
    canonical_boundaries: usize,
    verbose: bool,
}

impl GameHost {
    fn new(verbose: bool) -> Result<Self, CanwuError> {
        let authority = Canwu::demo(35)?;
        let ids = Canwu::demo_ids();
        let presentation = PresentationState::from_world(&authority.world(), ids.army);
        Ok(Self {
            authority,
            ids,
            presentation,
            accumulated_sim_nanos: 0,
            wall_elapsed: Duration::ZERO,
            frame: 0,
            reported_events: 0,
            next_progress_milestone: 25,
            canonical_boundaries: 0,
            verbose,
        })
    }

    fn run_phase(&mut self, phase: SpeedPhase, profile: FrameProfile) -> Result<(), CanwuError> {
        let mut remaining_millis = phase.wall_millis;
        let mut pattern_index = 0;
        let mut first_frame = true;

        while remaining_millis > 0 {
            let frame_millis = profile.frame_millis[pattern_index % profile.frame_millis.len()]
                .min(remaining_millis);
            remaining_millis -= frame_millis;
            self.render_frame(
                Duration::from_millis(frame_millis),
                phase.speed,
                first_frame || remaining_millis == 0,
            )?;
            first_frame = false;
            pattern_index += 1;
        }
        Ok(())
    }

    fn render_frame(
        &mut self,
        wall_dt: Duration,
        speed: GameSpeed,
        phase_edge: bool,
    ) -> Result<(), CanwuError> {
        self.frame += 1;
        self.wall_elapsed += wall_dt;

        let converted_wall_nanos = wall_dt.as_nanos() * BASE_SIM_SECONDS_PER_WALL_SECOND;
        self.accumulated_sim_nanos += converted_wall_nanos * speed.multiplier();

        let mut authority_changed = false;
        while self.accumulated_sim_nanos >= SIMULATION_MINUTE_NANOS {
            let receipts = self.authority.advance_canonical(SIMULATION_QUANTUM)?;
            self.canonical_boundaries += receipts.len();
            authority_changed |= !receipts.is_empty();
            self.accumulated_sim_nanos -= SIMULATION_MINUTE_NANOS;
        }

        if authority_changed {
            self.refresh_presentation_from_authority();
        }

        let progress = self.presentation.render(self.presentation_time());
        let crossed_milestone = self.crossed_progress_milestone(progress);
        let has_new_events = self.reported_events < self.authority.events().len();

        if self.verbose && (phase_edge || crossed_milestone || has_new_events) {
            self.print_frame(speed);
        }
        self.report_new_events();
        Ok(())
    }

    /// This host never backdates mid-quantum input. It deterministically rounds
    /// up to the next representable Canwu minute; another host may choose a
    /// different policy, but it must apply the same policy across render FPS.
    fn submit_player_command(&mut self) -> Result<(), CanwuError> {
        let captured_at = self.presentation_time();
        let due_at = if self.accumulated_sim_nanos == 0 {
            self.authority.time()
        } else {
            self.authority
                .time()
                .checked_add(SIMULATION_QUANTUM)
                .expect("the scripted command time must remain representable")
        };
        let envelope = CommandEnvelope::new(
            Issuer::Actor(self.ids.commander),
            Command::MoveArmy {
                army: self.ids.army,
                destination: self.ids.eastern_territory,
            },
        )
        .at_time(due_at);
        let request = CommandRequest::new(
            CommandRequestId::new(1),
            self.authority.revision(),
            envelope,
        );
        let ingress = self.authority.enqueue_command(due_at, 0, request)?;

        if self.verbose {
            println!(
                "INPUT: queued commander move as {:?}, captured_at={captured_at}, quantized_due_at={due_at}",
                ingress.ingress_id,
            );
        }

        // This drains ingress due at the current canonical time only. The
        // mid-quantum command remains queued until normal accumulation reaches
        // its quantized minute; future scheduled work is never jumped to.
        let receipts = self.authority.advance_canonical(SimDuration::ZERO)?;
        self.canonical_boundaries += receipts.len();
        self.refresh_presentation_from_authority();
        self.presentation.render(self.presentation_time());
        if self.verbose {
            self.print_frame(GameSpeed::X1);
        }
        self.report_new_events();
        Ok(())
    }

    fn refresh_presentation_from_authority(&mut self) {
        self.presentation
            .refresh_from_authority(&self.authority.world(), self.ids.army);
    }

    fn presentation_time(&self) -> PresentationTime {
        PresentationTime {
            canonical: self.authority.time(),
            sub_minute: Duration::from_nanos(
                u64::try_from(self.accumulated_sim_nanos)
                    .expect("the sub-minute accumulator must fit a Duration"),
            ),
        }
    }

    fn crossed_progress_milestone(&mut self, progress: Option<f64>) -> bool {
        let Some(progress) = progress else {
            return false;
        };
        let mut crossed = false;
        while self.next_progress_milestone <= 100
            && progress * 100.0 + f64::EPSILON >= f64::from(self.next_progress_milestone)
        {
            crossed = true;
            self.next_progress_milestone += 25;
        }
        crossed
    }

    fn print_frame(&self, speed: GameSpeed) {
        println!(
            "frame={:>5} speed={:<6} wall={:>7.3}s visual_time={} canwu_time={} {}",
            self.frame,
            speed.label(),
            self.wall_elapsed.as_secs_f64(),
            self.presentation_time(),
            self.authority.time(),
            self.presentation.describe(),
        );
    }

    fn report_new_events(&mut self) {
        let events = self.authority.events();
        if self.verbose {
            for event in &events[self.reported_events..] {
                println!(
                    "EVENT: at={} type={} {}",
                    event.timestamp,
                    event.kind.event_type(),
                    event.summary,
                );
            }
        }
        self.reported_events = events.len();
    }

    fn outcome(self, profile: FrameProfile) -> Result<RunOutcome, CanwuError> {
        Ok(RunOutcome {
            profile: profile.name,
            frames: self.frame,
            time: self.authority.time(),
            world: self.authority.world(),
            events: self.authority.events().to_vec(),
            commands: self.authority.commands().to_vec(),
            authoritative_state_hash: self.authority.authoritative_state_hash()?,
            checkpoint_hash: self.authority.checkpoint_hash().to_owned(),
            canonical_boundaries: self.canonical_boundaries,
        })
    }
}

struct RunOutcome {
    profile: &'static str,
    frames: u64,
    time: SimTime,
    world: WorldSnapshot,
    events: Vec<SimEvent>,
    commands: Vec<CommandRecord>,
    authoritative_state_hash: String,
    checkpoint_hash: String,
    canonical_boundaries: usize,
}

fn run(profile: FrameProfile, verbose: bool) -> Result<RunOutcome, CanwuError> {
    let mut host = GameHost::new(verbose)?;
    if verbose {
        println!("\n{} render loop", profile.name);
    }

    host.run_phase(SPEED_SCHEDULE[0], profile)?;
    host.submit_player_command()?;
    host.run_phase(SPEED_SCHEDULE[1], profile)?;
    host.run_phase(SPEED_SCHEDULE[2], profile)?;

    let time_before_pause = host.authority.time();
    let remainder_before_pause = host.accumulated_sim_nanos;
    let presentation_before_pause = host.presentation_time();
    let frames_before_pause = host.frame;
    assert!(
        remainder_before_pause > 0,
        "the pause fixture must preserve a fractional simulation minute"
    );
    host.run_phase(SPEED_SCHEDULE[3], profile)?;
    assert!(
        host.frame > frames_before_pause,
        "pause must keep rendering"
    );
    assert_eq!(
        host.authority.time(),
        time_before_pause,
        "pause must not advance Canwu"
    );
    assert_eq!(
        host.accumulated_sim_nanos, remainder_before_pause,
        "pause must not add desired simulation time"
    );
    assert_eq!(
        host.presentation_time(),
        presentation_before_pause,
        "pause must freeze presentation time"
    );

    host.run_phase(SPEED_SCHEDULE[4], profile)?;
    assert_eq!(
        host.accumulated_sim_nanos, 0,
        "the scripted schedule should end on a simulation-minute boundary"
    );
    host.outcome(profile)
}

fn std_duration(duration: SimDuration) -> Duration {
    let minutes = u64::try_from(duration.as_minutes())
        .expect("presentation interpolation requires a non-negative duration");
    Duration::from_secs(
        minutes
            .checked_mul(60)
            .expect("presentation duration must remain representable"),
    )
}

fn assert_fps_independent(left: &RunOutcome, right: &RunOutcome) {
    assert_ne!(
        left.frames, right.frames,
        "the render segmentations must differ"
    );
    assert_eq!(left.time, right.time, "Canwu time changed with render FPS");
    assert_eq!(
        left.world, right.world,
        "world state changed with render FPS"
    );
    assert_eq!(left.events, right.events, "events changed with render FPS");
    assert_eq!(
        left.commands, right.commands,
        "command ordering changed with render FPS"
    );
    assert_eq!(
        left.authoritative_state_hash, right.authoritative_state_hash,
        "authoritative state hash changed with render FPS"
    );
    assert_eq!(
        left.checkpoint_hash, right.checkpoint_hash,
        "checkpoint hash changed with render FPS"
    );
    assert_eq!(
        left.canonical_boundaries, right.canonical_boundaries,
        "canonical boundary count changed with render FPS"
    );
}

fn assert_expected_outcome(outcome: &RunOutcome) {
    let ids = Canwu::demo_ids();
    let command_at = SimTime::from_minutes(1);
    let arrival_at = SimTime::from_minutes(1_081);
    let final_time = SimTime::from_minutes(1_082);

    assert_eq!(outcome.time, final_time, "the scripted final time changed");
    let army = outcome
        .world
        .army(ids.army)
        .expect("the demo army must remain present");
    assert_eq!(
        army.location, ids.eastern_territory,
        "the scheduled arrival must execute"
    );
    assert!(
        army.transit.is_none(),
        "the army must no longer be in authoritative transit"
    );

    assert_eq!(
        outcome.commands.len(),
        1,
        "the move command must be accepted"
    );
    let command = &outcome.commands[0];
    assert_eq!(command.accepted_at, command_at);
    assert_eq!(command.envelope.expected_time, Some(command_at));
    assert!(matches!(
        &command.envelope.command,
        Command::MoveArmy { army, destination }
            if *army == ids.army && *destination == ids.eastern_territory
    ));

    let event_timeline = outcome
        .events
        .iter()
        .map(|event| (event.kind.event_type(), event.timestamp))
        .collect::<Vec<_>>();
    assert_eq!(
        event_timeline,
        vec![
            ("move_ordered", command_at),
            ("army_arrived", arrival_at),
            ("knowledge_updated", arrival_at),
            ("report_dispatched", arrival_at),
        ],
        "the expected command and scheduled-arrival event timeline changed"
    );
    assert_eq!(
        outcome.canonical_boundaries, 2,
        "the command and arrival should settle at two canonical boundaries"
    );
}

fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

fn main() -> Result<(), CanwuError> {
    println!(
        "1x host policy: {BASE_SIM_SECONDS_PER_WALL_SECOND} simulation seconds per wall second"
    );
    println!("Frames are predefined; this example never sleeps or reads the real clock.");

    let sixty = run(SIXTY_FPS_ISH, true)?;
    let thirty = run(THIRTY_FPS_ISH, false)?;
    assert_expected_outcome(&sixty);
    assert_expected_outcome(&thirty);
    assert_fps_independent(&sixty, &thirty);

    println!(
        "\nFPS INVARIANT: {} frames={} and {} frames={} -> time={}, events={}, boundaries={}, state_hash={}..., checkpoint={}...",
        sixty.profile,
        sixty.frames,
        thirty.profile,
        thirty.frames,
        sixty.time,
        sixty.events.len(),
        sixty.canonical_boundaries,
        short_hash(&sixty.authoritative_state_hash),
        short_hash(&sixty.checkpoint_hash),
    );
    Ok(())
}
