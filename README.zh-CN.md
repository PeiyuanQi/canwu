# Canwu（参伍 / 35 Engine）

[English](README.md) | [简体中文](README.zh-CN.md)

<img src="assets/branding/canwu-logo-zh-cn.png" alt="Canwu 参伍历史模拟引擎标志" width="320">

Canwu 是一个使用 Rust 编写的无界面历史模拟引擎。它负责模拟历史世界、
以可重复的方式推进时间、验证外部命令，并记录事件和因果关系。它还可以分别
记录每个人掌握的信息，而不是让所有角色都看到真实的世界状态。

Canwu 不负责画面、音频、动画或正式的产品界面。游戏、研究工具、Python
程序、网页客户端和 AI 智能体通过公开 API 使用 Canwu。

目前的 v0.3 开发版本保留小型移动场景，并加入面向《社稷》领域插件的确定性
十四阶段结算边界。插件可以声明分阶段读写，以稳定规则竞争资源预留，提交
本边界或下一边界可见的变更，并保存可供重放核验的完整边界证据。移动场景
继续演示命令验证、行程调度、因果事件和按角色延迟送达的知识。这个版本向
《社稷》符合性目标迈出了实质一步，但尚不代表已经完全符合。

## 工作区

- `canwu-core`：稳定 ID、可重复的随机数和结构元数据
- `canwu-time`：不依赖画面帧率的历史时间
- `canwu-event`：可保存的事件，以及原因和结果之间的关系
- `canwu-world`：历史实体和只读世界快照
- `canwu-knowledge`：每个角色知道什么，以及信息来自何时
- `canwu-sim`：不公开的模拟状态、命令、调度和插件
- `canwu-api`：供程序、智能体、解释工具和调试工具使用的公开 API
- `canwu-debug`：只使用公开 API 的小型参考客户端

修改架构边界前，请先阅读[架构说明](docs/architecture.md)和
[最终设计](docs/end-state.md)。版本发布和兼容规则见
[版本说明](docs/versioning.md)。

## 版本和平台

Canwu 使用语义化版本。工作区中的所有 crate 当前版本都是 `0.3.0`，并会使用
同一个版本一起发布。根目录 `Cargo.toml` 中的版本号是唯一标准。

Canwu 支持 Windows、macOS 和 Linux。模拟相关 crate 不依赖具体平台。参考
调试客户端通过 `eframe` 使用 OpenGL，并在 Linux 上支持 Wayland 和 X11。
持续集成会检查这三个操作系统。

## 开发

本地环境、常用命令、项目规则和 Contributor License Grant（贡献者许可授权）
都写在 [CONTRIBUTING.md](CONTRIBUTING.md) 中。外部贡献者提交拉取请求时必须
接受其中的授权条款。

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

本项目按照 Canwu License 1.0 以源代码可用的方式发布。对于同一个产品系列，
在适用的连续 12 个月内，产品收入不超过 1,000 万美元时，个人、社区、研究、
教育、非营利和商业使用都不需要支付版税。只有超过该门槛的收入才按照累进
边际费率计算版税。

商业产品必须显示 [Canwu 官方标志](BRANDING.md)，并在产品或面向用户的材料
中说明该产品使用了 Canwu。产品可以保持专有，独立的下游代码也不需要公开
源代码。完整且具有约束力的条款以 [LICENSE](LICENSE) 为准。第三方依赖继续
使用各自的许可证，详见
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)。
