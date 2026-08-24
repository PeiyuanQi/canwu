# canwu-technology

`canwu-technology` is Canwu's published experimental technology simulation
domain extension. It models immutable technique revisions, evidence-bearing
attempts, holder-relative claims and observations, local capability,
implementation, use-specific adoption, and teaching opportunities.

The crate depends only on `canwu-api`. It deliberately provides no global
technology tree, era level, research-point currency, or automatic unlock.

Run the neutral end-to-end host example:

```text
cargo run -p canwu-technology --example technology_diffusion
```

The example uses tracked commands for deliberate changes and an exact-version
execution intent before result ingress accepts a provider-completed attempt;
a separate passive ingress records the measured observation. Phase 12 consumes
the intent atomically with the result and terminal operation before phase 13
may publish holder knowledge. The example then
qualifies practice, installs one implementation, adopts one use, links an
apprenticeship opportunity to the learner's program, and
proves snapshot restoration and exact replay. An opportunity does not grant
the learner knowledge automatically. Qualifications and implementations carry
explicit validity/install start times. Practice transmission cites the exact
qualification or implementation that made its source capable when the
opportunity opened, so neither a later installation nor later deactivation can
rewrite that history. A newly created opportunity must cite the source version
current in that boundary's replayed state; once created, only its own `active`
flag may change, and only from open to closed. Resuming requires a new
opportunity citing a source capability current at that time. An implementation
retains its exact installation-time qualification evidence; a later
qualification update does not implicitly stop the implementation, which must
be deactivated explicitly when it can no longer serve as a source.
New implementations must bind a qualification and assets that are still current
and active. Trial or committed adoption must likewise bind current active
implementations; an older exact version remains historical evidence, not fresh
authorization to install, recommit, or spread a practice.

Attempts and production runs must stay inside their authorized intent window.
Observations cannot predate their attempts, and assessments cannot predate the
claim or evidence they assess. Restore validation recomputes these causal cuts
and the raw payload continuation manifest instead of trusting decoded fields.

Program provider requirements are enforced when a provider result is
submitted. Authorization can therefore leave a pending intent whose eventual
result is rejected when the submitting provider does not satisfy the program.

Use `validate_technology_runtime` or the module-owned snapshot, checkpoint, and
replay wrappers when restoring persisted technology state. These checks resolve
the retained bodies of exact record versions, deeply verify technology-owned
holder knowledge, and re-run domain semantics after the kernel's structural and
commitment validation. Technology payloads declare older exact bodies needed by
future validation through the engine's payload-required continuation contract;
after sealing, reconstruct with the committed provider segment before continuing
authoritative work.

Capacity exhaustion settles deterministically as explicit plugin rejection
events. It does not leave admitted operations or knowledge publications queued
to poison every later boundary.

Runtime invention is a provider-created immutable `TechniqueRevision` bound to
an active investigation, adaptation, or reverse-engineering program, a pending
exact-version execution intent, and explicit discovery evidence. A normal
command cannot directly create or take ownership of result evidence.
