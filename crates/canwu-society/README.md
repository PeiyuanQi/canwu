# canwu-society

`canwu-society` is an experimental, unpublished Canwu extension for aggregate
population dispositions, social influence, organization topology,
institutional alignment, policy pressure, and actor-relative estimates.

It intentionally contains no religion, doctrine, ritual, historical era,
rebellion, or war types. Applications provide those meanings through data and
downstream rules.

The API may change before the crate has an independent consumer.

Use `from_society_snapshot_json` for snapshot rehydration. It performs the
engine's normal snapshot checks, then recomputes the root record's
payload-to-core-reference binding and persisted society derivations before
returning the simulation.
