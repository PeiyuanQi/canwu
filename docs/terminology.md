# Canwu Terminology / 参伍术语表

This document is the canonical Chinese-English terminology reference for
Canwu's public architecture, integration guides, tutorials, and agent-facing
documentation. It covers stable concepts that recur across those surfaces; it
does not attempt to translate every Rust identifier or temporary proposal term.

Use the Chinese term in Chinese prose and the English term in English prose.
Keep code identifiers exactly as written. On first use, write the Chinese term
followed by the English term or code identifier when that helps disambiguate it,
for example, **模拟插件** (`simulation plugin`) or **决策票据**
(`DecisionTicket`).

Linked terms already have a more detailed guide or architecture page. Unlinked
terms are currently defined only here and in their code-level API documentation.

Each concept has one preferred public term. The consolidated wording table below
marks alternatives as deprecated or restricted: deprecated terms must not be
introduced in new prose, while restricted terms remain valid only in the stated
technical context. Existing code identifiers and historical records are not
renamed by this policy.

## Product and runtime boundaries

| 中文规范写法 | English | Code or API | Meaning and use |
| --- | --- | --- | --- |
| [参伍](../website/src/content/docs/architecture/index.mdx) | Canwu | `Canwu` | The product name and main public API entry point. Do not translate the brand as a generic noun. |
| [无界面历史模拟引擎](../README.zh-CN.md) | headless historical simulation engine | — | The whole product category: Canwu simulates historical worlds without owning rendering, audio, or production UI. |
| [模拟内核](../website/src/content/docs/architecture/index.mdx) | simulation core | `canwu-sim` | The private runtime that owns mutable authoritative state, scheduling, settlement, persistence, and replay. |
| [上层应用](../website/src/content/docs/developer/integration.mdx) | host application | — | A game, research tool, service, client, or agent system that embeds Canwu. In Chinese prose, do not use “主机程序” for this concept. |
| [对外 API](../website/src/content/docs/developer/integration.mdx) | public API | `canwu-api` | The supported API boundary for application code. |
| [模拟运行](../website/src/content/docs/developer/persistence.mdx) | simulation run | `run_id` | One causally continuous execution with its own identity, inputs, state, and evidence. |
| [场景](../website/src/content/docs/tutorials/move-army.mdx) | scenario | `Scenario` | The initial world and configuration used to create a simulation. |
| [权威状态](../website/src/content/docs/developer/reading-state.mdx) | authoritative state | `SimulationSnapshot` | State owned and validated by Canwu; it is the basis for commitments, persistence, and replay. |
| [真值](../website/src/content/docs/developer/reading-state.mdx) | ground truth | — | Facts present in authoritative state whether or not a particular actor knows them. |
| [角色相对视图](../website/src/content/docs/developer/reading-state.mdx) | actor-relative view | `CanwuViewer`, `ViewerContext` | A read surface derived from one actor's knowledge and authority rather than from omniscient state. |
| [只读快照](../website/src/content/docs/developer/reading-state.mdx) | read-only snapshot | `WorldSnapshot` | A detached state value for trusted reads; it is not a mutable reference to the live runtime. |
| 模拟粒度 | simulation granularity | `SimulationGranularity` | Domain-neutral engine level: `aggregate`, `group`, or `actor`. Do not replace these generic terms with a CM-specific ontology in Canwu core. |
| 聚合层 | aggregate | `SimulationGranularity::Aggregate` | Coarse population-, economy-, or region-scale state. A host may map it to Population. |
| 群体层 | group | `SimulationGranularity::Group` | Bounded social, institutional, military, or organizational group. A host may map it to Special Group. |
| 角色层 | actor | `SimulationGranularity::Actor` | A person or other principal with knowledge and authority. A host may map it to Character. |

## Time, input, and settlement

| 中文规范写法 | English | Code or API | Meaning and use |
| --- | --- | --- | --- |
| [现实时间](../website/src/content/docs/tutorials/continuous-game-loop.mdx) | wall time | — | Real elapsed time observed by the upper application. It never enters authoritative state directly. |
| [模拟时间](../website/src/content/docs/tutorials/continuous-game-loop.mdx) | simulation time | `SimTime`, `SimDuration` | Deterministic, representable time settled by Canwu. |
| [表现时间](../website/src/content/docs/tutorials/continuous-game-loop.mdx) | presentation time | — | Client-side animation or interpolation time; it may be fractional and is not authoritative. |
| [命令](../website/src/content/docs/developer/integration.mdx) | command | `Command`, `CommandEnvelope` | A typed request to change authoritative state through validation and authority checks. |
| [领域命令](../website/src/content/docs/tutorials/move-army.mdx) | domain command | `Command::Plugin` | Typed intent defined by a domain integration and validated through public canonical ingress. |
| [规范化输入](../website/src/content/docs/architecture/events.mdx) | canonical ingress | `IngressRecord` | The persisted, deterministically ordered input path for commands, communication, information, and scheduled systems. Write the English term on first use when needed. |
| [准入](../website/src/content/docs/architecture/events.mdx) | admission | — | The decision and cut that determine which pending inputs belong to a boundary. It is not a synonym for ingress. |
| [调度工作](../website/src/content/docs/architecture/events.mdx) | scheduled work | — | Work assigned a deterministic simulation due time and insertion order. |
| [结算边界](../website/src/content/docs/architecture/settlement.mdx) | transaction | `BoundaryRequest`, `BoundaryRecord` | One validated transaction that admits work, runs systems, records evidence, and commits or rolls back. |
| [结算阶段](../website/src/content/docs/architecture/settlement.mdx) | settlement phase | `BoundaryPhase` | One position in the fixed boundary execution order. A phase is ordering, not an independent settlement algorithm. |
| [原子提交](../website/src/content/docs/architecture/settlement.mdx) | atomic commit | — | Applying all validated changes from a boundary together so observers never see partial state. |
| [回滚](../website/src/content/docs/architecture/settlement.mdx) | rollback | — | Restoring state, evidence, counters, and random positions when a boundary fails. |

## Plugins and extensions

| 中文规范写法 | English | Code or API | Meaning and use |
| --- | --- | --- | --- |
| [插件](../website/src/content/docs/developer/extensions.mdx) | plugin | — | Generic term. Qualify it when confusion with Codex or other tooling plugins is possible. |
| [模拟插件](../website/src/content/docs/developer/extensions.mdx) | simulation plugin | `SimulationPlugin` | A runtime registration unit that implements Canwu's public plugin contract. Its name, version, and semantic hash identify the executable behavior used for loading and exact replay. |
| [模拟领域扩展](../website/src/content/docs/tutorials/cases/local-community-diffusion.mdx) | domain extension | — | An optional domain-specific delivery package built on public engine contracts. It may contain one or more plugins, domain types, schemas, commands, queries, and supporting APIs. There is no required `DomainExtension` trait. |
| [模拟模块](../website/src/content/docs/tutorials/cases/local-community-diffusion.mdx) | simulation module | — | An informal, human-facing name for a coherent simulation capability, such as the social diffusion simulation module. It may be implemented by an extension and its plugins, but it is not itself an engine contract. |
| [文化系统 SDK](../website/src/content/docs/architecture/culture-law.mdx) | culture system SDK | `canwu-culture` | A published optional authoring, compilation, and lifecycle extension above `canwu-society`; it validates content and produces a deterministic execution plan without becoming a core subsystem. |
| [法律制度化](../website/src/content/docs/architecture/culture-law.mdx) | legal institutionalization | `canwu-law` | An experimental downstream extension that advances proceedings from admitted social evidence, uses controller decisions to record pending legal intents, and atomically writes legal sources, rules, and immutable versions; culture never writes legal state directly. |
| [法律渊源版本](../website/src/content/docs/architecture/culture-law.mdx) | legal source version | `LegalSourceVersion` | An immutable source record for an instrument, judgment, agreement, customary-recognition finding, received-law schedule, or another configured basis for a legal claim. |
| [法律规则](../website/src/content/docs/architecture/culture-law.mdx) | legal rule | `LegalRule` | The stable identity and mutable head for one bounded normative rule; its immutable changes live in `LawVersion` records. |
| [法律版本](../website/src/content/docs/architecture/culture-law.mdx) | law version | `LawVersion` | A create-only immutable normative change tied to a stable `LegalRule`, with exact source and predecessor refs, normative relations, applicability, effective and validity state, and evidence; enforcement is separate. |
| [法律记录引用](../website/src/content/docs/architecture/culture-law.mdx) | legal record reference | `LegalRecordRef` | The exact kind and identity of a legal object within the sharded legal runtime; it is not a kernel `DomainRecordVersionRef` because law owns its object/version semantics while persisting plan, directory, shard, and archive-head records separately. |
| [文化目标代际引用](../website/src/content/docs/architecture/culture-law.mdx) | cultural target generation reference | `CulturalTargetGenerationRef` | The exact target and generation cited by a legal proposal or version, preventing retirement or reactivation from silently rebinding old evidence. |
| [文化执行计划](../website/src/content/docs/architecture/culture-law.mdx) | compiled culture plan | `CompiledCulturePlan` | An externally immutable, content-hash-bound runtime plan containing compact IDs, reverse indexes, budgets, and lifecycle policy compiled from a culture definition. |
| [休眠目标](../website/src/content/docs/architecture/culture-law.mdx) | dormant target | `Dormant` | A culture target with no engaged population or admitted work that has left hot settlement indexes but retains an explicit reactivation descriptor. |
| [退休目标](../website/src/content/docs/architecture/culture-law.mdx) | retired target | `Retired` | A culture target removed from hot dynamic state while its historical identities and evidence remain available through tombstone and archive contracts. |
| [技术模拟扩展](../website/src/content/docs/tutorials/technology-diffusion.mdx) | technology simulation extension | `canwu-technology` | The optional generic extension for technique revisions, evidence, local capability, implementation, use-specific adoption, and transmission opportunities. It is not a global technology tree. |
| [历史研究插件](../website/src/content/docs/tutorials/technology-diffusion.mdx) | historical research plugin | `canwu-history-research` | An optional simulation plugin that records a researcher's bounded assessment of evidence without changing base technology truth. |
| [命令插件](../website/src/content/docs/tutorials/command-plugin.mdx) | command plugin | `register_command()` | A simulation plugin that registers schema-validated commands and handlers. |
| [边界系统](../website/src/content/docs/architecture/settlement.mdx) | boundary system | `BoundarySystemContract` | A declared system that runs in a settlement phase with bounded reads, writes, resources, and randomness. |
| [领域 schema](../website/src/content/docs/developer/extensions.mdx) | domain schema | `DomainRecordType` | A versioned structural contract for plugin-owned domain data. Keep `schema` in code-adjacent Chinese prose. |
| [领域记录](../website/src/content/docs/developer/extensions.mdx) | domain record | `DomainRecord` | Typed, persisted domain state owned by a plugin rather than by the generic world model. |
| 插件组件 | plugin component | `PluginComponentRecord` | Plugin-owned component state attached through declared keys and visibility. |
| [插件语义环境](../website/src/content/docs/developer/persistence.mdx) | plugin semantic environment | `PluginDescriptor` | The exact plugin identities, versions, and semantic hashes required to load or exactly replay persisted state. |
| 参考内容包 | reference content pack | — | A first-party or downstream package of versioned, namespaced, serializable domain definitions, scenario data, localization, and provenance. It supplies content to a domain extension; it is not a kernel subsystem or a solver. |
| 参考整合包 | reference integration | — | A replaceable public-API implementation that maps generic domain capabilities to a small world, production, information, or society model. It may contain runtime plugins and host-adapter code. |
| 入门套件 | starter kit | — | A runnable host and composition example that combines compatible reference content and integrations into a complete vertical slice. It is reference code, not a privileged engine path. |
| [财政模拟扩展](../website/src/content/docs/tutorials/cases/ming-fiscal.mdx) | fiscal simulation extension | `canwu-fiscal` | The generic fiscal-procedure domain extension. It owns law, regional adoption, assessment, authorization, remission, receipt, audit, aggregates, and holder-relative knowledge reports, but not resource balances or transfers. |
| [历史财政语境](../website/src/content/docs/tutorials/cases/ming-fiscal.mdx) | fiscal historical context | `FiscalHistoricalContext` | The explicit historical year and mode used to evaluate active periods and reform candidates. It changes through canonical ingress and never applies reforms automatically. |
| [财政覆盖单元](../website/src/content/docs/tutorials/cases/ming-fiscal.mdx) | fiscal coverage cell | `FiscalCoverageCell` | One period-region-mechanism cell resolved to supported, archetype fallback, explicit unknown, or not applicable by deterministic priority. |
| [财政执行凭证](../website/src/content/docs/tutorials/cases/ming-fiscal.mdx) | fiscal execution receipt | `FiscalExecutionReceiptPacket` | A settlement request citing exact typed adapter-result versions. The admitted receipt derives externally observed quantity and disposition from those results. Within one fiscal runtime state, each exact version and each `(evidence kind, external_operation_id)` pair can settle at most one receipt. It does not own resource truth. |
| [资源模拟扩展](../website/src/content/docs/tutorials/cases/production-economy.mdx) | resource simulation extension | `canwu-resource` | The optional conserved-quantity extension for exact resource/unit revisions, accounts, protected floors, demand, reservation, allocation, transfer, accepted delivery, consumption, loss, fulfillment, and deterministic receipts. It does not own money or markets. |
| [生产模拟扩展](../website/src/content/docs/tutorials/cases/production-economy.mdx) | production simulation extension | `canwu-production` | The optional production-asset extension for processes, sites, facilities, capacity, work orders, work in progress, execution, projects, and output settlement. Site form and requirements are revisioned data, not universal building levels. |
| [资源能力阶段](../website/src/content/docs/tutorials/cases/production-economy.mdx) | resource capability stage | `ResourceCapabilityStageV1` | An effective-dated, source-cited statement about prospecting, characterized occurrence, permitted access, extraction readiness, operating production, depletion, or explicit unknown/not-applicable status. It is not a timeless deposit flag. |
| [受保护存量底线](../website/src/content/docs/tutorials/cases/production-economy.mdx) | protected stock floor | `ProtectedFloorPolicyRevisionV1` | A revisioned policy reserving an amount such as seed, subsistence, or emergency stock from ordinary allocation unless a separately authorized action overrides it. |
| [在制品](../website/src/content/docs/tutorials/cases/production-economy.mdx) | work in progress | `WorkInProgressVersionV1` | Inputs already admitted to an unfinished production execution, with exact process, facility, resource, and evidence bindings. It is neither available inventory nor settled output. |
| [军事模拟扩展](../website/src/content/docs/tutorials/military-domain.mdx) | military simulation extension | `canwu-military` | The optional military domain extension for forces, operations, combat, occupation, military knowledge, and cross-domain receipts; it is not a universal combat model. |
| [军事补给参考消费者](../website/src/content/docs/tutorials/cases/production-economy.mdx) | force-supply reference consumer | `canwu-force-supply-reference` | A replaceable integration that proves military logistics can consume the shared resource API while owning force-local demand, readiness, shortage consequences, and requisition saga state. It is not a universal combat model. |
| [本地稀缺度投影](../website/src/content/docs/tutorials/cases/production-economy.mdx) | local scarcity projection | `LocalScarcityProjection` | A detached, holder-bound read model explaining observed supply, demand, buffers, route access, security, and policy. It does not claim a price. |
| [价格压力投影](../website/src/content/docs/tutorials/cases/production-economy.mdx) | price-pressure projection | `PricePressureProjection` | A detached read model materialized only from qualifying price-bearing evidence and a revisioned interpretation rule. It neither forms a market nor settles a trade. |

## Evidence, randomness, and persistence

| 中文规范写法 | English | Code or API | Meaning and use |
| --- | --- | --- | --- |
| [事件](../website/src/content/docs/architecture/events.mdx) | event | `SimEvent` | A serializable causal record in authoritative evidence, not a UI notification. |
| [因果关系](../website/src/content/docs/architecture/events.mdx) | causality | `CauseRef` | The recorded relationship explaining why an event or change occurred. |
| [相关标识](../website/src/content/docs/architecture/events.mdx) | correlation ID | `correlation_id` | A stable identifier that groups records belonging to one authoritative causal root. |
| [随机流](../website/src/content/docs/architecture/randomness.mdx) | random stream | `RandomStreamKey` | A named, versioned deterministic source owned by a declared mechanic. |
| [抽样证据](../website/src/content/docs/architecture/randomness.mdx) | draw evidence | `RandomDrawRecord` | The recorded stream, range, operation, and result of one deterministic random draw. |
| [随机决策策略](../website/src/content/docs/tutorials/cases/uncertainty-resolution.mdx) | random decision policy | `DecisionPolicyKind::Random` | A bounded ticket selector resolved by a declared boundary system using replayable random evidence; it is not a general world-event generator. |
| [决策选项权重](../website/src/content/docs/tutorials/cases/uncertainty-resolution.mdx) | decision option weight | `DecisionOptionWeight` | A canonical option-ID and nonnegative integer weight used to map one bounded draw to an existing ticket option. |
| [操作定址随机抽样](../website/src/content/docs/architecture/randomness.mdx) | operation-keyed random draw | `random_sample_for_operation` | A draw addressed by stable cause, operation ID, target, occurrence, purpose, and stream identity so retries and replay cannot consume a different sample. |
| [资源预留](../website/src/content/docs/tutorials/phased-boundary.mdx) | reservation | `ReservationRequest` | A declared request against a conserved resource pool before allocation is settled. |
| [资源分配](../website/src/content/docs/architecture/settlement.mdx) | allocation | `ReservationAllocation` | The deterministic result of settling competing reservations against supply. |
| [可见性](../website/src/content/docs/architecture/settlement.mdx) | visibility | `StateVisibility` | The policy controlling which state and evidence can reach which readers. |
| [快照](../website/src/content/docs/developer/persistence.mdx) | snapshot | `SimulationSnapshot` | A complete persisted state image validated on restore. |
| [检查点](../website/src/content/docs/developer/persistence.mdx) | checkpoint | `SimulationCheckpoint` | A validated compact base state from which later journal evidence can continue. |
| [检查点日志](../website/src/content/docs/developer/persistence.mdx) | checkpoint journal | `CheckpointJournal` | A checkpoint bundled with subsequent evidence for compact persistence and restoration. Use the code identifier in code-adjacent prose. |
| [精确重放](../website/src/content/docs/developer/persistence.mdx) | exact replay | `replay_from_journal()` | Reconstructing and verifying the same run from recorded inputs and evidence without rerunning external choices. |
| [平行现实](../website/src/content/docs/developer/persistence.mdx) | alternative reality | — | A new causal reality created by choosing different inputs from a shared prior state; it is not exact replay. |
| [派生分支](../website/src/content/docs/developer/persistence.mdx) | fork | `fork()` | An independent simulation copied from an existing point so it can advance with new inputs. Use `fork` when referring to the API call. |
| [状态承诺](../website/src/content/docs/architecture/settlement.mdx) | state commitment | `CommitmentRoots` | A cryptographic commitment that binds authoritative state or evidence for validation. |
| [语义哈希](../website/src/content/docs/developer/extensions.mdx) | semantic hash | `semantic_hash` | A stable declaration of plugin behavior used to reject incompatible load or replay environments. |
| [精确领域记录版本](../website/src/content/docs/tutorials/technology-diffusion.mdx) | exact domain-record version | `DomainRecordVersionRef`, `domain_record_version()` | A domain-record identity, version, and establishment source used to preserve the meaning of historical evidence after the current record changes. |

## Actors, knowledge, and decisions

| 中文规范写法 | English | Code or API | Meaning and use |
| --- | --- | --- | --- |
| [角色](../website/src/content/docs/developer/reading-state.mdx) | actor | `PersonId` | A person or in-world principal whose knowledge and authority constrain a view or action. |
| [角色知识](../website/src/content/docs/developer/reading-state.mdx) | actor knowledge | `ActorKnowledge` | What one actor knows, including source, confidence, and time. It is stored separately from ground truth. |
| [报告](../website/src/content/docs/architecture/events.mdx) | report | `EventKind::ReportDispatched` | Information dispatched or delivered between actors before it can update their knowledge. |
| [持有人相对知识](../website/src/content/docs/tutorials/cases/routed-correspondence.mdx) | holder-relative knowledge | `KnowledgeHolderRef`, `KnowledgeReadCut` | Facts read from one person, institution, office, or other holder ledger at an exact cut, never from omniscient state. |
| [通信](../website/src/content/docs/tutorials/cases/routed-correspondence.mdx) | correspondence | `CorrespondenceOperation` | Addressed communication demand composed with information delivery and, when needed, physical or signal transport. |
| [改道](../website/src/content/docs/tutorials/cases/routed-correspondence.mdx) | reroute | `ItineraryRevision` | A successor itinerary for the same delivery attempt after disruption or changed route knowledge. |
| [投递重试](../website/src/content/docs/tutorials/cases/routed-correspondence.mdx) | delivery retry | `DomainReference` role `"previous_attempt"` | A new successor delivery attempt after terminal failure; it is not a reroute of the old attempt. |
| [投影](../website/src/content/docs/developer/reading-state.mdx) | projection | — | A materialized, actor-scoped representation derived without exposing authoritative domain state. |
| [权限](../website/src/content/docs/developer/integration.mdx) | authority | `CommandAuthority` | The validated right to issue a command or perform an action. Do not translate `authoritative` as 权限; authoritative state is 权威状态. |
| [签发者](../website/src/content/docs/developer/integration.mdx) | issuer | `Issuer` | The identity presented as the source of a command. Authority validation determines whether that identity may act. |
| [控制者](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | controller | `DecisionControllerBinding` | The entity or process bound to make decisions for a seat. Keep `controller` beside the Chinese term in code-adjacent prose. |
| [席位](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | seat | `seat_id` | A stable decision-making role that can be bound to a controller. |
| [决策策略](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | policy | `DecisionPolicy` | A human, external, LLM, or deterministic evaluator that selects among current decision options. |
| [决策票据](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | decision ticket | `DecisionTicket` | A persisted request for one controller to choose among versioned, currently valid options. |
| [决策选项](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | decision option | `DecisionOption` | One candidate choice offered by a decision ticket. |
| [决策尝试](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | decision attempt | `DecisionAttemptRecord` | An authoritative record of an admitted, accepted, or rejected attempt to resolve a ticket. |
| [决策轨迹](../website/src/content/docs/tutorials/cases/warlord-aid-decision.mdx) | decision trace | `DecisionTrace` | Persisted scores, evidence, policy identity, selected option, and outcome used for explanation and replay. |

## Consolidated wording

| Preferred term | Deprecated or restricted alternatives | Scope and migration guidance |
| --- | --- | --- |
| [模拟内核](../website/src/content/docs/architecture/index.mdx) / simulation core | `kernel` | Restricted: use **simulation core** in product and integration prose; **kernel** remains permitted in architecture and internal implementation descriptions. |
| [上层应用](../website/src/content/docs/developer/integration.mdx) / host application | 主机程序 / host program | Deprecated: use **上层应用** and **host application** for a game, service, client, research tool, or agent system that embeds Canwu. |
| [对外 API](../website/src/content/docs/developer/integration.mdx) / public API | public facade | Deprecated: use **public API** for the supported application boundary and the broader API surface. |
| [模拟插件](../website/src/content/docs/developer/extensions.mdx) / simulation plugin | 运行时模拟插件 / runtime simulation plugin; 模拟规则插件 / domain plugin | Restricted: use the longer forms only to contrast runtime plugins with agent tools or to describe a plugin's domain role. They are not separate plugin kinds. |
| [结算边界](../website/src/content/docs/architecture/settlement.mdx) / transaction | settlement boundary | Deprecated: use **transaction** for the validated Canwu unit. |
| [平行现实](../website/src/content/docs/developer/persistence.mdx) / alternative reality | 反事实分支 / counterfactual branch; branch (when a different input creates a new causal run) | Deprecated: use **alternative reality** for a new causal reality created by divergent input; use **fork** only for the API operation and copied simulation. |
| [权限](../website/src/content/docs/developer/integration.mdx) / authority | permission (when meaning `CommandAuthority`) | Restricted: use **authority** for the validated right to act; retain **permission profile** for the separate run-policy context. |

## Maintenance rule

When a public concept is added or renamed, update this file and the bilingual
website terminology pages in the same change. Add a link only after a focused
website page exists; the terminology table itself is sufficient until then.
