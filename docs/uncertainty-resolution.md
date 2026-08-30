# Uncertainty, Random Decisions, and External Selectors

This document defines how Canwu represents uncertainty without weakening exact
replay. The same design applies to legal proceedings, personal choices,
religious or cultural change, crowds, institutions, and other domain
extensions.

## English

### Classify the uncertainty before choosing a mechanism

| Situation | Canwu mechanism | Why |
| --- | --- | --- |
| A fact is missing, disputed, or unknowable to the actor | Preserve `Unknown`, `Indeterminate`, or contested evidence | Absence of knowledge is not a random fact generator. |
| A person, council, institution, or bounded agent chooses among legal options | `DecisionTicket` with Utility, Rule, Random, Human, External, or LLM policy | Every selector sees the same versioned option set and cannot invent authority. |
| A discrete incident genuinely has a probability | A declared boundary system and an operation-keyed random draw | The draw, producer, purpose, cause, and outcome become replay evidence. |
| A large population changes by an expected rate | Deterministic integer settlement with persisted remainder | Aggregate expectation should not become millions of unrelated coin flips. |
| Equal results require a tie-break | An explicit canonical tie-break rule, unless the authored model explicitly declares a random decision | Container order, thread order, and wall time are never valid tie-breaks. |

The important distinction is between a **selector** and a **world event**. A
run may configure Random, Human, External, or LLM selection for the same bounded
ticket contract before opening the ticket; a registered controller policy is
not replaced at runtime. An LLM must not replace a stochastic epidemic,
weather incident, equipment failure, or other world event. Those events remain
boundary-system mechanics with random draw evidence.

### Replayable random policy

`DecisionPolicyKind::Random` is not an executable `DecisionPolicy` that draws
outside a transaction. A boundary system:

1. reads an open ticket through `SimulationView`;
2. calls `random_sample_for_operation` with a stable operation ID and
   `RandomOperationTarget::DecisionTicket`;
3. supplies canonical `DecisionOptionWeight` values in
   `BoundaryDirective::ResolveDecisionRandomly`;
4. lets the kernel validate the draw and option set, then generate canonical
   decision ingress for the next eligible boundary.

One source boundary may generate at most one Random decision resolution, and
the generated ingress is due immediately at the next eligible boundary. This
keeps its global revision guard unambiguous; independent decisions should be
resolved by separate source boundaries.

The resulting `DecisionTrace.random` records the `RandomDrawId`, draw value,
bound, and weights. The `RandomDrawRecord.outcome` records the ticket version
and selected option. Source-boundary failure rolls back both draw and generated
ingress; target-boundary rejection is recorded through the ordinary decision
attempt path. Reload and exact replay never rerun an unrecorded policy.

The runnable reference is
`crates/api/canwu-api/examples/uncertainty_resolution.rs`.

### External and LLM selector interface

External and LLM adapters receive `ExternalDecisionRequest`: ticket identity,
ticket version, context, and available option descriptors. They do not receive
the serialized authoritative command actions. A host invokes its chosen model
or service and parses a strict response into `ExternalDecisionResponse`, whose
meaning is only “select this existing option ID for this ticket version.”

The admitted `DecisionIngressRequest`, controller policy identity, optional
external evidence, selected option, and any authority-derived nested command
are persisted. Exact replay replays those records and does not call the model
again. `QueuedLlmPolicy` is a reference in-process adapter, not durable network
or model infrastructure.

### Domain examples

- **Law:** quorum, eligibility, ballot counting, threshold, competence, and
  known legal facts remain deterministic. If an authored model represents
  unresolved bargaining or ratification as a probabilistic pass/fail choice,
  expose `pass` and `fail` on a ticket and weight them explicitly. A law seat
  may instead use a Human, External, or LLM controller. `canwu-law` preserves a
  compatible controller pre-registered under its stable seat controller ID;
  otherwise it creates the backward-compatible default Human controller.
- **Person:** personality, knowledge, doctrine, obligations, and current
  options belong in ticket context. Utility or Rule policies give deterministic
  behavior; Random represents bounded behavioral variability; Human, External,
  and LLM use the same option set. No policy may create a command not already
  represented by an option.
- **Belief or religion:** population-scale diffusion uses deterministic integer
  transfers and persisted remainder. A discrete revival, schism, suppression,
  conversion episode, or leader decision may use a boundary draw or decision
  ticket according to whether it is an incident or an actor choice.
- **Crowd:** routine aggregate movement uses deterministic rates and capacity
  allocation. A panic, riot, stampede, or sudden dispersal may be a discrete
  operation-keyed incident. A council or organizer response is a decision
  ticket and may use Random or LLM selection.

### Modeling checklist

1. Name the uncertain proposition and who or what resolves it.
2. Separate unknown facts from stochastic events and bounded choices.
3. List legal options before selecting a policy.
4. Use integer weights and stable option IDs for a Random policy.
5. Bind operation-keyed draws to exact evidence, target identity, ticket
   version, operation ID, and draw slot.
6. Keep LLM output structured and limited to one existing option ID.
7. Put mutations behind canonical decision or command ingress.
8. Test snapshot restore, exact replay, rollback, stale ticket versions, and
   evidence tampering.

## 中文

### 先判断“不确定”属于哪一类

| 情况 | 参伍机制 | 原因 |
| --- | --- | --- |
| 事实缺失、有争议，或角色无法得知 | 保留 `Unknown`、`Indeterminate` 或相互冲突的证据 | 不知道事实，不等于随机生成一个事实。 |
| 人物、议会、机构或有边界的智能体在合法选项中选择 | 使用带 Utility、Rule、Random、Human、External 或 LLM 策略的决策票据 | 所有选择器面对同一份有版本的选项，不能凭空创造权限。 |
| 离散事件确实具有发生概率 | 使用声明过的边界系统和操作定址随机抽样 | 抽样、生产者、用途、原因与结果都进入可重放证据。 |
| 大规模人群按期望比例变化 | 使用确定性整数结算并持久化余数 | 群体期望不应变成数百万次彼此独立的临时掷骰子。 |
| 相等结果需要打破平局 | 使用明确的规范 tie-break；只有规则明确要求时才使用随机决策 | 容器顺序、线程顺序和现实时间都不能决定结果。 |

最重要的边界是区分**选择器**和**世界事件**。同一票据契约可以在开票前由不同
运行配置选择 Random、Human、External 或 LLM；已注册 controller 的 policy 不能
在运行中替换。LLM 不能替代疫情、天气、设备故障等随机世界事件。后者仍应由
边界系统抽样并留下抽样证据。

### 可重放的随机决策策略

`DecisionPolicyKind::Random` 不会在事务外部直接调用随机数。边界系统应当：

1. 通过 `SimulationView` 读取开放的决策票据；
2. 使用稳定 operation ID 和 `RandomOperationTarget::DecisionTicket` 调用
   `random_sample_for_operation`；
3. 在 `BoundaryDirective::ResolveDecisionRandomly` 中提交规范排序的
   `DecisionOptionWeight`；
4. 由模拟内核校验抽样和选项集合，并为下一可用边界生成标准决策准入。

每个来源边界最多生成一个 Random 决策结算，生成的准入立即在下一可用边界到期。
这样全局 revision guard 始终没有歧义；彼此独立的随机决策应由不同来源边界结算。

最终的 `DecisionTrace.random` 保存 `RandomDrawId`、抽样值、上界和权重；
`RandomDrawRecord.outcome` 保存票据版本和所选选项。来源边界失败时，抽样与生成的
准入一起回滚；目标边界上的陈旧版本或权限错误仍按普通决策尝试记录。恢复和精确
重放不会再次调用未记录的策略。

可运行参考见
`crates/api/canwu-api/examples/uncertainty_resolution.rs`。

### External 与 LLM 选择接口

External 和 LLM 适配器只接收 `ExternalDecisionRequest`：票据标识、票据版本、
上下文和当前可用选项描述，不接收权威命令载荷。上层应用调用自己选择的模型或
服务，把严格格式化的回答解析为 `ExternalDecisionResponse`；它只表达“为这个版本
选择现有的某个 option ID”。

正式进入模拟的是 `DecisionIngressRequest`、controller 的策略身份、可选的外部
证据、选项结果，以及从 controller 权限派生的嵌套命令。精确重放只重放这些记录，
不会再次调用模型。`QueuedLlmPolicy` 是进程内参考适配器，不是持久化网络或模型服务。

### 领域案例

- **法律：**法定人数、投票资格、计票、通过门槛、权限范围和已知法律事实保持
  确定。如果规则把未建模的协商或批准过程表示成“通过/不通过”的概率选择，就在
  票据上明确提供 `pass`、`fail` 并配置权重；也可以给同一席位使用 Human、External
  或 LLM controller。`canwu-law` 会保留上层应用用稳定席位 controller ID 预注册的
  兼容策略；没有预注册时仍创建兼容旧行为的默认 Human controller。
- **人物：**性格、知识、信条、义务和当前可选行动进入票据上下文。Utility 或 Rule
  提供确定行为；Random 表示有边界的行为波动；Human、External、LLM 使用同一组选项。
  任何策略都不能创造票据中不存在的命令。
- **信仰或宗教：**人口尺度的传播使用确定性整数转移和持久化余数。一次复兴、分裂、
  压制、集体皈依或领袖决定，应根据它属于“离散事件”还是“角色选择”，分别使用
  边界抽样或决策票据。
- **人群：**日常聚合流动使用确定性比例和容量分配；恐慌、骚乱、踩踏或突然散去可
  建模为操作定址的离散随机事件；议会、组织者或领袖的应对则是决策票据，可以使用
  Random 或 LLM 选择。

### 建模检查表

1. 写清楚不确定命题，以及由谁或什么机制结算。
2. 分开“未知事实”“随机事件”和“有边界的选择”。
3. 在选择策略之前先列出所有合法选项。
4. Random 策略使用整数权重和稳定 option ID。
5. 操作定址抽样绑定精确证据、目标身份、票据版本、operation ID 和 draw slot。
6. LLM 输出保持结构化，只能返回一个现有 option ID。
7. 所有变更继续经过标准决策或命令准入。
8. 验证快照恢复、精确重放、回滚、陈旧票据版本和证据篡改。
