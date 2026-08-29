# Ming fiscal case / 明代财政案例

## Outcome / 交付结果

This case uses Canwu's complete, replaceable fiscal reference stack for game
and research prototypes. The stack covers the Ming founding in 1368 through the
Southern Ming in 1662, with 1663-1683 exposed as an optional Zheng maritime
continuation. It does not add Ming-specific rules to the simulation core.

这个案例使用参伍提供的可替换、可运行财政参考栈，适合游戏原型和研究工具。核心历史
范围从 1368 年明朝建立延伸到 1662 年南明终结；1663-1683 年郑氏海上政权作为
显式可选延续。明代专属规则不进入模拟内核。

| Layer / 层 | Package | Owns / 负责 | Must not own / 不负责 |
| --- | --- | --- | --- |
| Generic domain extension / 通用领域扩展 | `canwu-fiscal` | Law, regional adoption, assessment, remission, authorization, audit, candidates, reports / 法规、地区采纳、核算、减免、授权、审计、候选改革、报告 | Currency balances, grain stocks, market prices, shipments / 货币余额、粮食库存、市场价格、运输 |
| Reference content / 参考内容 | `canwu-ming-fiscal` | Periods, regions, fiscal definitions, transitions, coverage, provenance / 时期、地区、财政定义、转型、覆盖与出处 | Runtime mutation or automatic reform / 运行时写入或自动改革 |
| Reference integration / 参考整合 | `canwu-ming-fiscal-reference` | Runnable scenario composition and restore path / 可运行场景组合与恢复路径 | Privileged engine access / 特权内核访问 |
| Host adapter / 上层适配 | Game or research application / 游戏或研究应用 | Real resource transfer and exact evidence submission / 真实资源转移与精确证据提交 | Rewriting fiscal records outside canonical ingress / 绕过规范化输入改写财政记录 |

## Longitudinal model / 纵向时间模型

Compilation retains the full selected historical scope. The initial year only
selects active period labels; it does not discard earlier or later rules.
`FiscalHistoricalContextPacket` advances the explicit historical year and mode
through canonical ingress. A date can make a reform eligible, but only an
authority-bound action changes regional procedure. `ApplyTransition` installs
all target rules and suspends superseded rules atomically; `ChangeAdoption` is
reserved for stage administration outside a declared transition.

编译后仍保留所选范围内的完整历史时段。初始年份只选择当前活动时期，不会裁掉
更早或更晚的规则。`FiscalHistoricalContextPacket` 通过规范化输入改变显式历史
年份和模式。年份只能让改革进入候选集合；`ApplyTransition` 会原子安装全部目标
法规并停用被取代的法规，`ChangeAdoption` 只管理不属于已声明转型的采纳阶段。
因此 1581 年不会触发全帝国“一条鞭法开关”。

The pack is a longitudinal content catalog, not one mandatory live ledger for
the entire 1368-1683 range. Each fixture selects a bounded year slice. Long campaigns may advance
the explicit context inside the compiled scope, while the host remains
responsible for checkpointing or sharding its own resource and logistics
ledgers.

参考包是一套纵向内容目录，不要求把 1368-1683 年的整个范围塞进同一个常驻账本。
每个 fixture 选择一个有界年份切片；长期战役可以在已编译范围内推进显式历史语境，但资源与
物流账本的存档、归档或分片仍由上层应用负责。

Runtime fiscal state is bounded to 4,096 assessments, 8,192 execution requests,
8,192 receipts, 32 evidence versions per receipt, and a 32 MiB serialized-state
budget. Aggregate rebuilds use single-pass indexes. For campaigns beyond these
experimental limits, the host must create separate simulation runs or shards;
the fiscal extension has no in-place partition or archive API, and the host is
responsible for cross-shard operation-ID deduplication.

运行时财政状态最多分别容纳 4,096 条核算、8,192 条执行请求和 8,192 张凭证；
每张凭证最多引用 32 个证据版本，序列化状态上限为 32 MiB。聚合重建使用单遍
索引。超过这些实验性边界时，上层应用必须拆成独立模拟运行或分片；财政扩展
没有原地分区或归档 API，跨分片外部操作 ID 去重也由上层应用负责。

The pack divides the chronology into eight playable periods:

1. Founding reconstruction, 1368-1392.
2. Registration-centered early Ming, 1393-1429.
3. Silver commutation and treasury consolidation, 1430-1499.
4. Regional levy consolidation, 1500-1579.
5. Regional spread of Single Whip practices, 1580-1617.
6. Late-Ming wartime surcharges, 1618-1643.
7. Southern Ming fiscal fragmentation, 1644-1662.
8. Optional Zheng maritime continuation, 1663-1683.

## Coverage and uncertainty / 覆盖与不确定性

The authored matrix spans eight periods, eight regions, and eleven fiscal
mechanisms, producing 704 explicit cells. Numeric priority resolves broad
defaults and specific declarations. Equal-priority overlap fails compilation,
so source-file order cannot select an interpretation.

参考包按 8 个时期、8 个地区、11 种财政机制形成 704 个显式覆盖单元。宽泛默认值
和具体声明通过数字优先级解析；同优先级重叠会导致编译失败，文件顺序不能暗中
选择历史解释。

| Status / 状态 | Meaning / 含义 |
| --- | --- |
| `supported` | The cell has cited definitions suitable for direct simulation / 有出处支持、可直接模拟 |
| `archetype_fallback` | A bounded comparative archetype is available, with limitations / 只有受限类比原型，并明确限制 |
| `explicit_unknown` | The pack deliberately supplies no behavior / 参考包明确不提供行为 |
| `not_applicable` | The mechanism does not apply to that cell / 该机制不适用于此单元 |

Every provenance entry carries a claim scope and forbidden inferences. A
promulgated quota is not actual collection, registered population is not actual
population, and a court's territorial claim is not stable control.

每条出处都包含可支持的论断范围和禁止推断。法定额度不等于实际征收，登记人口
不等于实际人口，朝廷宣称的辖区也不等于稳定控制。

## Fiscal procedure and resource truth / 财政程序与资源真值

An assessment records what is due. An execution authorization records what an
institution may collect, remit, disburse, reserve, or return. Neither changes a
resource balance. A host performs the real operation in the owning resource,
market, production, or logistics domain, then publishes a typed adapter result
and calls `enqueue_execution_receipt`. At that live settlement boundary,
admission decodes every exact typed result version and matches its request,
execution kind, payment form, resource, source, target, and unit. The persisted
quantity and disposition are derived from those results rather than declared by
the caller. Within one fiscal runtime state, neither one exact adapter-result
version nor one `(evidence kind, external_operation_id)` pair can settle two
receipts. The four dispositions are fulfilled, partial, rejected, and excused.

核算记录“应当发生什么”，执行授权记录机构可以征收、汇解、支出、储备或退还
什么；二者都不会直接改变资源余额。上层应用在资源、市场、生产或物流领域执行
真实操作，发布类型化适配结果，再调用 `enqueue_execution_receipt`。在运行时结算
边界，接纳过程会解码每个精确类型化结果版本，并逐项核对请求、执行类型、支付
形态、资源、来源、目标和单位；持久化凭证的数量与处置结果从证据派生，不由调用
方自行声明。在单个财政运行时状态内，同一精确适配结果版本或同一
`（证据种类，外部操作 ID）` 组合都不能结算两张凭证。处置结果可以是完成、
部分完成、拒绝或豁免。

Historical content periods and host-defined accounting cycles are separate.
Aggregates are partitioned by institution, mechanism, scope, accounting cycle,
unit, and payment form. They keep assessed and remission-granted quantities
separate from collected, remitted, disbursed, reserved, and returned execution.
Only remission and collection reduce assessment outstanding. Actor reports are
holder-relative knowledge records with non-invertible quantized ranges,
confidence, and evidence, not exact truth copied into `FiscalState`.

历史内容时期与上层应用定义的核算周期彼此独立。聚合按机构、机制、范围、核算
周期、单位和支付形态分区；应征、减免与征收、汇解、支出、储备、退还分别记录，
只有减免和征收会减少应征未结量。角色报告是带范围、置信度和证据的持有人相对
知识记录；估计值使用不可反推精确真值的量化区间，不会把精确真值复制进
`FiscalState`。

## Playable fixtures / 可玩起点

| Fixture | Design purpose / 设计目的 |
| --- | --- |
| `hongwu-1391` | Registered land, labor service, and grain obligations without equating registration with capacity / 登记土地、徭役和粮税，不把登记值当作真实能力 |
| `wanli-1581` | Compare entrenched, implemented, and merely accepted regional Single Whip adoption / 比较一条鞭法在不同地区的巩固、实施和仅接受状态 |
| `hongguang-1644` | Separate court, commander, merchant-credit, salt, and regional-treasury authority during territorial crisis / 在领土危机中分开朝廷、将领、商人信用、盐务和地区财库权责 |

Hongguang represents military levy, merchant credit, and regional treasury as
three independent transitions, so each domain acts under its own authority
binding. / 弘光起点把军费征收、商人信用与地区财库拆成三项独立转型，使每个领域
都只能使用自己的权限绑定推进。

Run the starter with:

```text
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- wanli-1581
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongguang-1644
```

Add `--days <N>` to continue the deterministic simulation after the one-off
sample cycle. `--cadence daily|monthly|annual` selects the calendar-boundary
marker, and `--step-days <N>` overrides its fixed simulation-day quantum. The
monthly and annual defaults use 30 and 365 simulation days because `SimTime` is
minute-based and does not claim a Gregorian calendar. A partial final period is
captured by a cadence-free boundary, so monthly or annual systems do not run
early. For example:

```text
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391 --days 365 --cadence daily
```

Each command runs assessment, authorization, typed external execution,
receipt, report materialization, and semantic validation. The integration
registers `ReferenceWorldPlugin`, `MingFiscalExecutionAdapterPlugin`, and a
configured `FiscalPlugin`. Use `restore_ming_fiscal_reference` and
`replay_ming_fiscal_reference` so restore and exact replay receive that same
plugin set and run the reference-specific validator.
