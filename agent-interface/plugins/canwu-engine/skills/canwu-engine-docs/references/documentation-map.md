# Canwu documentation map

Use this file only to choose sources. Read the selected source before answering.
Resolve local paths from the Canwu repository root, not from this skill folder.

## Entry points

| Need | Local source | Official fallback |
| --- | --- | --- |
| English project overview and quick start | `README.md` | `https://github.com/PeiyuanQi/canwu/blob/main/README.md` |
| Chinese project overview and quick start | `README.zh-CN.md` | `https://github.com/PeiyuanQi/canwu/blob/main/README.zh-CN.md` |
| Repository documentation index | `docs/README.md` | `https://github.com/PeiyuanQi/canwu/blob/main/docs/README.md` |
| Canonical Chinese-English terminology and distinctions | `docs/terminology.md` | `https://canwu.org/reference/terminology/` |
| Chinese tutorial index | `website/src/content/docs/tutorials/index.mdx` | `https://canwu.org/tutorials/` |
| English tutorial index | `website/src/content/docs/en/tutorials/index.mdx` | `https://canwu.org/en/tutorials/` |

## Tutorials and examples

| Question | Tutorial source | Detailed or executable source |
| --- | --- | --- |
| First successful action, explicit time advancement, or actor-relative knowledge | Chinese: `website/src/content/docs/tutorials/move-army.mdx`; English: `website/src/content/docs/en/tutorials/move-army.mdx` | `crates/integrations/canwu-reference-world/examples/starter.rs` |
| Canonical movement, active person self-movement, letter cargo, authority, custody, or arrival delivery | `docs/proposals/movement-order-mechanism.md`; `docs/architecture.md` | `crates/api/canwu-api/src/lib.rs`, `crates/runtime/canwu-sim/src/runtime/mod.rs`, and `crates/runtime/canwu-sim/src/runtime/scheduling.rs` |
| Pause, speed controls, FPS independence, wall/simulation/presentation clocks, or interpolation | Chinese: `website/src/content/docs/tutorials/continuous-game-loop.mdx`; English: `website/src/content/docs/en/tutorials/continuous-game-loop.mdx` | `docs/continuous-game-loop.md` and `crates/api/canwu-api/examples/continuous_game_loop.rs` |
| Schema-validated plugin commands, issuer checks, or declared state access | Chinese: `website/src/content/docs/tutorials/command-plugin.mdx`; English: `website/src/content/docs/en/tutorials/command-plugin.mdx` | `crates/api/canwu-api/examples/plugin.rs` |
| Daily or turn boundaries, supply and demand, reservation, allocation, or multi-system settlement | Chinese: `website/src/content/docs/tutorials/phased-boundary.mdx`; English: `website/src/content/docs/en/tutorials/phased-boundary.mdx` | `crates/api/canwu-api/examples/phased_boundary.rs` |
| DecisionTicket, controller binding, dynamic options, utility evaluation, Human/External/LLM policy boundaries, decision traces, or a neighboring warlord's military-aid request | Chinese: `website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx`; English: `website/src/content/docs/en/tutorials/cases/warlord-aid-decision.mdx` | `crates/api/canwu-api/examples/decision_ticket.rs` and `crates/api/canwu-api/tests/decision_framework.rs` |
| Social diffusion simulation module: aggregate social or belief diffusion, cohorts, multidimensional disposition, social influence, organization topology, institutional policy, public/private divergence, or actor estimates | Chinese: `website/src/content/docs/tutorials/cases/local-community-diffusion.mdx`; English: `website/src/content/docs/en/tutorials/cases/local-community-diffusion.mdx` | `docs/proposals/social-belief-framework.md`, `crates/extensions/canwu-society/examples/local_community_diffusion.rs`, and `crates/extensions/canwu-society/tests/framework.rs` |
| Confidential delivery, nonexclusive access, copy/derivative lineage, intended delivery after interception, audience release, or holder-isolated knowledge | Chinese: `website/src/content/docs/tutorials/cases/confidential-copy-release.mdx`; English: `website/src/content/docs/en/tutorials/cases/confidential-copy-release.mdx` | `crates/extensions/canwu-information/examples/confidential_copy_release.rs` and `crates/extensions/canwu-information/tests/case_conformance.rs` |
| Ephemeral channels, interception without a persistent primary instance, failed and delegated interpretation, performer/commissioning-holder separation, or restricted review distribution | Chinese: `website/src/content/docs/tutorials/cases/encoded-interception.mdx`; English: `website/src/content/docs/en/tutorials/cases/encoded-interception.mdx` | `crates/extensions/canwu-information/examples/encoded_interception.rs` and `crates/extensions/canwu-information/tests/runtime_adapter.rs` |
| Fiscal procedure: scope ledgers, accounting cycle books, action batches and per-record revisions, receipts, the fiscal archive, appointment policies, observers and reports, report cadence and institution granularity, the report share and retention knobs | Chinese: `website/src/content/docs/architecture/fiscal.mdx` and `website/src/content/docs/tutorials/cases/ming-fiscal.mdx`; English: `website/src/content/docs/en/architecture/fiscal.mdx` and `website/src/content/docs/en/tutorials/cases/ming-fiscal.mdx` | `docs/ming-fiscal-case.md`, `docs/proposals/world-scale-fiscal.md`, `docs/proposals/fiscal-observers-at-scale.md`, `docs/proposals/fiscal-and-society-limits.md`, and `crates/integrations/canwu-ming-fiscal-reference/src/lib.rs` |
| Knowledge publication: record bodies and holder entries, grouped `PublishKnowledge` directives, the publication budget, retention, one event per directive | Chinese: `website/src/content/docs/architecture/knowledge-information.mdx`; English: `website/src/content/docs/en/architecture/knowledge-information.mdx` | `docs/proposals/fiscal-observers-at-scale.md`, `docs/architecture.md` ("Knowledge model"), `crates/runtime/canwu-sim/src/runtime/knowledge.rs`, and `crates/model/canwu-knowledge/src/ledger.rs` |
| Resource skipped packets and deferrals, the production archive, output-key refusals | Chinese: `website/src/content/docs/architecture/resources-production.mdx`; English: `website/src/content/docs/en/architecture/resources-production.mdx` | `docs/proposals/resource-refusals-without-stalls.md` and `docs/proposals/production-archive-volume.md` |
| Continuation checkpoints, sealing with work in flight, pinned evidence, journal frontiers, record-value chunks | Chinese: `website/src/content/docs/developer/persistence.mdx`; English: `website/src/content/docs/en/developer/persistence.mdx` | `docs/proposals/continuation-checkpoint.md`, `docs/proposals/record-store-ceiling-and-cost.md`, and `crates/api/canwu-api/src/lib.rs` |
| Society transfer settlement mode and the society queue | Chinese: `website/src/content/docs/tutorials/cases/local-community-diffusion.mdx`; English: `website/src/content/docs/en/tutorials/cases/local-community-diffusion.mdx` | `docs/proposals/fiscal-and-society-limits.md` and `crates/extensions/canwu-society/src/plugin.rs` |

When no checkout is available, follow links from the corresponding official
tutorial index or open the example under
`https://github.com/PeiyuanQi/canwu/tree/main/crates/api/canwu-api/examples`.

## Design contracts

| Topic | Primary source | Useful sections or companion source |
| --- | --- | --- |
| Headless boundary, dependency direction, world/time/events, public interfaces, knowledge, plugins, settlement, renderer integration | `docs/architecture.md` | Match the section heading to the user's term; inspect `crates/api/canwu-api` for exact APIs |
| Intended product surfaces, historical state, causality, decisions/controllers, persistence, counterfactuals, geography, plugins, debug client | `docs/end-state.md` | Use for design intent and destination, then confirm implemented behavior in current sources |
| Named engine requirements and acceptable evidence | `docs/engine-conformance.md` | Requirements E01-E16 and the conformance evidence section |
| SemVer, snapshot formats, migrations, hashing, replay, and supported platforms | `docs/versioning.md` | Use for compatibility claims; do not infer compatibility from examples |
| Generic holder-relative information flow, publication, evidence, keyed randomness, compact continuation, and the neutral lifecycle extension | `docs/proposals/information-flow-mechanism.md` | Use the implementation checklist only to verify milestone status: `docs/proposals/information-flow-mechanism-todo.md` |
| Knowledge ledger cost and commitment format 5, engine phase profile, command admission history index | `docs/proposals/knowledge-ledger-cost-and-authentication.md`, `docs/proposals/reference-archive-and-replay-profile.md`, `docs/proposals/command-admission-history-index.md` | `docs/versioning.md` 0.16.0 entry for the measurements and known limits, and its 0.17.0 entry for what changed since (commitment format 6, the publication budget, retention) |
| Shared publication, the publication budget, knowledge retention, chunked commitment format 6, fiscal report cadence and institution rows, the continuation gate | `docs/proposals/fiscal-observers-at-scale.md` | `docs/versioning.md` 0.17.0 entry for the formats, API delta, upgrade notes, measurements, and known limits; `docs/releasing.md` for the gate |
| Law provider index, information delegated authority, military draw evidence after a seal | `docs/proposals/law-information-military-correctness.md` | `docs/architecture-culture-law.md`; website `architecture/culture-law.mdx`, `architecture/knowledge-information.mdx`, and `architecture/military.mdx` in both languages |
| Request outcome retention and retired request IDs; initial declarations checked at load, boundary ingress without an early refresh, publication reservations, request ID 0 | `docs/proposals/request-outcome-retention.md`, `docs/proposals/engine-load-and-budget-guards.md` | `docs/versioning.md` 0.18.0 entry (continuation format 3, upgrade notes, known limits); website `developer/persistence.mdx` and `architecture/knowledge-information.mdx` in both languages |
| Recurring departures, edge conditions, the compiled routing index (`canwu-routing.v2`) | `docs/proposals/recurring-traversal.md` | website `architecture/routing-movement.mdx` in both languages; `crates/mechanisms/canwu-routing/examples/routing_scale.rs` |
| Society intergroup relations, cohort classes, outcome retention, one write per settlement | `docs/proposals/society-intergroup-relations.md` | `docs/architecture.md` society section; website `architecture/settlement.mdx` and the local-community-diffusion case in both languages |
| Aggregate caseload, caseload profile, the law plan record written once, provider index default, delegation ledger expiry | `docs/proposals/law-aggregate-caseload.md` | website `architecture/culture-law.mdx` and `architecture/knowledge-information.mdx` in both languages |
| Fiscal archive helpers on the run, the sealed cut from `sealed_evidence_cursor`, the compact report part (`fiscal_report` schema 6), seal at each save | `docs/proposals/fiscal-archive-and-due-instant-cost.md` | website `architecture/fiscal.mdx` in both languages; `benchmarks/README.md` world-scale fiscal profile |
| Resource consumption batches, live buckets, custodian index, finished leases leaving the root, host expiries | `docs/proposals/resource-series-scale.md` | website `architecture/resources-production.mdx` in both languages; `crates/extensions/canwu-resource/README.md` |
| Reference package archive receipt chain and operations skipped for room | `docs/proposals/reference-archive-receipts.md` | `docs/architecture.md` reference integrations; website `architecture/resources-production.mdx` in both languages |
| Production scope roots, per-root limits, the batch command, residue archives, folded incident receipts, refusals with a trace | `docs/proposals/production-series-scale.md` | `docs/architecture.md` production paragraphs; website `architecture/resources-production.mdx` and `developer/persistence.mdx` in both languages |
| Runtime state copies: persistent current state and scheduler, shared journals, constant-time `fork`, the journal copy on the first append, the cheaper restore | `docs/proposals/runtime-state-copies.md` | `docs/architecture.md` fork-and-save paragraph; website `developer/persistence.mdx` in both languages; `benchmarks/README.md` paged-record and continuation profiles |
| The paged restore budget and assembled bytes, the paged knowledge ledger and chunked compact body (paged format 7), the seal that hashes each record once | `docs/proposals/paged-restore-budget-and-seal-cost.md` | `docs/versioning.md` 0.18.0 entry; website `developer/persistence.mdx` (paged checkpoints) in both languages |
| Movement leg holds (`DelayLeg`, `MovementLegHoldV1`, the scheduled arrival), incident evidence and delivery attempts that must be current, the checked arrival due of a departed leg | `docs/proposals/movement-delay-leg.md` | `docs/versioning.md` 0.19.0 entry; `docs/architecture.md` movement paragraphs; website `architecture/routing-movement.mdx` (leg timing) in both languages; `crates/extensions/canwu-movement/README.md` |
| Listing pending ingress after a seal or a continuation restore (`pending_ingress` on `Canwu`, `CompactedCanwu`, and `CanwuRun`), a trusted-host read | `docs/proposals/pending-ingress-read.md` | `docs/versioning.md` 0.19.0 entry; `docs/architecture.md` sealing paragraph; website `developer/persistence.mdx` (continuation checkpoints) in both languages |
| Partial transfer loss (`RecordTransferLoss`), the transfer link history, account rescope (`enqueue_resource_account_rescope`), tracked external inflow under a bound lease, lease operation bindings (`CompletionOperationBindingV1`), two local acceptances on one handover record | `docs/proposals/resource-transit-place-and-inflow.md` | `docs/versioning.md` 0.19.0 entry; `crates/extensions/canwu-resource/README.md`; website `developer/production-economy.mdx` and `architecture/resources-production.mdx` in both languages |

Official design fallbacks:

- `https://github.com/PeiyuanQi/canwu/blob/main/docs/architecture.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/end-state.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/engine-conformance.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/versioning.md`
- `https://github.com/PeiyuanQi/canwu/blob/main/docs/continuous-game-loop.md`

## Exact API questions

Start at `crates/api/canwu-api/src/lib.rs` and search for the named public type or
method. Use the matching example to show composition. Do not use
`crates/runtime/canwu-sim` as client documentation, and do not infer a public guarantee
from a private implementation detail.
