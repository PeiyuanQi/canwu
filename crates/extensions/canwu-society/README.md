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
payload-to-core-reference binding and persisted society derivations, and
re-derives every queued rebase or lifecycle delta from the ingress journal
before returning the simulation. Hosts that restore society beside other
plugins call `validate_society_runtime` after
`Canwu::from_snapshot_json_with_plugins`.

A `PolicyPressure` may record its provenance: an optional `issuer` (a
government, organization, or person bound into the record's core references)
and a `decision_version`; a non-zero version requires an issuer. Both fields
are omitted from the encoding when unused.

Two canonical ingress types reach the society owner through one queue. The
public `cohort_headcount_rebase_v1` packet (`CohortHeadcountRebaseV1`) rebases
a cohort to an external conserved stock: the cited stock version must be the
current version of a record outside `canwu.society` when the packet is
admitted, or the rebase is rejected as `stale_external_stock`. The internal
`society_lifecycle_delta_v1` packet (`SocietyLifecycleDeltaV1`) carries a
lifecycle provider's target-scoped rule, alignment, and release changes, for
example from `canwu-culture`; deltas apply idempotently. An event-driven
phase-12 intake queues admitted packets in admission order, at most
`MAX_SOCIETY_INGRESS_QUEUE` at once (further packets are rejected with a
`society_ingress_rejected_v1` event); only the intake writes the queue, so an
initial scenario that seeds a non-empty queue is refused with
`InvalidAuthority`. The next Daily phase-7 settlement applies the queue after
cohort transfers and before institutional decisions and transitions, and
records each outcome in `SocietyCohortExchangeLedger::rebases` or
`lifecycle_deltas`. A rebase re-proportions every distribution of the cohort
with integer largest-remainder allocation, so each distribution totals the
new headcount and every bucket stays within one unit of its exact share. A
lifecycle delta whose release is blocked (a released alignment carries a
stored institutional decision, which is permanent, or another live dependency
appeared) still deactivates the target's rules and records the target in
`blocked_releases`.

A cohort transfer binds its source digest to the two cohorts it moves, so
ordinary disposition transitions do not invalidate a deferred transfer; it
moves the proportional share current when it settles. A due transfer whose
cohorts changed after intake (for example through a rebase or another
transfer), or that no longer fits, settles as a terminal ledger outcome
(`stale_source` or `rejected`) instead of failing later boundaries; a retry
needs a new operation ID. Transfers and rebases clear the materialized
aggregates, mobilization candidates, and projections, which the same Daily
boundary rematerializes. The society plugin remains the only writer of
`canwu.society:state`.
