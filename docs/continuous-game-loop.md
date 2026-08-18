# Continuous-Time Game Loop Integration

For real-time or continuously running strategy games, keep rendering and wall
time outside Canwu. Accumulate scaled wall time in the host, advance Canwu
through deterministic simulation time, and interpolate presentation state
separately. This is a recommended reference integration, not the only valid
host architecture.

Run the public-only example with:

```text
cargo run -p canwu-api --example continuous_game_loop
```

The example models two hosts with the same wall-time speed schedule and player
command but different frame segmentation: one is approximately 60 FPS and the
other approximately 30 FPS. Frame durations are predefined, so the example is
fast, deterministic, suitable for CI, and does not call `sleep` or read a real
clock.

## Three clocks

- **Wall clock:** frame durations supplied by the application or game
  framework.
- **Simulation clock:** authoritative integer-minute time owned by Canwu.
- **Presentation clock:** Canwu time plus the host's sub-minute accumulator,
  used only for interpolation.

The host chooses a base calendar rate and applies the proportional speed
multiplier:

```text
converted_wall_elapsed = wall_elapsed * base_calendar_rate
simulation_elapsed = converted_wall_elapsed * game_speed
```

The reference host stores that result as integer nanoseconds. It calls
`advance_canonical(SimDuration::minutes(1))` only when at least one whole Canwu
minute has accumulated and retains the fractional remainder for later frames.
No sub-minute wall time is discarded, and Canwu is not asked to perform a full
simulation step for every render frame.

`advance_canonical` is important here: it settles earlier scheduled work and
canonical ingress before reaching the requested target, so high game speeds do
not skip arrivals or reorder commands. The scripted player order enters through
`enqueue_command`. Because Canwu time is currently integer minutes, the host
must define how input captured between minutes maps to a canonical timestamp.
The example uses a deterministic ceiling policy: it never backdates input and
queues a half-minute input for the next representable minute. A zero-duration
canonical advance admits only work already due at the current timestamp; future
ingress remains queued until normal accumulation reaches it.

The input capture timestamp must itself come from a frame-independent input or
wall-time timeline. If a host defines the timestamp only as "the render frame
that happened to poll the input," different FPS can change the input time before
Canwu ever sees the command.

## Presentation interpolation

The authoritative public `WorldSnapshot` exposes army transit as `from`, `to`,
`departed_at`, and `arrives_at`. The renderer copies that detached information
into presentation state and computes:

```text
visual_progress =
    (presentation_time - departed_at) / (arrives_at - departed_at)
```

This floating-point value never feeds back into Canwu. The example enters pause
with a non-zero half-minute remainder. Render frames continue, but no scaled
simulation time is accumulated, so Canwu time, the preserved remainder, and
presentation progress all remain fixed.

At the end, the example compares Canwu time, world state, commands, events,
authoritative state hash, checkpoint hash, and canonical boundary count across
the two FPS profiles. It also checks the expected accepted command, arrival
time, event sequence, final territory, and cleared authoritative transit. The
render-frame counts differ; the authoritative result does not.
