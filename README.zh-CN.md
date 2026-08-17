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

## 快速开始

安装满足根目录 `Cargo.toml` 中 `rust-version` 要求的 Rust 工具链，然后运行
无界面移动示例：

```text
cargo run -p canwu-api --example move_army
```

如需查看只使用公开 API 的分阶段插件示例：

```text
cargo run -p canwu-api --example phased_boundary
```

## 项目结构

- `canwu-core`：稳定 ID、可重复的随机数和结构元数据
- `canwu-time`：不依赖画面帧率的历史时间
- `canwu-event`：可保存的事件，以及原因和结果之间的关系
- `canwu-world`：历史实体和只读世界快照
- `canwu-knowledge`：每个角色知道什么，以及信息来自何时
- `canwu-sim`：不公开的模拟状态、命令、调度和插件
- `canwu-api`：供程序、智能体、解释工具和调试工具使用的公开 API
- `canwu-debug`：只使用公开 API 的小型参考客户端

[文档索引](docs/README.md)汇总架构契约、社区指南和法律声明。
`agent-interface` 保存供引擎使用者和仓库维护者使用的技能工具，它们不是运行时
模拟插件。`website` 和 `assets` 保存社区网站与项目素材。

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

智能体接口和技能位于 [`agent-interface`](agent-interface/) 目录。外部使用者可以
使用
[`canwu-engine-usage`](agent-interface/plugins/canwu-engine/skills/canwu-engine-usage/SKILL.md)
技能。贡献者和维护者使用
[`canwu-developer`](agent-interface/plugins/canwu-developer/skills/) 下的技能；发布
流程使用
[`canwu-developer-release`](agent-interface/plugins/canwu-developer/skills/canwu-developer-release/SKILL.md)。

## 最小 API 示例

```rust
use canwu_api::{Canwu, Command, CommandEnvelope, Issuer, SimDuration};

let mut canwu = Canwu::demo(35)?;
let ids = Canwu::demo_ids();

canwu.submit(CommandEnvelope::new(
    Issuer::Actor(ids.commander),
    Command::MoveArmy {
        army: ids.army,
        destination: ids.eastern_territory,
    },
))?;
let events = canwu.advance(SimDuration::days(1))?;
# Ok::<(), canwu_api::CanwuError>(())
```

`crates/canwu-api/examples/phased_boundary.rs` 提供了一个只依赖公开 API 的
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
持续集成会检查这三个操作系统。
