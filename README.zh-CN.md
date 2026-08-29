# 参伍引擎 Canwu Engine

[English](README.md) | [简体中文](README.zh-CN.md)

网站：[canwu.org](https://canwu.org)

<img src="assets/branding/canwu-banner-zh-cn.png" alt="Canwu 参伍引擎横幅" width="720">

Canwu 是一个使用 Rust 编写的无界面历史模拟引擎。它负责模拟历史世界、
以可重复的方式推进时间、验证外部命令，并记录事件和因果关系。它还可以分别
记录每个人掌握的信息，而不是让所有角色都看到真实的世界状态。

Canwu 不负责画面、音频、动画或正式的产品界面。游戏、研究工具、Python
程序、网页客户端和 AI 智能体通过公开 API 使用 Canwu。

Canwu 面向的不只是一个可随意修改的游戏状态对象。它提供确定性时间、经过
验证并带权限语义的命令、原子结算、角色相对知识、类型化扩展点、因果证据、
存档加载验证、精确重放和显式实时证据封存。引擎本身保持领域中立：应用通过
公开契约定义自己的
规则与内容，而不是把应用专属类型加入内核。

项目仍在积极开发。公开示例有意保持小而清晰，方便开发者检查、测试，并把
这些保证复用到更大型的游戏、研究环境和智能体驱动模拟中。

## 作为依赖使用

Rust 应用应依赖官方支持的对外 API，而不是直接依赖实现 crate：

```toml
[dependencies]
canwu-api = "=0.7.0"
```

需要持久化 Canwu 存档的应用应固定已发布的引擎版本，并且只在同时提供明确
存档迁移时升级。上例选择不可变的 `0.7.0` 版本，而不是持续变化的 `main` 分支。

`canwu-api` 依赖图中的 crate 会一并发布，供 Cargo 解析依赖。它们属于实现
细节，不建议应用代码直接依赖，也不单独承诺兼容性。模拟领域扩展也会作为
crates.io package 正式发布，应用可以固定已发布的扩展版本；但在 1.0 之前它们
仍然是可选模块，API 可能独立演进。

## 快速开始

安装 Rust 1.88 或更高版本，然后运行已经迁出的参考世界入门示例：

```text
cargo run -p canwu-reference-world --example starter
```

如需查看只使用公开 API 的分阶段插件示例：

```text
cargo run -p canwu-api --example phased_boundary
```

如需查看包含动态选项、效用评估、按控制者权限生成命令、决策轨迹、存档与精确
重放的 `DecisionTicket` 示例：

```text
cargo run -p canwu-api --example decision_ticket
```

如需运行社会传播模拟模块示例：

```text
cargo run -p canwu-society --example local_community_diffusion
```

如需运行基于证据的技术流程：

```text
cargo run -p canwu-technology --example technology_diffusion
```

如需运行无锡本地与无锡到北京的经路线规划通信示例：

```text
cargo run -p canwu-correspondence --example routed_correspondence
```

## 项目结构

- `canwu-core`：稳定 ID、可重复的随机数和结构元数据
- `canwu-decision`：决策票据、控制者、决策轨迹、通用效用评估器和策略 SDK 接口
- `canwu-time`：不依赖画面帧率的历史时间
- `canwu-event`：可保存的事件，以及原因和结果之间的关系
- `canwu-knowledge`：每个角色知道什么，以及信息来自何时
- `canwu-routing`：确定性的角色相对路线规划
- `canwu-transport`：行程、保管权交接、容量预订和运送执行
- `canwu-sim`：不公开的模拟状态、命令、调度和插件
- `canwu-api`：供程序、智能体、解释工具和调试工具使用的公开 API
- `canwu-reference-world`：可替换的示例实体、脱离式投影、移动插件、路由适配器
  和可运行的持久化/重演入门示例
- `canwu-debug`：建立在公开 API 与参考整合包之上的小型参考客户端
- `canwu-information`：已正式发布的信息生命周期扩展
- `canwu-correspondence`：建立在寻路、运输与信息生命周期之上的已正式发布
  通信模拟领域扩展和模拟插件
- `canwu-society`：已正式发布的社会传播模拟模块（`social diffusion simulation module`）；
  在架构上属于建立在 `canwu-api` 之上的模拟领域扩展（`domain extension`）
- `canwu-culture`：已正式发布的文化编写、编译与生命周期扩展，建立在
  `canwu-society` 之上
- `canwu-law`：实验性的确定性法律编写、制度程序、版本化法律、适用、
  承继与退休扩展
- `canwu-technology`：已发布的通用技术模拟扩展，负责证据、本地能力、实施、
  按用途采用和传播机会
- `canwu-history-research`：已发布、位于基础技术真值下游的三个可选历史研究评估插件
- `canwu-fiscal`：已发布的通用财政程序扩展，负责地区法规采纳、核算、减免、
  授权、执行凭证与报告
- `canwu-ming-fiscal`：带出处的明代财政参考内容，核心范围为 1368-1662 年，
  并提供延伸至 1683 年的可选郑氏分支
- `canwu-ming-fiscal-reference`：组合财政插件与参考世界插件的洪武、万历和
  弘光可运行场景

[crate 结构图](crates/README.md)展示仓库分层、精确的依赖 DAG 和发布顺序。
[文档索引](docs/README.md)汇总架构契约、社区指南和法律声明。`agent-interface`
保存面向引擎使用者的打包技能；仓库贡献者技能原生位于 `.agents/skills`，
Claude 兼容入口位于 `.claude/skills`。这些工具不是运行时模拟插件。
`website` 和 `assets` 保存社区网站与项目素材。

修改架构边界前，请先阅读[架构说明](docs/architecture.md)和
[最终设计](docs/end-state.md)。

## 开发

欢迎提交代码、错误报告、示例、文档改进和严谨的架构讨论。本地环境和贡献
条款见 [CONTRIBUTING.md](CONTRIBUTING.md)。编码智能体还必须遵循
[AGENTS.md](AGENTS.md) 以及更靠近目标目录的说明。

<details>
<summary><strong>开发流程</strong></summary>

1. 阅读 `AGENTS.md`、`docs/architecture.md`、`docs/end-state.md`，以及目标目录
   附近的其他仓库说明。
2. 检查 `git status`，保留已有工作；互不相关的并行改动应使用 worktree。
3. 说明要改变的约束，找出所有受影响的表面，并完成最小且完整的一组改动。
   语义变更不要和大规模文件移动或生成文件刷新混在一起。
4. 把测试作为可长期保存的证据。只有必要、可复用、很可能在未来合理变更下
   失败，并且具有实质验证价值的测试才进入仓库。这里的实质验证价值，是指覆盖
   多步骤约束、公开契约、持久化与重演边界或失败恢复路径，且其证明能力超出格式、
   lint、编译或简单访问器断言。范围更窄的一次性验证直接运行即可。项目不设置
   TDD 要求、测试数量指标或覆盖率目标。随后运行：

   ```text
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo check -p canwu-debug
   ```

5. 公开 API 或文档发生变化时，运行受影响的公开示例和
   `cargo doc --workspace --no-deps`。
6. 架构、持久化、重放、权限、确定性或性能改动必须经过独立审查；只提交范围
   清晰并且验证通过的里程碑。

详细的项目层级和变更表面映射见 [AGENTS.md](AGENTS.md)。

</details>

## 智能体技能

面向引擎使用者的智能体接口和技能位于 [`agent-interface`](agent-interface/) 目录。外部使用者可以
调用
[`$canwu-engine-docs`](agent-interface/plugins/canwu-engine/skills/canwu-engine-docs/SKILL.md)
查找并解读官方教程与设计文档，再使用
[`$canwu-engine-usage`](agent-interface/plugins/canwu-engine/skills/canwu-engine-usage/SKILL.md)
获得公共 API 指导。使用参伍开发游戏的下游开发者可以调用
[`$canwu-game-create`](agent-interface/plugins/canwu-developer/skills/canwu-game-create/SKILL.md)
构建可运行的游戏纵向切片；历史研究者和开发者可以调用
[`$canwu-history-create`](agent-interface/plugins/canwu-developer/skills/canwu-history-create/SKILL.md)
构建带有来源和不确定性的历史模拟。两者都可以再使用
[`$canwu-common-build-run-explorer`](agent-interface/plugins/canwu-developer/skills/canwu-common-build-run-explorer/SKILL.md)
实现按随机种子重跑和角色相对时间线。贡献者和维护者原生使用
[`canwu-contributor-design`](.agents/skills/canwu-contributor-design/SKILL.md)
和
[`canwu-contributor-release`](.agents/skills/canwu-contributor-release/SKILL.md)
技能；Claude 兼容入口位于 [`.claude/skills`](.claude/skills/)，并指向对应的
权威技能文件。
面向维护者的软件包与 registry 操作步骤见
[`docs/releasing.md`](docs/releasing.md)。

## 最小 API 示例

```rust
use canwu_api::{Canwu, CommandRequest, CommandRequestId, EntityRef, Issuer, SimDuration};
use canwu_reference_world::{
    MovementCommand, ReferenceWorldPlugin, demo_scenario, order_movement,
};

let (scenario, ids) = demo_scenario()?;
let plugin = ReferenceWorldPlugin;
let mut canwu = Canwu::new_with_plugins(35, scenario, &[&plugin])?;

let envelope = order_movement(
    Issuer::Actor(ids.commander),
    &MovementCommand {
        subject: EntityRef::Army(ids.army),
        destination: ids.eastern_territory,
        cargo: Vec::new(),
    },
)?
.at_time(canwu.time());
canwu.enqueue_command(
    canwu.time(),
    0,
    CommandRequest::new(CommandRequestId::new(1), canwu.revision(), envelope),
)?;
let events = canwu.advance_canonical(SimDuration::days(1))?;
# Ok::<(), canwu_api::CanwuError>(())
```

`crates/api/canwu-api/examples/phased_boundary.rs` 提供了一个只依赖公开 API 的
插件示例：它发布并申请守恒资源、读取已声明的分配结果，并提交带明确来源的
边界证据。

## 许可证

Canwu 是按照 [Apache License 2.0](LICENSE) 发布的开源软件。任何人都可以在
开源或专有产品中使用、修改和分发 Canwu，无需支付版税或提交收入报告。分发
时必须遵守 Apache 许可证，并保留适用的许可证和 [NOTICE](NOTICE) 材料。

Apache 许可证不要求产品显示 Canwu 标志或公开鸣谢；如自愿使用项目标志，
请遵循[品牌指南](docs/community/branding.md)中关于避免暗示背书的说明。第三方
依赖继续使用各自的许可证，详见
[第三方许可证清单](docs/legal/third-party-licenses.md)。

## 支持的平台

Canwu 支持 Windows、macOS 和 Linux。模拟相关 crate 不依赖具体平台。参考
调试客户端通过 `eframe` 使用 OpenGL，并在 Linux 上支持 Wayland 和 X11。
持续集成会检查这三个操作系统。整个 workspace 和已发布 crate 均要求 Rust
1.88 或更高版本。
