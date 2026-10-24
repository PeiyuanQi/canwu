# canwu-military-reference

Runnable reference composition of the Canwu engagement engine
(`canwu-military`) on the reference world, through `canwu-api` only.
`demo_engagement_scenario()` installs the synthetic rulebook of
`canwu-military-reference-content` and six forces mirrored onto the
reference world's places; the values are synthetic demonstrations, not
historical claims.

The starter marches the reference army with the reference world's movement
plugin and places its force on arrival, fights a field engagement stepped by
the daily wake cadence and promoted to half-hour steps, invests, breaches,
and storms a place, sends a river crossing against an interceptor, receives
each closed fight's outcome signal in a tiny host plugin, and prints the
summaries and a government's reports:

```text
cargo run -p canwu-military-reference --example military_starter
```
