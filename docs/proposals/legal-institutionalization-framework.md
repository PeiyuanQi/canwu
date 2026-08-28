# Legal Institutionalization Framework

Status: proposed downstream extension after the first `canwu-culture`
implementation slice. This document designs how cultural signals can become
law without adding legal semantics to Canwu core or making culture a direct
state writer.

## Decision

Add an optional `canwu-law` domain extension. It consumes bounded,
evidence-bearing `CulturalSignalBatch` records, prepares institutional
`DecisionTicket` requests, and writes versioned legal records only after a
validated authorized command is admitted through canonical ingress.

`canwu-culture` remains responsible for cultural dispositions, propagation,
organization, and generic signals. `canwu-law` owns jurisdiction, legal
procedures, enacted rules, effective dates, repeal, and legal interpretation.
Political, election, administration, education, and enforcement extensions own
their respective processes and may consume enacted law.

The legal extension is not a magic conversion from a percentage of believers
to a statute. Cultural support is one evidence-bearing input to a procedure;
authority, jurisdiction, institutional rules, scarce capacity, opposition, and
the selected decision determine whether a law is enacted.

## Ownership and dependencies

```text
canwu-information / canwu-correspondence
              |
              v
        canwu-culture
              |
              +--> CulturalSignalBatch
                         |
                         v
                    canwu-law
              (proposal, procedure, law versions)
                         |
                         v
              canwu-decision + canwu-api + canwu-event
                         |
                         v
        election / administration / education / justice adapters
```

`canwu-law` may depend on `canwu-culture`, `canwu-decision`, and `canwu-api`.
The core, `canwu-api`, and `canwu-society` must not depend on legal semantics.
The law extension can also accept generic social evidence from another
compatible provider when the signal contract and plan hash are declared in the
run manifest.

## Legal records

The first implementation should keep a small set of records with explicit
ownership. The payload status below is legal meaning; the generic
`DomainRecordLifecycle` still controls record persistence.

### Jurisdiction and institution

`LegalJurisdiction` binds a stable jurisdiction ID to its parent jurisdiction,
territory scope, competent institutions, and procedure profile. A
`LegalInstitution` binds an institution entity to the jurisdiction, authority
seats, quorum, vote or appointment rules, and the command subject that may
adopt a law. These are domain records, not new core entity kinds.

### Legal proposal

`LegalProposal` is a non-enacted, versioned proceeding input. It contains:

- proposal identity, jurisdiction, sponsor, and subject references;
- a bounded list of typed clause or eligibility-rule descriptors;
- the required procedure and deadline;
- status such as `draft`, `submitted`, `deliberating`, `adopted`, `rejected`,
  `expired`, or `withdrawn`;
- source `CulturalSignalBatch` references and other evidence;
- the `DecisionTicket` identity and option version when a decision is open.

The proposal may cite a cultural target generation, but that citation is
evidence. It does not give the culture extension authority to mutate the
proposal or enact the law.

### Enacted law version

`LawVersion` is the authoritative legal result. It contains:

- stable law identity and monotonic version;
- jurisdiction and bounded scope;
- typed clauses or eligibility rules;
- `effective_from` and optional `effective_until` simulation times;
- adoption institution, authorized decision origin, and procedure result;
- evidence and causal references to the accepted proposal and decision;
- explicit successor, amendment, repeal, or expiry links when applicable.

An enacted law is not deleted when a culture target retires. A repeal or
amendment is a new legal command and a new versioned record change.

## Cultural signal to law

The bridge uses the existing next-boundary ingress contract:

1. Information and correspondence resolve access and interpretation, then
   produce a bounded `CultureExposureSignalBatch`.
2. Culture settlement updates awareness, assent, practice, public alignment,
   organization tie, and mobilization state. It emits a bounded
   `CulturalSignalBatch` containing target generation, scope, strength,
   persistence, and evidence; cadence remains part of the compiled effect
   binding.
3. `canwu-law` uses reverse indexes from signal kind and scope to affected
   jurisdictions and open proposals. It evaluates only dirty proceedings.
4. The law extension creates or refreshes a `DecisionTicket` with legal options
   such as adopt, amend, reject, defer, or refer to another institution. The
   ticket contains actor-relative facts and evidence, not mutable runtime state.
5. An authorized controller selects an existing option. The decision attempt
   and trace are persisted; a selected option enqueues a validated legal
   command through canonical ingress.
6. The legal command validates jurisdiction, institution authority, procedure,
   version guards, effective dates, clause limits, and evidence references. It
   then creates or updates `LegalProposal` and `LawVersion` records atomically.
7. Election, administration, education, justice, or other downstream adapters
   consume the enacted law at their own declared boundary. Their results emit
   new evidence and bounded feedback signals; they do not rewrite the law
   record without a legal command.

The authoritative path is therefore:

```text
CulturalSignalBatch
  -> LegalProposal / DecisionTicket
  -> authorized DecisionAttempt and DecisionTrace
  -> validated legal command ingress
  -> LawVersion
  -> downstream enforcement and feedback
```

## Effect persistence and retirement

The culture effect persistence class determines what the legal extension may
do with a signal:

| Culture effect | Legal interpretation |
| --- | --- |
| `Pulse` | Opens or updates a proposal opportunity; no durable law exists yet. |
| `Level` | Supplies current support or legitimacy pressure. If it ends, law may review the proposal, but repeal is never automatic. |
| `Commitment` | Once accepted by the legal command, becomes provenance for a durable `LawVersion`; culture retirement does not retract it. |
| `Evidence` | Cited history only; it cannot directly open or mutate a legal proceeding. |

A culture target cannot retire while an unadmitted legal effect batch, a live
level dependency, or a scheduled legal continuation requires its current
generation. A proposal that merely cites an already accepted commitment as
evidence does not keep the culture target hot. This distinction lets
`women_political_equality` retire after a suffrage rule is enacted while the
`VotingEligibilityRule` remains active.

If a legal rule intentionally depends on a live cultural level, the legal
extension must record that dependency and define its own review, expiry, or
renewal rule. The culture runtime only emits the end of the level; it does not
silently repeal the law.

## Authority and procedure

Legal commands must use the existing authority chain:

```text
legal facts -> DecisionTicket -> controller selects option
  -> DecisionAttempt / DecisionTrace
  -> canonical command ingress
  -> legal authority and procedure validation
  -> LawVersion commit
```

The command subject must be the competent legal institution. The command
context must carry the validated `decision_controller_id`, decision origin,
seat, permission profile, request identity, and expected revision/time guards.
An external service or LLM may recommend an option, but it cannot manufacture
legal authority or submit a raw law payload that bypasses the ticket.

Multi-boundary procedures use a persisted `LegalProposal` and explicit
continuation ingress. Quorum, vote counts, vetoes, signatures, ratification,
and effective-date delays are deterministic domain data. A failed step is an
expected rejected attempt or an atomic boundary rollback, not a partial law.

## Persistence, knowledge, and replay

- Legal records implement `DomainRecordType` and use strict schemas, typed
  references, versioned payloads, and explicit mutation policies.
- Every enacted version cites the accepted proposal, decision trace, cultural
  signal evidence, and any procedure records through `CauseRef` or domain
  references.
- Public law can be exposed through a domain projection; private drafts,
  dissent, and actor-specific knowledge use `ViewerContext` and the holder
  ledger rather than a truth fallback.
- Exact replay consumes recorded decision ingress, command ingress, boundary
  records, and evidence. It does not rerun a human, service, or model policy.
- Repeal and amendment are append-only legal results. Historical law versions
  and their evidence remain queryable under the normal archive contract.

## Scale and bounded work

The legal extension must not build a target-by-proposal-by-institution
cross-product. It should compile:

- numeric IDs for jurisdictions, institutions, proposals, clauses, and
  procedure profiles;
- reverse indexes from signal kind/scope to affected proceedings;
- dirty proposal and jurisdiction sets for each boundary;
- per-procedure limits for clauses, evidence, options, fan-out, and pending
  continuations;
- a plan hash and budget manifest bound to the run.

For `P_delta` dirty proposals, `C_delta` affected clauses, and `V_delta`
observer entries, the intended steady-state work is approximately
`O(P_delta + C_delta + V_delta)`, subject to declared bounded fan-out. A
catalog of historical laws or retired cultural targets must not change active
proposal settlement cost.

## Examples

### Women's suffrage

The culture package defines `women_political_equality` and emits public
alignment, organization capacity, and legitimacy-pressure signals for a
jurisdiction. The law extension opens a proposal with options to adopt,
restrict, defer, or reject a voting-eligibility rule. A competent assembly or
constitutional authority selects an option through `DecisionTicket`. After
procedure validation, `LawVersion` writes a `VotingEligibilityRule` with a
deterministic effective date. The election adapter reads that rule. Retiring
the culture target later stops new propagation but leaves the enacted rule and
its enforcement history intact.

### Human-rights principles

The content pack can define separate targets such as universal dignity,
equality before law, due process, and expression rights. Each may feed
different legal proposals. The law extension decides whether a specific
jurisdiction can adopt a clause under its own procedure; no single aggregate
"human-rights percentage" automatically becomes a constitution.

## Conformance gates

Before calling the legal extension complete, add tests for:

- cultural signals cannot directly mutate legal records;
- only a controller-bound authorized command can enact, amend, or repeal law;
- stale proposal, ticket, and law versions are persisted as safe rejections;
- a commitment accepted into law survives culture retirement;
- a live level dependency blocks retirement until it is resolved;
- proposal, law, evidence, archive, snapshot, fork, and exact replay remain
  consistent;
- unrelated cultural targets, retired targets, and observers do not perturb
  keyed legal results;
- work and memory follow dirty proposals and declared procedure budgets.

## Implementation order

1. Define jurisdiction, institution, proposal, procedure, and law-version
   schemas with strict cardinality and clause budgets.
2. Add a signal bridge that validates culture plan hashes, target generations,
   scope, persistence class, and evidence before admission.
3. Build `DecisionTicket` generation and legal command authority validation on
   the existing `canwu-decision` and `canwu-api` contracts.
4. Implement atomic proposal/adoption/amendment/repeal records and exact
   replay, including effective-time validation.
5. Add election/administration/education/justice adapters and actor-relative
   legal projections.
6. Add a benchmark with active, dirty, pending, adopted, repealed, and
   historical law populations before considering sharded legal persistence.

The legal extension remains a downstream ruleset. A historical first-party
case may instantiate rights or suffrage content only after these generic
contracts pass their conformance and benchmark gates.
