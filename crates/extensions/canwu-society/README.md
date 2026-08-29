# canwu-society

`canwu-society` is Canwu's published **social diffusion
simulation module**. Architecturally, it is a **domain extension** built on the
public engine contracts rather than a kernel subsystem. It owns aggregate
population dispositions, social influence, organization topology,
institutional alignment, policy pressure, and actor-relative estimates while
reusing Canwu's settlement, event, decision, knowledge, persistence, and replay
infrastructure.

It intentionally contains no religion, doctrine, ritual, historical era,
rebellion, or war types. Applications provide those meanings through data and
downstream rules.

The crate is an official optional release. Its API follows Canwu's pre-1.0
compatibility policy and may evolve in a future SemVer release.

Use `from_society_snapshot_json` for snapshot rehydration. It performs the
engine's normal snapshot checks, then recomputes the root record's
payload-to-core-reference binding and persisted society derivations before
returning the simulation.
