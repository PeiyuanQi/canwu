# Social Belief Framework Implementation TODO

Status: baseline implementation and independent review complete. The first
culture authoring and lifecycle implementation slice is also complete; its
boundary-driven ingress, incremental settlement, and benchmark work remain
follow-up items in the [culture authoring SDK and lifecycle design](culture-authoring-sdk-and-lifecycle.md).

This checklist implements only the generic `canwu-society` social diffusion
simulation module described in
[`social-belief-framework.md`](social-belief-framework.md). The three historical
comparisons are design stress tests only. They are not examples, fixtures,
or CM content. The implementation includes exactly one neutral composite
tutorial case; it does not implement the three comparisons.

## 1. Approval and baseline

- [x] Receive explicit approval to start the generic implementation.
- [x] Refresh Canwu `main`, create an isolated worktree, preserve concurrent
      changes, and confirm the merged status of DecisionTicket and generic
      actor-knowledge / information work.

## 2. Contract and crate boundary

- [x] Add the approved direction to `docs/architecture.md`, `docs/end-state.md`,
      and the relevant conformance section.
- [x] Define the minimal public and internal types, stable errors, schema
      versions, ownership, cadence, phases, and random streams.
- [x] Add the published `crates/extensions/canwu-society` social diffusion simulation
      module without creating a dependency from Canwu core or `canwu-api` back
      to it.

## 3. Authoritative data model

- [x] Implement cohort records, lifecycle validation, and module-owned affiliation
      target references.
- [x] Implement sparse multidimensional disposition distributions and
      deterministic remainders keyed by stable transition identity.
- [x] Implement sparse social influence edges and generic organization topology.
- [x] Implement institutional alignment and orthogonal policy pressure records.

## 4. Settlement behavior

- [x] Implement deterministic aggregate transition calculation using integer
      headcounts and canonical ordering.
- [x] Enforce conservation, reference validity, bounds, ownership, and atomic
      rollback at the boundary.
- [x] Implement influence and organization traversal with explicit bounds and no
      dense world-sized matrix.
- [x] Apply institutional and policy inputs without directly rewriting private
      assent, and produce mobilization candidates without creating conflict.
- [x] Emit causal evidence for every committed authoritative change.

## 5. Decisions and actor knowledge

- [x] Connect named-person and institution choices through DecisionTicket's
      public controller-neutral API; never create per-person cohort tickets.
- [x] Publish authorized estimates through the existing `ViewerContext` boundary
      while keeping ground truth, knowledge, private assent, public alignment,
      and presentation separate; forbid fallback to raw society records.

## 6. Focused generic verification

- [x] Add tests for conservation, persisted remainders, target ancestry
      validation, unrelated-target isolation, exact derived materialization,
      and payload-to-core-reference binding.
- [x] Add tests for public/private divergence, institutional adoption without
      instant conversion, authority rejection, engine-issued decision
      provenance, and valid EPOCH / negative-time boundaries. Mobilization and
      conflict remain separated by the crate's type and ownership boundary.
- [x] Add actor-privacy, domain-extension-validated save/load, replay, fork, and plugin
      rehydration tests; rely on the engine's workspace tamper and rollback
      suites for the generic record and boundary substrate.
- [x] Add neutral structural sparse-scaling workloads proving that active
      signal and narrow observer-projection indexes grow with active outputs and
      inactive target catalog growth does not materialize a dense cohort/target
      matrix; organization traversal is explicitly capped by validated passes.

## 7. Single tutorial case

- [x] Add one runnable neutral example, “local community diffusion and
      institutional response,” covering cohorts, influence, one institutional
      decision, public/private divergence, actor-relative estimation, and
      domain-extension-validated save/load, fork, and exact replay.
- [x] Document that same example as one bilingual tutorial case using the
      repository's tutorial navigation; do not create additional cases or a
      historical mapping.

## 8. Completion gate

- [x] Run formatting, clippy, workspace tests, debug-client check, rustdoc,
      public examples, the structural sparse workload, website build, skill
      validation, and `git diff --check`.
- [x] With explicit authorization, complete the repository-required independent
      review before any commit.
- [x] Stop after the generic system and single tutorial are verified. Historical
      cases, CM integration, additional tutorials, publication, and core
      promotion require separate approval.

## Current stop point

- [x] Generic architecture designed.
- [x] Implementation TODO reduced to the actual requested scope.
- [x] User approval received.
- [x] Proper implementation completed and verified.
- [x] Independent review authorized and completed.
- [x] Commit and push requested.

## Follow-up: culture authoring and retirement

- [x] Define a serializable `CultureDefinition` authoring schema and structured
      cardinality/fan-out budget errors.
- [x] Compile definitions into an immutable, hash-bound execution plan with
      numeric IDs, reverse indexes, and bounded adjacency.
- [ ] Integrate dirty-set settlement with incremental society aggregate/projection
      refreshes; the public dirty-set API and current full-state path now exist.
- [x] Add `Active`, `Dormant`, and `Retired` target lifecycle records,
      generation-bound tombstones, explicit reactivation, complete persisted
      runtime hydration, and atomic society lifecycle synchronization.
- [ ] Add next-boundary information-to-culture inputs and batched
      culture-to-domain effect outputs.
- [ ] Add society/culture benchmarks proving retired catalog growth does not
      change active settlement cost and that work follows dirty state.
- [ ] Add a historical content case only after the generic SDK and lifecycle
      conformance gates pass.

## Follow-up: legal institutionalization

- [ ] Define jurisdiction, legal institution, proposal, procedure, and law
      version schemas as a downstream `canwu-law` extension.
- [ ] Bridge `CulturalSignalBatch` to dirty legal proposals and
      controller-bound `DecisionTicket` options.
- [ ] Commit, amend, repeal, and expire law versions through validated
      canonical commands with causal evidence and exact replay.
- [ ] Add actor-relative legal projections and election, administration,
      education, and justice adapters.
- [ ] Add legal active/dirty/procedure benchmarks before historical rights or
      suffrage content is promoted to a first-party case.
