# Technology and Historical Research Framework: Single-Milestone Plan

This is one delivery milestone. Items are execution order, not separate releases
or postponed scope.

## Plan and review

- [x] Start an isolated worktree at the latest verified `origin/main`.
- [x] Write the initial implementation specification.
- [x] Obtain engine, home-hardware, and historical-simulation plan reviews.
- [x] Resolve the first review's blocking findings in the specification.
- [x] Carry the resolved plan into independent implementation review.

## Generic host surface

- [x] Add revision-bound `DomainRecordPage` queries to `Simulation` and `Canwu`.
- [x] Implement kind-range plus overlay-page merging without copying unrelated
  records; prove the selected page remains bounded among 100,000 unrelated records.

## Technology extension

- [x] Add published experimental `canwu-technology`, depending only on `canwu-api`.
- [x] Implement bounded typed records, immutable revisions, exact version refs,
  schemas, deterministic reference evaluator, and semantic validation.
- [x] Implement tracked command operations, passive/provider ingress, phase
  handlers, versioned execution intents with atomic phase-12 consumption,
  exact retry/collision behavior, actor/institution knowledge, and read-safe
  queries.
- [x] Implement module-owned snapshot/checkpoint/fork/replay restore validation.
- [x] Reserve terminal outcomes inside the 5,000-record cap; enforce every
  64-entry collection and reject future attempt, claim, and production facts.
- [x] Recompute payload continuation manifests, enforce causal evidence cuts and
  current active dependencies, and settle capacity overflow as durable rejection
  events without poisoning later boundaries.
- [x] Declare payload-required exact-version continuations, prove
  provider-backed compact archive reconstruction can continue, and deeply
  validate module-owned holder knowledge during restoration.
- [x] Add one runnable neutral flow through intent, provider attempt,
  qualification, implementation, use adoption, teaching opportunity, restore,
  fork, and replay; provider requirements remain explicit extension inputs.

## Historical research suite

- [x] Add published experimental `canwu-history-research` with separately selectable
  sources, practice, and production-archaeology assessment plugins.
- [x] Add bounded assessment ingestion, contradiction/supersession, exact
  evidence links, O(1) unrelated-ingress exit, and host-side read-only analysis.
- [x] Restrict contradiction/supersession to prior assessments of the same exact
  subject, enforce as-of evidence time and bounded dating, and persist expected
  command/capacity rejection paths.
- [x] Prove that disabling history plugins leaves base technology truth and
  outcomes unchanged and that manifest mismatch fails closed.

## Cross-validation and performance

- [x] Run all five profiles through catalog, command, result-ingress, boundary,
  and authoritative-state paths; separately prove their named counterfactuals
  with the same data-driven evaluators.
- [x] Prove authority, failed/successful attempts, explicit qualification and
  adoption, teaching without automatic knowledge, exact historical versions,
  save/load, fork, exact replay, and manifest mismatch.
- [x] Extend the standalone harness with interactive and bounded pressure
  profiles, all-history-plugin coverage, 20 years of real monthly work,
  file persistence/reconstruction/replay/RSS,
  and a separate 100,000-unrelated-record structural paging test.

## Documentation and delivery

- [x] Align the complete first-party crate DAG, architecture, end-state,
  versioning, terminology, crate READMEs,
  benchmark docs, and one bilingual website tutorial/navigation entry.
- [x] Run all repository, public-example, Rustdoc, benchmark, and website checks.
- [x] Obtain independent code review and separate bilingual website-copy review;
  resolve all blockers.
Release gate (performed after this source checklist is committed): refresh from
latest `origin/main`, keep one conventional feature commit, push without force,
create and merge one PR, wait for Pages, and verify live pages plus a deployed
asset.
