# Fiscal Throughput and Authority

Status: implemented in 0.15.0. It closes downstream gaps G-41, G-44, G-48, and
G-57 in `canwu-fiscal`, and changes how the extension publishes reports so
that the first two can be used. The public fixtures are
`gap_g41_fiscal_idle_early_exit`, `gap_g44_fiscal_actions_per_settlement`,
`gap_g48_fiscal_rebind_authority`, `gap_g57_fiscal_audit_binding`,
`fiscal_report_parts`, and `fiscal_reports_after_seal` under
`crates/extensions/canwu-fiscal/tests/`.

The release entry is in [Versioning and Persistence](../versioning.md). The
other 0.15.0 proposals are
[society settlement cadence and runtime revision](society-cadence-and-runtime-revision.md),
[same-instant supersession and unplaced persons](same-instant-supersession-and-unplaced-persons.md),
[keyed-draw index, draw ID, and boundary profile](keyed-draw-index-and-settlement-profile.md),
[resource archive lookup and volume](resource-archive-lookup-and-volume.md),
and
[single-step completion lease and runtime protected-floor install](resource-single-step-lease-and-floor-install.md).

`canwu-fiscal` keeps its catalog and its state in one domain record each. Four
event-driven boundary systems work on the state: `settle-fiscal-ingress-v1`
(phase 7), `evaluate-fiscal-transition-candidates-v1` (phase 10),
`aggregate-fiscal-state-v1` (phase 12), and `materialize-fiscal-reports-v1`
(phase 13). Commands are admitted by `apply_fiscal_action_v1`, which queues one
`fiscal_action_v1` item that the next boundary settles.

All four changes rest on one rule, which 0.15.0 makes explicit:

> A fiscal record is validated where it enters a run. Plugin activation
> validates the catalog and state of a new scenario or a restored snapshot,
> the catalog is create-only, and every write of the state is validated before
> it is proposed. The plugin's own handlers decode a committed record without
> validating it again.

Sequential settlement reaches the size of one report record sixty-four times
faster than 0.14.0 did, so the report an observer holds is no longer one
record; see [Reports](#reports).

## Idle boundaries (G-41)

### Problem

An event-driven system runs in every boundary that admits an event or any
ingress, whichever plugin the ingress is for. Each of the four fiscal systems
decoded and validated the catalog and the state before it looked for work, so
every such boundary paid for four full decodes of the fiscal state, however
little it had to do with fiscal procedure.

### Decision

Each system first establishes from cheap reads that it would propose nothing,
and returns before it decodes a record.

| System | Returns early when | Read used |
| --- | --- | --- |
| Settlement | the boundary admitted no ingress of this plugin | the boundary's own admitted ingress |
| Transition candidates, aggregates | the state's current version was established by an earlier boundary | `SimulationView::current_domain_record_version` |
| Reports | no earlier phase of this boundary changed the state | `SimulationView::proposed_domain_record_version` |

The filed fix, "return when the boundary admitted no fiscal ingress", is right
for settlement only. None of the four systems does time-driven work: nothing
reads the clock except to stamp a new entry, and the historical year changes
only through ingress. But the two derived systems do work that no fiscal
ingress triggers. A scenario may install a state whose transition candidates
and aggregates are empty or out of date, and the first boundary that admits
anything computes and writes them. Skipping that boundary would change the
records and hashes of runs that exist today.

The rule for the derived systems therefore follows what they compute.
Transition candidates and aggregates are functions of the rest of the state
and of the immutable catalog, and both systems run in every boundary that
writes the state. A state whose current version an earlier boundary
established was left consistent by that boundary, so the systems would propose
nothing for it. A state this boundary changed is recomputed, and so is a state
still at its initial-scenario version, whose authored derived fields nothing
has checked yet. That last case keeps paying two decodes in every
event-bearing boundary until the first write; a state that needs derivation is
written, and leaves the case, in the first such boundary.

The report system already published only for a state its own boundary changed.
It now establishes that before it decodes, and it publishes only the parts of
a report that changed (see [Reports](#reports)).

Because idle boundaries no longer validate the records, activation does: an
invalid fiscal catalog or state is refused when the run is created or
restored. Under 0.14.0 it was accepted and then failed the first boundary that
admitted anything. Inside a run, the handlers and command admission decode the
committed records without re-validating them, and without copying the record.

### Invariants

- On a boundary without fiscal ingress, a state that an earlier boundary wrote
  gets no proposal from any system, as in 0.14.0, so the early exits change
  no evidence, record, or hash there. A state still at its scenario version
  gets its derived fields in the same boundary as in 0.14.0. With them the
  aggregate system now writes the report plan, and the report is published in
  parts ([Reports](#reports)).
- The early-exit decisions read the boundary's own admitted ingress, the
  current-version provenance index, and the boundary's own proposals. None
  depends on evidence a seal can remove, so a sealed run decides as its exact
  replay does.
- A committed fiscal state is always valid: only this plugin writes it, every
  write is validated, and activation validates what a scenario or snapshot
  brings in.

Activation validates the state but does not recompute its derived fields, and
a state a boundary has written no longer has them recomputed while idle. A
restored state whose derived fields were altered outside the engine keeps
them until its next fiscal write; 0.14.0 corrected them at the next
event-bearing boundary.

### Measurement

Measured with `fiscal-profile` in `benchmarks/performance-harness`, which
builds a fiscal state with 48 institutions, 1,920 settled assessments, 1,920
execution requests, and 3,840 action outcomes (2,155,406 bytes of JSON), and
times boundaries through the public API in a release build. An idle boundary
admits one ingress item of another plugin. Four observers each see two
institutions, which is what one 0.14.0 report record could hold. Every run is
recorded under `benchmarks/baselines/`, three rounds of 31 samples for each
release and one run of each with `--observers 0`, and the tables below are
computed from those files:

- `2026-10-13-fiscal-boundary-<release>-round<n>-elapsed.json`, and
- `2026-10-13-fiscal-boundary-<release>-no-observers-elapsed.json`.

Each figure is the median of the three round medians, with the lowest and
highest round median in brackets. The 0.14.0 files were recorded first, the
same source built against the 0.14.0 crates and run alternately with an
earlier state of this change. The 0.15.0 files were recorded later the same
day, on the branch as it is, after the other packages of the release had
landed under it. The machine was running other builds both times, so the
figures are a comparison, not a budget.

| Idle boundary | 0.14.0 | 0.15.0 |
| --- | ---: | ---: |
| Run without fiscal records (engine floor) | 0.036 ms (0.035 to 0.053) | 0.037 ms (0.036 to 0.037) |
| Fiscal state that a boundary has written | 136 ms (134 to 142) | 0.833 ms (0.787 to 0.838) |
| Fiscal state still at its scenario version | 152 ms (150 to 167) | 12.1 ms (11.9 to 13.2) |
| Written state, no observer (one round) | 121 ms | 0.037 ms |

The 0.8 ms that remains is not fiscal work. With `--observers 0`, so that no
report has been published, the same boundary takes 0.037 ms, the engine floor:
the four observers' reports, 24,529 bytes of payload each, are knowledge
records the engine carries through every boundary. A state still at its
scenario version pays two decodes and the two derivations. For scale, the
0.15.0 round medians for this state were 4.14 to 5.05 ms to decode it,
1.50 to 1.59 ms to copy it, 1.47 to 1.49 ms to validate it, and
11.7 to 12.4 ms to encode it.

### Persistence, replay, and format

No persisted shape changes. Snapshot, commitment, checkpoint, and
evidence-journal formats are unchanged, and so is the fiscal state schema
version. The plugin semantic hash changes with the package (see
[Compatibility](#compatibility)).

## Several actions per boundary (G-44)

### Problem

Settlement rejected every fiscal action after the first change it made in a
boundary: once an action or an execution receipt had settled, each later
action of that boundary was recorded as stale. One action could apply per
boundary, so an institution that assesses many scopes in one accounting step
had to spread them over as many boundaries.

### Decision

Actions settle **sequentially, in admission order**, each applied to the state
the previous one left and fully validated there. A batch-carrying action was
considered and rejected. Authority is per binding, so one batch could cover
one institution's scopes only, and a day on which several institutions act
would still need several actions in one boundary. A batch would also need a
second outcome model for partly valid batches. With sequential settlement one
action stays one command, one authority check, and one outcome.

**Expected revision.** Every action a boundary settles is compared with one
revision: the procedure revision the state had when the boundary began. An
issuer who admitted two actions against the same revision sees both settle, in
the order admitted, provided each is valid against the state the earlier one
left. An action decided against an older revision is stale, as before.

The boundary is the right unit because no issuer can observe a state between
two actions of one boundary: all of them were necessarily decided against
state committed before it. What the revision still has to protect inside a
boundary is a blind overwrite:

- A later action is stale when it would overwrite an adoption, authority
  binding, or audit binding that an earlier action of the same boundary
  created or changed. Its issuer decided against the object as it stood before
  the boundary. Reading what an earlier action did is allowed: an assessment
  may rely on an adoption that the action before it made operational.

The rule covers overwrites and nothing else. Actions that only create
(assessments, remissions, execution authorizations, audit findings) cannot
overwrite one another; identity reuse and over-claims are caught by the
validation each action runs against the running state. Two issuers'
remissions, or two execution authorizations, on one assessment in one
boundary therefore both apply when their sum validates.

An action settles in the boundary after the one that admits its command. A
receipt, a historical-context change, or another action that settles in the
admitting boundary advances the revision first and makes the action stale, as
in 0.14.0. Inside the settling boundary neither a receipt nor a
historical-context change makes an action stale, wherever the canonical
ingress order puts it: the action is still validated against the state they
left. For receipts that is new. A receipt records external evidence and
removes no option, but it does change the totals an issuer saw.

**Rejection.** An action that is stale, unauthorized at settlement, or invalid
is recorded as its own `Rejected` outcome, consumes its action ID, and
advances the procedure revision, exactly as a single rejected action did. The
state it was applied to is discarded, and the next action starts from the last
good state, so a rejected action does not affect the actions after it.

**Limit.** One boundary applies at most `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (64)
actions. Only an action that is current and authorized, and so reaches the
copy and validation of the whole state, counts; a stale or unauthorized action
is rejected without using the limit, so one institution's rejected actions
cannot crowd out another's. A further action is recorded as `Rejected` with
the stable reason `FISCAL_BOUNDARY_ACTION_LIMIT`, and a client reissues it
under a new action ID.

**No action fails the boundary for the others.**

- *Size at admission.* A request larger than `MAX_FISCAL_ACTION_JSON_BYTES`
  (32 KiB) is rejected as `InvalidPayload`, and so is an audit `finding`,
  remission `reason`, or execution `purpose` longer than
  `MAX_FISCAL_ACTION_TEXT_BYTES` (2,048 bytes), a `unit` or a referenced ID
  longer than `MAX_FISCAL_IDENTIFIER_BYTES` (160 bytes), or the ID of a new
  entry that is not canonical. Before, an action's text was limited only by
  the state's byte budget, and an action that exceeded the budget failed the
  boundary that settled it.
- *State byte budget.* An action whose result, with its outcome and the
  derived fields that follow from it, would not fit
  `MAX_FISCAL_STATE_JSON_BYTES` less a reserve of 16 KiB is rejected as its
  own outcome with `FISCAL_STATE_BUDGET_EXHAUSTED`. Settlement measures
  nothing while the result stays more than 1 MiB under that; closer, it
  settles the same packets again from the committed state and measures every
  action, to the byte. Unit tests reach the budget on a small state and hold a
  rejected outcome to exactly the size that fits, the digit a procedure
  revision gains included. The reserve is for the report plan: settlement
  measures it as if every observer could receive knowledge, and a plan that
  defers the parts of an observer who cannot lists the same parts under other
  keys, at most one observer ID more for each binding. The margin stands for
  what the two derived systems add later in the same boundary; derived fields
  that grew by more than 1 MiB in one boundary would still fail it. A receipt
  or a historical-context change that does not fit also still fails the
  boundary: both are host packets with no outcome to reject.
- *Outcome capacity.* An action that reaches settlement when no outcome slot
  is left, or when the byte budget cannot hold even its rejection, is dropped:
  it gets no outcome, its action ID is not consumed, it changes nothing, and
  one `canwu.fiscal.action_dropped.v1` event names it. Under 0.14.0 that
  boundary failed, and failed again on every retry. An accepted command whose
  action has no outcome after the next boundary was dropped; the event's
  summary names the action, and its affected entities name the institution
  the action was issued for. A domain-record institution is left out, because
  the engine fails a boundary whose event names a missing entity and the
  plugin cannot tell whether such a record still exists when the action
  settles. Once no outcome slot is left, admission rejects every command.
  Near the byte budget it keeps accepting them: settlement rejects an action
  that does not fit with `FISCAL_STATE_BUDGET_EXHAUSTED` while the rejection
  itself still fits, and drops the ones after that.

**Lifetime caps.** The crate has no archive, so its caps are lifetime caps,
and the new throughput reaches them sooner. `MAX_FISCAL_ACTION_OUTCOMES`
(16,384) is 256 boundaries of 64 outcomes away, and rejections use it up too;
a full state rejects every further command at admission.
`MAX_FISCAL_ASSESSMENTS` (4,096) is 64 full boundaries of assessments away;
past it each assessment is rejected as its own outcome and the boundary
commits.

**Reports and events.** The report system still runs once per boundary,
however many actions settled in it, and publishes what changed in each
observer's report (see [Reports](#reports)). Each settled action still emits
one `canwu.fiscal.action_settled.v1` event.

### Invariants

- Outcomes within a boundary are a function of the admission order, which the
  canonical ingress queue fixes.
- The state after a boundary is the state after its last applied action; every
  intermediate state was validated.
- The procedure revision advances by one for each recorded outcome, receipt,
  and historical-context change, as before.
- An action is applied, rejected, or dropped on its own; none fails settlement
  for the others. A boundary still fails, as before, for a receipt whose
  evidence does not validate, a historical context outside the pack, an item
  no admitted command stands behind, or a receipt or context change past the
  state's byte budget.

### Cost

The same profile admits one assessment from each of 40 institutions against
one revision, then settles them. The figures come from the same recorded
rounds as the idle table.

| Boundary | 0.14.0 | 0.15.0 |
| --- | ---: | ---: |
| Admits the 40 commands | 2,055 ms (1,402 to 2,243) | 178 ms (177 to 178) |
| Settles the 40 actions | 742 ms (626 to 875), 1 applied | 602 ms (601 to 619), 40 applied |

Under 0.14.0 each of the other 39 actions needed a boundary pair of its own.
Admission is cheaper because it decodes the committed state once per command
without validating it again: about 4.4 ms per command here, from 35 to 56 ms.
An applied action copies and validates the whole state, about 3 ms at this
size, so the limit of 64 bounds that part of a boundary at about 0.2 s
for a state this large.

The rest of a settling boundary is the cost of the fiscal state being one
record. It is written twice, by settlement and then by aggregation, and the
engine validates, journals with its previous version, and hashes each write.
That cost is the same for one action as for 40, and this proposal does not
change it.

Within 1 MiB of the byte budget a boundary settles its actions a second time,
and that pass derives and encodes the whole state once for each applied
action. This was not measured. By the component costs above it is about a
third of a second for each action at 32 MiB, so a full boundary of 64 actions
there takes on the order of twenty seconds. A state that close to its budget
is at the end of its life in any case.

### Persistence, replay, and format

No persisted shape changes. The settlement system declares one more event
type, `canwu.fiscal.action_dropped.v1`, so its descriptor changes. The settled
outcomes of a boundary that admits more than one action differ from 0.14.0 by
design.

## Authority rebinding (G-48)

### Problem

Authority bindings were installed with the state and nothing could change
them. An acting actor is admitted only while the binding's exact authority
basis is the current version of its record, so when the application renewed
or replaced the grant, the binding kept citing a superseded version and the
acting actor was rejected for good. A new office holder could not become a
binding's authorized actor at all.

### Decision

```rust
FiscalAction::RebindAuthority {
    authority_binding_id,
    authorized_actor,
    acting_actor,
    authority_basis,
}
```

The action replaces the three actor fields of one binding together.

- **Who may issue it.** It is issued under the binding it replaces
  (`authority_binding_id` equals the request's binding), by one of two
  issuers. The first is the binding's authorized actor, directly or as a
  decision origin. The second is the institution deciding as a body: a
  decision controller whose command subject and decision origin are the
  binding's institution, with no responsible actor. It exists because a person
  who can no longer act cannot hand a binding over. An acting actor cannot
  rebind, directly or as the responsible actor of an institution decision, and
  neither can any other person: an institution decision is resolved by its
  responsible actor first.
- **What the institution role amounts to.** It is the power to appoint. Such a
  controller can name any actors, or none; and on a binding that names no
  authorized actor, an institution decision with no responsible actor holds
  the principal role, as it did in 0.14.0, so after vacating a binding the
  controller acts as principal itself. A host registers a controller of that
  shape only for the body that appoints the binding's actors.
- **Persons.** Every person the action names must exist. The command lists
  them as entities its queued item affects, which the engine checks against
  its entity registry at admission; that registry also holds a person who has
  no placement in the world projection. A missing person is a rejected command
  (`EntityNotFound`). The settled event of an applied rebind lists the same
  entities, and the engine checks those again when the boundary commits. The
  public view has no entity-registry read, so settlement cannot turn that
  second check into a rejected outcome; it cannot fail for an admitted
  command, because the registry never loses an entity.
- **Basis.** `acting_actor` and `authority_basis` are set together. The basis
  must be the current version of an application record of a kind the plugin
  was configured to read, when the command is admitted and again in the state
  the settling boundary began with; otherwise it is rejected with
  `FISCAL_AUTHORITY_BASIS_NOT_CURRENT`. A record that advances inside the
  settling boundary is not seen: the rebind installs a basis that is already
  superseded, and the acting actor is rejected at the next admission.
- **Settlement.** Every action now carries the role its issuer held at
  admission, and settlement re-checks that role against the state the action
  settles on, and that the role admits the action. An action of an authorized
  actor who has since been replaced is rejected with
  `FISCAL_ISSUER_NOT_BOUND`; an action of an acting actor whose binding no
  longer cites the basis he was admitted under is rejected with
  `FISCAL_ACTING_BASIS_NOT_CURRENT`.
- **Evidence.** The binding records the rebind in `rebound_by`: the action ID,
  the time, and the issuer's binding, role, and person. The action's outcome
  names the command.

The binding changes hands in one commit. Actions admitted earlier settle in
admission order around the rebind: those before it under the previous actors,
those after it rejected. The new actors are admitted from the first admission
after that commit. At no committed state can both actors act, or neither.

Currentness is read from the current-version provenance index
(`SimulationView::current_domain_record_version`), whose answer a seal cannot
change. The acting-actor check, which 0.14.0 made through
`domain_record_version_is_current`, now uses the same read at admission and at
settlement, with the same result.

Command admission can observe that a record kind is readable but not which
list declared it. Activation therefore accepts a basis of any external kind the
plugin reads: one declared with `FiscalPlugin::with_authority_basis_kinds`, or
an external evidence kind passed to `FiscalPlugin::new`. A fiscal record can
never be a basis. This keeps activation and admission in agreement, so a run
that settled a rebind can always be restored.

### Invariants

- A binding's institution never changes, and bindings are never created or
  removed by a rebind.
- A basis installed at runtime was current in the state its boundary began
  with.
- No action applies under a role its issuer did not hold in the state it
  settled on.

### Persistence, replay, and format

`FiscalAuthorityBinding` gains `rebound_by: Option<FiscalRebinding>`, omitted
from the serialized binding while it is `None`. `FiscalAction` gains the
`rebind_authority` variant, which appears in command and ingress payloads only.
The queued `fiscal_action_v1` item, which the plugin writes at admission and
reads at settlement, now carries the issuer's binding, role, person, and basis
where it carried only an acting basis.

The bases that `rebound_by`, an audit binding, and a finding's `entered_by`
name are not declared as required evidence. No decision reads them again, but
after `seal_evidence` a superseded version among them may no longer be
provable.

## Audit bindings (G-57)

### Problem

`RecordAudit` was admitted only under the authority binding of the institution
that owns the audited scope. An auditor from another office could not issue
it, so the settled finding could only ever have been entered by the audited
side, and it did not say by whom.

### Decision

A separate binding, beside the authority bindings:

```rust
pub struct FiscalAuditBinding {
    pub id: String,
    pub institution: EntityRef,       // the audited institution
    pub auditor: PersonId,
    pub authority_basis: DomainRecordVersionRef,
    pub rebound_by: Option<FiscalRebinding>,
}
```

`FiscalState::audit_bindings` holds them by ID. A request names an audit
binding in the same `authority_binding_id` field; the two ID spaces are
disjoint. Audit binding IDs are one namespace across all institutions, and
all of them share the cap of `MAX_FISCAL_RUNTIME_BINDINGS`. Under an audit
binding:

- only `RecordAudit` is admitted, and only for a target whose scope the
  audited institution owns;
- the issuer must be the auditor, directly or as the deciding or responsible
  actor of a validated decision on the audited institution;
- the basis must be the current version of its record at admission and in the
  state the settling boundary began with (`FISCAL_AUTHORITY_BASIS_NOT_CURRENT`
  otherwise).

An `auditors` set on the authority binding was considered and rejected:

- An auditor is not one of the institution's actors. In a separate map, no
  check of who may act for the institution ever sees him, so "gains no other
  fiscal authority" holds by structure rather than by remembering to exclude a
  set at every check.
- An audit commission has its own basis and lifetime. A set would have to
  share the acting actor's basis or carry none.
- Several auditors with different commissions are separate entries, and a
  finding can name the one it was entered under.

**The finding names its issuer.** `FiscalAuditFinding::entered_by` records the
binding, role, person, and authority basis of whoever entered the finding: the
auditor of an audit binding, or an actor of the institution's own authority
binding. It is `None` only for findings settled before 0.15.0.

**Runtime changes.**

```rust
FiscalAction::RebindAudit { audit_binding_id, auditor, authority_basis }
```

installs an audit binding, replaces its auditor and basis, or, with both
`None`, removes it. It follows the authority rebind: issued under an authority
binding of the audited institution, by its authorized actor or by the
institution deciding as a body, never by an acting actor, the auditor, or
another institution; the auditor must exist; the basis must be current at
admission and in the state the settling boundary began with; the binding
records the action in `rebound_by`. A finding of the previous auditor that
settles after the rebind is rejected with `FISCAL_ISSUER_NOT_BOUND`. An
auditor's authority also ends, without any fiscal action, when his basis
record advances or retires.

**What the binding establishes.** Attribution, not independence. The audited
institution installs, replaces, and removes its own auditors, and can remove
one before a finding admitted under his binding settles. The basis only has to
be the current version of some application record the plugin reads; whether
that record commissions this auditor is the application's to say. Who appoints
an auditor is left open for the owner to decide. The alternatives are audit
bindings that only a scenario installs, a `RebindAudit` limited to re-pointing
the same auditor at a renewed basis, and an appointing institution distinct
from the audited one.

### Invariants

- An audit binding grants `RecordAudit` on one institution's scopes and nothing
  else.
- An audit binding's institution never changes.
- Every finding settled from 0.15.0 names who entered it.

### Persistence, replay, and format

`FiscalState` gains `audit_bindings`, omitted from the serialized state while
it is empty. `FiscalAuditFinding` gains `entered_by`, omitted while `None`.
`FiscalAction` gains the `rebind_audit` variant.

## Reports

### Problem

An observer's report was one knowledge record, published whole to every
observer by every boundary that changed the fiscal state. Two things followed.

- **Size.** The engine limits a knowledge payload to 65,536 bytes, and a
  report carried one fact of about 334 bytes for each aggregate its observer
  could see, so one record held about 196 facts. The boundary that would
  publish a larger report failed, and failed again on every retry. 0.14.0
  approached that size by one assessment a boundary; sequential settlement
  approaches it by sixty-four.
- **Supersession after a seal.** The report system found the record to
  supersede through `SimulationView::knowledge_changes_by_correlation_prefix`,
  which reads retained boundary evidence. After `seal_evidence` removed the
  publishing boundary the read returned nothing, the next report superseded
  nothing, the holder kept two current reports, and the sealed run failed
  exact replay.

### Decision

**Shape.** A report is one summary record and parts of
`FISCAL_REPORT_FACTS_PER_PART` (16) facts. Each is a line of records of its
own: a record supersedes the record before it on its line and no other. The
summary carries what a 0.14.0 report carried besides its facts (the actor,
`as_of`, the confidence), the number of facts and parts, and the numbers of
the parts that are pending. The payload type is `FiscalReportRecord`, schema
version 2 of `fiscal_report`.

**Partition.** An observer's facts are divided in aggregate-ID order, and an
aggregate ID is now stable: an aggregate keeps the ID it was first given, and
a new aggregate takes the next one. Aggregates that appear in the same
derivation are numbered in key order. A state that comes with no aggregates,
or with all of them numbered as a derivation numbers them, therefore gets the
numbering 0.14.0 gave. A state that comes with some of its aggregates,
numbered from zero without a gap, keeps those IDs and has the missing ones
appended, where 0.14.0 numbered all of them again in key order. Any other
numbering is replaced by one in key order.

The partition follows how aggregates grow. An accounting cycle adds
aggregates across institutions, and the aggregates that change afterwards are
the recent ones, as their assessments are collected or remitted. In ID order
every new aggregate goes into the last part, the parts before it keep their
members for good, and a part goes quiet as its cycles close. Two alternatives
were rejected:

- *A part for each institution and accounting cycle.* It is the natural key,
  but a line has to be told from the others by a knowledge subject (below),
  and neither an accounting cycle nor a piece of one is anything a subject can
  name. It would also need a second rule for a cycle that outgrows one record.
- *Parts by a hash of the aggregate key.* Membership would be stable without
  stable IDs, but each new cycle would scatter over every part and republish
  all of them.

A part holds a fixed number of facts rather than a number of bytes, so its
members never depend on what the facts say. That needs every text of a fact
bounded, and the state now is: an assessment's unit and accounting cycle, and
the namespace, name, and ID of a scope's domain-record institution together,
are held to `MAX_FISCAL_IDENTIFIER_BYTES` (160), as admission already held the
first two. A full part of ordinary identifiers is about 6 KiB. With every text
at its longest and made only of characters JSON has to escape it is 55,916
bytes, which a unit test pins under the limit.

**Lines and subjects.** `SimulationView::knowledge_heads(holder, kind,
subjects)` returns the holder's current records that carry every listed
subject. One holder holds a line for the summary and for each part of every
observer binding that names its actor, and bindings may share an actor, so
each line needs a subject that no other line has. A subject is a role and a
target, and the target has to exist. The one record a fiscal report is
certainly about is the fiscal state: an institution need not be a registered
entity, and a part is neither an entity nor a record. The line is therefore
named by its roles:

- a line number is the observer binding's place among the state's observer
  bindings times 1,024, plus 0 for the summary or 1 plus the part number;
- its four hexadecimal digits are four subjects, each the role
  `report_line_<place>_<digit>` on the fiscal state record;
- every record also carries the `fiscal_state` subject it always carried.

Two lines differ in at least one place, so neither's subjects contain the
other's, and the four digit subjects select exactly one line. Sixty-four
observer bindings of 1,024 lines are the 65,536 numbers four digits have.
Observer bindings never change, so a line keeps its number. That is what the
scheme rests on: a line number is an ordinal place, sound only while no
action can add, remove, or reorder observer bindings. An action that rebinds
observers would have to keep every binding's place, or publish every report
whole again on its new lines.

Two alternatives were rejected. The institution as a subject would fail the
boundary for an institution that is not a registered entity, which a scenario
can author, and still could not name a part. One plugin-owned domain record
for each line, to be the subject's target, would add state and writes that
nothing else reads.

**What a boundary publishes.** The aggregate system holds the aggregates of
the previous derivation while it computes the new ones. It compares each
observer's parts before and after and writes the result, the report plan, with
the aggregates: `FiscalState::report_plan` lists the parts to publish for
each observer. The report system publishes the plan: the listed parts and the
observer's summary, each superseding what `knowledge_heads` returns for its
line. Published reports are never read back.

- A boundary that changes no fact an observer can see publishes nothing to
  that observer, so `as_of` is the time the report last changed. A change that
  the observer's confidence cannot resolve changes no fact.
- The first plan of a state lists every part, and gives an observer that has
  no fact yet a summary of nothing.
- A plan is carried out only by the boundary that computed it: it records the
  procedure revision it was computed at, and the report system ignores a plan
  of another revision.
- A plan means something only in a state a boundary has written. One that
  comes with a scenario, as in a state exported from another run, is ignored:
  the first system that writes the state clears it, so the first derivation
  plans every report whole. A plan may list at most
  `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` records to publish.

The plan is kept in the state because the comparison needs the previous
facts, and the state is where they are. `knowledge_heads` returns record IDs,
not content, and reading every published part back in every fiscal boundary
would cost the whole report each time.

**The record to supersede.** The head of a line comes from the committed
knowledge ledger and the records the boundary in flight published in an
earlier phase, which is what `knowledge_heads` reads. A live run, a run that
sealed its evidence, a run restored from sealed segments, and exact replay
therefore supersede the same record. The call normally returns one ID, or
none for a line's first record. If a line ever held more than one current
record, the next record would supersede all that are returned.

The other reads of retained evidence in the crate's boundary systems were
checked. Settlement reads the ingress items the boundary admits and, for each
fiscal action, the command that queued it. An item is pending ingress until
the boundary that admits it, and `seal_evidence` refuses to seal while any
ingress is pending, so no seal can come between a command and the settlement
of its action: the item and the command are still retained. Everything else
reads the fiscal records, the current-version provenance index, the boundary's
own proposals, or `replay_stable_domain_record_version`.

**Limits.** No fiscal report can reach a limit the engine sets on one record
or one publication.

| Engine limit | Engine | Fiscal reports |
| --- | ---: | --- |
| Payload bytes of a record | 65,536 | at most 55,916 for a full part |
| Subjects of a record | 64 | 5 |
| Records a record supersedes | 64 | the head of its line: 1 |
| Records of one publication | 1,000 | an observer's summary and at most 256 parts |
| Publications of one system in a boundary | 64 | one for each observer; at most 64 observers |
| Records of one boundary, all plugins | 10,000 | at most `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY` (1,024) |

Only the last can bind: sixty-four observers who each see everything have
16,448 records between them. When more parts changed than one boundary
publishes, the boundary still commits:

- every observer with a changed part gets its summary;
- the records that are left go to parts, those an earlier plan deferred
  first, then in order of observer and part;
- the rest are deferred. The plan records them, the summary names them in
  `pending_parts`, and the next boundary that changes the fiscal state
  publishes them, whatever that change is.

Until then the report is incomplete: its holder has an older record for a
deferred part, or none. A boundary without fiscal ingress does not publish
them, because finding them would mean decoding the state on every idle
boundary. The engine's per-boundary limit is shared by every plugin, so the
fiscal share is bounded here but whether a boundary fits also depends on the
others.

Deferral shows an observer a little of what its binding does not, and only
when one boundary has more than 1,024 report records to publish. The
boundary's records are shared in observer order, so which parts of a first
report are pending depends on the size of the reports of the observers before
it. And deferred parts go out with the next change of the fiscal state,
whoever made it and whatever it was: an action of another institution that
settlement rejects is enough. Their arrival tells the observer that something
changed.

**Observers who cannot receive knowledge.** The engine refuses to publish to a
holder who does not exist or is dead, and a refused publication fails its
boundary. Under 0.14.0 either ended the run at the observer's next report.

- *A person who does not exist.* The state record now references the person
  of every observer binding, under the role `observer`. The engine checks a
  record's references against its entity registry when a run is created or
  restored and at every write, so a state that names an observer who does not
  exist is refused where it enters a run. The plugin cannot make the check
  itself: a view has no read of the entity registry.
- *A dead person.* The aggregate system reads person availability and plans
  nothing for an observer whose actor is dead. What changes in that
  observer's report is deferred instead, outside the boundary's record limit,
  so it takes nothing from the other observers; and being a set of part
  numbers, it cannot grow past the report's 256 parts. The dead observer's
  report stays as it was last published. If the person is alive again, the
  next plan takes those parts first, and they are published with a new summary
  by the next boundary that changes the fiscal state. Between the revival and
  that change the revived observer holds his old report unchanged: its
  summary names none of the parts deferred while he was dead and carries the
  old `as_of`, so a report whose `pending_parts` was empty when he died looks
  complete until then. The report system asks
  the same question before it publishes and skips an observer the engine would
  refuse. Both systems run after the last phase that can change availability,
  so they read the answer the engine checks the publication against.

**No ceiling.** An accounting partition is assessed once, so every aggregate
has one assessment, a report has at most `MAX_FISCAL_ASSESSMENTS` (4,096)
facts, and 256 parts hold any report the state allows. The bounds a part
relies on are checked where a state enters a run, and nothing an action can do
exceeds them, so no action is rejected for the size of a report and no
boundary fails for it.

**Reading a report.** `fiscal_reports(viewer)` reads the fiscal report
records the viewer's holder currently holds and returns one `FiscalReport` for
each observer binding, keyed by binding ID; `assemble_fiscal_reports` does
the same from payloads. A complete report's `projection` equals what
`compute_projections` derives from the fiscal state it was published from, so
a reader gets what a 0.14.0 report record held. The records are in the
holder's own ledger and nowhere else, and a part holds only facts of
institutions its binding names. An incomplete report says which of its parts
are pending: `FiscalReport::pending_parts` names them and `is_stale` tells a
fact of a pending part, which may be older than `as_of` or missing, from a
current one.

### Invariants

- A holder has at most one current record on each report line.
- A complete report, assembled, equals what `compute_projections` derives
  from the fiscal state at the report's `as_of`.
- A boundary publishes a part only when its facts changed since the line's
  current record, when an earlier plan deferred it, or in a state's first
  plan.
- No fiscal report record exceeds an engine limit on a record or a
  publication, and one boundary's fiscal report records never exceed
  `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`.
- What to publish and what to supersede are decided from the fiscal state,
  person availability, and the knowledge ledger, never from retained evidence.
- No report publication is refused for its holder: a state whose observer
  does not exist is refused where it enters a run, and nothing is published
  to an observer who is dead.

### Cost

The fiscal profile of [Idle boundaries](#measurement) was also run with one
observer that sees all 48 institutions (`--observers 1
--observed-institutions 48`). Its report has 1,921 facts once the first action
has settled, almost ten times what one record held, and 0.14.0 cannot run the
case: the boundary that publishes its first report fails. Three rounds are
recorded as
`2026-10-13-fiscal-boundary-0.15.0-wide-observer-round<n>-elapsed.json`, and
the first column repeats the 0.15.0 rounds of the other tables.

| | Four observers of two institutions | One observer of 48 |
| --- | ---: | ---: |
| One observer's report as first published | 7 records, 24,529 bytes | 122 records, 580,780 bytes |
| Idle boundary, state a boundary has written | 0.833 ms (0.787 to 0.838) | 4.69 ms (4.57 to 4.71) |
| Boundary that admits the 40 commands | 178 ms (177 to 178) | 268 ms (265 to 271) |
| Boundary that settles the 40 actions | 602 ms (601 to 619) | 625 ms (608 to 632) |
| Report records that boundary publishes | 8 records, 3,122 bytes | 4 records, 12,599 bytes |

- **What a boundary publishes.** The settling boundary adds 40 aggregates.
  Each of the four observers gets its summary and the one part they joined;
  the one wide observer gets its summary and three parts. Under 0.14.0 the
  same boundary published every observer's whole report again, four records
  of about 27 KiB. A holder's ledger keeps superseded records, so what a
  boundary publishes is what the run's knowledge grows by.
- **Idle boundaries.** The fiscal systems return before they decode anything
  in both cases. The difference is the engine carrying the knowledge the run
  holds through each boundary: about 8 microseconds for each KiB of report
  in the two columns.
- **Admission.** Fiscal admission reads no knowledge. The 2.3 ms a command
  that the wide report adds is again the engine's cost of the knowledge held.
- **Settlement.** Planning and publishing the parts cost little next to the
  two writes of the fiscal state.
- **What grows with a run.** Two costs grow with the knowledge a run has
  published, and this profile, whose runs are a few boundaries long, does not
  show them. `knowledge_heads` makes two passes over the holder's whole
  ledger for each record published. And the engine carries all held
  knowledge through every boundary, at the rate above, while a ledger keeps
  superseded records, so every publication slows every later boundary. At the
  3.0 KiB a settling boundary publishes here, 10,000 fiscal boundaries add
  about 250 ms to each boundary. Both are costs of the engine that predate
  this change, which publishes about 35 times less than 0.14.0 did. They are
  left to a gap of their own. 0.16.0 removes both from the engine: settling
  a boundary no longer does engine work in proportion to held knowledge, and
  `knowledge_heads` reads an index of current records (see
  [knowledge ledger cost and authentication](knowledge-ledger-cost-and-authentication.md#measurements)).

### Persistence, replay, and format

`FiscalState` gains `report_plan`, omitted from the serialized state until a
report has been planned. The `fiscal_report` knowledge schema is at version 2,
with a new schema hash and the line roles among its subjects; version 1 is no
longer registered. A scenario whose knowledge carries a version-1 report
record is refused when the run is created, because the engine accepts no
record of a schema that is not registered. Aggregate IDs, a derived field, can
differ from 0.14.0 once aggregates are added to a state that has some.
`FiscalReportFact::id` is omitted where it is empty, which is inside a part. A
report record cites as its origin the state version its facts were computed
from, the last one its boundary proposed; 0.14.0 cited the first.

The fiscal state record references the person of every observer binding, and
its record schema declares the `observer` reference role. A state record with
observers that 0.14.0 encoded therefore has to be encoded again
(`FiscalState::into_record`). The aggregate system and the report system
declare the `canwu.core.person_availability` read.

## Compatibility

0.15.0 is a pre-1.0 minor release.

- **Formats.** Snapshot, commitment, checkpoint, and evidence-journal formats
  are unchanged. `FISCAL_RUNTIME_SCHEMA_VERSION` stays 2: every new field is
  optional and omitted while unused, as the acting-actor fields were when they
  were added. The payload of a fiscal state written by 0.14.0 decodes
  unchanged, with no audit bindings, no `rebound_by`, no `entered_by`, and no
  report plan, and encodes back to the same bytes; a unit test holds a 0.14.0
  state to that. The state record around the payload is not the same: its
  references now carry an `observer` entry for each observer's person, so a
  record that 0.14.0 encoded with observers is refused when the run is
  created, and is built again with `FiscalState::into_record`.
  The `fiscal_report` knowledge schema is at version 2.
- **Plugin identity.** The `canwu-fiscal` semantic hash changes, because
  settlement, admission, activation, and reports behave differently. The
  settlement system's descriptor declares one more event type, the aggregate
  and report systems one more read, the state record schema one more
  reference role, and the report schema has a new version, hash, and subject
  roles. Like any version change, 0.15.0 does not load 0.14.0 saves, so no
  report record of version 1 has to be read or migrated.
- **Source.** `FiscalAction` has two new variants, and
  `FiscalAuthorityBinding` and `FiscalAuditFinding` each have one new public
  field, so exhaustive matches and struct literals need one more arm or field.
  `FiscalState` has the new public field `report_plan`. Added:
  `FiscalAuditBinding`, `FiscalRebinding`, `FiscalActionIssuer`,
  `FiscalIssuerRole`, `FiscalState::audit_bindings`, the report types
  `FiscalReport`, `FiscalReportRecord`, `FiscalReportSummary`,
  `FiscalReportPart`, and `FiscalReportPlan`, the readers `fiscal_reports`
  and `assemble_fiscal_reports`, the limits `MAX_FISCAL_ACTIONS_PER_BOUNDARY`,
  `MAX_FISCAL_ACTION_JSON_BYTES`, `MAX_FISCAL_ACTION_TEXT_BYTES`,
  `MAX_FISCAL_IDENTIFIER_BYTES`, `FISCAL_REPORT_FACTS_PER_PART`,
  `MAX_FISCAL_REPORT_PARTS`, and `MAX_FISCAL_REPORT_RECORDS_PER_BOUNDARY`, and
  the stable reasons `FISCAL_AUTHORITY_BASIS_NOT_CURRENT`,
  `FISCAL_ISSUER_NOT_BOUND`, `FISCAL_BOUNDARY_ACTION_LIMIT`, and
  `FISCAL_STATE_BUDGET_EXHAUSTED`.
- **Behavior visible to existing runs.** Several actions admitted to one
  boundary now settle instead of all but the first being rejected as stale. An
  action is no longer stale because a receipt settled before it in the same
  boundary. An invalid fiscal catalog or state is refused at activation
  instead of failing the first event-bearing boundary. A binding may cite as
  its authority basis a record of an external evidence kind, which activation
  rejected before. Admission rejects as `InvalidPayload` an oversized request,
  over-long text or references, and a non-canonical ID of a new entry or
  accounting cycle, which settlement rejected as an outcome or which failed
  the boundary. A boundary that admits more actions than the state has outcome
  slots commits instead of failing.
- **Reports.** A reader that decoded one `fiscal_report` record as a
  `FiscalProjection` calls `fiscal_reports` instead. A boundary that changes
  nothing an observer can see publishes nothing to that observer, and a
  report's `as_of` is the time it last changed; 0.14.0 published every report
  whole in every boundary that changed the fiscal state. A holder with several
  observer bindings gets one report for each, under the binding's ID. An
  aggregate keeps its ID, and a fact its label, when later aggregates are
  added; 0.14.0 renumbered both in key order. A report larger than one record
  is published instead of failing its boundary. A sealed run supersedes the
  reports it published before the seal. An authored state is refused at
  activation when an assessment's unit or accounting cycle, or the namespace,
  name, and ID of a scope's domain-record institution together, is longer than
  `MAX_FISCAL_IDENTIFIER_BYTES`, and when an observer binding names a person
  who does not exist. A state record with observers has to be encoded by
  0.15.0, which adds their persons to its references. A report plan that comes
  with a scenario is ignored. Nothing is published to an observer whose actor
  is dead, where 0.14.0 failed the boundary.
