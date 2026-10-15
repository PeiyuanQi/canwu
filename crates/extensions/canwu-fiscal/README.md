# canwu-fiscal

`canwu-fiscal` is the generic fiscal-procedure extension for Canwu. It models
versioned fiscal rules, regional adoption, assessments, remissions, execution
authorizations, evidence-backed receipts, audits, reform candidates, strategic
aggregates, and holder-relative estimated reports published as knowledge.

It deliberately does not own money, grain, inventory, market quotes, transport,
or physical transfer truth. A resource or logistics adapter performs the real
operation, then submits a `FiscalExecutionReceiptPacket` citing exact external
domain-record versions through `enqueue_execution_receipt`; the packet and the
evidence envelope (`FiscalExecutionEvidence`) name the scope
(`scope_binding_id`) whose request they settle. At the live settlement
boundary the extension requires each cited version to be the current version
of its record, decodes the generic execution-evidence envelope, validates
every field against the authorization, and derives the receipt quantity and
disposition from the evidence.

No receipt or historical-context change fails its boundary. One that
settlement cannot accept changes nothing and is refused with one
`canwu.fiscal.ingress_rejected.v1` event, whose summary names the ingress and
the packet and ends with a stable reason, and the packets after it settle in
the same boundary:

| Reason | Constant | When |
| --- | --- | --- |
| `malformed_payload` | `FISCAL_INGRESS_MALFORMED_REJECTION` | the payload does not decode |
| `request_unknown` | `FISCAL_RECEIPT_REQUEST_UNKNOWN` | the scope holds no hot request with the receipt's request ID (it never had one, or the request's book was archived) |
| `cycle_closed` | `FISCAL_RECEIPT_CYCLE_CLOSED` | the request's book is closed |
| `evidence_count_invalid` | `FISCAL_RECEIPT_EVIDENCE_COUNT` | no evidence, or more than 32 versions |
| `evidence_kind_unapproved` | `FISCAL_RECEIPT_EVIDENCE_KIND_UNAPPROVED` | a kind the state does not approve |
| `evidence_not_current` | `FISCAL_RECEIPT_EVIDENCE_NOT_CURRENT` | a cited version is not its record's current version |
| `claim_mismatch` | `FISCAL_RECEIPT_CLAIM_MISMATCH` | a claim differs from the scope, the request, or the other claims, or predates the request |
| `quantity_exceeded` | `FISCAL_RECEIPT_QUANTITY_EXCEEDED` | the receipts would fulfil more than the request authorizes |
| `operation_reused`, `evidence_reused` | `FISCAL_RECEIPT_OPERATION_REUSED`, `FISCAL_RECEIPT_EVIDENCE_REUSED` | already settled by a receipt of the scope's hot books |
| `receipt_id_conflict` | `FISCAL_RECEIPT_ID_CONFLICT` | a hot receipt of the scope has the ID with other content |
| `context_outside_pack` | `FISCAL_CONTEXT_OUTSIDE_PACK` | a context year outside the pack |
| `state_budget_exhausted` | `FISCAL_INGRESS_BUDGET_REJECTION` | the result would not fit the state's byte budget, or the state holds 8,192 hot receipts |
| `invalid_packet` | `FISCAL_INGRESS_INVALID_REJECTION` | anything else that leaves the state invalid, after the message that says why |

A refused receipt consumes nothing, so a corrected one can settle later; if a
cited record changes before the receipt settles, cite its new version. A
configured evidence-kind allowlist and the host integration's redundant
semantic validator keep arbitrary world records from proving execution. An
exact evidence version settles at most one receipt: its claim names the scope
and request it settles, and a request's receipts are all hot while its book
takes receipts. An `(evidence kind, external_operation_id)` pair settles at
most one receipt among the receipts of one scope's hot books; across scopes,
and after a book is archived, unique operation keys are the evidence
domain's guarantee, as the resource extension's are. Evidence must not
predate its request, by simulated time; a request ID used again in a later
book of the same scope, at the same simulated time as the evidence of an
archived request of that ID, could take that evidence again, so use each
request ID once per scope for the life of a run.

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
as the action does not, and neither does archive maintenance, which never
advances the revision.

One boundary applies at most `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (64) actions
and rejects the rest with `FISCAL_BOUNDARY_ACTION_LIMIT`; a client reissues
those under new action IDs. A stale or unauthorized action is rejected without
using the limit. Reports are still published once per boundary, however many
actions settled in it.

An action applies at most once, without the state keeping every outcome for
the life of the run. Every action has a target, `FiscalActionTarget`: the
ledger of the scope a scope action names, the office of the institution a
binding, proposal, or transition action changes (keyed by
`fiscal_office_key`), or the observer binding an observer action changes.
Action IDs are per target. Each target keeps its `FISCAL_RECENT_OUTCOMES` (8)
most recent outcomes in a ring and an outcome floor, the procedure revision of
the newest outcome the ring has evicted. Admission refuses, from that hot
state alone, a request whose action ID has an outcome in its target's ring
(`IdempotencyConflict`), one that names a procedure revision the state never
had (`ValueOutOfRange`, `FISCAL_REVISION_UNKNOWN`), and one whose revision is
at or below its target's floor (`ValueOutOfRange`,
`FISCAL_REVISION_EXPIRED`), which is how a re-send of an evicted outcome is
refused. So a client issues an action against the revision it read, never a
later one. Settlement skips, without an outcome or an event, a copy of an
action whose outcome its target's ring holds or the boundary has already
settled. Every outcome is recorded for good in its
`canwu.fiscal.action_settled.v1` event (`FISCAL_ACTION_SETTLED_EVENT`), whose
summary `FiscalSettledAction::from_summary` reads back:
`FiscalSettledAction::from_events` lists them, and
`FiscalState::recent_outcome` answers for the outcomes still hot.

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
`MAX_FISCAL_REPORT_PLAN_BYTES` (169,200 bytes), more than any plan can take,
and less `FISCAL_ARCHIVE_HEAD_RESERVE_BYTES` (2 MiB), which archive
maintenance may use, so a change that adds many derived fields is measured
with them. When the
whole boundary does not fit, the packets are settled again with each one
measured. An execution receipt or a historical-context change that does not
fit changes nothing: the state keeps neither the receipt nor its evidence
versions and external operations, the procedure revision does not advance,
and one `canwu.fiscal.ingress_rejected.v1` event, whose summary names the
ingress and ends with `FISCAL_INGRESS_BUDGET_REJECTION`, is its only record.
A state near its budget holds too many hot books: close and archive them
([Accounting cycles and the archive](#accounting-cycles-and-the-archive)). A
host that must record its executions watches for that event, or for the
state's size. Near the budget each receipt or
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
next boundary was dropped. That happens only near the byte budget, where
admission keeps accepting commands: settlement rejects an action that does
not fit with `FISCAL_STATE_BUDGET_EXHAUSTED` while the rejection itself still
fits, and drops the ones after that. No count of outcomes runs out.

A boundary that admits no fiscal ingress costs no fiscal decode once any
boundary has written the state. Fiscal records are validated when the plugin
is activated, on a new scenario or a restored snapshot, and on every write;
the plugin's own handlers do not validate them again. Activation does not
recompute the derived fields of a state, and an idle boundary no longer does
either, so a state restored with altered transition candidates or aggregates
keeps them until its next fiscal write.

## Accounting cycles and the archive

The state is still one record, `canwu.fiscal:state`, under its 32 MiB budget,
but what it holds is laid out by scope. Each scope binding has a
`FiscalScopeLedger` with its adoptions and one `FiscalCycleBook` for each hot
accounting cycle, which holds the cycle's assessments, remissions, execution
requests and receipts, and audit findings; each institution has a
`FiscalOffice` that counts its hot books per cycle. Objects inside a scope
have IDs unique among the scope's hot objects, and every reference to one
names its scope: `GrantRemission`, `AuthorizeExecution`, and `RecordAudit`
carry `scope_binding_id`.

- `OpenAssessment` opens its book. `CloseAccountingCycle` closes one book of
  one scope; a closed book takes audit findings and nothing else
  (`FISCAL_CYCLE_CLOSED` for an assessment, remission, or authorization, and
  `cycle_closed` for a receipt). A ledger holds at most
  `MAX_FISCAL_HOT_BOOKS_PER_LEDGER` (64) books (`FISCAL_HOT_BOOKS_EXHAUSTED`).
- An archive commit takes closed books out of the state into the fiscal
  archive. It raises each scope's watermark to the greatest cycle it archived
  and, in the commit that archives an institution's last hot book of a cycle,
  archives that institution's totals of the cycle once and raises the office
  watermark. No book is opened at or below either watermark
  (`FISCAL_CYCLE_ARCHIVED`), so an accounting partition (scope, cycle, unit,
  payment form) is assessed at most once for the life of a run without
  reading the archive. Cycle IDs compare in byte order; name them so that
  they sort (`"1644"`, `"1644-03"`).
- The host archives. `FiscalState::prepare_fiscal_archive` selects closed
  books, in (scope, cycle) order, up to a book limit and
  `MAX_FISCAL_ARCHIVE_COMMIT_RECORDS` (4,096) records, and builds the blobs
  (`MAX_FISCAL_ARCHIVE_RECORDS_PER_BLOB`, 64, records each), membership,
  temporal, and lookup-index pages, and directory of a
  `PreparedFiscalArchiveBatchV1`. `enqueue_fiscal_archive` stores and verifies
  them in a `FiscalArchiveStore`, persists the retention handle, and queues
  the commit (`fiscal_archive_commit_v1`); `finalize_fiscal_archive_retention`
  queues its acknowledgement once the commit has settled. Prepare and enqueue
  in one gap between boundaries. Both work on `Canwu` and `CompactedCanwu`
  (`FiscalArchiveRuntime`). On a run that has sealed evidence, pass the end
  cursor of its last sealed segment: a book citing an evidence version
  established after it is refused at preparation
  (`FiscalArchiveError::UnsealedCitations`) until a seal covers it. The cut is
  the host's to pass; the functions cannot read it from the run, so a sealing
  host that passes `None` skips that refusal. Preparation refuses
  (`FiscalArchiveError::Backpressure`) while the state holds
  `MAX_FISCAL_ARCHIVE_PENDING_HANDLES` (64) unacknowledged retention handles,
  of applied or stale commits.
- A commit settles before every other fiscal packet of its boundary and is
  checked against the state alone: settlement rebuilds the records it binds
  from the books it names and compares. A commit prepared from an older
  state ends as a stale no-op, with a receipt and a handle the host
  acknowledges; a copy of a commit whose handle the state already holds,
  because the same batch was queued again, records nothing; each is recorded
  in its `canwu.fiscal.archive_maintenance.v1` event. A stale commit past the
  head's room (64 unacknowledged stale handles), and one that does not match,
  is refused with the refusal event and changes nothing. An acknowledgement
  retires its handle once. Neither advances the procedure revision, so no
  action becomes stale, and a boundary whose only change is archive
  maintenance publishes no report.
- No boundary reads the archive store. Admission, settlement, idempotency, and
  partition uniqueness use the hot state only, so a run settles exactly the
  same whether or not its store can be read, and restore and exact replay
  need no store. Only the host's archive operations read a store, the one the
  host passes them: `archived_fiscal_record` looks up an archived book, an
  institution's totals of a cycle, or a maintenance receipt by
  `FiscalArchiveKeyV1`, through the cumulative lookup index whose pages the
  directory root authenticates; `audit_fiscal_archive` checks the whole
  archive against the state's head.
- A store reports failures as `FiscalArchiveError`; the store trait and the
  error are the fiscal crate's own, and the lookup index uses the
  `canwu-resource` lookup-page functions behind them. `()` is a store that
  holds nothing.

The hot state keeps at most 4,096 assessments, 8,192 execution requests, and
8,192 receipts, with 32 evidence versions per receipt, under the 32 MiB
budget. None of these is a lifetime cap: closing and archiving books frees
them, and a run makes as many of each as it archives. Aggregate rebuilds use
single-pass indexes over the hot books. Each closed book's per-cycle totals
are archived with it; the strategic aggregates cover the hot books.

## Observers and reports

A `FiscalObserverBinding` names who holds its report, `knowledge_holder`: the
person of its `actor`, or, with no actor, a government, so a court or a
ministry can hold a report no one person does. `controlled_by` names the
institution that may change the binding: `RebindObserver` re-points it to
another actor and holder, and `RemoveObserver` removes it. Either is issued
under an authority binding of that institution, by its principal or by the
institution deciding as a body, never by an acting actor; a binding with no
controlling institution cannot change, and a removed one takes no further
action (`FISCAL_OBSERVER_REMOVED`).

An observer's report is one summary record and parts of 16 facts, one fact
for each aggregate the observer's binding lets it see. Read it with
`fiscal_reports(&viewer)`, which returns one `FiscalReport` for each observer
binding whose report the viewer's holder holds, keyed by binding ID, or, as
a trusted host and for any holder, a government included, with
`fiscal_reports_of_holder`; its `projection` is what `compute_projections`
derives from the fiscal state. A report record on its own
(`FiscalReportRecord`, version 3 of the `fiscal_report` knowledge schema) is
one summary or one part, not a report.

- Each binding's report lines are numbered from its `line_slot`, a number
  below `MAX_FISCAL_OBSERVERS` (64) that no other binding of the state has
  and that a removed binding keeps, so a slot is never used again. Rebinding
  or removing one binding leaves every other binding's lines where they
  were.
- When a binding is re-pointed to another holder, the new holder gets the
  whole report at the next publication, and the old holder's report is
  withdrawn: its summary is
  superseded by one marked `withdrawn`, and the reading functions leave the
  report out. A removed binding's report is withdrawn the same way. A
  withdrawn holder keeps the records it was given; they are history, not a
  current report.

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
  holder is a dead person, nothing is published to that observer and the
  report stays as it was; what changes in it is kept in the report plan, by
  part, and is published by the first boundary after the person is alive
  again that runs the fiscal systems: the boundary that revives the person,
  when it admits any ingress or event. A government holder is always
  reachable.
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

A scenario whose knowledge carries a report record of schema version 1 or 2
is refused when the run is created, because those versions are no longer
registered. A state is refused at activation when it could not be reported:

- when an observer binding names a person or government holder that does not
  exist, a holder that is not its actor's person or, with no actor, a
  government, or a line slot that is out of range or another binding's. The
  state record references the holder of every observer binding, and the
  engine checks references against its entity registry, so encode a state
  with `FiscalState::into_record`;
- when an assessment's unit or accounting cycle, or the namespace, name, and
  ID of a scope's domain-record institution together, is longer than
  `MAX_FISCAL_IDENTIFIER_BYTES` (160), which bounds a fact so that a part
  always fits one knowledge record.

Two costs grow with what a run has published, and both are the engine's.
Finding the record a part supersedes reads the holder's whole ledger, twice
for each record published. And the engine carries all held knowledge,
superseded records included, through every boundary, so every publication
slows every later boundary. `docs/proposals/fiscal-throughput-and-authority.md`
gives the measured rate.
