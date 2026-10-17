# Canwu Architecture

## Settlement foundations / 结算系统底层构成

Canwu exposes fourteen ordered settlement phases, but those phases are not
fourteen independent algorithms. At the lowest level, the runtime combines two
state-write paths, one deterministic allocation primitive, and one
cross-cutting visibility policy:

```mermaid
flowchart TB
    Inputs["Commands / events / scheduled work<br/>命令 / 事件 / 调度工作"]
    Claims["Offers + competing claims<br/>供给 + 竞争性申请"]

    subgraph Writes["State-write paths / 状态写入路径"]
        Immediate["1. Immediate write<br/>立即写入"]
        Boundary["3. Staged atomic boundary commit<br/>分阶段批量原子提交"]
    end

    Allocation["2. Deterministic reservation and allocation<br/>确定性资源仲裁与分配"]
    Visibility["4. Visibility policy (sidecar)<br/>可见性策略（附加属性）"]
    State["Authoritative state<br/>权威状态"]
    Evidence["Evidence, replay, and hashes<br/>证据、重演与哈希"]

    Inputs --> Immediate
    Inputs --> Boundary
    Claims --> Allocation
    Allocation -. "allocation result / 分配结果" .-> Boundary
    Visibility -. "SameBoundary / NextBoundary" .-> Boundary
    Immediate --> State
    Boundary --> State
    Boundary --> Evidence
```

### English

1. **Immediate write** is the direct command/event path. The operation applies
   atomically on its own, without staging for a boundary commit, and either
   commits completely or rolls back completely. It remains the compatibility
   path for the existing movement slice and legacy event reactors. Synchronous
   reactors are therefore compatibility-only: nested event re-entry is bounded
   by `MAX_SYNCHRONOUS_REACTION_DEPTH`, and a limit breach rolls back the
   enclosing command or event application. New mechanics should use phased
   boundary systems, which collect proposals before one deterministic commit.
2. **Deterministic reservation and allocation** is a calculation primitive, not
   a state-write path. Systems publish capacity and competing claims; the
   kernel sorts them by pool, descending priority, explicit tie-break key, and
   reservation identity, then records a fulfilled, partial, or rejected result.
3. **Staged atomic boundary commit** is the authoritative batch-write path for
   new domain mechanics. Systems propose changes against one immutable boundary
   snapshot; the kernel validates the combined proposal and commits it as an
   atomic bundle, or restores the entire boundary on failure. Allocation results
   often feed this step, but it can also commit changes that do not use
   reservations.
4. **Visibility policy** is not a fourth settlement algorithm. It is a
   cross-cutting attribute attached to staged writes: `SameBoundary` exposes a
   value to later systems in the current boundary, while `NextBoundary` keeps it
   out of current-state reads until the boundary has finished. In other words,
   it is a sidecar-like policy on the staged commit path, not an independent
   writer or solver.

The practical classification is therefore: **1 and 3 are write mechanisms; 2
computes a result; 4 controls when a staged result becomes readable**. The
fourteen phases provide ordering, validation, evidence, and replay around these
foundations rather than introducing fourteen separate settlement models.

### 中文

1. **立即写入**是命令或事件的直接处理路径。操作不经过边界的暂存与提交，
   单独原子执行，要么完整提交，要么完整回滚。它仍是既有移动逻辑和旧式事件
   reactor 的兼容路径。
   因此，同步 reactor 只保留为兼容能力：嵌套事件重入受
   `MAX_SYNCHRONOUS_REACTION_DEPTH` 限制，超过限制会回滚整个外层命令或
   事件处理。
   新机制应使用分阶段 boundary system，先收集提案，再统一确定性提交。
2. **确定性资源仲裁与分配**是一种计算原语，不是状态写入路径。系统先
   发布资源容量和竞争性申请；内核再按资源池、优先级（降序）、显式
   tie-break 键和 reservation identity（预留标识）进行排序，得出“满足、部分
   满足或拒绝”的分配结果。
3. **分阶段批量原子提交**是新领域机制使用的权威批量写入路径。各系统
   基于同一份不可变边界快照提出变更；内核统一校验整组提案，然后
   作为一个原子批次提交。任何致命错误都会恢复整个边界。资源分配结果
   经常在这里落地，但没有资源仲裁的变更也可以直接使用这条路径。
4. **可见性策略**不是第四种独立的结算算法，而是附着在分阶段写入上的
   横切属性：`SameBoundary` 让当前边界后续阶段可以读取结果；
   `NextBoundary` 则把结果隔离到本边界结束之后，下一边界才能从当前状态
   读到。也就是说，它类似 sidecar 的策略元数据，而不是独立的写入器或
   求解器。

因此最实用的分类是：**1 和 3 负责写入；2 负责计算分配结果；4 负责控制
   分阶段结果何时可读**。十四个阶段负责把这些底层机制组织成确定的顺序、
   校验、证据和重演流程，而不是提供十四种彼此独立的结算算法。

## Format 8 contract / Format 8 契约

Before 1.0, Canwu uses a clean persistence break. Format 8 requires a declared
run manifest, a declared or `CompatibilityV1` run configuration (the plain
constructors write `CompatibilityV1`), canonical initial scenario, versioned
commitments, content-addressed state-page envelopes, and a self-contained replay
journal. The engine does not load or silently migrate pre-8 saves.
`SimulationGranularity` supplies the generic
`aggregate` / `group` / `actor` levels; Population, Special Group, and Character
are Celestial Mandate mappings owned by a downstream reference integration.
Southern Ming and WWII content therefore does not belong in Canwu core.

## Boundary

```mermaid
flowchart TB
    subgraph Applications
        Games[Games / Renderers]
        Research[Research Tools]
        Python[Python / Bindings]
        Agents[AI Agents]
        Debug[Debug UI]
    end
    subgraph Public[Public Interfaces]
        Programmatic[Programmatic API]
        Viewer[Viewer / Agent API]
        DebugApi[Debug API]
    end
    CQE[Command / Query / Event]
    Core[Canwu Historical Simulation Core]

    Applications --> Public
    Public --> CQE
    CQE --> Core
```

Applications never receive mutable access to live state. The programmatic API
can request generic authoritative records and evidence as detached data. The
viewer API binds an actor or institution and queries only that holder's
knowledge records. Concrete world projections belong to domain integrations.

## Workspace crate dependency DAG / 工作区 crate 依赖 DAG

The graph shows normal, direct dependencies between first-party workspace
crates. An arrow from A to B means **A depends on B**; external and development
dependencies are omitted. / 下图只展示第一方工作区 crate 之间的普通直接依赖。
A 指向 B 表示 **A 依赖 B**；外部依赖和开发依赖未列入。

```mermaid
flowchart TB
    subgraph Tools["Tools / 工具"]
        Debug["canwu-debug"]
    end

    subgraph Extensions["Extensions / 扩展"]
        Correspondence["canwu-correspondence"]
        Information["canwu-information"]
        Society["canwu-society"]
        Culture["canwu-culture"]
        Law["canwu-law"]
        Technology["canwu-technology"]
        History["canwu-history-research"]
        Fiscal["canwu-fiscal"]
        Resource["canwu-resource"]
        Production["canwu-production"]
        Military["canwu-military"]
        Movement["canwu-movement"]
    end

    subgraph Examples["Examples / 示例"]
        MingFiscal["canwu-ming-fiscal"]
        EconomyContent["canwu-economy-reference-content"]
        MingReference["canwu-ming-fiscal-reference"]
        ForceSupply["canwu-force-supply-reference"]
        EconomyReference["canwu-economy-reference"]
        ReferenceWorld["canwu-reference-world"]
        MilitaryContent["canwu-military-reference-content"]
        MilitaryReference["canwu-military-reference"]
    end

    subgraph PublicApi["Public API / 对外 API"]
        Api["canwu-api"]
    end

    subgraph RuntimeMechanisms["Runtime and mechanisms / 运行时与机制"]
        Sim["canwu-sim"]
        Transport["canwu-transport"]
        Routing["canwu-routing"]
    end

    subgraph Models["Models / 模型"]
        Decision["canwu-decision"]
        Event["canwu-event"]
        Knowledge["canwu-knowledge"]
    end

    subgraph Foundation["Foundation / 基础"]
        Core["canwu-core"]
        Time["canwu-time"]
    end

    Debug --> Api
    Debug --> MingReference
    Correspondence --> Api
    Correspondence --> Information
    Information --> Api
    Society --> Api
    Culture --> Society
    Culture --> Api
    Law --> Api
    Technology --> Api
    History --> Api
    History --> Technology
    Fiscal --> Api
    Resource --> Api
    Production --> Api
    Production --> Resource
    Production --> Technology
    Military --> Api
    Movement --> Api
    MingFiscal --> Api
    MingFiscal --> Fiscal
    EconomyContent --> Api
    EconomyContent --> Production
    EconomyContent --> Resource
    EconomyContent --> Technology
    MingReference --> Api
    MingReference --> Fiscal
    MingReference --> MingFiscal
    MingReference --> ReferenceWorld
    ForceSupply --> Api
    ForceSupply --> EconomyContent
    ForceSupply --> Resource
    EconomyReference --> Api
    EconomyReference --> EconomyContent
    EconomyReference --> ForceSupply
    EconomyReference --> Production
    EconomyReference --> ReferenceWorld
    EconomyReference --> Resource
    EconomyReference --> Technology
    MilitaryContent --> Api
    MilitaryContent --> Military
    MilitaryReference --> Api
    MilitaryReference --> Military
    MilitaryReference --> MilitaryContent
    MilitaryReference --> ReferenceWorld

    Api --> Core
    Api --> Decision
    Api --> Event
    Api --> Knowledge
    Api --> Routing
    Api --> Sim
    Api --> Time
    Api --> Transport
    ReferenceWorld --> Api
    Debug --> ReferenceWorld

    Sim --> Core
    Sim --> Decision
    Sim --> Event
    Sim --> Knowledge
    Sim --> Time

    Transport --> Core
    Transport --> Routing
    Transport --> Time

    Routing --> Core
    Routing --> Time

    Decision --> Core
    Decision --> Time
    Event --> Core
    Event --> Time
    Knowledge --> Core
    Knowledge --> Time
```

## Architecture layers / 架构分层

Canwu is intentionally useful at more than one level. The engine supplies
deterministic simulation contracts; domain extensions supply reusable mechanics;
reference kits supply runnable defaults; and host applications decide which
world, content, presentation, and product rules to compose. The lower layers do
not depend on the historical content of the upper layers.

```mermaid
flowchart TB
    subgraph Engine["Canwu engine / 参伍引擎"]
        Core["Kernel<br/>time, commands, settlement,<br/>knowledge, decisions, replay"]
        Api["Public API<br/>canwu-api"]
        Core --> Api
    end

    subgraph Domains["Generic domain extensions / 通用模拟领域扩展"]
        Tech["Technology<br/>technology state and solvers"]
        Info["Information<br/>content access and delivery"]
        Correspondence["Correspondence<br/>demand, address, and delivery orchestration"]
        Society["Society<br/>population and social diffusion"]
        Fiscal["Fiscal procedure<br/>law, adoption, assessment, receipts"]
        Production["Production / economy<br/>assets, recipes, markets"]
        Movement["Movement<br/>transport lifecycle, capacity pools, reports"]
    end

    subgraph Kits["Reference content and starter kits / 参考内容与入门套件"]
        Packs["Reference content packs<br/>technology, society, fiscal, economy data"]
        Integrations["Reference integrations<br/>world and economy adapters"]
        Starters["Starter hosts<br/>runnable vertical slices"]
        Packs --> Integrations
        Integrations --> Starters
    end

    subgraph Apps["Host applications / 上层应用"]
        CM["Celestial Mandate"]
        UserGames["User games and research tools"]
        Clients["Clients, UI, maps, agents"]
        CM --> Clients
        UserGames --> Clients
    end

    Api --> Domains
    Domains --> Packs
    Domains --> Integrations
    Starters -. "starter/template" .-> UserGames
    Packs --> CM
    Integrations --> CM
    Domains --> CM
```

The arrows describe dependency and composition, not ownership of all the state
below them. A reference content pack is data consumed by a domain extension. A
reference integration is executable domain or host code that maps generic
capabilities to a small, inspectable world model. A starter host is a complete
consumer that demonstrates the composition without becoming part of the
kernel.

This classification prevents two opposite mistakes: putting historical or
opinionated examples into `canwu-core`, and leaving new users with only low-level
contracts and no working model to extend.

## Dependency direction

```mermaid
flowchart LR
    core[canwu-core]
    time[canwu-time]
    event[canwu-event]
    reference_world[canwu-reference-world]
    knowledge[canwu-knowledge]
    decision[canwu-decision]
    sim[canwu-sim]
    api[canwu-api]
    debug[canwu-debug]

    event --> core
    event --> time
    knowledge --> core
    knowledge --> time
    decision --> core
    decision --> time
    sim --> core
    sim --> time
    sim --> event
    sim --> knowledge
    sim --> decision
    api --> sim
    reference_world --> api
    api --> knowledge
    debug --> api
    debug --> reference_world
```

`canwu-sim` owns the mutable runtime. `canwu-reference-world` owns the example
entities and detached projection through typed domain records and plugin
commands. This makes the command boundary a structural property instead of a
UI convention.

### World and event model ownership / 世界与事件模型所有权

The dependency DAG above describes the current implementation, not the target
ownership of every public type. The accepted
[world and event ownership audit](proposals/world-event-ownership-audit.md)
establishes this invariant: generic engine crates own deterministic simulation
contracts, while reference integrations own period- and application-specific
world entities and event payloads. / 上面的依赖 DAG 描述当前实现，并不代表每个
公开类型的最终归属。已经接受的审计确立同一条边界：通用引擎 crate 拥有确定性
模拟契约，参考整合包拥有时代或应用特定的世界实体和事件载荷。

That ownership split is now complete: `canwu-reference-world` owns the model,
movement behavior, detached projection, and routing adapter; the source
`canwu-world` package has been retired from the workspace and dependency DAG.
The remaining `WorldSnapshot` projection is a current reference/runtime API,
not a persistence migration surface. Format 8 does not load old saves.
`canwu-event` now contains only generic contracts: `EventKind` is a type label
plus flattened structured fields, while concrete movement, arrival, letter,
report, knowledge, and debug payload structs live outside that crate. / 按此
方向，这项迁移已经完成：`canwu-reference-world` 拥有模型、移动行为、脱离式
投影和路由适配器；`canwu-world` 源码包已从 workspace 与依赖 DAG 退役。
当前 `WorldSnapshot` 只作为参考整合与运行时投影保留；Format 8 不加载旧存档。
`canwu-event` 现在只包含通用契约：`EventKind` 由类型标签和扁平化结构
字段组成，具体移动、到达、信件、报告、知识和调试载荷结构均位于该 crate 之外。

The event extraction is a Rust source-API break: callers replace enum
construction and exhaustive matching with `from_payload`, `event_type`,
`is_type`, and typed field/payload decoding. Format 8 also fixes canonical
field ordering and commitment versions; pre-8 snapshots and journals
are rejected rather than migrated. / 事件迁出会破坏 Rust 源 API：调用方需以通用
构造、类型标签和强类型字段或载荷解码替代枚举构造与穷举匹配。Format 8 同时固定
规范字段顺序并提升承诺版本；Format 8 以前的快照和日志会被拒绝，不在内核中迁移。

## Decision framework

`canwu-decision` is the official headless decision SDK. It defines persisted
decision tickets, versioned dynamic options, controller bindings, persisted
decision attempts and traces, a reusable weighted utility evaluator, policy
contracts for the `Utility`, `Rule`, `Human`, `External`, and `Llm` policy kinds
(`DecisionPolicyKind`), the `Random` kind that a boundary draw resolves, and a
guarded utility policy that composes rules, utility, and a bounded random
tie-break. Domain packages still define when a decision exists, what its
context means, which options are legal, and which domain command an option
represents.

The authoritative flow is:

```text
actor-relative facts -> DecisionTicket -> policy selects an existing option ID
    -> canonical decision ingress -> DecisionAttempt -> DecisionTrace
    -> validated command ingress
```

Policies do not receive mutable simulation state or command authority. A local
policy evaluates the explicit ticket projection and can select only an option
already present on that ticket. External and LLM adapters serialize an even
narrower request containing context and available option descriptors but no
authoritative action payload. Human, External, and LLM responses bind the
ticket version, so a response computed before `ReplaceOptions` is rejected as
stale. The controller binding, not the policy, derives command issuer, decision
origin, seat, permission profile, and command subject. Normal command admission
then validates that derived authority before any command mutation. A command
handler can also require `CommandContext::decision_controller_id`; the engine
sets it only for the nested command of validated decision ingress, so callers
cannot manufacture DecisionTicket provenance with `CommandEnvelope::with_authority`.

Random selection stays inside the boundary. A declared boundary
system uses `random_sample_for_operation` with
`RandomOperationTarget::DecisionTicket`, supplies canonical
`DecisionOptionWeight` values through `ResolveDecisionRandomly`, and lets the
kernel generate the canonical resolution ingress for the next eligible
boundary. `DecisionTrace.random` and `RandomDrawRecord` cross-reference the
ticket version, selected option, draw value, bound, producer, and purpose. A
failed boundary rolls back both the generated ingress and the random stream.
Each source boundary may generate at most one such resolution, due immediately
for the next eligible boundary, so its global revision guard remains
unambiguous.
This is a selector for a bounded ticket, not a replacement for a stochastic
world-event mechanic; discrete incidents still belong to their owning boundary
system, and unknown facts remain unknown rather than being randomized.

`GuardedUtilityPolicy` composes the local selectors. Its ordered guard rules
return `RuleChoice::Select`, `Defer`, `Exclude`, or `NoMatch`; the first select
or defer decides at the `Guard` stage, while `Exclude` removes one option,
records `excluded by <guard>: <reason>` as its blocker, and continues. A later
rule that selects an excluded option is an error, including in
`OrderedRulePolicy`. The remaining options are scored by the weighted utility
evaluator. When `random_tie_break` is set and at least two options score within
the `u64` near-equivalence margin of the best score, the policy returns
`DecisionOutcome::PendingRandom` with exactly those candidates, weight 1 each in
option-ID order; otherwise the best score wins and equal scores fall back to the
lowest option ID. A pending tie-break is never an authoritative resolution. A
boundary system resolves it with the same `ResolveDecisionRandomly` directive,
passing the pending decision as the resolution's `tie_break`, and the draw
covers only those candidates. The controller binding must opt in with
`with_random_tie_break`, which only utility-policy controllers may do. Traces
record the `DecisionStage` (`Guard`, `Utility`, or `Random`) and the guard IDs
that fired. The policy identity reuses `DecisionPolicyKind::Utility` and carries
a BLAKE3 `semantic_hash` over the guard policy identity, guard IDs, utility
weights, margin, and tie-break flag, so a binding rejects a reconfigured policy
that keeps its ID and version with `PolicyMismatch`. Guard behavior itself is
application code outside that hash; change a guard ID or the guard policy
version when it changes.

Draw evidence can come only from a boundary resolution. Host-authored decision
ingress that carries draw evidence is rejected live and during snapshot
validation, for random-policy controllers and tie-breaks alike. A
`ResolveDecisionRandomly` directive also fails its boundary before any draw is
committed when the ticket's person decision maker (`DecisionMakerUnavailable`)
or its controller's authority person (`IssuerUnavailable`) is unavailable in the
person availability committed before that boundary. An availability change
staged earlier in the same boundary does not fail the directive, because
failing would roll the change back and repeat it on every retry; the draw
commits, the end-of-boundary sweep cancels the ticket, and the generated
resolution is rejected at admission. Tie-break and random-policy systems should
therefore skip such tickets, read through `SimulationView::person_availability`.

Registration, opening, option replacement, resolution, and cancellation enter
the runtime through `DecisionIngressRequest`. They use request IDs, revision
guards, deterministic queue order, atomic boundary settlement, and exact-retry
semantics. A selected command option carries a serialized existing Canwu
command; it must exactly match the nested command request admitted with the
resolution. Decisions cannot bypass the command boundary or invent a new
authority envelope.

Admission also consults core person availability. `Open` records an expected
`DecisionMakerUnavailable` rejection when the ticket's person decision maker is
unavailable, and then an `IssuerUnavailable` rejection when its assigned
controller's authority person is unavailable, so a ticket is never opened for a
controller that could not resolve it. `Resolve` records `IssuerUnavailable`
when the controller's authority person is unavailable. The authority person is
the actor of `DecisionAuthority::Actor` or the responsible actor of
`DecisionAuthority::Institution`; council and no-responsible-actor authorities
are unaffected. Host preparation through `prepare_decision` and
`drive_decision` applies the same two checks before evaluating a policy.

`DecisionTicketDraft::parent_ticket` records decision lineage. At `Open` the
parent must be a terminal ticket in hot decision history with exactly the same
`decision_maker`, or with an assigned controller bound to the same non-empty
`DecisionControllerBinding::seat_id` as the new ticket's controller (a seat
succession); an archived or absent parent is rejected as `TicketNotFound`,
and an open parent or another maker and seat as `InvalidDecision`. The parent is copied
onto `DecisionTicket` and `DecisionTrace`. A ticket cannot be reassigned to
another controller, so a decision whose controller can no longer act continues
as a new ticket for an available controller that names the cancelled ticket as
its parent.

Decision state is authoritative persisted state. Snapshots retain controller
bindings, tickets, deadlines, versions, admission attempts, and traces; loading
validates entity identities and reconstructs accepted and rejected outcomes from
admitted decision ingress. A `Deferred` resolution records a trace and bumps the
ticket version but leaves the ticket open under its original deadline, so live,
restored, and replayed runs expire it at the same boundary. Revision,
ticket-version, closed-ticket, and similar expected conflicts become persisted
rejected attempts, so one bad request cannot poison the canonical ingress queue.
Decision and nested command request IDs are nonzero and globally
collision-checked before persistence. Decision state has its own optional
commitment root. Exact replay replays recorded decision ingress and verifies the
resulting attempts, state, and traces; it deliberately does not rerun a possibly
external, human, or nondeterministic policy. The recorded draw and generated
decision ingress are the replay inputs for Random policy.

### Deterministic outcomes, reloads, and forks

Deterministic replay is not an anti-save-scumming policy. Canwu guarantees that
the same authoritative state, command or decision ingress, plugin semantic
environment, and simulation-time inputs produce the same random draws and
outcomes. Reloading a snapshot before an already admitted event therefore does
not reroll that event. A host may still expose `fork()` or another branch
workflow so a player, AI planner, or researcher can try a different command;
that is a new causal branch, not a replay of the original run.

Whether a product permits manual save branches, exposes only one write-through
save, or offers research-only forks belongs to the host application's save
policy. The engine persists enough state for all three policies but does not
silently convert deterministic replay into irreversible play.

`Simulation`, `CompactedSimulation`, `Canwu`, `CompactedCanwu`, and every
fork of them are `Send` (`Sync` is not promised): the run holds its plugin
archive provider as `Arc<dyn PluginArchiveObjectProvider + Send + Sync>`,
which a fork shares, so a host takes `fork()` on the simulation thread, moves
the fork to a worker, and advances, seals, checkpoints, or saves it there
while the simulation thread keeps settling the original; what the worker
writes is byte-equal to the same steps on the original thread, and nothing a
boundary decides depends on the thread it runs on. Since 0.18.0 the current
state, the scheduler, and the retained journals are persistent, shared
structures, so `fork()` is constant-time (53 ms to 0.03 ms at 65,536
records) and copies no journal; the first append on either side while both
still hold the shared journals copies them instead, so a save fork that is
still alive at the original's next boundary moves that copy (about 53 ms at
65,536 records) onto the original's boundary, where the fork paid it before,
and a sealed run's tail keeps it small. Four rules keep that safe. A save-only fork never loads from the provider (`checkpoint_journal_json`,
`continuation_checkpoint`, and `seal_evidence` read only the run's own state
and evidence; a fork that is also advanced, restored, or validated loads at
the same time as the original). A provider gives every run one answer per
`(namespace, object_id)`, whatever the order or concurrency of the calls,
and blocks while it is busy rather than returning `None` or an error, which
a boundary would take for a missing object. The host keeps what a live fork
reads: deleting the lookup pages an archive commit superseded gives up every
earlier point of the run that reads them, a fork whose head precedes the
commit included, and the same holds for any package archive object a fork's
state still names. And a fork's seal is the fork's: it leaves the original's
evidence and seal history alone, and the segments it returns belong to the
fork's save. Host-side archive writes stay with the thread that drives the
run they belong to. See [a `Send` runtime](proposals/send-runtime.md).

## Simulation domain extensions / 模拟领域扩展

Canwu calls an optional domain-specific module built on the public engine
contracts a **domain extension** (**模拟领域扩展**). A domain extension owns its
domain state, rules, commands, and actor projections while reusing kernel
infrastructure such as events, settlement, decisions, persistence, and replay.

`canwu-correspondence` is a published correspondence domain
extension. It is also a runtime `SimulationPlugin`: the first term describes
ownership of reusable communication policy and evidence, while the second
describes how its authoritative commands and boundary systems execute. It
depends on `canwu-api` and `canwu-information`; neither dependency points back
to it.

`canwu-information` owns the neutral information lifecycle. An
interpretation's `InterpretationPayload` may carry an `AuthenticityFinding`:
the interpreting holder's judgment of whether a
representation's claimed source is accepted, with a basis of at most
`MAX_AUTHENTICITY_BASIS_BYTES` and a per-mille confidence. The finding must cite
one of the interpreted representations at its exact current version, and that
representation must carry a claimed source; otherwise the operation is rejected
as `invalid_lifecycle`. Whether a forgery is detected, and with what
probability, stays an application draw; the extension records only the
finding. `validate_delegation_claim` is domain-neutral and takes the time to
check as `at`.

`canwu-society` is a published **social diffusion simulation
module** (**社会传播模拟模块**). It is a domain extension built on
`canwu-api`, not a dependency of `canwu-api` or a new kernel subsystem. It
models the reusable part of population-scale belief and affiliation change without introducing
religion-specific types into Canwu core.

Its authoritative root record contains ordered, sparse state for:

- cohorts with integer headcounts and application-defined classifications;
- active cohort/affiliation disposition distributions across separate
  awareness, private assent, practice, public alignment, organization tie,
  mobilization, and visibility dimensions;
- social influence edges and bounded organization topology;
- institutional alignments and orthogonal policy pressures;
- stable integer transition remainders, aggregates, mobilization candidates,
  and authorized observer estimates.

Transition rules execute on the plugin's settlement cadence (`Daily` by
default) in canonical key order with integer per-settlement rates and
persisted per-rule remainders. Every active cohort/target distribution must
conserve the cohort headcount. Missing pairs are materialized only when a rule
actually addresses them, so runtime state grows with active relationships and
edges rather than a dense territory/cohort/target/channel product.

The settlement cadence is an option of the plugin.
`SocietyPlugin::with_settlement_cadence(cadence)` runs the one settlement
system, `settle-social-transitions` in phase 7, on a host-chosen
`SystemCadence` category. Since 0.18.0 that system also materializes the
derived views its changes cleared (mobilization candidates, aggregates, and
the projections of observers whose estimates changed) before it writes, so a
settlement writes the root at most once and journals one record change,
"Settled society"; the separate phase-10, -12, and -13 systems of earlier
releases are gone, and a system of another plugin that reads the root in
phases 8 to 13 of a settling boundary reads the views materialized. A rebase
or a cohort transfer touches only its cohorts' distributions, and the whole
root is validated once before the write. `SocietyPlugin::DAILY` and
`SocietyPlugin::default()` keep `Daily`, and `EventDriven` is refused at
registration. Canwu keeps no calendar, so a cadence is a category the host
attaches to boundaries, not a length of time: a `Monthly` society settles
whenever the host settles a `Monthly` boundary. A rate applies once per
settlement and the remainder is carried exactly, so elapsed simulated time
never enters the arithmetic and an application authors each rate for the
cadence it registers. The cadence is part of the persisted plugin descriptor;
a run is restored and replayed only with a plugin of the same cadence.

Everything the phase-7 system writes waits for the cadence: due cohort
transfers, queued rebases, lifecycle deltas and pressure revisions,
institutional decisions, and transitions. An application that needs such a
change within a day registers a `Daily` society. How a settlement checks the
cohort transfers due at it is a second option of the plugin,
`SocietyPlugin::with_transfer_settlement` (`TransferSettlement`), recorded in
its identity like the cadence. `Batch`, the default, checks every due
transfer against the state at the start of the settlement, the state every
issuer of the period could see, and applies the ones that still fit in
admission order, each to the state the earlier ones left; a transfer whose
cohorts an earlier settlement changed after it was admitted settles as
`stale_source`, and one whose quantity no longer fits its source cohort as
`insufficient_source`, a recorded outcome. `OneByOne` keeps the 0.15.0 rule:
each due transfer is checked against the state the earlier transfers of the
same settlement left, so of the due transfers that share a cohort one
completes and the others settle as `stale_source`. The owner-side queue is a
small head record (`SocietyIngressQueue`, schema 2) and one
`SocietyIngressBatch` record per admitting boundary, so its evidence follows
what each boundary admits, never the queue's length. The queue holds at most
`MAX_SOCIETY_INGRESS_QUEUE` (4,096) packets; one that arrives while it is full
is refused by the boundary that admits it, with a terminal `queue_full`
outcome in the cohort exchange ledger and one event, and a
`revise_society_pressure` command is refused at admission while the queue is
full. A third option, `SocietyPlugin::with_outcome_retention`
(`OutcomeRetention`), bounds the cohort exchange ledger: under `Keep`, the
default, every outcome stays for the run; under `LatestSettlement` a
settlement that records rebase or lifecycle delta outcomes first drops those
recorded at an earlier time, while transfer outcomes (which make a resent
transfer idempotent) and pressure revision outcomes are kept. Like the
cadence and the transfer settlement mode, the retention is plugin identity.

Since 0.18.0 the society also holds **intergroup relations**: directed,
typed relations between two **cohort classes** (`CohortClass { dimension,
value }`, every cohort whose classification carries that value) in an
optional region (a territory, a government, or a domain entity), each with
acceptance, tension, and trust per mille, bounded evidence, and a revision.
They live in their own record, `canwu.society:intergroup-relations`
(`IntergroupRelations`, at most 4,096 edges, with a `kinds` vocabulary), so
a settlement never rewrites them and a revision never rewrites the root. A
scenario authors the record (`IntergroupRelations::into_record`); run
creation and every restore check each class an edge names against the
root's cohorts. The public `intergroup_relation_revision_v1` packet creates
or revises edges by expected revision and applies as a whole in phase 7 of
the boundary that admits it; every refusal is a
`society_intergroup_relation_rejected_v1` event with a stable code and the
boundary commits. A region is an identifier checked when the edge is created,
not a record reference, so its owner can still retire or delete it; an edge
is never removed. The settlement neither reads nor writes the relations:
`load_intergroup_relations` is the host read, and actors learn of relations
through knowledge the application publishes. `SocietyState::relation_vocabulary`
applies the same idea to organization relation kinds: while it is empty any
canonical kind is admitted, otherwise validation refuses another.

A settlement that changes nothing writes nothing. `last_transition_at` is the
time of the last settlement in which a transition changed a distribution or a
carried remainder. `last_mobilization_at`, `last_aggregation_at`, and
`last_projection_at` are the times those derived views were last
materialized: while one is set its view matches the authoritative state, a
change the view depends on clears it, and the next settlement materializes the
view once and sets it again, also when the view comes out empty. An idle
society therefore writes no root version and emits no record-change event.

An observer's projection changes only with that observer's estimate. After
its phase-7 changes the plugin recomputes each observer's entries and keeps
the projection, `observed_at` included, of every observer whose entries are
the same. `observed_at` is therefore the time that observer's own estimate
last changed, and it tells the observer nothing about cohorts outside its
scope or about changes it cannot detect. It is recorded history: the rest of
the root does not determine it, and validation only bounds it from above.

The social diffusion simulation module deliberately separates this chain:

```text
information exposure != awareness != private assent != public alignment
    != organization tie != mobilization candidate != political conflict
```

Institutional choices use ordinary `DecisionTicket` controllers and a
validated plugin command. The command records a pending policy component; the
next society settlement applies it to institutional alignment before proposing
population transitions. A ruler or policy therefore cannot directly set a
population belief percentage. Phase 10 produces mobilization candidates only;
downstream political or conflict packages decide what, if anything, follows.

An institution revises pressure at runtime with the `revise_society_pressure`
command, whose payload is a `PressureRevisionDecision` and whose authority is
that of `set_institutional_policy`: a validated `DecisionTicket` controller,
the alignment's institution as command subject, and a decision origin that
matches the alignment's authorized actor. A decision changes nothing for a
cohort outside its alignment's affected cohorts. Through the alignment the
institution may revise the pressure fields of a `PolicyPressure` row of the
alignment's target that it issued and whose affected cohorts lie within the
alignment's; the `active` flag and reach of an influence edge of the target
whose target cohort lies within the alignment's; and the `active` flag and
base reach of an organization node of the target, only through an alignment
that covers every cohort, because an organization acts on every cohort of its
target. The rule bounds cohorts, not relations: through
`organization_relations` an organization revision also changes the strength of
related organizations, which may belong to other targets. A decision never
creates or removes a row and never changes a transition rule; it revises at
most `MAX_PRESSURE_REVISION_ROWS` (256) rows and applies as a whole or not at
all. The command queues the internal `society_pressure_revision_v1` packet,
which the settlement applies in admission order before its transitions, so a
revision takes effect at the first society settlement after the boundary that
admits the packet. Each alignment has its own revision version sequence,
guarded by the decision's `expected_version`; a resend of the applied decision
changes nothing. `SocietyCohortExchangeLedger::pressure_revisions` keeps the
latest applied and the latest rejected decision of each alignment, and every
outcome is also emitted as an event.

An expected refusal of a society command is a recorded command rejection, and
a defective society packet is skipped with a `society_ingress_rejected_v1`
event or settles as a rejected outcome. Only an invalid society record fails a
boundary.

A `PolicyPressure` may record its provenance: an optional `issuer` (a
government, organization, or person, bound into the record's core references)
and a `decision_version`, which requires an issuer when non-zero. Both fields
are omitted when unused. Besides pressure revisions, two canonical ingress
types reach the society owner through the same queue. The public
`cohort_headcount_rebase_v1` packet
(`CohortHeadcountRebaseV1` with a `RebaseReason`) rebases a cohort to an
external conserved stock: the cited stock must be a record outside
`canwu.society`, or the rebase is rejected as `invalid_external_stock`, and it
must be that record's current version when the packet is admitted, or the rebase
is rejected as `stale_external_stock`. The internal `society_lifecycle_delta_v1`
packet (`SocietyLifecycleDeltaV1`) carries a lifecycle provider's target-scoped
rule, alignment, and release changes, one packet per target, for example from
the culture boundary plugin. The event-driven phase-12 system
`intake-society-ingress` queues admitted packets in the
`canwu.society:ingress-queue` record, because the society writer runs only on
boundaries that carry its settlement cadence and only one system may write the
state in a phase. The next phase-7 settlement applies the queue after cohort
transfers and before institutional decisions and transitions, and records each
outcome in the cohort exchange ledger (`rebases`, `lifecycle_deltas`,
`pressure_revisions`). A rebase re-proportions every
distribution of the cohort with integer largest-remainder allocation, so each
distribution totals the new headcount and every bucket stays within one unit of
its exact share. Only the intake writes the queue: an initial scenario that
seeds a non-empty queue is refused with `InvalidAuthority`, and the module-level
restore check (`from_society_snapshot_json` or `validate_society_runtime`)
re-derives a restored snapshot's queued packets from the ingress journal, which
must match the packets admitted at their recorded boundaries. The society plugin
remains the only writer of `canwu.society:state`.

Applying a cohort transfer invalidates the derived aggregates, mobilization
candidates, and projections it affects, and the transfer digest binds only the
state the transfer depends on.

Actor-facing queries require a valid `ViewerContext` and return only a
previously materialized projection for that actor. Absence is an authorization
error, never a request to decode the authoritative society record. Generic
snapshot validation checks the plugin manifest, record schema, commitments,
and referenced core-entity existence. The module-level
`from_society_snapshot_json` boundary (`from_society_snapshot_json_with_plugin`
for a run on another settlement cadence) additionally recomputes the society
payload-to-reference binding and every persisted aggregate, mobilization
candidate, actor projection, and pending institutional-policy component before
the restored simulation is returned. Optional materialization timestamps keep
`SimTime::EPOCH` and negative simulation times available as real boundary
times. Fork and exact replay use the same serialized authoritative state.

The culture authoring layer and downstream legal institutionalization extension
are specified together in the [culture and legal institutional systems](architecture-culture-law.md).
That design covers authored definitions, deterministic compilation, dirty-set
settlement, Active/Dormant/Retired lifecycle, bounded cultural signal batches,
controller-mediated legal procedure, versioned law records, authority,
visibility, persistence, and exact replay. The detailed implementation proposals
remain [culture authoring SDK and lifecycle design](proposals/culture-authoring-sdk-and-lifecycle.md)
and [legal institutionalization framework](proposals/legal-institutionalization-framework.md).

The `canwu-law` crate implements that legal boundary. Its compiled plan,
directory, order/jurisdiction shards, coordinator, culture-dependency records,
and archive heads are independently versioned. Typed `LegalMutation` ingress,
holder context, expected host-record versions, and decision outcomes are
revalidated inside the plugin boundary before one kernel-atomic domain-record
mutation bundle. The decision outbox uses a durable
prepare/decision/ACK protocol whose ACK proves accepted controller and ticket
outcomes, so ingress archival does not invalidate recovery. Persistent dirty,
deadline, scheduled-version, live-culture-dependency, participation, outbox,
and applicability indexes keep ordinary settlement proportional to due or
changed work rather than immutable history. Since 0.18.0 the plan record
holds only the compiled plan, its hash, and its budgets and is written once;
the ledger counters live in the coordinator shard and the evidence
declaration in the directory, and an order or jurisdiction shard exists only
once it holds something, so a plan of 1,400 jurisdictions starts with six
law records and a law boundary decodes the plan once. A plan may also
declare a **caseload profile** (`CaseloadProfileDefinition`: case classes,
review levels, kept periods, reports per boundary), and a host reports
routine justice in aggregate with `caseload_intake_v1` packets
(`enqueue_caseload_intake`): the admission system settles each report into
an **aggregate caseload** record per jurisdiction and period
(`AggregateCaseloadRecord`: intake and resolutions per class, the pending
stock carried from period to period, review queues per level, and the
**clearance rate**), kept in a fixed ring of slots per jurisdiction and read
with `load_legal_caseload`. Periods are contiguous, stocks are conserved, a
repeat of the newest period is idempotent, and every refused report changes
nothing and is one `canwu.law.caseload_rejected.v1` event with a stable
code; the records declare no evidence, never load the legal runtime, and are
bounded when the plan compiles, outside the legal state budget. A case the
host wants to follow on its own is still a `LegalCase`, counted in its
class's intake. The legal provider index keeps the newest
`max_provider_packets` packets (256 by default), and the stale-proposal
check examines at most `max_mutations_per_boundary` dirty procedures per
boundary, deferring the rest. See the
[law aggregate caseload proposal](proposals/law-aggregate-caseload.md).

Procedure stages tally integer vote weights, can require a
number of unit blocks in the `For` position with a `status-quo` or
`casting-seat:<seat>` tie-break, and can be advisory `Consultation` stages whose
ballots never count and which complete at their deadline. A stage that stops
accepting ballots expires its pending seat work, and a late seat response is
recorded as a rejected outcome instead of failing the plugin boundary. The
[culture and legal institutional systems](architecture-culture-law.md#weighted-unit-block-and-consultation-stages)
page specifies the tally, compile checks, and consultation rules.

### Technology and historical research / 技术与历史研究

`canwu-technology` is a published domain extension built only
on `canwu-api`. It owns generic metric schemas, immutable technique revisions,
program intent, experiment and production evidence, holder/site capability,
installed implementations, use-specific adoption, and transmission
opportunities. It does not own a global technology tree, era levels, research
points, inventory, markets, information artifacts, transport, or historical
case labels.

The reference evaluator applies bounded integer thresholds and alternative
requirement groups. It returns criterion evidence rather than an `invented`
flag. An experiment can succeed without creating qualification; qualification
can exist without installation; installation can exist without use-specific
adoption; a transmission opportunity never grants knowledge or capability by
itself. Qualifications declare `valid_from` / optional `valid_until`,
implementations declare `installed_at`, and practice transmission records the
exact qualification or implementation that made the source capable when the
opportunity opened, or instead an external source (manifest-bound content
evidence with a declared reliability) for an off-map teacher that has no live
capability record. This permits historical entry without allowing a later
installation to justify an earlier transmission. At creation, that exact
source must also be the current version in the boundary's replayed state; later
deactivation blocks new transmission without invalidating prior opportunities.
After creation, the opportunity may only transition from active to closed; a
restart requires a new opportunity citing a then-current source. An
implementation's exact qualification is installation-time evidence, so a later
qualification version does not implicitly stop the installed asset; games must
stop the implementation itself to prevent it from serving as a new transmission
source. Deliberate
changes use tracked plugin commands, while resolved provider results and passive
observations enter through a separate declared ingress. Program provider
requirements are checked when a result is submitted, so intent creation may
produce a pending intent whose eventual provider result is rejected.

Dependencies whose meaning relies on a mutable domain-record body use an exact
`DomainRecordVersionRef`. The trusted host and declared plugin views can resolve
the retained body for that exact version, so a later update cannot reinterpret
older evidence. Generic `EvidenceRef` citations validate retained identity and
existence only; they do not by themselves establish relevance or historical
truth. A compacted receipt still proves existence, but its body and
`evidence_time` require retained or archive-provided content, and a seal keeps
a version's receipt only while the version is current or a live dependency
declares it. A record's current version is the exception: its live record and
runtime provenance index keep its body and time after the establishing boundary
is sealed, so a compact run resolves it exactly as its replay does. Plugin views
cannot load archived content, so another version's existence, body, and time can
differ between a compact run and its exact replay. A rule whose outcome must
replay exactly therefore reads each cited exact version through
`SimulationView::replay_stable_domain_record_version`. It resolves only the
committed current version, a version proposed earlier in the boundary and not
superseded since, or an initial-scenario version, and the exact reference still
fixes the cited meaning after the record changes. `canwu-history-research`
assessment admission, `canwu-fiscal` receipt settlement, and `canwu-fiscal`
commutation-quote admission follow this rule. Generic evidence has no body
read of that kind, but its existence and time are seal-stable: the journal
counters and the time index answer `evidence_exists` and `evidence_time` for
a sealed ordinal reference as the exact replay does, so
`canwu-history-research` checks a generic citation the same way in every run.
Module-owned restore wrappers re-run technology semantics after normal core
snapshot, checkpoint, or replay validation.

Historical fidelity is downstream and optional. `canwu-history-research`
provides separately selectable source, practice, and production-archaeology
plugins. They create bounded, append-only records of an assessor's method,
date, uncertainty, citations, contradictions, and supersession. They never
replace authoritative attempts, assets, production runs, capability, or
adoption. Omitting them leaves base technology outcomes unchanged and avoids
their record and handler cost.

### Resources, production, and force supply / 资源、生产与军事补给

`canwu-resource` is the shared conserved-quantity boundary. It owns revisioned
resource and unit definitions, accounts, demands, reservations, exact
allocation legs, transfers, consumption, losses, fulfillment, and operation
receipts. Protected floors, custody, acceptance, explicit conversions, and
holder-relative reports are part of that boundary. Transport reach alone never
counts as delivery, and no consumer may debit an account directly.

`ResourceDemand::source_policy` is persisted before allocation. `Pooled` preserves
all open accounts matching the exact resource/unit revisions. `ExactAccounts`
accepts 1–256 sorted, unique account IDs owned in custody by the requester, all
open and matching those revisions. Admission and allocation validate the list;
only eligible accounts contribute to scarcity, minimum useful quantity and
partial fulfillment, with no fallback to the pool. Protected floors and transfer
authority remain separate checks. Exact selection iterates the bounded list;
the pooled path retains its existing account scan.

The policy can be amended with an expected revision only before any allocation
or fulfillment. The reservation-by-demand index retains consumed reservations
while transfers are in flight, even when fulfillment is zero. Reservations become
archive candidates only with terminal demand closure; terminal demands cannot be
amended. The policy participates in request digests, snapshot identity, replay
and terminal demand archives. Retained reservations must match the policy while
their demand is retained; active reservations require that demand. Archive
payloads retain canonical policy shape and authenticated demand digests.

The resource root is one `ResourceState` in memory and persists in parts: the
live root, the domain record `resource:runtime:v1` (`ResourceLiveRootV1`),
holds open demands with their reservations and allocation legs, active
transfers, leases, report grants, and observation heads, and only counts, in
its `ResourceHotTerminalLedgerV1` and (since 0.18.0) `ResourceLiveLedgerV1`,
what it does not hold: each hot terminal record (the root's archive
candidates) is an entry of a hot terminal bucket `resource:hot-terminal:{n}`
(`ResourceHotTerminalBucketV1`, about two records per bucket), and each
account, access grant, definition, unit revision, and custodian index entry is
an entry of a live bucket `resource:live:{n}` (`ResourceLiveBucketV1`, about
four records per bucket), each chosen by a domain-separated hash of its
identity among a bucket count fixed for the life of the root. Both bucket
families are one record kind, `canwu.resource` `bucket`
(`ResourceBucketRecord`). The custodian index
(`ResourceCustodianAccountsV1`) lists the accounts one holder holds, so a
holder's stock is read without a scan. `ResourceState::into_records`
persists a state and `from_records` restores it, checking that every entry
lies in its bucket, that the ledgers count exactly the stored entries, that
every account lies in its custodian's index, that every consumption names
the fulfillment it recorded, and that the whole state validates; activation
runs the same checks. `into_record` refuses a root that holds hot terminal
or live records, so a scenario installs opening accounts with
`into_records`. A boundary reads and writes what it touches: before it
settles a packet the resource plugin attaches the outcome under every key
the packet settles under, every hot terminal and live record the payload
names (a leg's account, a demand's source accounts and access grant, an
account's definition and unit), and their closures, and every rule that
counts records, totals balances, or asks whether a floor policy is in use
reads the attached records and the ledgers together, so a partial root
settles as the whole root would (every debug build settles each boundary
again on the whole root and asserts the same proposal). It then writes the
live root and the buckets whose entries changed: settling one operation
writes the same records whatever the number of accounts, creating an
account writes its bucket and its custodian's index, and a pooled demand's
allocation pass attaches every account. A boundary with nothing for the
resource plugin reads nothing of the root, and one that would write over a
hot terminal or live record it did not attach fails with `invalid_boundary`.
Another plugin reads the root with `resource_root_in_view` or
`ResourceRootReader`, naming the hot terminal records it needs and the live
records with `attach_live` or `attach_custodian_accounts`, or
`whole_resource_root_in_view`, and declares the read of
`resource_hot_terminal_state_key()` beside the live root's; a reader of a
consumption follows `ResourceConsumption::fulfillment` to its fulfillment
without a search; the host's `resource_state` reads the whole root. A
finished lease (released or expired, its report reservation published)
leaves the root at once; its receipts stay hot terminal records until an
archive commit takes them, and an acquisition identity still starts one
lease for the run. A host expires grants with
`enqueue_resource_completion_expiry_tick`; `Expire` is refused at enqueue
because a host cannot know which boundary admits a packet. See the
[resource root write cost proposal](proposals/resource-root-write-cost.md)
and the [resource series scale proposal](proposals/resource-series-scale.md).

A provider that settles many small consumptions sends one **consumption
batch** (`ResourceConsumptionBatchRequestV1`, `ConsumeBatch`): up to 256
exact allocation-leg consumptions under one completion lease, entering only
through the provider's adapter ingress with a typed consumption intent per
item under the item's reserved key (`resource_consumption_batch_item_key`).
Items settle in order; each is refused alone or consumed with its own
outcome, several items may draw on one account, the batch outcome totals
them (`remainder` is the refused quantity), and the lease completes once.
Room is measured on the whole result, so a batch that does not fit is
skipped and records nothing (`consumption_batch_can_fit` tells a provider
what can ever fit); `max_batch_items_per_boundary` (canonical 32, hard 2,048)
bounds the items a boundary settles, and a batch past it is deferred with
the `boundary_budget` skip event to the next boundary at the same instant.

Terminal resource records leave the root in archive commits that the host
application prepares with `prepare_resource_archive`, stores, and queues. One
commit carries up to 64 pages (16,384 records), and a run may make any number
of commits; a commit applies only to the root it was prepared from, named by
a digest of the state revision, the admitted sequence, the number of hot
terminal records, and the archive head. The commit that clears a backlog
rewrites one bucket per archived record, so a host commits a large backlog in
batches and acknowledges each before it prepares the next. Each commit directory commits to a cumulative lookup index, so the
per-operation check for an archived operation key reads the head directory and
one authenticated lookup page per index level, however many commits the run
has made. A boundary only reads archive objects, by content id, and a missing
or malformed object is an error, never a miss. Within one boundary the
resource plugin settles archive commits and retention acknowledgements before
its other packets, and an archive commit never fails for capacity. A restore,
an exact replay, or a fork reads the lookup index of each archive head it
passes, so the archive store keeps those pages. The index functions take key
digests and are public, so the production archive, the fiscal archive, and
the reference package archive keep an index in the same format under their
own namespaces. See the
[resource archive lookup and volume proposal](proposals/resource-archive-lookup-and-volume.md).

No packet a host application or a holder can send fails a resource boundary.
Room is measured on the result: a refused operation needs room for its
outcome only, an applied one for the records it adds and the reservations it
makes. A packet whose result does not fit the archive-candidate or hot-outcome
budget (`max_archive_candidates`, 256 by default, at most 16,384) or the
operations' share of the state byte cap, whose operation key a different
request holds or lies under the reserved prefix, or that does not decode or
fails its settlement checks is skipped: it changes nothing, leaves its key
free, and is named by one `canwu.resource.ingress_skipped.v1` event with a
stable reason (`operation_key_taken`, `reserved_operation_key`,
`failed_settlement_checks`, `archive_backlog`, `reserved_capacity`,
`root_limit`), and the sender sends it again when room returns. Work another
plugin scheduled, which may never be sent again (a production output batch,
or a completion step that can settle later as the same step), is instead
deferred to the plugin itself through the internal
`resource_deferred_packet_v1` ingress, one minute after the first deferral and
doubling up to 1,024 minutes, while an archive commit or a lease release can
make room; the output acknowledgement then carries `settled_at`. A resend
whose outcome is recorded settles nothing. An expiry tick skipped for room
schedules no successor; the next applied archive commit resumes it. What
still fails a resource boundary is an invalid record or an engine invariant:
an unreadable or forged archive object during a key lookup, a forged archive
packet, a root that does not validate for a reason other than a limit,
maintenance writes past `MAINTENANCE_STATE_BYTE_RESERVE` in one boundary, a
hot terminal bucket that is malformed or short of what the ledger counts,
the `invalid_boundary` backstop above, and arithmetic or encoding failure.
An expiry tick whose settlement key a host completion operation holds is a
resend or is skipped with `operation_key_taken` and scheduled again. See the
[resource refusals without stalls proposal](proposals/resource-refusals-without-stalls.md).

A completion lease becomes active through four operations (acquire, grant,
prepare, activate) because a lease may have several participants, each of
which grants and prepares on its own. When the resource plugin is the only
participant, which means exactly that the lease's `expected_participants` is
`{"canwu-resource"}`,
`ResourceCompletionOperationV1::SingleStepLease` takes the lease from nothing
to active in one operation. It runs the four steps with the checks each has on
its own and yields the same activation certificate, so the operation the lease
authorizes cannot tell which path produced it, and it leaves one outcome and
one receipt instead of four of each. It enters only through the completion
ingress (`enqueue_resource_completion_operation`, or a plugin that declares
that ingress), never as a tracked holder command, because an activated lease
does not expire and holds its exact target locks. A domain refusal is the
operation's recorded rejection and leaves no acquisition, grant, or lock
behind; a refused single step needs room for its outcome only, and one whose
applied result does not fit is skipped with `archive_backlog` or
`reserved_capacity`. Only a failed archive lookup still fails the settling
boundary. The four-step path remains for a lease that needs a grant owned by
another plugin or that another plugin coordinates; a tracked `Acquire` must
be eligible at the instant it is admitted and cite current evidence. Operation
keys under `resource:completion:` are reserved for completion operations.

A protected floor is an immutable `ProtectedFloorPolicyRevision` of one of two
kinds. An authored revision has an identity its author chose; the scenario
installs it with `ResourceState::install_protected_floor_policy`, it stays in
the root, and an account is pointed at it by id. A content-addressed revision
(`ProtectedFloorPolicyRevision::content_addressed`) derives its identity from
its content, so one id names one content for the whole run. An account's
custodian installs one at runtime by sending its body in
`ResourceProtectedFloorRequestV1::install`: `SetProtectedFloor` installs the
revision and points the account at it together or not at all, and a
content-addressed revision is always named with its body. A root holds at most
`MAX_PROTECTED_FLOOR_POLICY_REVISIONS` (4,096) revisions of both kinds, one
quota shared by every custodian, and an install at the bound is a recorded
rejection. Every applied `SetProtectedFloor` retires the content-addressed
revisions that no account and no demand in the root refers to. See the
[single-step lease and floor install proposal](proposals/resource-single-step-lease-and-floor-install.md).

A tracked resource command that reuses an operation key for a different
request never fails a boundary: it is refused at admission when the key's
outcome is in the root or the archive, and its packet is skipped with
`operation_key_taken` when it was admitted before the first request settled.
An allocation pass, an adapter packet, or a production output batch whose key
another request holds is skipped the same way.

A `Pooled` or `ExactAccounts` source policy selects accounts; it does not prove
cross-custodian delegation, which only a `Granted` policy backed by an access
grant expresses.
The host application controls use of pooled demands and requester identity.
Holder-relative reports keep their existing visibility contract and do not
automatically disclose the source list. Standalone DTOs default a missing field
to `Pooled`; strict snapshots still require the exact engine and plugin identity.

`ResourceOperationRequestV1::RecordLoss(ResourceAccountLossRequestV1)` records
an account-level loss with its own `loss_id` and cause. The phase-7 lifecycle
writer debits the account in place and settles a `ResourceLoss` with
`account: Some(..)` and `transfer: None`, which conservation counts as admitted
loss under `ResourceOperationKind::Loss`. The debit never touches reserved
stock, respects the protected floor unless `allow_protected` is set, and
requires the exact account revision and a completion certificate; a stale
revision rejects. A tracked command must come from the account custodian, and
canonical adapter ingress may cite the cause record as its provider source.
Holder observation heads, reports, and witnesses carry
`ResourceLossObservationV1` entries gated like transfer details.

`ResourceOperationRequestV1::BeginExchange(ResourceExchangeStartRequestV1)`
starts two `ResourceTransferStartRequestV1` legs atomically: both transfers are
created or neither is, and the single `BeginExchange` outcome cites both
transfer IDs in `cited_transfers` whether it applied or rejected. There is no
exchange-level certificate. The parties agree on `ResourceExchangeTermsV1`
(the exchange key plus each leg's transfer ID, exact allocation, and
destination), and `leg_operation_keys` derives each leg's operation key from
the terms digest. Each leg carries its own completion certificate for that
derived key, held by the leg's source custodian, so each lease consents to the
whole exchange and authorizes no other terms. A tracked exchange command must
come from the `leg_a` source custodian. The two transfers' terminal
dispositions remain independent.

`ResourceTransferDispositionV1::AcceptLocal` settles a transfer that is still
`PendingDispatch` with no transport link when its source and destination
accounts declare the same `ResourceAccount::place_scope`; the terminal
certificate locks the exact `handover_evidence` record. A missing or mismatched
scope rejects as `invalid_definition` and an attached transport as
`invalid_lifecycle`. The scope is host-declared when the scenario installs an
account, immutable, and defaults to `None`; a tracked `CreateAccount` that sets
it fails with `InvalidAuthority`. A tracked command must come from the
destination custodian. None of these operations grants access to another
custodian's stock; only an access grant does. The resource plugin semantic hash
and holder-report knowledge schema cover these operations, and fields left at
their defaults are omitted from the canonical encoding.

A delegated access grant (`ResourceAccessGrantV1`) records a
grantor custodian's consent that a grantee may draw on its stock of one exact
resource and unit revision, up to `cap_quantity` within the half-open window
`valid_from..valid_until`, citing the exact `authority_evidence` record version
that justifies it. Grantor and grantee are `KnowledgeHolderRef` values.
`ResourceOperationRequestV1::IssueAccessGrant` is admitted only as a tracked
command whose subject is the grantor custodian and whose authority evidence is
the exact current version of its record. Admission checks this against the
current-version provenance index rather than retained evidence, so a run whose
evidence was sealed admits exactly what its replay admits. An earlier version
is rejected: without retained evidence its establishment source cannot be
proven, and the applied outcome keeps the citation as exact evidence that
resource restore checks. `RevokeAccessGrant` also comes from the grantor, with
the expected grant revision, and is refused once anything is reserved or
debited under the grant. Adapter ingress cannot issue or revoke grants. The
resource command descriptor therefore reads the administrative domain-record
set, under which `SimulationView::current_domain_record_version` resolves a
record of any kind.

A demand draws on a grant through
`ResourceDemandSourcePolicyV1::Granted { grant_id, accounts }`. Admission and
allocation require the grant to be active with the requester as grantee, the
demand's resource and unit to match, the demand's window to lie inside the
grant window, every listed account to be open and custodied by the grantor,
and the quantity to fit the remaining cap. The grant is checked before
scarcity arbitration, and nothing falls back to pooled or other accounts.
`ResourceAccessGrantRecordV1` keeps `cap_quantity = remaining + reserved +
debited`: allocation moves quantity to reserved, its debit moves it to debited,
so each unit is charged once, and released or expired reservations return to
remaining. `ResourceState::validate` rejects a grant whose reserved or debited
quantity disagrees with the reservations, transfers, and consumptions drawn
under it, and a grant-backed transfer must name exactly the grant its demand
drew on.

The grantor's consent is the grant; the debit of a granted allocation runs
under the grantee's own completion lease. An activated lease does not expire,
so a granted debit settles only in a boundary whose time equals the debit's
certified time, and the grant window must contain that time; a debit certified
inside the window but settled later is rejected. In an exchange the rule
applies per granted leg, and a granted allocation pass supplies nothing unless
the grant is current at both its requested time and the settling boundary. A
lease held by
anyone else, including the grantor, cannot debit a granted allocation. A
tracked `BeginTransfer` or `BeginExchange` leg on a granted allocation comes
from the grantee, the transfer records its `access_grant`, and the grantee
controls its cancellation, return, and loss. Grants stay hot, bounded by
`MAX_RESOURCE_ACCESS_GRANTS`, and `resource_access_grant_status` is the
holder-bound read for the grantor or grantee. Who may grant whom stays
application authority.

`AmendDemand` cannot change a demand's lifecycle status, rejection reason, or
requester; such an amendment becomes a durable rejection. A live production
completion settles its output credit: the credit cites the production runtime
version pinned as the execution's `output_source`, and the resource runtime
accepts the locked record at or after its locked version while requiring the
pinned source exactly. Production credits settle only through the production
output batch ingress; the generic resource adapter ingress rejects them.

Non-force consumption providers publish a top-level `resource_consumption_intents`
map in their owned, active domain record. Map keys equal the IDs of sealed
`ResourceConsumptionIntentV1` entries. The resource adapter requires exactly one
Authorized entry matching the complete allocation leg, demand/account revisions,
consumption identity and operation key; it rejects maps larger than
`ResourceLimitsV1::max_operation_outcomes` before decoding entries. Register the
source kind with `ResourcePlugin::new`, and submit through canonical adapter
ingress with the exact current source as `consumer_evidence`. The completion
certificate must bind that source and operation key, with the existing holder,
participant, time and lease checks. A digest verifies content consistency; it
does not grant authority. The provider owns intent authorization and retirement;
resource settlement owns the debit and receipt. The force-supply reference keeps
its specialized adapter, which also accepts an earlier version of its source
record. This provider contract is part of the resource plugin semantic identity
and introduces no callback registry or core schema.

`canwu-production` is a downstream production-asset extension. It owns
processes, sites, facilities, capacity allocation, work orders, work in
progress, production execution, facility projects, and output settlement. It
consumes exact resource outcomes and process-specific technology evidence. A
site form such as household work, distributed workshop, government workshop,
or concentrated plant is revisioned data rather than a universal building
level. Roads, canals, fortifications, institutions, money, trade, and combat
remain outside this extension unless a replaceable integration explicitly
composes them.

`ProductionOperation::CompleteExecution` may carry
`realized_output_per_mille` and `realization_evidence`. Phase 7 floor-scales
every output settlement quantity by the ratio and stores both on the
execution; `None` means `NOMINAL_REALIZED_OUTPUT_PER_MILLE` (1,000), and
evidence is rejected for a nominal ratio. Any other ratio must be positive
(a total loss cancels the work order instead), must not scale any output leg to
zero, may not exceed the process revision's `max_realized_per_mille` (default
1,000; a larger bound admits evidenced yields above nominal), and requires
evidence of a kind listed in the process revision's
`realization_evidence_kinds`, which is empty by default so a default process
admits only nominal output. The evidence must be the current exact
domain-record version of its record. The holder, lifecycle, ratio, and kind
rules run before the evidence record is resolved, so a rejection does not
reveal whether another record exists. The resource credit and output
acknowledgement settle exactly the scaled quantities. The production plugin
declares the administrative domain-record read to resolve that evidence; a
nominal completion omits both fields from its serialized form.

A seal keeps every record's current version but drops an earlier version's
body, and its receipt unless a live dependency declares it, while exact replay
keeps both, and a plugin view cannot load archived bodies. Production admission
therefore reads only current versions: every exact domain-record version that
a `StartExecution` or `CreateFacilityProject` cites, or reads through a cited
record such as an adoption's application or an observation's attempt, must be
the current version of its record when the operation is applied, as must
realization evidence.
`AdvanceFacilityProject` does not reread a project's provider and technology
evidence, whose exact bodies and bindings its creation already validated, and
rechecks only its live resource evidence. A compact run therefore admits and
rejects exactly the production commands its exact replay does. Every hot
execution and facility project, terminal ones included, declares the record
versions restore validation reads for it as payload-required until the
archive removes it; neither the production root nor the resource root
declares an initial-scenario version, which stays readable after any seal and
which the engine refuses as a declaration.

The production archive follows the resource archive: each commit directory
names the root of a cumulative lookup index under
`canwu.production.archive.lookup-page`, one commit carries up to
`MAX_ARCHIVE_COMMIT_PAGES` (16) blobs of `MAX_ARCHIVE_PAGE_ENTRIES` (64)
records, the maintenance receipt of an applied commit is an archive candidate
that the next commit archives first, so the root holds at most one and a run
may make any number of commits, and `archived_production_terminal_record`
reads the head directory, one page per level, and one blob. Settling a commit
reads no store: `enqueue_production_archive` requires the batch to equal what
the current root and the store derive, the boundary rebuilds an applied
commit's blobs and directory from the root's records and compares them,
records any other commit as stale, keeps the applied handle of a duplicate,
and settles a repeated acknowledgement as a no-op; archive commits settle
before the plugin's other packets. Restore validation resolves each compacted
resource input through the resource archive's lookup index and audits that
archive once whenever it has a head.

No production packet fails a boundary by not decoding, and no completion step
or continuation witness by being refused: each emits
`canwu.production.ingress_refused.v1` (an acknowledgement
`canwu.production.output_ack_refused.v1`) and settles nothing. An operation
packet settles only when the admission of a tracked production command queued
it, so every applied operation went through command admission. The output
acknowledgement carries `settled_at`, every outcome's request digest commits
to it, and the execution records it as `output_settled_at`, so an output
batch the resource plugin deferred still settles and archives.
`StartExecution` refuses, as a recorded rejection, an output key under the
reserved prefix, one another unsettled execution names, or one a resource
outcome holds in the root or its archive, more than
`MAX_PRODUCTION_OUTPUT_BATCH_CREDITS` (64) outputs, and an output set the
resource root can never hold; a process may declare at most 64 outputs. What
still fails a production boundary is a failed store read, an engine
invariant, counter overflow, and a later system's write past the root's
256 MiB byte cap; since 0.18.0 a forged completion step, witness, archive
commit, or acknowledgement another plugin schedules is refused with a trace.
See the
[production archive volume proposal](proposals/production-archive-volume.md).

Since 0.18.0 production scales by **scope roots**: a host installs one
`ProductionState` per scope (one per county, say) under
`canwu.production:runtime:scope:{scope}` beside the unchanged primary root,
every packet names its root, and the lifecycle system loads, settles,
writes, and journals only the roots a boundary's packets name, then writes
a small boundary activity record that tells the later systems which roots
changed their production lifecycle and by which packet, so a boundary with
no production work reads one small record and the 64,000-site world
settles a scope's batch as fast as the 4,096-site one. Each root carries its
own `ProductionLimitsV1`, fixed by the scenario (canonical 16,384 sites,
facilities, work orders, executions, and allocations; 4,096 operations and
1,024 batch items a boundary), and the per-root budgets add up across
roots. A scope root's lease and output keys begin with `{scope}/` and the
primary root's hold no `/`, so roots never share a resource-side key. A
host settles many operations of one holder against one root in one tracked
`apply_production_batch_v1` command, item by item with per-item outcomes; a
batch with a stale expected revision, or past the root's operations for the
boundary, is refused whole and sent again. Nothing a root settles stays for
life: ended work orders, ended leases, and unclaimed outcomes archive as
residues, incident receipts fold into a digest chain, and dominated
rolled-out observation heads are pruned. See the
[production at series scale proposal](proposals/production-series-scale.md).

`canwu-force-supply-reference` proves that a second independent domain can
consume the same resource API. It owns force-local recurring demand,
consumption intent, readiness and shortage consequences, and the requisition
saga. Its package archive, which the economy reference also uses, keeps the
resource archive's lookup index under its own namespace, so a lookup inside a
boundary reads the head directory, one page per level, and one blob on a hit;
a reused operation ID is skipped with an `operation_rejected` event; and the
enqueue helpers refuse, host-side, a commit the root's receipt or handle room
cannot take. Since 0.18.0 the acknowledgement of a commit's retention handle
folds every receipt recorded under it into the root's digest chain
(`PackageArchiveReceiptChainV1`), so a root keeps only the receipts of
unacknowledged commits (at most `MAX_PACKAGE_ARCHIVE_MAINTENANCE_RECEIPTS`,
64) however many commits a run makes; an operation whose outcome finds no
hot slot, or whose result passes the share of the byte cap operations may
fill, is skipped with its own `operation_skipped` event (`archive_backlog`
or `root_limit`), records nothing, and settles under the same ID once an
archive commit made room; and `force_operation_outcome` and
`economy_operation_outcome` read a settled outcome hot or archived. See the
[reference archive receipts proposal](proposals/reference-archive-receipts.md).
It cannot write civilian population, cooperation, harvest, property, or
occupation state; the receiving integration applies or rejects those typed
externality intents at an exact expected revision. In `canwu-economy-reference`
the exact target is the local economy, which must be unchanged since the
revision it had when the requisition locked the economy record.

`canwu-economy-reference` composes these packages with routing and transport in
a runnable synthetic grain loop. It also provides detached, holder-bound local
scarcity and evidence-qualified price-pressure projections. Those projections
are read-only decision inputs: they neither move resources nor form a market,
and price pressure remains explicitly unknown without a qualifying executed,
quoted, administered, or contracted price observation. Source-cited Ming and
China-industrialization model cards live in
`canwu-economy-reference-content`; they are replaceable content, not engine
truth or scenario branches.

`DomainRecordPage` is a trusted-host query bound to one authoritative revision,
kind, exclusive record cursor, and limit. Subsequent pages reject a stale
revision. Boundary views use the same ordered kind range and merge only bounded
overlay pages, avoiding copies of unrelated record kinds. Format 8 boundary
rollback and proposal overlays share persistent domain roots and validate
affected closures instead of cloning the whole domain map. The recorded
home-hardware profile still treats 100 sites as paced interactive use and 500
sites as non-interactive pressure evidence for the technology extension's own
semantic workload; see
[`benchmarks/2026-08-22-technology.md`](../benchmarks/2026-08-22-technology.md).

### Fiscal procedure / 财政程序

`canwu-fiscal` owns fiscal procedure, not resource balances. Its
`FiscalAuthorityBinding` names the standing `authorized_actor` of an
institution and may also name an `acting_actor` together with an exact
`authority_basis` record version, such as a host grant record. Both fields are
set together, the acting actor must differ from the authorized actor, and the
basis must be of an external record kind the plugin reads: one declared with
`FiscalPlugin::with_authority_basis_kinds` or an external evidence kind passed
to `FiscalPlugin::new`. Activation rejects a binding that cites any other
kind. A command from the acting actor, directly or as a decision-ticket
origin, is admitted only while the basis is the current version of its record,
and the fiscal settlement system checks it again against the state the
settling boundary began with. Once the basis record advances or retires, the
acting actor is rejected with `FISCAL_ACTING_BASIS_NOT_CURRENT`
(`InvalidAuthority`), while the authorized actor is still admitted. Every
basis check reads the current-version provenance index
(`SimulationView::current_domain_record_version`), whose answer a seal cannot
change.

Bindings start in the scenario and change at runtime only through a settled
rebind. `FiscalAction::RebindAuthority` replaces a binding's authorized actor,
acting actor, and authority basis in one commit, so a renewed grant or a new
office holder does not strand the binding. It is issued under the binding it
replaces, by the authorized actor or by the institution deciding as a body: a
decision controller whose command subject and decision origin are the
binding's institution, with no responsible actor. An acting actor cannot
rebind, and neither can any other person. The institution role is the power to
appoint, so a host registers a controller of that shape only for the body that
appoints the binding's actors. Every action carries the role its issuer held
at admission, and settlement re-checks that role against the state the action
settles on: an action of a replaced actor is rejected with
`FISCAL_ISSUER_NOT_BOUND`. The binding records the rebind in `rebound_by`.

A `FiscalAuditBinding`, held in its office record beside the authority
bindings, admits an auditor from outside an institution. Under it
only `RecordAudit` is admitted, only on targets whose scope the audited
institution owns, and only while the binding's own authority basis is current
(`FISCAL_AUTHORITY_BASIS_NOT_CURRENT` otherwise). Who installs and removes an
auditor is the institution's `FiscalAppointmentPolicy`: an `appointer` (an
institution or a person), the `nominators` whose `NominateAuditor` proposal
the appointer needs before it installs unless the policy says
`appoints_without_nomination`, and the `petitioners` whose
`PetitionAuditorRemoval` it needs before it removes unless it says
`removes_without_petition`. The appointer decides a pending proposal with
`DecideAuditAppointment`, and every settled finding names who entered it, who
appointed the auditor, and the nomination it rests on. Without a policy no
audit binding changes at runtime (`FISCAL_APPOINTMENT_POLICY_MISSING`);
`RebindAudit` keeps its 0.15.0 meaning only under a policy that makes the
audited institution its own appointer without nomination or petition. A policy
that names an appointer outside the audited institution makes an audit
independent; what the policy says stays an application decision.

The fiscal state is partitioned into records so that no boundary reads or
writes the whole of it: a root, the archive head, one office record per
institution (its authority bindings, audit bindings, appointment policy, hot
cycle totals, its scopes' mechanisms, and the observers that see it), one
scope ledger per scope binding (its adoptions and one accounting cycle book
per hot cycle, which holds that cycle's assessments, remissions, execution
requests, receipts, and audits), jurisdiction records, observer records, the
binding directory, scope index pages, report queue pages, and the report
schedule (`FiscalRecordKind`). `FiscalState` remains the model a host authors and
reads; `into_records` partitions it and `load_fiscal_state` assembles it. A
fiscal command is a `FiscalActionBatch` of up to `MAX_FISCAL_BATCH_ENTRIES`
(4,096) entries under one authority binding. Every action has a target
(`FiscalActionTarget`: a ledger, an office, a jurisdiction, or an observer),
names its target's current revision, and settles against the target alone, so
actions on different targets never make each other stale. A target keeps a
ring of its `FISCAL_RECENT_OUTCOMES` (8) latest outcomes and the revision
floor below which outcomes are forgotten; admission refuses a repeated action
ID in the ring, a revision above the target's, and one at or below the floor,
and refuses a whole batch for one inadmissible entry, naming it. Several
batches settle in one boundary, in admission order, each entry validated
against the state the previous one left, and no entry fails the boundary: it
is applied or rejected as its own outcome, published in
`canwu.fiscal.action_settled.v1`, and every limit is a rejected outcome, among
them `MAX_FISCAL_ACTIONS_PER_BOUNDARY` (65,536) actions and
`MAX_FISCAL_ACTIONS_PER_RECORD_PER_BOUNDARY` (64) per target in one boundary,
`MAX_FISCAL_PARTITION_KEYS` (4) partition keys per scope ledger, 64 hot books
per ledger and hot cycles per office, and the record byte budgets, 256 KiB
for a ledger and 1 MiB for an office or jurisdiction, each held at settlement
less a 4 KiB reserve for the outcome ring. Execution receipts arrive in
packets of up to `MAX_FISCAL_RECEIPTS_PER_PACKET` (4,096) through
`enqueue_execution_receipts`; a receipt or a historical-context change that
does not validate or fit is refused with one
`canwu.fiscal.ingress_rejected.v1` event carrying a stable reason, never a
failed boundary.

The two fiscal boundary systems are event-driven: `settle-fiscal-ingress-v1`
(phase 7) runs only on a boundary that admitted fiscal ingress and touches
only the records its packets name, and `materialize-fiscal-reports-v1` (phase
13) runs in every boundary that admitted an event or ingress, reads the root
first, and returns when no report is due and nothing is parked. A host whose
boundaries can pass without any event registers
`FiscalPlugin::with_report_cadence(cadence)`, the wake cadence: the
`SystemCadence` on which the phase-13 system
`wake-fiscal-reports-on-cadence-v1` runs, distinct from a binding's report
cadence, while the report system itself stays event-driven. On a boundary
of that cadence with no
event it reads the root alone and, when a report is due, schedules the drain
packet due at once, so the next boundary at the same instant runs the report
system (`EventDriven` is refused; the cadence is part of the plugin
descriptor). Plugin activation audits the records instead
(`audit_fiscal_records`), so a record set that is not the exact partition of
a valid state is refused when a run is created or restored.

`CloseAccountingCycle` closes a cycle's books; closed books leave the hot
records through fiscal archive commits that the host prepares with
`prepare_fiscal_archive(run, book_limit, store)`, stores, and queues
(`enqueue_fiscal_archive`, `finalize_fiscal_archive_retention`), in the
format and with the cumulative lookup index of the resource archive under
the fiscal namespaces, so an archived book or a cycle's institution totals
is read by key through the head directory, one page per level, and one blob.
The helpers read only the records a commit needs (the root, the archive
head, a partial read of every ledger, and the ledgers and offices of the
books the commit takes), never the whole fiscal state. A commit may cite
only records whose evidence lies at or below the run's sealed cut, which the
helpers read from the run itself (`sealed_evidence_cursor`), so a host
cannot pass a wrong cut; a run that retains all its evidence refuses
nothing, so a sealing host seals at each save, before it writes a
continuation checkpoint, if every archived citation is to lie in a sealed
segment.

An observer's report is held by the observer's knowledge holder, which may be
a government with no actor, as one summary record and, per hot cycle slot (64
per observer), parts of `FISCAL_REPORT_ROWS_PER_PART` (16) rows, published as
version 6 of the `fiscal_report` knowledge schema on stable lines (`1 + slot
× 1,024 + part`); a part lists its rows' institutions once and encodes rows
and facts as arrays, which `FiscalReportPart` decodes. A binding's `granularity` says what a row is: `Scope`, one
row per scope of the institutions it sees, or `Institution`, one row per
visible institution and mechanism, its facts the sums of the institution's
hot books of the cycle per unit and payment form, rendered at the binding's
confidence; a part of institution rows cites the office's current version
and counts the books it summed, which is weaker provenance than a scope
part's ledger citations. A binding's `cadence` says when the report is due:
`OnChange`, in the boundary that changes something it shows, or `Interval {
every, offset }`, at the aligned times `offset + k × every` after
`SimTime::EPOCH`, served by the first boundary at or after each due time
that runs the report system; once the report has been published, its
summary is republished at every due time, so silence never means "no
report". An active `Interval` binding always has
one queue entry, due at its next due time, which settlement adds work to
without moving it; the report schedule record keeps each queue page's
earliest due time and the pages with parked entries, and the root keeps the
earliest of all, so a boundary before it reads the root alone. The
materialization system publishes only the parts whose digest changed, each
superseding the current record of its own line, found with
`SimulationView::knowledge_heads`, so report supersession never depends on
retained evidence. It publishes within the fiscal share of the engine's
publication budget (`FiscalReportBudget`, set with `with_report_budget`:
by default half of `KnowledgeLimitsV1::CURRENT`, at least two bodies, two
entries, and 128 KiB, carried in the report system's name) and the budget
left at its cut, each holder's records one `Explicit` group packed into
directives up to the per-directive bounds; the parts left over wait in the
report queue, named
in the summary's pending list, and go out with the next boundary at the
same instant through the internal drain packet, without a fiscal change. An
observer whose holder is a dead person is parked; each run of the report
system re-checks the next `FISCAL_PARKED_CHECKS_PER_BOUNDARY` (16) parked
entries after the root's cursor, in rotation. The `fiscal_report` schema's
retention is the host's choice, `with_report_retention`
(`RetireWhenSuperseded` by default, so a holder keeps one record per line;
`Keep` keeps every superseded part). The wake cadence, the report share,
and the retention are recorded in the plugin descriptor and fixed for a
run. Hosts and clients read reports with
`fiscal_reports(&viewer)` and a trusted host any holder's with
`fiscal_reports_of_holder`. See the
[fiscal throughput and authority proposal](proposals/fiscal-throughput-and-authority.md),
the [fiscal and society limits proposal](proposals/fiscal-and-society-limits.md),
the [world-scale fiscal proposal](proposals/world-scale-fiscal.md), and
[fiscal observers at scale](proposals/fiscal-observers-at-scale.md).

### Reference content and starter kits / 参考内容与入门套件

The intended reference-kit layer is a first-party consumer of the public
contracts. It is a planned, growing product layer rather than a claim that the
repository already provides a dynamic content-pack loader. This design does
not introduce a required `ContentPack` runtime trait. It exists because a
developer should be able to begin with a complete, runnable simulation and
replace one part at a time, rather than design every domain record and
integration before seeing a result. It is intentionally a growing collection,
not one fixed demonstration package.

| Package kind | Owns | Does not own |
| --- | --- | --- |
| **Reference content pack** | Versioned technology, society, economy, scenario, localization, and provenance data | A new solver, the kernel, or a mandatory historical worldview |
| **Reference integration** | A public-API implementation that maps generic capabilities to a small world, production, or information model | Generic domain semantics or a user's world model |
| **Starter kit** | A runnable host, composition code, commands, projections, and a documented vertical slice | A privileged runtime path or a hidden engine dependency |

Reference content is data-first. A pack may be serialized data or a thin Rust
crate that produces owned serializable values. It should use namespaced IDs,
explicit schema versions, dependency declarations, licensing, and provenance.
The generic extension remains responsible for validation and settlement. A pack
must not require the extension to branch on a scenario ID or a historical case
label.

Reference integrations are deliberately replaceable. For example, a simple
technology integration can map `fiber_source`, `clean_water`, and
`sheet_forming` capabilities to its own buildings and resources. Another game
can use the same content pack with a different economy or map adapter. The
integration may contain one or more runtime plugins and a host adapter, but it
must use only the supported public API.

Starter kits should demonstrate the complete public path: scenario creation,
content selection, validated commands, boundary settlement, actor-relative
projections, save/load, fork, and exact replay. Their code is reference code,
not a special engine mode. Larger collections can add more packs and
integrations without changing the kernel or forcing all users to load the same
content.

Content selection is resolved before a simulation run begins. The selected pack
identities, versions, schema versions, and content hashes belong in the run
manifest and initial scenario binding. Runtime systems consume the validated
materialized records rather than reading external files during settlement. This
keeps reference kits compatible with the existing plugin semantic environment,
snapshot validation, and exact replay guarantees.

The recommended growth path is:

1. Build one small but complete starter vertical slice using public APIs only.
2. Extract its reusable definitions into reference content packs.
3. Add a second integration that uses the same pack with a different world or
   economy model.
4. Grow the catalog by domain and period, while keeping each pack and
   integration independently versioned and replaceable.
5. Add discovery or registry tooling only after the package manifests and
   compatibility rules have proven stable.

## World, time, and events

A validated command produces an event and optional scheduled work. Internal
scheduled continuations are ordered by `(simulation timestamp, insertion
sequence)`. Host-facing work uses one persisted ingress queue for commands,
plugin-defined communication/acknowledgement/information packets, decision
mutations, and calendar work. Queue
order is `(due time, class, descending priority, issue time, ingress ID)`, with
classes ordered command, communication, acknowledgement, information,
decision, then scheduled system. Late input is rejected rather than inserted behind a
committed boundary. A boundary system may schedule a typed follow-up packet;
even a zero-delay packet becomes eligible only after the current admission cut,
so it settles at a second boundary at the same simulation timestamp instead of
retroactively changing the boundary that created it. New scheduled work must
use representable checked time arithmetic rather than saturation.
`canwu-time` exposes checked hour/day construction and checked time/duration
arithmetic for data-dependent values. Its convenience constructors and
operators never clamp; an out-of-range convenience operation fails loudly.
Initial `Scenario` values admit no army, person, or letter in transit:
in-flight state requires the command, event, correlation, and queue evidence
carried by a runtime snapshot. Scenario admission also rejects non-finite map
coordinates so every accepted state can round-trip through the JSON
persistence format.

A queued plugin ingress item can be withdrawn, strictly before its due time, by
its issuer only. `cancel_plugin_ingress` lets the host withdraw a public packet
it enqueued; `cancel_permitted_plugin_ingress` presents the owning plugin's
opaque registration permit for the item's exact internal packet type and covers
host-enqueued items of that type plus items the same plugin scheduled itself;
and a boundary system proposes `BoundaryDirective::CancelPluginIngress` for an
item its own plugin scheduled inside the engine, choosing targets from
`SimulationView::cancellable_plugin_ingress`. The withdrawal is appended as its
own `IngressPayload::PluginCancellation { cancelled, authority, reason }`
record, with `IngressCancellationAuthority` naming `Host`, `PluginPermit`, or
`BoundarySystem` and a canonical reason of at most
`MAX_INGRESS_CANCELLATION_REASON_BYTES`. The record is never queued or
admitted; its due time equals its issue time, the withdrawn item leaves the
queue in the same operation, and nothing is rolled back. The errors reuse
existing codes: due, admitted, archived, or already withdrawn items fail with
`LateIngress`; another issuer's item with `InvalidAuthority`; an unknown ID
with `EvidenceUnavailable`; a non-plugin target or invalid reason with
`InvalidPayload`; and a declared read-only run with `InteractionReadOnly`. A
boundary directive that names a foreign, due, or already withdrawn item, or
that repeats another proposal's target, fails the whole boundary. Like
enqueueing, a withdrawal does not advance the authoritative revision. The
ingress commitment, archive index, and boundary hash already cover the record,
restore rebuilds the pending queue without withdrawn items, exact replay
re-applies host and permit withdrawals at their journal cut, and a withdrawn
item never counts as delivered evidence.

An event's `correlation_id` identifies one authoritative causal root, not an
arbitrary time bucket. Event children must retain their parent's correlation;
boundary emissions must match their boundary's correlation; and one
correlation cannot be reused by unrelated command, boundary, system, or root
event chains. Snapshot validation pre-indexes boundary-emission ownership,
resolves parent IDs by their contiguous journal position, and carries the
resolved root forward in one pass. The correlation-specific work is
`O(E log C + M)` over raw input, where `M` is the number of recorded boundary
emissions; for a valid journal `M <= E`. It uses `O(E + C)` auxiliary storage
instead of recursively rescanning each parent chain or each boundary's
emissions. Runtime views use the same contiguous-ID rule for retained tails;
archived IDs deliberately return no record until an archive adapter is
supplied.

Runtime and snapshot validation share a `ValidationContext` evidence boundary.
The runtime backend resolves the retained tail and marks older, valid IDs as
archived; the snapshot backend resolves the complete journal. Cause and
directive rules are implemented once against that interface, while backend
availability determines only whether a historical record can be inspected.
This keeps compaction from silently changing the meaning of a valid reference
and makes malformed runtime input and malformed snapshot evidence follow the
same canonical checks.

`EventKind` is domain-neutral: `event_type()` returns the persisted `type` tag,
and `fields()` exposes its flattened structured payload. Domain owners may use
`from_payload` and `decode_payload` to keep strongly typed payloads outside the
generic event crate. For a plugin event, `event_type()` remains the compatibility
kind label `"plugin"`. Consumers that need its namespaced identity should use
`qualified_event_type()`, which returns `plugin_name.event_type`; the structured
`plugin` and `event_type` fields remain the authoritative serialized values.

Player-facing event projection reuses the same deterministic resolver for
built-in and plugin events. A plugin may register an `EventAudience` for an
event type (`public`, one or more actors, one knowledge holder,
`affected_actors`, or `private`) with `PluginRegistrar::register_event_audience`
in its persisted `PluginDescriptor`; an undeclared plugin event is private by
default. `CanwuViewer::visible_changes_since` returns the changes one viewer may
see under these rules. `Canwu::viewer_context(actor)` checks the actor against
the run configuration and returns a detached `ViewerContext` bound to the
current checkpoint hash; the context becomes stale after authoritative state
changes, and consumers such as `canwu_society::projection_for_viewer` reject a
context that no longer equals a freshly derived one. No viewer input can
upgrade a private event to public. This audience policy governs player
projections only; plugin system subscriptions and declared state reads remain
separate runtime permissions. Because the declaration is persisted with the
plugin descriptor, snapshot loading and replay use the same visibility rule.

```mermaid
sequenceDiagram
    participant Client
    participant API
    participant Sim
    participant Scheduler
    participant Knowledge

    Client->>API: reference-world movement command
    API->>Sim: validated plugin command envelope
    Sim->>Sim: validate all preconditions
    Sim->>Scheduler: schedule arrival
    Sim-->>Client: MoveOrdered event
    Client->>API: advance(1 day)
    API->>Scheduler: execute due work
    Scheduler->>Sim: ArmyArrival
    Sim->>Knowledge: commander update now
    Sim->>Scheduler: delayed report to observer
    Sim-->>Client: attributable events
```

## Public interfaces

- Programmatic API: generic entity identity, typed domain-record reads,
  commands, events, time, snapshots, forks, schemas, and plugin descriptors.
- Domain integration API: typed plugin commands and detached read projections;
  `canwu-reference-world` is the runnable example, not a foundation dependency.
- Viewer API: actor- or institution-scoped knowledge queries through
  `CanwuViewer`, evidence-free rule-evaluation traces through
  `CanwuViewer::evaluation_traces`, plus event/failure explanations from
  committed evidence.
- Debug API: trusted entity/domain-record/event reads. The reference UI uses
  the same command dispatcher and reference-world plugin as every other client.

Packaged engine-user skills live under `agent-interface/`; the `canwu-engine`
plugin teaches external agents to use the public API and domain integrations.
Repository contributor skills live natively under `.agents/skills/`, with
Claude-compatible loaders under `.claude/skills/`. These agent tools are
development interfaces and are separate from runtime `SimulationPlugin`
implementations registered in `canwu-sim`.

## Knowledge model

Ground truth and knowledge are separate stores. The original actor-relative
army observations remain a compatibility projection. The general mechanism is
an append-only holder ledger keyed by `KnowledgeHolderRef`, which can identify a
person or an eligible institution/entity. Plugins register versioned,
namespaced `PluginKnowledgeSchema` contracts and receive explicit
`KnowledgeWriteGrant` capabilities; ordinary world, component, and domain-record
write ownership does not imply permission to publish knowledge.

A `KnowledgeRecord` preserves schema, holder, typed subjects, JSON payload,
reported and learned times, confidence, origin evidence, and explicit
supersession or contradiction links. It is the assembled form a reader gets:
the ledger stores each **record body** (`KnowledgeRecordBody`: everything but
the holder and the relations) once, and gives each holder a **holder entry**
(`HolderEntry`) with the holder's own stored local ID, from the holder's own
counter, and the holder's own `supersedes` and `contradicts` lists. One body
can therefore serve many holders, and a holder cannot tell a shared body from
a private one. Publication enters only through validated phase-4 or phase-13
`PublishKnowledge` directives, each a list of groups (`KnowledgePublicationGroup`:
sorted, unique holders and the drafts published to every one of them) under
one supersession mode: `KnowledgeSupersession::Explicit` applies each draft's
own relations to the one holder of its group, and `CurrentOnSubjects` lets
the kernel compute, per holder, the holder's current records of the draft's
kind that carry every subject of the draft and name them in that holder's
entry. The kernel enforces schema ownership, holder eligibility, declared
visibility, relation integrity, canonical ordering, per-directive bounds,
the per-boundary publication budget, atomic settlement, and persisted
`KnowledgePublished` evidence (one event per directive, naming every holder,
with audience `EventAudience::KnowledgeHolders`). It does not interpret
whether a record is a message, report, intercepted copy, sensor observation,
rumor, or analysis; transport, interception, audience expansion,
interpretation, belief change, and presentation policy remain extension
concerns.

A boundary has one **publication budget** (`KnowledgeLimitsV1`: 32,768
record bodies, 32,768 holder entries, and 64 MiB of payload per boundary;
1,000 bodies and 4,096 entries per directive; 64 KiB per body), spent by
every publishing system in dispatch order: phase 4 before phase 13, and
within a phase in the registry's `(phase, plugin, system)` order. A system
reads what is left with `SimulationView::knowledge_publication_budget()`,
which returns the remainder after every proposal of earlier phases and of
earlier systems of its own phase, with the per-directive bounds, and needs
no read declaration; `KnowledgeRecordDraft::payload_bytes()` is exactly the
byte charge. A publisher that reads the budget and defers what does not fit
never fails a boundary; one that overshoots fails it with
`KnowledgeLimitExceeded`, because a publication the kernel cut would leave
the plugin's own state writes believing the records were published. There is
no per-system share by default: fairness between plugins is the host's
composition problem, and the ordered read makes it deterministic. Since
0.18.0 a host may give a plugin a **publication reservation**
(`KnowledgePublicationReservation`, set with
`PluginRegistrar::reserve_knowledge_publication` or by registering the
plugin through `KnowledgeReservedPlugin`): a share of every boundary's
budget, held at each boundary where the plugin's knowledge-writing system is
due, that systems of other plugins are not shown and cannot take, whatever
their dispatch order. The reserving plugin reads its share through the same
`knowledge_publication_budget()`. The reservation is part of the plugin
descriptor, so of the run's identity; restore validation charges each
knowledge change by the same rule. Overshooting what a system was shown
still fails the boundary.

Each registered `PluginKnowledgeSchema` declares a **retention** policy
(`KnowledgeRetention`), part of the plugin descriptor. Under `Keep` every
entry stays for the life of the run. Under `RetireWhenSuperseded`, when a
new entry of holder *H* supersedes *H*'s entry *S*, *S* is cleared from *H*
in the commit of the publishing boundary: it leaves the entries, the
local-ID map, and the current-record index; every entry of *H* that named
*S* drops that ID, so every relation of an entry resolves within its holder
at all times; and a body no holder holds leaves the ledger. Retention
follows the superseding entry's schema, never fails a boundary, and is a
state transition of the boundary, never tied to a seal, so a sealed run and
its unsealed twin keep the same state hash and checkpoint hash. `FullHistory`
of a retiring schema shows only current entries and `KnowledgeQueryResult::retention`
says so; `knowledge_changes_by_correlation*` keep returning what a boundary
published, from retained evidence; saves hold only current entries. Local
IDs never move or restart: a holder emptied by retention keeps its counter.

A record may supersede or contradict only records of the same holder and
schema kind that are visible at the publisher's cut: records committed by an
earlier boundary, whatever their learned time, and records the same boundary
published with `SameBoundary` visibility in an earlier phase. Settlement and
snapshot validation share this rule, so a load accepts a relation exactly when
settlement admits it. A publishing system finds those records with
`SimulationView::knowledge_heads`, which returns the global IDs of a holder's
current records of one schema kind, selected by typed subjects, from committed
state and the boundary being settled. It returns at most 64 IDs, the newest
when more match, and does not depend on retained evidence, so a sealed run
supersedes the same records as its exact replay.
`knowledge_changes_by_correlation` and its prefix form scan retained boundary
evidence and must not drive a decision that has to replay exactly.

The holder ledgers are a persistent structure (`HolderLedgers`: a shared
store of bodies and a map of `HolderLedger`s built on `im`) that the runtime
shares between the committed state, every transaction checkpoint, the stable
state of phases 1 to 9, and forks instead of copying; a publication copies
only the paths it changes, and the journal shares the ledger's `Arc` bodies
instead of copying them, so a body is encoded once and hashed once per
boundary. Each holder ledger keeps a derived index of its current records by
schema kind and by kind and subject, keyed by SipHash keys that store the
exact kind or subject beside the key and fall back to exact comparison
after any collision (a lookup never depends on key values, and neither the
commitment nor a restore uses them), rebuilt on decode and never
persisted, which `knowledge_heads` reads, and the ledgers keep a count of
bodies per namespace. A boundary that publishes nothing therefore does no
work in proportion to the knowledge a run holds, and a publishing boundary
pays for its own bodies and entries. Under `Keep`, superseded entries stay
in the ledger, in `FullHistory` views, and in every snapshot, as before. The
iterators of these ledgers, and of the persistent decision log, are forward
only: `im` 15.1 drops elements when an ordered collection is iterated
backwards past three tree levels, so the engine keys each set of current
records newest first and never walks an `im` map in reverse. The commitment
root of the knowledge is `canwu.commitment.knowledge.v3` over the actor
projection root and a chunked ledger root: a 32-byte leaf hash per entry,
cached and recomputed only where an entry changed; chunk roots over 256
local IDs; a holder root over its chunks; the ledger root over the holder
roots (commitment format 6), so an entry can leave and a touched entry
rehashes one chunk. The boundary record hash covers every field of the
boundary's knowledge changes, bodies by digest. A restore rebuilds the
ledger boundary by boundary from the journal, recomputes every
`CurrentOnSubjects` list, applies retention, checks each directive's bounds
and the budget, and compares the result with the saved ledger. Evidence
that cites a record at its initial-scenario version resolves through an
index of `(reference, version)` built once per scenario and shared by the
runtime, its forks, and snapshot validation. See the
[knowledge ledger cost and authentication proposal](proposals/knowledge-ledger-cost-and-authentication.md)
and [fiscal observers at scale](proposals/fiscal-observers-at-scale.md).

Holder queries are owned, bounded, and deterministically ordered. Current-head
and full-history views can filter by schema, subject, and learned-time cuts.
Pagination cursors bind the holder, canonical query hash, holder-relative read
cut, and final record position, so a cursor cannot be reused against a changed
query or a changed holder projection. Holder-local record IDs are stored
with each entry, come from the holder's own counter, and hide unrelated
global ledger gaps; a page is ordered by `(learned_at, local)`, so a cursor
stays valid across the clearing of other entries. Read-cut roots and cursor
bindings are derived during reads; no mutable query index is persisted or
included in the authoritative state hash. Any future cache or secondary
index must be rebuilt from the canonical ledger and remain outside
authoritative commitments.

## Person availability and creation

Person availability is a separate ordered core map keyed by `PersonId`, not a
field of the legacy `Person` projection. `PersonAvailability` holds a
`LifeState` (`Alive`, `Dead`, `Missing`), a `CustodyState` (`Free`,
`Detained`, `Hostage`, `Captive`, `Hiding`, `Exile`), an optional custodian
entity, and the time it took effect. A person without an entry is alive and
free. `PersonAvailability::is_available` is false when the person is not alive
or is detained or captive; hostage, hiding, and exile stay admissible, and an
application may restrict them further in its own rules.

Only a boundary system changes availability, through
`BoundaryDirective::SetPersonAvailability` from phase 7 or phase 10, and only
when its contract declares `StateKey::core_person_availability()` as a write.
The core key is kernel-owned, so several systems may declare it; two writes for
the same person in one boundary fail with `DuplicateBoundaryWriter` and roll the
boundary back. Scenarios cannot declare initial availability. Each committed
change is recorded as a `BoundaryPersonAvailabilityChange` on the hash-chained
boundary record.

Unavailability has fixed admission consequences. Command admission rejects a
request with `IssuerUnavailable` when `Issuer::Actor`, the authority's decision
actor, or an institution's responsible actor is unavailable. Decision rules are
described under the decision framework. Knowledge publication to a dead
person's holder ledger fails with `InvalidKnowledgeHolder`. At the end of the
boundary that makes a person unavailable, after random decision ingress is
materialized, the kernel cancels every open ticket whose decision maker is that
person with `DECISION_MAKER_UNAVAILABLE_REASON` (`decision_maker_unavailable`),
then every remaining open ticket whose assigned controller's authority person
is that person with `CONTROLLER_AUTHORITY_UNAVAILABLE_REASON`
(`controller_authority_unavailable`). A ticket that qualifies for both carries
the decision-maker reason. The cancelled IDs are recorded in ticket-ID order in
the change's `cancelled_tickets` and `cancelled_controller_tickets`, and
snapshot validation recomputes both lists.

`BoundaryDirective::CreatePerson` creates a person at runtime from a phase-7
system that declares `StateKey::core_people()`. The `PersonDraft` supplies name,
roles, initial availability, committed provenance, and an optional
`PersonPlacement` (a government and a location in the `WorldSnapshot`
projection). A placed draft creates a `Person` record there. An unplaced draft
creates only an identity in the entity registry with availability and a
creation receipt: `SimulationView::person` and `Canwu::world` hold no record
for it, its name and roles are kept on the creation evidence only, and the
application keeps the person's attributes in its own records. The built-in
movement, letter, and army-arrival report paths read the projection, so they
do not reach an unplaced person. In both cases the kernel allocates the
`PersonId` from a persisted monotonic counter that starts past every scenario
person. The person commits at the end of the
boundary and is visible to systems from the next boundary. The correlation must
be unique per plugin, system, and boundary. `BoundaryReceipt::created_persons`
and the persisted created-person registry bind it to the allocated ID, which a
system later reads with `SimulationView::persons_created_by_correlation`; the
lookup does not depend on retained evidence. Fork and exact replay reproduce
the same IDs.

Hosts read committed availability through `Canwu::person_availability` and its
compact counterpart; boundary systems use `SimulationView::person_availability`
after declaring the read. The snapshot's `person_availability` map,
`created_persons` registry, and person counter are omitted while empty or zero,
so a run that never uses these contracts serializes and hashes the same as a run
without them. When present, they become optional sub-roots of the world
commitment and a counter in the control root. Validation rebuilds availability,
the registry, and the counter from boundary evidence, and checks that the
projection's `Person` records are exactly the scenario's and the placed
creations'.

## Routing and transport execution

Routing and transport are additive domain capabilities, not a second kernel
scheduler or a replacement for the information ledger. Their ownership split
is:

| Layer | Responsibility |
| --- | --- |
| reference integration or domain records | topology, stations, timetables, availability, terrain, and historical content |
| `canwu-routing` | pure deterministic planning over an actor-relative `PlanningSnapshot` |
| `canwu-transport` | itinerary revisions, leg execution, custody handoffs, bookings, capacity pools, pure booking allocation, and completion saga records |
| `canwu-movement` | the lifecycle of movement orders and transport executions: leg settlement, capacity-pool allocation, incidents, retention, and holder-relative movement reports |
| `canwu-information` | per-recipient delivery attempts and immutable logical deadlines |
| `canwu-correspondence` | communication demand, holder-relative address/network resolution, accepted routes, incident policy, and exact cross-extension evidence |
| application/channel/infrastructure adapter | prepares content and dispatches, publishes period-specific network knowledge, decides incidents and hazards, and admits scarce capacity outside `canwu-movement` pools |
| `canwu-sim` | canonical ingress, scheduling, rollback, persistence, and replay |

`RoutePlan.estimated_arrival_at` is an execution estimate. It must not rewrite
`DeliveryAttempt.due_at`, which is the immutable information-completion
deadline. A reroute supersedes an itinerary revision without creating a new
attempt; a retry creates a successor attempt record. A failed attempt leaves its
dispatch active until a sender-authorized recovery command either creates that
successor attempt with a fresh execution and deadline or explicitly finalizes
the dispatch. Replanning the same attempt is available only while it is
`WaitingForRoute` and transport is `ReplanPending`. Physical handoff is distinct
from knowledge relay, and arrival first enters an explicit arrival-pending saga
before an admitted completion operation reconciles the two extensions.

The correspondence plugin plans and resolves the address from the carrier
holder's ledger. A carrier other than the sender is admitted only when the
request's `carrier_delegation` cites an admitted `delegate_carrier_v1` command
(`CarrierDelegationRequest`) that the carrier issued under its own command
authority, whose `DelegationClaimV1` names the carrier as `performed_by`, the
sender as `performed_for`, lists `carry_correspondence`, and has a validity
interval covering each dispatch. The claim lives in the carrier's command
rather than in the sender's request, so a sender cannot vouch for itself or
plan from another holder's private ledger. The plugin records each accepted
delegation, from the next boundary, as the carrier's current delegation for that
sender; a newer delegation for the same pair replaces it, which is also how a
carrier withdraws one. Admission of an initiate or retry command resolves
`CorrespondenceIntent::carrier_authority` from that record rather than from the
command, so a run whose evidence was sealed decides exactly as its replay, and a
replaced delegation can no longer be cited. Settlement re-checks only the
admitted claim's validity interval, and replanning the same attempt does not
re-check it, so bound `expires_at` when the carrying should end.
Naming another holder without that delegation is not read authority. The
engine publishes none of the carrier's knowledge to the sender.

`CorrespondenceIncidentKind::CarrierSeized { seized_by, custody_handoff }` ends
the attempt, unlike `Interception`: the current leg fails, a terminal seizure
handoff is recorded under `custody_handoff`, the attempt closes as failed, and
the dispatch stays active for the sender's retry or finalization. A carrier
waiting for a route can also be seized. A zero handoff ID, a malformed seizer,
or the carrier as its own seizer rejects the incident; a handoff ID that
already names another handoff is kept as suppressed evidence. Only the carrier
holder receives an `attempt_report` (`CorrespondenceAttemptReport`), which does
not name the seizer; a sender that delegated the carrying learns of the seizure
only when the application relays it. The correspondence request contract
exposes only
`CorrespondenceCapacityAdmission::Unconstrained`: the plugin neither checks nor
persists a booking. Constrained transport requires a future admission variant
backed by exact booking or simulation-reservation evidence.

The plugin's holder-relative planning rule is also public.
`planning_snapshot_from_holder_knowledge(view, holder, observed_at)` reads only
the holder's current planning-knowledge heads through
`planning_knowledge_query`, admits exactly the endpoints and connections that
ledger asserts at the read cut the view derives, and fails when a known
connection names an unknown endpoint. It returns the `PlanningSnapshot`, whose
`knowledge_cut` commits to that cut, together with a `KnowledgeReadCutDigest`:
a hash over the holder and the versioned schema and holder-local ID of every
admitted fact. The digest is evidence of what the planner read, not a
commitment over the ledger. `planning_snapshot_from_knowledge_result` is the
pure form over a query result the caller already holds, such as a restricted
viewer's, and rejects a paginated result. The caller does not pass a read cut:
the engine derives it. The view read is system access, so callers must derive
`holder` from admitted authority, not from an unvalidated payload.

`MovementSubjectRole::PersonsGroup` moves an aggregate of people as one subject,
normally identified by an application domain record; like `Cargo`, it requires a
positive quantity, here a head count, and
`MovementSubjectRole::requires_quantity` states the rule. `Handoff.kind`
distinguishes `HandoffKind::Planned`, the default omitted from JSON, from
`Seizure { by }`, custody taken by an entity outside the itinerary. A seizure
follows the same leg rules as a planned handoff, except that a terminal seizure
names its failed source leg as both ends: custody leaves the itinerary and no
leg receives it. `ItineraryRevisionReason::ExternalCondition { record, version,
kind }` cites an application-owned condition record at an exact positive version
with a non-empty label and is validated for both the initial itinerary and a
reroute before mutation. Transport records these facts; incidents, hostility,
hazards, and their authority stay in application systems.

Transport also records capacity pools, and a terminal seizure closes an
execution to further progress (`TRANSPORT_SEMANTIC_VERSION` is
`canwu-transport.v5`). A `TransportCapacityPoolV1` offers a windowed quantity
of one interchangeable resource held by a custodian; confirmed bookings hold
`booked`, consumed bookings hold `consumed`, and the pool revision advances with
every change. `allocate_capacity_bookings` is a pure function over one pool
revision: it visits `CapacityBookingRequestV1` values by descending priority,
ascending `valid_from`, tie-break key, admission sequence, and booking
identity, confirms each booking in full when its window lies inside the pool
window, has not ended, and fits the remaining capacity, and otherwise fails it
with a `CapacityAllocationFailureV1` reason, so a later, smaller request may
still fit. Each `BookingAllocationV1` carries replayable
`CapacityBookingAllocationEvidenceV1` with a domain-separated digest. A booking
may be confirmed, failed, or cancelled before its window opens but consumed
only inside it; an execution can request a booking, be cancelled, and settle
its arrival without a delivery attempt. A terminal seizure must be recorded on
a leg of the active revision of a non-terminal execution, only one is allowed,
no other custody may leave that leg, and afterwards the execution can no
longer fail or reroute a leg, arrive, settle, book capacity, or record another
handoff: its owner closes it as failed or cancelled.

`canwu-movement` owns the lifecycle of these records, because plugins build
on `canwu-api`, which depends on `canwu-transport`, so a plugin inside
`canwu-transport` would form a dependency cycle. `MovementPlugin`
keeps every admitted order and execution in one `MovementState` record
(`canwu.movement:runtime`) and advances executions only through the transport
transition methods. The tracked command `apply_movement_operation_v1` carries a
`MovementCommandV1` from the issuer's own holder with an `Order`, `StartLeg`,
`CompleteLeg`, `FailLeg`, `Reroute`, `RecordHandoff`, `RequestBooking`,
`Cancel`, or `OfferPool` operation. Moving anyone but the owner needs an
`authority_basis`: the current version of a declared application record whose
references name the owner as `movement_grantee` and every other subject as
`movement_subject`. Seizures and reconciliations are accepted only from
application systems through the `movement_incident_v1` ingress with an exact
evidence record of a declared kind. Operation keys are unique per holder, or
per evidence kind for incidents, and each has one durable outcome.

The phase-7 system `movement_lifecycle_apply_v1` applies admitted operations
and incidents in admission order, runs one allocation pass per pool, applies
due legs from the internal `movement_leg_due_v1` ingress, and retires closed
executions; phase 8 validates the candidate runtime, and phase 13 publishes
`canwu.movement/movement_report` knowledge. Pools are allocated in phase 7
rather than through kernel phase-6 reservations, which must name fixed
identities at registration, may grant partial quantities, and cannot order by
window start or admission sequence. A departing leg consumes its confirmed
bookings; a leg whose booking failed or expired fails with
`capacity_unavailable`. Reports are holder-relative: the owner and operator see
the current execution, a delayed remote observer sees coarse progress as of its
delay, holders without a grant see nothing, and reports go only to living
person holders. A closed execution retires once final reports are out, and at
most 365 days after it closed; `MovementLimitsV1` bounds orders, pools,
outcomes, and reports with per-owner and per-holder quotas. The plugin never
draws randomness.

The router supports fixed, scheduled, recurring, and piecewise traversal.
Historical content can therefore express foot, horse, road, river, sea,
1900/1940 rail, air, telegraph, or other signal systems as data: a ferry
that sails every six hours is one `TraversalModel::Recurring { period,
offsets, duration }` pattern (departures at `EPOCH + k × period + offset`,
looked up exactly in integer minutes) rather than a slot per sailing.
`RoutingPolicy::algorithm` selects stable Dijkstra ordering
(`FifoDijkstraV1`, the default, for FIFO timetables) or a bounded
label-correcting search (`BoundedLabelCorrectingV1`) for non-FIFO
timetables. Since 0.18.0 (`canwu-routing.v2`) a host compiles a
`PlanningSnapshot` once per network revision into a `RoutingIndex`
(validated once, the network hashed once, a dense adjacency behind an `Arc`),
moves it to a new read cut with `reobserve`, and plans through it in about a
hundredth of the time of a cold `plan_route`; `plan_route` keeps its
signature and compiles a borrowed index per call. **Edge conditions**
(`EdgeConditionSet` of `EdgeConditionOverlay`s: a window on one connection
that is `Closed` or `Adjusted` in duration, resource cost, and risk) apply
by departure instant through `plan_route_with_conditions` without rebuilding
the network, and a plan records the set's digest in
`RoutePlan.edge_conditions_digest`. Capacity is a persistent transport
booking, not hidden mutable state in a route cache. `RoutingCache` is
derived, digest-keyed (`index_key` keys plans without hashing the network),
rebuildable, and excluded from authoritative commitments.

Movement uses the admitted `OrderMovement` intent. Army and person movement are
implemented by the same command boundary, while persisted movement state keeps
subject-specific transit and custody invariants. A person travelling
voluntarily uses actor-bound authority whose command subject is that same
person; army command and future cargo or equipment movement use distinct
capability policies. `EntityRef` remains an identity union rather than proof
of mobility, so unsupported subjects fail closed. The detailed boundary,
capability matrix, active-movement semantics, and migration rules are in
[`movement-order-mechanism.md`](proposals/movement-order-mechanism.md).

If a disaster occurs, a domain system records an explicit leg failure and
evidence. Transport enters `ReplanPending`, takes a new planning snapshot, and
installs a new immutable itinerary revision. The router never invents the
disaster, mutates world truth, or silently reads facts outside the observer's
knowledge cut. The full ownership and M1–M3 checklist lives in
[`docs/proposals/routing-transport-mechanism.md`](proposals/routing-transport-mechanism.md)
and [`docs/proposals/routing-transport-mechanism-todo.md`](proposals/routing-transport-mechanism-todo.md).
The implemented composition boundary and Wuxi delivery slice are documented in
[`docs/proposals/correspondence-mechanism.md`](proposals/correspondence-mechanism.md).

## Plugins

Plugins register schemas, typed command handlers, legacy
event reactors, and phased boundary systems. Registration is atomic:
duplicate plugin, command, system, schema, state owner, phase writer, or
reservation offerer claims reject the complete plugin registration without
changing the live registry. Immediate handlers use `SystemContract`;
authoritative phased systems use `BoundarySystemContract`, which declares
phase, cadence, reads, writes, reservation offers and requests, later allocation
reads, owned random streams, emitted records, and visibility. Immediate and
phased handlers cannot write the same `StateKey`. Every executable plugin also
declares a package version and a 64-character semantic hash; either value or
any serialized contract mismatch blocks snapshot rehydration and replay.

The runtime enforces declared reads for core collections, plugin components,
and reservation results. It rejects every component write that is undeclared or
targets another owner's `StateKey`. Persisted component identity is the typed
tuple `(plugin, state key, entity, component)`; text separators cannot alias
records. Executable order is always canonical and never depends on plugin
registration order.

Plugins may also register application-defined `DomainRecordSchema` values.
Each schema owns one namespaced `DomainRecordKind`, declares whether instances
are entity identities or non-entity records, validates payload fields, and
defines typed reference roles with cardinality and retired-target rules.
Instances use stable string `DomainRecordRef` identities and can be created,
updated with an expected version, retired with an optional same-kind successor,
or deleted only after retirement. Deletion retains a versioned tombstone so an
identity cannot be silently reused. The kernel validates the complete mutation
bundle, including cross-record references, schema ownership, successor state,
and external live dependencies, before commit. A successor must be active when
the retirement is admitted; later retirement of that successor can extend a
stable, cycle-free succession chain without invalidating earlier links. Domain
record collections are ordered and are queryable through both `Simulation` and
`Canwu`. Initial domain records may reference entities listed only in
`Scenario::entities`; snapshot restore accepts them just as the live state
check does. Scenarios that contain initial domain records must use a plugin-aware
constructor such as `new_with_plugins`; ordinary constructors reject them
instead of returning a half-configured runtime that could emit an unloadable
snapshot.

Domain packages can bind those stable identities to compile-time marker types
with `DomainRecordType` and `TypedDomainRecordRef<T>`. A sealed associated class
drives both schema classification and the automatically derived
`DomainEntityType` or `DomainValueType` capability, so a type cannot present a
record schema and an entity reference at the same time. Typed references
serialize exactly as `DomainRecordRef` and validate their namespaced kind during
deserialization. `DomainRecordSchema::for_entity` and `for_record`,
`DomainRecordDraft::from_typed`, typed simulation/view queries, and
`DomainRecord::decode_payload` provide a typed package path while the
authoritative snapshot keeps the existing schema-validated representation.
The typed path does not change the checkpoint or snapshot format.

Domain record state is boundary-only: immediate reactors and commands cannot
write a record kind as an untyped component. Boundary systems declare the
record kind's `StateKey`, propose `MutateRecord` directives, and read current or
invariant-candidate values through `domain_record` and
`proposed_domain_record`. This keeps lifecycle changes inside the same atomic
visibility and rollback contract as other authoritative domain changes.

## Phased settlement boundary

`settle_boundary(BoundaryRequest)` is the authoritative extension path for new
domain mechanics. It atomically executes internal scheduled continuations
strictly before the requested time, admits and processes due canonical ingress,
then executes equal-time internal scheduled continuations before taking the
immutable boundary snapshot. It visits all fourteen settlement phases in order.
Caller-supplied cadence categories are
canonicalized; event-driven systems are selected when admitted events or
ingress exist. `advance_canonical` and `step_canonical` select the earlier of
internal scheduled work and canonical ingress so hosts cannot step past due
work. Equal-time host ingress is processed before internal scheduled
continuations, preserving the declared ingress-before-scheduled-system order. A
system that declares `canwu.core.ingress` read access can resolve only the
admitted plugin packets owned by its own plugin; future, command, calendar,
and other-plugin payloads remain unavailable through that view. Systems within a
phase execute by `(plugin name, system name)`. The boundary builds one sparse,
non-iterated admission index from the packets admitted at that boundary, so
repeated lookups neither rescan the queue nor allocate against total history.

A system that carries work into a later boundary can budget the boundaries a
period of simulated time opens. After declaring `canwu.core.boundary_schedule`
read access, a phased boundary system calls
`SimulationView::boundaries_between(from, to)` for the half-open window
`from..to`. `BoundaryWindow` lists the boundaries settled in the window, the
boundary being settled when its time is in the window, and each distinct due
time still queued in the window in the canonical ingress and scheduled-work
queues. Each `BoundarySummary` carries the boundary's time, canonical cadences,
and admitted or queued ingress counted by class, kind (command, plugin packet
type, calendar, decision, or maintenance), and origin (`BoundaryIngressOrigin`:
ordinary host ingress, work a boundary system or command handler scheduled,
work recorded with an event as its cause, or engine and host-labelled work by
name), without payloads, so a host application can count sources it does not
own and classify them by its own rules. Grouping by the kind of cause rather
than the exact causing boundary or command keeps a summary's groups bounded by
the registered packet types. A system declares how far back it reads as
`BoundarySystemContract::boundary_schedule_horizon` (at most
`MAX_BOUNDARY_SCHEDULE_HORIZON`, 366 days, and only with the read); the run
keeps settled boundaries for the longest declared horizon, but each system may
look back only as far as its own, so a window that starts further back from the
boundary time than the reader's horizon returns `EvidenceUnavailable` whatever
other plugins declare. Hosts read back to the run's horizon
(`Canwu::boundary_schedule_horizon`). The core keeps no calendar:
`SimTime::day` is a plain count of 1,440-minute days, and any other period is
the application's to compute. Because the read names other plugins' queued
work, it is a deliberate exception to the reader-scoped `canwu.core.ingress`
view: only phased boundary systems may declare it, and plugin commands and
synchronous event-driven systems are refused at registration. The queued list
is a lower bound: a boundary's own proposals are queued at commit, and work not
yet queued or a boundary the host settles directly is not listed. The settled
ledger is an index derived from the boundary and ingress journals and the
persisted plugin descriptors: it is updated at commit, rolled back with a
failed boundary, kept across a seal, and rebuilt from the full journal on
restore and by exact replay, so live, sealed, restored, and replayed runs read
the same list, and it adds nothing to snapshots or commitments. Boundary
evidence does not record legacy scheduled actions, so only queued summaries
count them. Hosts read the ledger through `Canwu::boundaries_between` and
`CompactedCanwu::boundaries_between` and must not pass it to player, agent,
observer, or remote clients. See the [boundary window
proposal](proposals/boundary-window.md).

A host can turn on a wall-time profile of settled boundaries with
`set_boundary_profiling(true)` and take the profiles with
`take_boundary_profiles()`, on `Canwu` and `CompactedCanwu`. Each
`BoundaryProfile` gives the wall time of one settled boundary, of each
plugin handler it ran (boundary systems, plugin command handlers, and
event-driven plugin systems) by kind, phase, plugin, and name, and of each
engine phase (`BoundaryEnginePhase`: admission, system dispatch, staging,
commit, knowledge, journal, and hashing), so a host can check a plugin against
a time budget and see where the simulation core's own time goes; the handlers
and the engine phases partition the boundary's time to within one microsecond
per listed value. `replay_from_journal_with_boundary_profiles` profiles an
exact replay from its first recorded boundary and hands each profile over as
the boundary settles. The profiler is host context outside authoritative
state: it is never persisted, committed, or replayed, no plugin can read it,
settlement reads no clock while it is off, and a profiled run produces the
same receipts, evidence, and hashes. It is off in a new, restored, replayed,
and forked runtime, a failed boundary leaves no profile, and the runtime keeps
at most `MAX_RETAINED_BOUNDARY_PROFILES` (1,024) profiles that were not taken
yet. Owner-authorized maintenance participants run outside settlement and
appear in no profile.

The kernel owns ingress, snapshot, ordinary commit, and conditional-transition
commit. Phase-six systems publish resource capacity and competing claims.
Allocation sorts by pool, descending priority, explicit tie-break key, and
reservation identity, then records fulfilled, partial, or rejected results.
Only systems with an explicit `reservation_reads` declaration can consume an
allocation.

Phase-seven changes are staged against the immutable boundary snapshot.
Same-boundary values are exposed through the normal read-only overlay, while
next-boundary values remain hidden from current-state reads until settlement has
finished. Invariant systems can separately inspect every staged candidate with
`proposed_component` or `proposed_domain_record`, still subject to their declared
read set. Ordinary changes commit at phase nine. Historical candidates stage a
separate transition bundle for phase eleven, which the kernel first audits
against every transition manifest ready at the boundary (see below). Strategic aggregation and
perspective/report materialization use the same ownership and visibility rules.
Any fatal error restores time, queues, state, journals, random state, counters,
and boundary records to their pre-boundary values.

Each successful boundary persists its ID, time, correlation, cadence set,
admitted command attempts, accepted commands, admitted and boundary-generated
ingress, and events, reservation evidence, allocations, random draws, field
changes, domain record lifecycle changes, person availability changes with the
tickets they cancelled, created persons, rule-evaluation traces, transition
manifest registrations and audits, exact producer plugin/system/phase/
visibility provenance, a deterministic state hash, and the previous and current
boundary hashes. Every
committed domain record change has one indexed, causally linked evidence event.
Snapshot loading reconstructs the initial record store from this history,
deterministically reapplies each commit stage, and requires the result to equal
the persisted ordered store. It also reconstructs queued command attempts and
calendar cadences in admission order rather than treating boundary membership as
sufficient evidence. Reservations,
component writes, command authority, and event entities are checked against the
domain identities available to the originating proposal and after its atomic
commit stage, so rehashed evidence cannot consume another system's invisible
same-stage creation or refer to an entity before creation or after deletion.
Declared seat institutions must exist both in manifest-bound genesis and in the
persisted final state.
Boundary-caused events do not invoke
legacy immediate reactors; they enter the next boundary through normal event
admission. Format 8 snapshots validate this evidence and require exact plugin
identity and descriptor rehydration before continuation. Pre-8 saves and
journals are rejected before runtime construction; no legacy migration path
exists in the pre-1.0 engine.
Boundary-aware replay uses command admission lists to reconstruct operation
order and rejects any regenerated boundary whose complete evidence differs from
the journal.

The hot runtime keeps monotonic attempt, accepted-command, and event admission
cursors. Each settlement reads only the unadmitted journal tails and advances
the cursors after the boundary record commits, so admission work is proportional
to newly admitted evidence instead of all prior boundaries and journals.
Admitting one command is likewise independent of the retained history: the
retained commands, attempts, and ingress are `RetainedJournal` values that
change only through the runtime's push, truncate, seal, and restore methods,
which keep a derived `CommandHistoryIndex` (queued requests, the first attempt
per request, the first legacy-direct command, the first unqueued attempt) in
step, and the unqueued-history, legacy-command, cached-outcome, and
request-in-use checks read it instead of rescanning the journals. The index
is never persisted, hashed, or committed; a seal and every restore rebuild it,
and each answer equals the scan it replaces (see the
[command admission history index proposal](proposals/command-admission-history-index.md)).
What remains per command is the transaction checkpoint that clones the
people, letter, army, plugin component, and scheduled action maps for
rollback, which grows with the world's size, not its history. The
cursors are persisted derived metadata: loading validates them against the
global boundary-prefix proof, and failed settlement restores them with the rest
of the boundary. Format 8 does not derive them for older snapshots because
older snapshots are outside the supported load contract.

Append-only events, commands, command attempts, ingress, boundary records, and
random draws have one internal owner, `RuntimeEvidence`, separate from mutable
world/knowledge/plugin state. Public flat snapshots and replay journals retain
their existing serialized shapes. Checkpoint-journal format 6 is the separate
incremental persistence path: `SimulationCheckpoint` captures current state,
scheduler, counters, metadata, and the already-computed full commitment roots
while leaving every append-only evidence array empty. `EvidenceCursor` records
the exact cut through all six journals, and `EvidenceJournalSegment` stores only
the records after a prior cut; a segment's per-journal cursors need not end at
one boundary. Loading requires segments to start at the global
zero cut, remain contiguous, advance at least one journal, encode truthful end
cursors, and finish exactly at the checkpoint cut. It then reconstructs the
flat snapshot in memory and runs the current validation path, so
the checkpoint roots, boundary chain, IDs, authority, causal evidence, and exact
replay contract bind the archived records as they bind a flat snapshot.
`CheckpointJournal` is a portable full-save convenience envelope; incremental
stores should persist the smaller current-state checkpoint and only newly
appended segments. The content-addressed form of that checkpoint is the paged
checkpoint: `Canwu` and `CompactedCanwu` expose `prepare_paged_checkpoint`,
which reads current state and emits only pages that are new to a
`StatePageProvider` already holding the previous checkpoint's pages, and
`from_paged_checkpoint_and_journal` with its `_with_plugins` variant, which
restores from the provider and the contiguous evidence prefix through the
same validation as a flat snapshot. A compact host's prefix is its sealed
segments followed by `journal_segment_since` the end of the last one. Since
0.18.0 (paged format 7) every paged restore takes a host-supplied
**restore budget** (`PagedRestoreBudget`): each value the restore assembles
from pages is charged against it before it is allocated, and a value past
it fails with `RestoreBudgetExceeded` before any chunk is read. The budget
bounds assembled bytes, not heap; a host restoring its own checkpoint
passes the descriptor's `assembled_bytes`, and for untrusted pages passes
a budget near the size it expects and authenticates the root page ID.
The knowledge ledger is paged as two content-addressed maps (bodies by ID,
holder ledgers by holder) and the compact body and hot decision state are
chunk pages, so no part of a paged checkpoint is limited to one page and a
world of thousands of report holders saves as a paged checkpoint; a restore
rebuilds the knowledge maps and requires the envelope's roots, so one
ledger has one paged form.

`CompactedSimulation` and `CompactedCanwu` add the explicit live archive
contract. Entering compact mode preserves the retained history;
`seal_evidence` then moves a settled tail into a caller-owned
`EvidenceJournalSegment` and advances the private retained-window cursor. The
caller keeps every returned segment in exact cursor order. A compact restore,
flat or paged, keeps its whole supplied prefix as retained evidence, so the
restored runtime's next seal returns one segment from the zero cut that
replaces every earlier sealed segment; prefixes for later checkpoints start
from that segment. Current-state checkpoints continue to carry the total cut,
while full snapshot and replay materialization require the sealed prefix to
be supplied again. Segment gaps, tampering, and checkpoint mismatches
therefore reach the same validator as a flat snapshot. `CompactedCanwu` has
every host read and ingress entry point of `Canwu` whose meaning does not need
full history (the archive-provider attach, viewers, `fork`, exact-version and
evidence reads, and the retained-tail journal reads); the methods that promise
full history (`snapshot`, `replay_journal`, the checkpoint journal) stay on
`Canwu`. `canwu_api::CanwuRun`, implemented by both, is the read-and-ingress
contract that extension host helpers take.

Sealing is fail-closed but works with work in flight. A seal moves events,
commands, and command attempts up to the boundary admission cursors and
ingress, boundaries, and random draws to the journal end, so pending ingress
and the unadmitted tail no longer block it. The continuation closure below
the archived cut, which is a function of state and the cut and never of when
the run sealed, stays in memory as pinned evidence: every pending ingress
record with its provenance fact (the boundary that generated it, with issuer,
target plugin, packet type, and status, or that no boundary did), the command
or event records that cause pending ingress and scheduled actions, every
`PayloadRequired` dependency a current record declares, and the identity facts
of every `IdentityOnly` dependency (the membership of a non-current record
version, the provenance of an ingress record). A boundary that replaces,
retires, or deletes a version whose establishing boundary is sealed pins the
version it replaces at commit, so a dependency declared on it can still be
kept. Before removal, the runtime derives a sorted `EvidenceDependency` set
and requires every declared dependency to resolve: a `PayloadRequired` item
from a retained or pinned record, a record-version membership from the
current index, a pin, or a fact, and an ordinal reference from the journal
counters, which prove that every ID at or below the archived cut existed. A
declared dependency that no longer resolves, including an ingress record
first declared after its seal and no longer held, fails the seal with
`ArchiveNotReady` and changes nothing; a plugin that declares an ingress
record a host cites checks it with `SimulationView::ingress_is_declarable`
first. A plugin must not declare an initial-scenario record version as
payload-required: it stays readable after any seal, and the engine refuses
the declaration; since 0.18.0 an initial-scenario record whose
payload-required or identity dependencies name an initial-scenario version,
its own version, or are malformed is refused when the plugin registers
(`InvalidDomainRecord`) and when a save restores (`InvalidSnapshot`), by the
same checks the seal applies, instead of starting a run that can never seal.
Two-phase sealing prepares immutable, content-addressed
segments, lets the host store them idempotently, then commits only the exact
prepared token; its commit accepts a pinned `PayloadRequired` item in place of
a receipted, loadable one. Segment manifests, receipt roots, and dependency
roots are part of the compact checkpoint commitment (`canwu.compact-checkpoint.v2`,
with the seal token in `canwu.evidence.seal-token.v2`) and are recomputed
during reconstruction; keyed draws are no longer evidence dependencies.
A live domain-record schema can declare sorted identity-only evidence
dependencies; their compact receipts remain reachable without retaining or
hydrating payload bytes. Generated plugin-ingress receipts also Merkle-bind the
provider plugin, packet type, and producing boundary.

After a seal, existence of sealed ordinal evidence comes from the journal
counters and its time from a run-length time index that each seal extends, so
`evidence_exists` and `evidence_time` answer alike in a live, sealed,
restored, and replayed run; a payload read of a sealed, unpinned record
(`command`, `event`, `ingress`, or the body of a non-current version whose
existence is known) reports `EvidenceContentUnavailable`. A boundary decision
that reads an evidence payload outside the closure is a plugin rule
violation, and the first-party extensions read none: `canwu-law` keeps a
bounded legal provider index of the provider packets its boundaries admitted,
`canwu-information` records each command grant's live delegations in one
`DelegatedAuthority` ledger record with an expiry index (an expired
delegation authorizes nothing and leaves the ledger at the next lifecycle
boundary) and finalizes publications through `knowledge_heads`, `canwu-military` cites the
admitted packet of every keyed draw, and the Ming fiscal validator reads only
current records, replay-stable versions, and the journal counters.

The runtime keeps compact canonical request commitments and original
outcomes/receipts for exact idempotency, for the run's request outcome
retention (`RunConfiguration::request_outcome_retention`, default 30
simulated days after the attempt), and the IDs of retired requests as
canonical ranges (`RequestIndex::retired_commands`) for the life of the run,
plus the prior boundary-chain head and
evidence-family flags needed for safe continuation. The five journal
commitments are resumable BLAKE3 stream hashes (the stream length, the
chaining values of completed subtrees, and the unfinished chunk), whose
exported form, the journal frontier, lets a run restored without its prefix
extend the same streams and reach the same checkpoint hash; the roots are the
same values a plain hasher computes. Ordinary `Simulation` history slices,
flat snapshots, and replay journals keep their full-history behavior;
compaction is available only through the dedicated type, so evidence never
disappears implicitly. Checkpoint-journal format 6 wraps the current snapshot
format 9 contract. Older checkpoint-journal envelopes are rejected rather than
reinterpreted or migrated in place.

A **continuation checkpoint** (`ContinuationCheckpoint`, format 2, hash
domain `canwu.continuation-checkpoint.v1`) is the save from which a run
continues without its evidence history: the current state with its checkpoint
hash and commitment roots, the journal end and settled cut, the boundary-chain
head, the five journal frontiers, the unadmitted tail, the pinned evidence,
the pending ingress IDs and provenance facts, the known record-version
memberships, the current-version provenance index, every keyed draw in
draw-ID order, the request index, the history flags, the time index, and the
boundary ledger, under one `continuation_hash`. `continuation_checkpoint()` on
`Canwu` and `CompactedCanwu` derives every part from the runtime, so an
unsealed run, a run sealed at any cuts, and a run restored from an earlier
continuation checkpoint write byte-identical checkpoints at the same cut.
`CompactedCanwu::from_continuation_checkpoint_verified` compares the
recomputed hash with the one the host stored, then checks formats, shape,
state, the keyed draws' values against the root seed, the identity (every
state root and the checkpoint hash recomputed from the built runtime), and
that the closure derived from the restored state equals the carried one,
before it rehydrates the plugins. Restore yields a `CompactedCanwu`; a
continuation runtime settles, seals, forks, and checkpoints as the
uninterrupted run does, with equal receipts and hashes, but cannot produce a
full snapshot, a replay journal, a checkpoint journal, or a paged checkpoint
(`HistoryUnavailable`). What restore cannot verify without history (the
carried evidence records, the pending set, the keyed entries' evidence, the
settled cut) is bound only by the host-stored hash, so the checkpoint is
ground truth for the host alone. See the
[continuation checkpoint proposal](proposals/continuation-checkpoint.md).

Boundary emissions enter the next boundary's admission cut, so an emitting
boundary remains retained until a later completed boundary admits those events.
The same rule keeps generated ingress retained through its own later admission
or terminal cancellation, and when a seal passes such ingress its record and
provenance fact are pinned instead.

The checkpoint/journal wire types, cursor logic, live sealing, compact
continuation indexes, export, and reconstruction helpers live in the dedicated
`canwu-sim` persistence module so storage work stays outside settlement and
command orchestration.

The remaining runtime bookkeeping is partitioned by responsibility rather than
stored as unrelated fields on the authoritative world container.
`RuntimeCurrentState` owns the mutable core world, actor-relative knowledge,
plugin components, generic domain records, decision state, and scoped
random-stream positions.
`RuntimeScheduler` owns the committed clock, scheduled actions, and pending
canonical ingress; `RuntimeCounters` owns monotonic identifiers, the
authoritative revision, and boundary-admission cursors; `RuntimeMetadata` owns
the initial scenario binding, run identity, plugin-registration state, replay
revision provenance, and current checkpoint commitment. These owners are
private implementation boundaries. Snapshot and replay formats remain flat,
and command application and phased settlement checkpoint only their writable
domains. Commands capture armies, actor knowledge, plugin components, scheduled
actions, counters, the event/command/attempt tails, registration state, and
commitments. Boundaries additionally capture generic records, random streams,
the complete scheduler and ingress queue, and every append-only journal cut.
In format 8, generic domain records are held by private persistent Patricia
roots. Boundary capture and simulation cloning share those roots in constant
time; a record mutation creates structurally shared successor paths rather than
cloning the complete flat record map. The store holds each record behind
`Arc` and shares it with the overlays a boundary builds, the evidence journal,
and `DomainRecordChange`; the current-version index is a persistent map whose
capture, restore, and fork share it; phase 7 applies its same-boundary writes
once and the phase-9 commit takes the prepared result when the roots and
requests are unchanged; and a Patricia node's page ID is computed lazily, once
per boundary. An idle boundary therefore allocates nothing per stored record,
and a record write costs the record's own encoding (about 24 µs and 91
allocations for a 2.45 KiB structured record in the recorded measurement).
The store owns canonical primary,
reverse-reference, successor, and predecessor commitments. Portable snapshots
deliberately remain flat and materialize records in stable reference order;
compact checkpoints instead persist content-addressed Patricia pages and
bounded decision locator segments, emitting only pages missing from the
provider. The state-page codec is `raw-canonical-v2`: Patricia nodes and
record-value chunks are binary pages whose page ID is the node hash; a record
longer than `STATE_PAGE_VALUE_CHUNK_BYTES` (1 MiB) is cut into content-addressed
1 MiB chunk pages that its leaf names, up to `MAX_PAGED_RECORD_BYTES`
(512 MiB), so an unchanged chunk is never stored again and no boundary can
fail on a record's size; only `prepare_paged_checkpoint` of a longer record
fails, with `value_out_of_range`. A record-page defect on load is
`invalid_snapshot`, a missing page `state_page_unavailable`. A restore
allocates what the pages describe, up to the ceiling per record, before it
can refuse a leaf that names one chunk many times, so a host authenticates a
checkpoint from an untrusted source by its root before restoring it. See the
[record store ceiling and cost proposal](proposals/record-store-ceiling-and-cost.md). Decision keys first select one of 4,096 stable hash-prefix buckets
and then one of 16 deterministic subsegments. Each page is limited to 64
entries and 1 MiB, so an exact cold lookup loads one committed segment instead
of decoding an ever-growing bucket. The hot decision-history commitment is an
incremental count/XOR/modular-sum accumulator that is rebuilt and checked on
restore rather than rescanning cold history at every boundary.
Ingress insertion checkpoints only its next identifier, evidence tail, exact
pending-queue entry, registration state, and commitments. None of these rollback
checkpoints clones immutable core maps or unrelated accumulated journals. Phased
settlement snapshots only current authoritative state for stable early-phase
reads; each system view borrows command, event, and ingress evidence for the
duration of its handler call. Later phases read the committed current state, so
same-boundary visibility remains unchanged without duplicating accumulated
history, scheduler, counters, or metadata. When an expected rejection is
detected before mutable command application, its rollback checkpoint is
narrower still: it preflights identifiers and revision, then checkpoints only
the attempt tail, affected counters and registration flag, commitment cache and
roots, and checkpoint hash.
The six rollback checkpoint definitions and their exact capture/restore logic
live in the dedicated `canwu-sim` transactions module; command, ingress,
settlement, and scheduling orchestration call those shared private boundaries.
The runtime partitions, evidence owner, and incremental commitment cache live in
the dedicated private `canwu-sim` state module. This is an implementation
ownership boundary only: public snapshots and replay journals remain flat and
unchanged.

Every current snapshot stores commitment format 6 roots for world, knowledge,
plugin components, generic records, scheduler state, commands and attempts,
events, ingress, random state/evidence, the boundary chain, run/plugin identity,
and runtime control counters. Unordered collections are canonicalized by stable
identity before hashing, so roots do not depend on insertion order. Checkpoint
domain `canwu.checkpoint.v4` combines those domain-separated roots with the
exact run-manifest hash and authoritative revision contract. Loading recomputes
and compares every root before accepting the outer checkpoint.

Format 8 boundaries write a `v1:`-tagged commitment over the current canonical
roots with the prior boundary-chain head, so settlement does not serialize and
hash the complete retained journals. When a snapshot is exactly at its
boundary head, loading derives the expected contract from the tag and compares
it with the independently validated current state; unknown tags are rejected. Runtime
checkpoint refresh keeps cloneable incremental hash
state for append-only commands, attempts, events, ingress, and random draws, and
feeds only newly appended journal tails into those roots. It also retains the
last canonical roots for world, knowledge, plugin components, domain records,
the scheduler, random streams, and run/plugin identity. The private mutation
helpers invalidate the domains they own; settlement remains conservative where
several domains can change together. Runtime control and the combined roots are
cheaply rebuilt at every checkpoint. The cache is cloned into each rollback
checkpoint, is restored by rollback, and is never trusted on load: snapshot
validation independently rebuilds every persisted root from serialized evidence
before the runtime cache is reconstructed.

Randomness is available to phased systems only through declared
`RandomStreamKey` values. The kernel derives each stream from the run root seed,
keeps its position independent from unrelated domains, and records every draw
automatically. Draws made by a boundary that later fails disappear with the
rest of that boundary. Core report-delay draws additionally name the exact
recipient, army, dispatch event, and arrival time they produced, and loading
recomputes that time from the recorded value. Validation also requires every
report-dispatch event to have exactly one such draw, so removing both draw and
stream progress cannot preserve an apparently coherent report history.

Operation-keyed draws are answered from the keyed-draw ledger, the record of
every drawn address, retained or sealed, that the runtime keeps as a
persistent ordered map beside the draw journal: a boundary commit extends it,
a failed boundary restores it, a seal and a fork share it, every restore
builds it once from the full draw journal, and a continuation checkpoint
carries it in draw-ID order. It is not hashed or committed, keyed draws are no
longer evidence dependencies, and neither settling a boundary nor a
checkpoint, a paged checkpoint, a seal, or a fork does work in proportion to
the draws a run has made; the ledger keeps one entry per drawn address for
the life of the run. `random_draw_for_operation` returns the sample
with the `RandomDrawId` of its draw evidence, so a record can cite the draw it
was computed from. An exact retry, with the same evidence reference, bound,
and purpose, is answered from the ledger whatever has become of its evidence,
so a sealed run answers as its unsealed replay does; a request that differs
from the recorded draw is refused with `RandomOperationConflict`. A fresh draw
cites seal-stable operation evidence: evidence produced earlier in the same
boundary, an ordinal reference the journal counters prove exists (sealed or
not), or a record's committed current or initial-scenario version. A retained
version that is no longer current is refused with
`InvalidRandomOperationEvidence` in every run, so a sealed run and its exact
replay accept the same first draws.

The legacy immediate command/event path remains for the movement slice and
compatibility examples. It is atomic, but it is not a substitute for the
fourteen-phase boundary and cannot own state also managed by phased systems.
`submit` preserves that direct compatibility path. `process_command` accepts an
owned tracked `CommandRequest` with an idempotency key, expected revision,
expected simulation time, typed issuer, and explicit seat/authority context.
Natural-clock hosts enqueue that request with `enqueue_command` and settle it
through `advance_canonical` or `step_canonical`; plugin packets use
`enqueue_plugin_ingress` and can be withdrawn before they are due with
`cancel_plugin_ingress` or `cancel_permitted_plugin_ingress`, and explicit
calendar work uses
`schedule_calendar_boundary`. Decision hosts use `enqueue_decision` for
controller/ticket/option lifecycle changes, or `drive_decision` to evaluate a
bound policy and enqueue an authoritative resolution. Accepted and expected-rejected command attempts are
persisted, hashed, admitted at a boundary, restored by save/load, and regenerated
by exact replay. Exact retries return the original outcome without new
mutation while the attempt is inside the run's request outcome retention
(default 30 simulated days); after that the request is retired, its ID stays
used, and any request with that ID, an exact retry or not, is refused with
`RequestOutcomeRetired` without an attempt, a revision, or an event;
request-ID collisions inside the window are fail-closed
(`IdempotencyConflict`) without creating evidence. The rule is simulated
time alone, so unsealed, sealed, continuation-restored, and replayed runs
answer alike. The persisted
authoritative revision advances exactly once for every accepted command,
persisted expected command rejection, and published settlement boundary. Failed
commands or boundaries and exact retries do not advance it. Bare clock movement,
queued but unadmitted ingress, and plugin setup do not create a revision;
expected simulation time independently detects clock and scheduled-work
advancement. Declared external commands require both guards. Live requests,
compatibility-only legacy-direct calls, and frozen replay inputs remain distinct;
only exact replay can consume `FrozenReplay`, and declared read-only runs reject
newly authored plugin ingress. Plugin boundary systems can return
`ScheduleIngress` to continue communication pipelines without host orchestration,
and `CancelPluginIngress` to withdraw a still-pending item their plugin
scheduled.
Recurring calendar policy and conservation bundles remain later conformance
work.

Command handlers receive an immutable `CommandContext` containing the issuer
asserted by the trusted in-process host, typed decision origin, seat and
permission-profile context, command-relevant run policy, the command ingress
mode (`CommandIngress`), command and attempt identities, request identity, revision, simulation time, and
expected revision/time guards alongside the read-only simulation view. Canwu
does not authenticate a freely constructed `CommandEnvelope`; network, IPC, and
account adapters must authenticate callers before selecting an `Issuer` and
authority context. Handlers return directives and cannot take a mutable world
reference. Directives can update declared components, emit attributable custom
events, or schedule future directives; a scheduled directive may not wrap
`EnqueuePluginIngress` at any depth (`InvalidPayload`), because the packet it
would fire carries a cause but no issue time that a load could check, and
`EnqueuePluginIngress` takes its own delay instead. `CommandPolicyContext` intentionally
omits run purpose, observation, and trace, preventing authoritative handlers
from branching on presentation-only dimensions.

Executable plugin handlers are stateless Rust function pointers. Deterministic
plugin state belongs in serialized, plugin-owned components; hidden mutex,
atomic, cache, counter, or RNG state is not part of the extension contract.
This keeps command rollback, failed-boundary recovery, forks, snapshots, and
replay independent.

Command application, each same-timestamp scheduled batch, and each phased
settlement are atomic. If fallible event or plugin processing fails, state,
time, queues, events, boundary records, random state, and ID counters return
to their values before the failed command, batch, or boundary. Commands and
phased boundaries use the explicit writable-domain checkpoints described above.
Scheduled batches checkpoint only armies, knowledge, plugin components, random
streams, clock and scheduled actions, counters, event/random-draw tails,
registration state, and commitments. A clock-only advance narrows that further
to time, registration state, and commitments. After command rollback, any
persisted rejection evidence uses the narrower rejection checkpoint. Plugin
directives validate every referenced entity before mutation. Snapshot loading
also proves that pending arrivals agree with army transit, move commands, order
events, timestamps, and correlations, and that pending or completed report
delivery agrees with its dispatch and arrival evidence.

Executable handlers are not serialized. A snapshot stores validated plugin and
system descriptors together with author-declared package versions and semantic
hashes. Continuation is blocked until every required plugin is rehydrated, and
registration must reproduce the exact stored identity and descriptor before
its handlers become active. `RunManifest` separately binds scenario, rules,
content, localization-sensitive contracts, run configuration, and source
provenance. A declared `RunConfigurationSnapshot` carries six orthogonal
run-policy dimensions and is validated against that manifest. Authoritative state
and boundary hashes normalize admission and presentation policy so changing
only observation or trace policy cannot change simulation-result identity or
RNG state. The checkpoint remains a save-container commitment and additionally
binds the exact full run-manifest hash, so differently authorized or observable
runs cannot masquerade as the same save even when their simulated state is
identical. Use
`ReplayJournal` and `replay_from_journal` for exact replay: the journal freezes
engine and snapshot versions, root seed, canonical initial scenario, authority
root, run manifest, run configuration, plugin descriptors, the plugin-registration
lifecycle state, accepted commands, accepted/rejected command attempts, canonical
ingress, boundaries, final time, and final checkpoint hash plus the final
authoritative revision before executing anything. Automatic package discovery
remains later work. Format 8 rejects older revision provenance and cannot export
a journal that claims current exact replay from an unsupported save. New plugin
registration closes after the first recorded tracked attempt (accepted or
expected-rejected), successful compatibility command, queued canonical ingress
item (including a withdrawal record), time advance, or phased settlement; exact snapshot rehydration remains allowed after that point.
Snapshots retain the run's initial time and reject a
registration-open flag when commands, events, queued work, component state,
counter movement, or elapsed simulation time proves execution already began.
There is no pre-1.0 continuation or migration exception for pre-8 data. Hosts
that need to retain those saves must use the old engine or perform
an explicit application-owned export outside Canwu.

### Transition manifests and audit / 转移清单与审计

A multi-owner conditional transition can declare its participants so that an
omitted writer cannot pass unnoticed. A phase-7, phase-10, or phase-12 system
whose contract declares `StateKey::core_transitions()` as a write proposes
`BoundaryDirective::RegisterTransitionManifest` with a `TransitionManifest`:
a `lineage_id`, an `attempt`, a `ready_at` boundary, and one
`TransitionParticipant` per plugin with `expected_pre` versions
(`DomainRecordVersionRef`) and `expected_post` versions
(`TransitionRecordVersion`). The kernel records the registering plugin as the
coordinator, so the identity is `TransitionManifestId { coordinator,
lineage_id, attempt }`. A phase-7 registration may be ready in the same
boundary; phase-10 and phase-12 registrations must name a later boundary, at
most `MAX_TRANSITION_READY_HORIZON` (1,024) boundaries ahead. Every participant
must be a registered plugin with a phase-10 system declaring the same write,
and every expected record must be of a registered kind. A coordinator may hold
one pending manifest per lineage and
`MAX_PENDING_TRANSITION_MANIFESTS_PER_COORDINATOR` (32) manifests, within
`MAX_PENDING_TRANSITION_MANIFESTS` (128) overall; a manifest lists at most
`MAX_TRANSITION_PARTICIPANTS` (16) participants and
`MAX_TRANSITION_EXPECTED_VERSIONS` (64) versions. Registration is a directive
rather than internal ingress, so it is checked against the contract and
recorded on the hash-chained `BoundaryRecord::transition_manifests`.

In phase 10 of the ready boundary, a listed participant's phase-10 system
declaring `core_transitions` proposes
`BoundaryDirective::StageTransitionWrite { manifest_id, writes }`. The kernel
unwraps the staged writes into ordinary directives, so every declared-write,
ownership, phase, and visibility rule applies, and they commit with that
system's visibility; an empty list records presence. Staging by an unlisted
plugin fails the boundary with `InvalidAuthority`. Before phase 11 commits, the
kernel audits every manifest ready at the boundary. When some but not all
participants staged, the boundary fails closed with
`TransitionParticipantMissing`. When an `expected_pre` version differs from the
committed state phase 10 read, or an `expected_post` version differs from the
version produced as of phase 11 (committed versions plus a dry run of the
boundary's pending next-boundary phase-7 and phase-10 writes), it fails with
`TransitionVersionMismatch`. Phase-12 and phase-13 writes may still change a
record afterwards. When every participant is silent, the manifest expires with
`TransitionAuditOutcome::Expired`, so a single-participant manifest cannot fail
for omission. That is also the recovery path from a ready boundary that fails
on every retry: settle it with a cadence under which no participant runs, then
register a new attempt. There is no withdrawal directive.

The `TransitionAuditRecord { manifest_id, ready_at, outcome, participants }`
is recorded on `BoundaryRecord::transition_audits` and
`BoundaryReceipt::transition_audits`; manifest IDs may be reused after
settlement, so `(manifest_id, ready_at)` identifies an audit. Systems of the
coordinator and participants that run after phase 11 read it through
`SimulationView::transition_audits`, and only they list pending manifests
through `SimulationView::transition_manifests`. Pending manifests live in
scheduler state, roll back with a failed boundary, persist as
`SimulationSnapshot::pending_transition_manifests` under an optional scheduler
sub-root, and are read by hosts through `pending_transition_manifests` on
`Simulation`, `CompactedSimulation`, `Canwu`, and `CompactedCanwu`. Snapshot
validation rebuilds the pending set from registration and audit evidence, and
exact replay proves the participant and version checks. A run that never
registers a manifest hashes the same as a run without this feature, and
phase-10 directives outside a manifest are unaffected.

### Rule-evaluation traces / 规则评估轨迹

Any phase-7 or phase-12 system may propose
`BoundaryDirective::RecordEvaluationTrace { trace }` without a contract
declaration. The `canwu-core` `EvaluationTraceRecord` names a `rule_id`,
`rule_version`, `subject`, ordered `EvaluationTerm { term_id, contribution,
evidence }` values, the `result` as the rule computed it, and the current
boundary. The kernel checks the shape, the subject identity, and that every
term's evidence is committed evidence visible to the proposal; it does not
require the result to equal the sum of the contributions. A trace is recorded
as `BoundaryEvaluationTrace { plugin, system, phase, trace }` in
`BoundaryRecord::evaluation_traces`, hashed into the boundary chain only when
present, and sealed and archived with its boundary record. It is never state:
no system, command handler, or policy can read it back.

`RunConfiguration::evaluation_limits` (`EvaluationLimitsV1`, set with
`with_evaluation_limits` and read through
`RunConfigurationSnapshot::evaluation_limits`) bounds traces per boundary
across all systems and terms per trace: 4,096 and 32 by default, 65,536 and
256 at most, with fixed bounds of 16 evidence references per term and 256
bytes per rule ID, version, or term ID. The default is omitted from the wire
and so from the configuration hash. Exceeding a bound fails the boundary with
`ErrorCode::EvaluationTraceLimitExceeded`; the engine never samples or drops
traces, and a zero bound forbids them. Phase-13 proposals cannot record traces.

`CanwuViewer::evaluation_traces(subject, after)` returns evidence-free
`EvaluationTraceView` values. A person or institution principal sees the traces
about its own entity, and the traces about another subject evaluated strictly
after the first knowledge change naming that subject reached its ledger,
ordered by boundary and phase; a next-boundary publication counts from the end
of its boundary. Research and developer principals see every trace, and a
public principal is refused with `InvalidKnowledgeAuthority`. The full record,
including evidence and the producing system, stays on the trusted
`Canwu::boundaries` read.

## External renderer integration

Renderers consume snapshots and events: territory points, route endpoints, army
locations, relationships, movement events, and knowledge views. A renderer may
turn them into sprites, meshes, SVG, ASCII, or tables. None of those concepts
enter Canwu's state model.

The reference debug client is an explicitly trusted host surface. Its person
inspector may use `admin_query_knowledge` to show the current generic holder
projection; player and remote clients must derive a restricted `CanwuViewer`
instead and have no route to audit origins or another holder's ledger.

## Portability and versions

The headless crates use portable Rust APIs and support Windows, macOS, and Linux.
Operating-system window-system features are confined to `canwu-debug`; Linux
enables Wayland and X11 while Windows and macOS use their native `eframe`
integration. CI verifies all three targets.

All first-party crates share one SemVer version from the workspace manifest,
and each depends on the exact same version of every other first-party crate
it uses (`=X.Y.Z`); nothing one version wrote is read by another. Persistent
snapshots additionally carry an independent format version, which names the
contract that version writes.

## Reusable-engine conformance

Canwu is developed against the normative engine-neutral capability profile in
[`engine-conformance.md`](engine-conformance.md). It requires deterministic
settlement, authority, ownership, atomic commit, knowledge, persistence,
lineage, packages, and publication through public extension points. Current
coverage and remaining gaps are tracked in the profile itself. The public-only
[`representative_conformance`](../crates/api/canwu-api/tests/representative_conformance.rs)
fixture composes independent packages across authority, settlement, typed
records, knowledge, randomness, persistence, replay, forking, and rollback.
