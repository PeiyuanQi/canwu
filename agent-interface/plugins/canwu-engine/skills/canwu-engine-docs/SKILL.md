---
name: canwu-engine-docs
description: Read, locate, compare, summarize, and explain Canwu's official tutorials and design documentation. Use when a user asks how to start or use Canwu, follow a foundation tutorial or scenario case, understand a public canwu-api example or an official experimental domain extension such as the social diffusion simulation module, implement DecisionTicket/controller/policy workflows, choose or translate canonical Chinese-English Canwu terminology, or investigate Canwu architecture, end-state design, engine conformance, versioning, deterministic time, actor-relative knowledge, plugins, persistence, replay, or renderer integration. Also use for requests in any language to find, cite, compare, or summarize Canwu documentation, tutorials, architecture, or design notes.
---

# Read Canwu Documentation

Base answers on the current official Canwu documentation instead of memory.
Keep code guidance on the public `canwu-api` boundary.

## Route the question

1. Read [references/documentation-map.md](references/documentation-map.md) and
   select the smallest useful source set.
2. Classify the request:
   - Use tutorials plus their runnable examples for onboarding and how-to
     questions.
   - Use design contracts for architecture, invariants, and rationale.
   - Use `docs/terminology.md` for canonical Chinese-English terms and their
     distinctions.
   - Use `canwu-api` source or rustdoc for exact current types, methods, and
     signatures.
   - Use the overview documents only when the user asks for a broad tour.
3. Prefer a current local Canwu checkout. Recognize the repository root by
   `Cargo.toml`, `docs/architecture.md`, and `crates/canwu-api`. If no checkout
   is available, open the official Canwu or GitHub URLs in the map.
4. Search before loading long files. In a checkout, start with a focused search
   such as:

   ```text
   rg -n "<term>" README.md README.zh-CN.md docs website/src/content/docs crates/canwu-api
   ```

5. Read the relevant section and enough surrounding context to preserve its
   qualifications. Never claim that a document says something without opening
   it during the task.
6. Match the user's language. Prefer the Chinese tutorial source for Chinese
   questions and the English source for English questions. Explain English-only
   design documents in the user's language while preserving API identifiers.

## Apply authority by claim type

- Treat `docs/architecture.md` and `docs/end-state.md` as the source of truth
  for dependency boundaries and intended design.
- Treat current `canwu-api` public source, rustdoc, and examples as the source
  of truth for exact callable interfaces.
- Use `docs/engine-conformance.md` for named reusable-engine requirements and
  their required evidence.
- Use `docs/versioning.md` for SemVer, snapshots, migrations, replay, and
  compatibility claims.
- Use `docs/terminology.md` as the source of truth for public Chinese-English
  naming. Keep code identifiers unchanged.
- Use the website tutorials for learning sequence and concise workflows, then
  inspect the corresponding example before giving detailed code guidance.
- Treat README material as orientation, not as a replacement for a more
  specific contract.

If sources disagree, state the mismatch. Prefer the design contracts for an
architectural boundary and the current public API for an exact signature; do
not silently combine incompatible statements.

## Preserve Canwu boundaries

- Recommend public `canwu-api` entry points for kernel capabilities. An official
  experimental domain extension may be used only when its documentation
  explicitly says it is built on `canwu-api`; preserve its unpublished and
  unstable status.
- Keep rendering, wall time, input devices, audio, and presentation state in
  the host application.
- Use actor-relative reads for actor-facing workflows. Do not replace missing
  knowledge with omniscient world state.
- Discover legal actions with `available_actions` before `act` when guiding an
  actor or AI agent.
- Never instruct a client to obtain mutable live simulation state or depend on
  `canwu-sim` directly.

## Answer from the sources

- Lead with the direct answer.
- For a tutorial question, include the runnable command, the minimal workflow,
  and the next relevant tutorial or example.
- For a design question, name the invariant, explain why it exists, and identify
  the affected public surface.
- Cite the exact document paths or official pages and name the relevant
  sections. Keep quotations short and prefer precise paraphrases.
- Distinguish documented behavior from an inference. If a required source
  cannot be opened, say so instead of presenting memory as documentation.
