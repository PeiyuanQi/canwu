# canwu-fiscal

`canwu-fiscal` is the generic fiscal-procedure extension for Canwu. It models
versioned fiscal rules, regional adoption, assessments, remissions, execution
authorizations, evidence-backed receipts, audits, reform candidates, strategic
aggregates, and holder-relative estimated reports published as knowledge.

It deliberately does not own money, grain, inventory, market quotes, transport,
or physical transfer truth. A resource or logistics adapter performs the real
operation, then submits a `FiscalExecutionReceiptPacket` citing exact external
domain-record versions through `enqueue_execution_receipt`. At the live
settlement boundary the extension requires each cited version to be the current
version of its record, decodes the generic execution-evidence envelope,
validates every field against the authorization, and derives the receipt
quantity and disposition from the evidence. If a cited record changes in a
boundary before the one that settles its receipt, the settling boundary fails,
and a receipt that is already due cannot be cancelled, so cite create-only
evidence kinds or cancel a pending receipt before its due time. A configured
evidence-kind allowlist and the host integration's redundant semantic validator
keep arbitrary world records from proving execution. Exact evidence versions and
`(evidence kind, external_operation_id)` pairs can each settle at most one
receipt within one `FiscalState`.

A `FiscalAuthorityBinding` names its standing `authorized_actor` and may also
name an `acting_actor` together with an exact `authority_basis` record version,
such as a host grant record whose kind is declared with
`FiscalPlugin::with_authority_basis_kinds` (activation rejects a kind the
plugin was not configured to read). Actions from the acting actor, directly or
as a decision-ticket origin, are admitted only while that basis is the current
version of its record, and settlement re-checks it against the state the
settling boundary began with. Once the basis record advances or
retires, the acting actor is rejected with `FISCAL_ACTING_BASIS_NOT_CURRENT`
while the authorized actor is still admitted.

`FiscalAction::RebindAuthority` replaces a binding's authorized actor, acting
actor, and authority basis in one commit, so a renewed grant or a new office
holder does not strand the binding. It is issued under the binding it replaces,
by the authorized actor or by the institution deciding as a body: a decision
controller whose decision origin is the binding's institution with no
responsible actor. An acting actor cannot rebind, directly or as the
responsible actor of an institution decision, and neither can any other
person. Every person a rebind names must exist. The new basis must be the
current version of its record when the command is admitted and again in the
state the settling boundary began with; a record that advances inside the
settling boundary is not seen, and the acting actor it names is rejected at
the next admission. An action of the previous actor that settles after the
rebind is rejected (`FISCAL_ISSUER_NOT_BOUND`, or
`FISCAL_ACTING_BASIS_NOT_CURRENT` for an acting actor), and the binding
records the action and its issuer in `rebound_by`.

The institution role is the power to appoint. A controller that decides for
an institution with no responsible actor can rebind a binding to any actors,
or to none, and on a binding that names no authorized actor it acts as
principal itself. Register a controller of that shape only for the body that
appoints the binding's actors.

A `FiscalAuditBinding` lets an auditor from outside an institution record
audit findings on it. It admits `RecordAudit` on targets whose scope the
audited institution owns, and nothing else, only while its own
`authority_basis` is current (`FISCAL_AUTHORITY_BASIS_NOT_CURRENT` otherwise).
Audit bindings live beside the authority bindings, not inside them, so no
check of who may act for an institution ever consults an auditor. They are
installed with the state or by `FiscalAction::RebindAudit`, which the audited
institution issues to install, replace, or remove one, under the rules of an
authority rebind. Every settled `FiscalAuditFinding` names who entered it,
under which binding and role, in `entered_by`. Audit binding IDs are one
namespace across institutions.

An audit binding establishes who entered a finding, not that the auditor is
independent of the institution he audits. The audited institution appoints
and removes its own auditors, and can remove one before a finding admitted
under his binding settles; the basis only has to be the current version of an
application record of a kind the plugin reads. Who appoints an auditor is an
open design decision, recorded in the Canwu repository's
`docs/proposals/fiscal-throughput-and-authority.md`.

The record versions that `rebound_by`, an audit binding, and a finding's
`entered_by` name are not declared as required evidence. After
`seal_evidence`, a superseded version among them may no longer be provable.

Historical content periods and host-defined accounting cycles remain separate.
Aggregates retain institution, mechanism, scope, accounting cycle, unit, and payment-form
dimensions. Assessment, remission, collection, remittance, disbursement,
reserve, and return remain distinct; only remission and collection reduce an
assessment's outstanding quantity.

Historical packs compile into an immutable catalog containing their full time
range. `FiscalHistoricalContextPacket` changes the current year or replay mode;
reforms remain explicit candidates rather than automatic global date switches.
`ApplyTransition` installs its target rules and suspends superseded rules in one
settled action. Procedure commands use a dedicated revision, so derived report
or candidate refreshes do not make a valid player decision stale.

Several actions settle in one boundary, in admission order. Each is compared
with the procedure revision the state had when the boundary began, so actions
admitted against the same revision settle together, each validated against the
state the previous one left; a rejected action is recorded as its own outcome
and does not affect the actions after it. A later action may build on what an
earlier one created, but it is rejected as stale if it would overwrite an
adoption or binding that an earlier action of the same boundary changed.
Actions that only create do not conflict: two remissions or two execution
authorizations on one assessment both apply when their sum validates. An
action settles in the boundary after the one that admits its command. A
receipt, a historical-context change, or another action that settles in the
admitting boundary makes it stale; a receipt that settles in the same boundary
as the action does not.

One boundary applies at most `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (64) actions
and rejects the rest with `FISCAL_BOUNDARY_ACTION_LIMIT`; a client reissues
those under new action IDs. A stale or unauthorized action is rejected without
using the limit. Reports are still published once per boundary, however many
actions settled in it.

No action fails a boundary for the others. Admission rejects as
`InvalidPayload` a request larger than `MAX_FISCAL_ACTION_JSON_BYTES`
(32 KiB), a `finding`, `reason`, or `purpose` longer than
`MAX_FISCAL_ACTION_TEXT_BYTES` (2,048 bytes), a `unit` or referenced ID longer
than `MAX_FISCAL_IDENTIFIER_BYTES` (160 bytes), and a new entry whose ID is
not canonical. An action whose result would not fit the state's byte budget,
less 16 KiB that is kept for the report plan, is rejected as its own outcome
with `FISCAL_STATE_BUDGET_EXHAUSTED`.

An action that settlement can give no outcome is dropped: it changes nothing,
its action ID is not consumed, and one `canwu.fiscal.action_dropped.v1` event
is its only record. The event's summary names the action, and its affected
entities name the institution the action was issued for, unless that is a
domain record. So an accepted command whose action has no outcome after the
next boundary was dropped. That happens in two cases. When no outcome slot is
left, an action admitted before the last slot was taken is dropped, and from
then on admission rejects every command. Near the byte budget admission keeps
accepting commands: settlement rejects an action that does not fit with
`FISCAL_STATE_BUDGET_EXHAUSTED` while the rejection itself still fits, and
drops the ones after that.

A boundary that admits no fiscal ingress costs no fiscal decode once any
boundary has written the state. Fiscal records are validated when the plugin
is activated, on a new scenario or a restored snapshot, and on every write;
the plugin's own handlers do not validate them again. Activation does not
recompute the derived fields of a state, and an idle boundary no longer does
either, so a state restored with altered transition candidates or aggregates
keeps them until its next fiscal write.

Fiscal collections are hard-capped at 4,096 assessments, 8,192 execution
requests, 8,192 receipts, 32 evidence versions per receipt, 16,384 action
outcomes, and a 32 MiB serialized state budget. These are lifetime caps: at
64 actions a boundary, the outcome cap is 256 boundaries away, rejected
actions use it too, and a full state rejects every further command at
admission; the assessment cap is 64 full boundaries of assessments away, after
which each assessment is rejected as its own outcome. Aggregate rebuilds use
single-pass indexes. A host that needs larger campaigns must create separate
simulation runs or shards; this crate has no in-place partition or archive
API, and the host owns cross-shard operation-ID deduplication.

An observer's report is one summary record and parts of 16 facts, one fact
for each aggregate the observer's binding lets it see. Read it with
`fiscal_reports(&viewer)`, which returns one `FiscalReport` for each observer
binding that names the viewer's actor, keyed by binding ID; its `projection`
is what `compute_projections` derives from the fiscal state. A report record
on its own (`FiscalReportRecord`, version 2 of the `fiscal_report` knowledge
schema) is one summary or one part, not a report.

- A boundary publishes only the parts whose facts changed, with the
  observer's summary. A boundary that changes nothing an observer can see
  publishes nothing to that observer, and `as_of` is the time the report last
  changed.
- Aggregates keep their IDs, and a new aggregate takes the next one, so new
  facts fill the last part and earlier parts keep their members. A state
  that comes with no aggregates, or with all of them numbered in key order,
  is numbered as 0.14.0 numbered it. One that comes with some of them,
  numbered from zero without a gap, keeps those IDs and has the missing ones
  appended; any other numbering is replaced by one in key order.
- Each record supersedes the record before it on its report line. The report
  system finds that record in the committed knowledge ledger
  (`SimulationView::knowledge_heads`), so a run that sealed its evidence
  supersedes exactly what its replay does.
- A report can be as large as the state allows: 4,096 facts in 256 parts. One
  boundary publishes at most `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024)
  report records over all observers. Parts that changed but did not fit are
  published by the next boundary that changes the fiscal state, whatever that
  change is. Until then the report's summary names them in `pending_parts`,
  `FiscalReport::is_complete` is false, and `FiscalReport::is_stale` tells a
  fact of a pending part, which may be older than `as_of` or missing, from a
  current one. Deferral shows an observer a little that its binding does
  not: a boundary's records are shared in the order of the observer
  bindings, so which parts are pending depends on the reports of the
  observers before it, and the arrival of pending parts says that the fiscal
  state changed, whoever changed it.
- The engine publishes no knowledge to a dead person. While an observer's
  actor is dead, nothing is published to that observer and the report stays
  as it was; what changes in it is kept in the report plan, by part, and is
  published by the first fiscal change after the person is alive again.
- A report plan (`FiscalState::report_plan`) means something only in a state
  a boundary has written. One that comes with a scenario is ignored, and the
  first derivation publishes every report whole.

A scenario whose knowledge carries a report record of schema version 1 is
refused when the run is created, because that version is no longer
registered. A state is refused at activation when it could not be reported:

- when an observer binding names a person who does not exist. The state
  record references the person of every observer binding, and the engine
  checks references against its entity registry, so encode a state with
  `FiscalState::into_record`;
- when an assessment's unit or accounting cycle, or the namespace, name, and
  ID of a scope's domain-record institution together, is longer than
  `MAX_FISCAL_IDENTIFIER_BYTES` (160), which bounds a fact so that a part
  always fits one knowledge record.

Observer bindings cannot change in a run, and report lines are numbered by a
binding's place among them: an action that changed observer bindings would
have to keep every binding's place or publish every report whole again.

Two costs grow with what a run has published, and both are the engine's.
Finding the record a part supersedes reads the holder's whole ledger, twice
for each record published. And the engine carries all held knowledge,
superseded records included, through every boundary, so every publication
slows every later boundary. `docs/proposals/fiscal-throughput-and-authority.md`
gives the measured rate.
