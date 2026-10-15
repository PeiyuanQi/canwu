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
quantity and disposition from the evidence. No receipt or historical-context
change fails its boundary: one whose payload does not decode, whose evidence
is no longer current or does not match its request, that cites an external
operation another receipt settled, that reuses a receipt ID with other
content, or a context year outside the pack, changes nothing and is refused
with one `canwu.fiscal.ingress_rejected.v1` event, whose summary names the
ingress and the packet and ends with `FISCAL_INGRESS_MALFORMED_REJECTION` or,
after the message that says why, `FISCAL_INGRESS_INVALID_REJECTION`. A
refused receipt consumes nothing, so a corrected one can settle later; if a
cited record changes before the receipt settles, cite its new version. A configured
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
check of who may act for an institution ever consults an auditor. Every
settled `FiscalAuditFinding` names who entered it, under which binding and
role, in `entered_by`. Audit binding IDs are one namespace across
institutions.

Who appoints and removes an institution's auditors is the institution's
`FiscalAppointmentPolicy`, declared with the state in
`FiscalState::audit_appointment_policies`, at most one per audited
institution:

- **Appointer.** The institution whose decision appoints and removes them. It
  decides with `FiscalAction::DecideAuditAppointment` under one of its own
  authority bindings, as that binding's principal or as a body; an acting
  actor never decides. A decision appoints the candidate of a pending
  nomination (`AcceptNomination`), appoints directly (`Appoint`), removes the
  auditor a pending petition names (`AcceptPetition`), removes directly
  (`Remove`), or dismisses a pending nomination or petition (`Dismiss`).
- **Nominators and petitioners.** Institutions, acting under one of their
  authority bindings, or persons the policy lists by name, acting under the
  policy's own ID (`FiscalIssuerRole::ListedActor`). A nominator proposes a
  candidate and an authority basis with `NominateAuditor`; a petitioner asks
  for the removal of a binding's auditor on evidence grounds with
  `PetitionAuditorRemoval`.
- **Unprompted decisions.** `appoints_without_nomination` and
  `removes_without_petition` say whether the appointer may act without one
  (`FISCAL_NOMINATION_REQUIRED`, `FISCAL_PETITION_REQUIRED` otherwise).
- **Tenure.** Replacing a seated auditor by another is a removal as well as
  an appointment, so it needs `removes_without_petition`; where the policy
  protects him, the appointer accepts a petition against him first and then
  fills the vacancy. Appointing into a vacancy, or the seated auditor again,
  removes nobody.

Each nomination and petition is a `FiscalAuditProposal` in
`FiscalState::audit_proposals`, recorded with its party and the action and
issuer that made it, and pending until it is resolved once: accepted or
dismissed by the appointer, or superseded. A party's new proposal of the same
kind on the same binding replaces its pending one, which is dropped; an
appointment supersedes every other nomination pending on the binding, and the
petitions only when it replaces the auditor they name; a removal supersedes
every other petition (nominations stay pending for the vacancy). Only the
audited institution's proposals are touched. Each party has room for
`MAX_FISCAL_PENDING_PROPOSALS_PER_PARTY` (4) pending proposals under a
policy, so no party can take another's; admission refuses a further one as
`ValueOutOfRange`. Each policy keeps its latest
`MAX_FISCAL_RESOLVED_PROPOSALS_PER_POLICY` (64) resolved proposals and drops
older ones. Authority and what a decision cites are
checked when the command is admitted and again when the action settles, and
every refusal is recorded: a rejected command at admission, a rejected
outcome at settlement. The binding names the decision in `rebound_by` and the
nomination it accepted, with its nominator, in `nomination`; a finding keeps
both as its auditor's binding held them (`appointed_by`, `nomination`); a
petition keeps who petitioned and, in its resolution, who decided.

`FiscalAction::RebindAudit` is the appointer's direct decision where the
policy makes the audited institution its own appointer and lets it act
without a nomination or petition: that institution then installs, replaces,
and removes its own auditors as in 0.15.0 (replacing needs both permissions),
and such a binding establishes
who entered a finding, not that the auditor is independent of the
institution he audits. Anywhere else the audited institution cannot change
its auditors (`FISCAL_ISSUER_NOT_APPOINTER`), and an institution with no
policy has only the audit bindings installed with the state
(`FISCAL_APPOINTMENT_POLICY_MISSING`). The basis only has to be the current
version of an application record of a kind the plugin reads.

The record versions that `rebound_by`, an audit binding, a nomination, and a
finding's `entered_by` name are not declared as required evidence. After
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
not canonical. An action whose result would not fit the state's byte budget
is rejected as its own outcome with `FISCAL_STATE_BUDGET_EXHAUSTED`.

No packet fails a boundary for the byte budget or for what the derived
systems add. Settlement measures every record its boundary will write: the
state as it leaves it, and that state with the transition candidates and the
aggregates the candidate and aggregate systems will write after it. It holds
the largest of them, without the report plan, to the budget less
`MAX_FISCAL_REPORT_PLAN_BYTES` (152,416 bytes), more than any plan can take,
so a change that adds many derived fields is measured with them. When the
whole boundary does not fit, the packets are settled again with each one
measured. An execution receipt or a historical-context change that does not
fit changes nothing: the state keeps neither the receipt nor its evidence
versions and external operations, the procedure revision does not advance,
and one `canwu.fiscal.ingress_rejected.v1` event, whose summary names the
ingress and ends with `FISCAL_INGRESS_BUDGET_REJECTION`, is its only record.
The crate has no archive, so a state at its budget stays there: a host that
must record its executions watches for that event, or for the state's size,
and starts a new run or shard before then. Near the budget each receipt or
context change of a boundary is measured on its own, at the cost of a
clone, a validation, two serializations, and a derivation of the state
(an estimated 30 to 40 ms at 2.2 MB, about half a second near 32 MiB), so
keep the receipts per boundary few there. An authored state whose derived fields, which its first
fiscal boundary writes, would not fit is refused when the run is created.

The fiscal action packet (`fiscal_action_v1`) is internal ingress. A host
cannot enqueue one; it issues an action with the `apply_fiscal_action_v1`
command, whose admission queues the packet. A packet settles only when its
ingress record names, as its cause, the command the packet carries, which
only that admission produces. A packet that a boundary system of another
plugin schedules into the action ingress is skipped with a
`canwu.fiscal.ingress_rejected.v1` event ending with `unauthorized_origin`, or
`malformed_payload` when it does not decode, and the boundary commits.

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
  owed, and the boundary schedules an internal `fiscal_report_drain_v1`
  packet due at once (`FISCAL_REPORT_DRAIN_INGRESS`), so the next boundary,
  at the same instant, publishes them, up to the same limit, without any
  change of the fiscal state. `advance_canonical` and `step_canonical` settle
  that boundary before they move on; `settle_boundary` refuses a later time
  until a boundary at that instant has admitted the packet, as it does for
  the packet an action command queues. A run that holds direct command
  history cannot add canonical ingress, so there no drain is scheduled and
  the owed parts wait for the next boundary that runs the fiscal systems.
  Until then the
  report's summary names them in `pending_parts`, `FiscalReport::is_complete`
  is false, and `FiscalReport::is_stale` tells a fact of a pending part,
  which may be older than `as_of` or missing, from a current one. Deferral
  shows an observer a little that its binding does not: a boundary's records
  are shared in the order of the observer bindings, so which parts are
  pending depends on the reports of the observers before it.
- The engine publishes no knowledge to a dead person. While an observer's
  actor is dead, nothing is published to that observer and the report stays
  as it was; what changes in it is kept in the report plan, by part, and is
  published by the first boundary after the person is alive again that runs
  the fiscal systems: the boundary that revives the person, when it admits
  any ingress or event.
- Publishing owed parts changes the fiscal state only in its report plan, so
  the procedure revision does not advance and no action becomes stale. A
  boundary finds owed parts by reading the plan's owed observers, and their
  actors, from the committed record without decoding the state; one in
  which nothing is owed to an observer who can receive it decodes nothing,
  as before. A queued drain packet is pending ingress, so like any other it
  holds back `seal_evidence` until the boundary that admits it.
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
