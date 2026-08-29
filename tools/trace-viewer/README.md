# Canwu Trace Viewer

这是一个不依赖后端或构建工具的通用 HTML trace viewer，位于 `canwu/tools/trace-viewer/`。它读取 Canwu trace 的：

- `manifest.json`：运行元信息、引擎版本、fixture、状态和 frame 数量。
- `steps.jsonl`：每个结算边界一行的 JSON frame。

查看器只依赖通用字段（`sequence`、`phase`、`receipt`、`boundary`、`checkpoint_hash`、`revision`），并会自动识别 `fiscal`、`technology`、`society` 等领域 snapshot。未知领域会以通用 JSON 摘要显示，不需要修改查看器核心。也就是说，财政只是一个可选适配器，不是查看器的数据模型。

## 通用 trace 约定

每个 frame 可以附带：

- `receipt`：结算后的通用计数、边界 ID、checkpoint hash；
- `boundary` 或 `boundary_record`：admitted events、record changes、emissions 等证据数组；
- 顶层领域字段（如 `fiscal`），或 `domains`、`domain_snapshots`、`extensions` 容器；
- 任意未知字段：会保留在“原始 frame”区域，不会被丢弃。

以后接入新领域时，优先让领域插件输出一个可序列化 snapshot；查看器只有在需要更高密度摘要时才增加领域 renderer。

## 使用

查看器会把 `SimTime`（单位：分钟）转换成便于阅读的公元日期时间。若 trace 提供
`fiscal.historical_year` 或 `fiscal.state.historical_context.year`，则将该年份的
`1 月 1 日 00:00:00` 作为模拟起点；否则使用 `1970-01-01 00:00:00`。界面同时保留
原始 `SimTime` 分钟值，避免把展示日期误认为引擎的权威时间类型。该转换是展示约定，
不改变 Canwu 的无日历模拟时间模型。

先在 Canwu 仓库根目录生成样例：

```powershell
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391
```

然后在 `canwu` 仓库根目录启动一个静态服务器：

```powershell
python -m http.server 8000
```

打开 `/tools/trace-viewer/`，点击“读取默认样例”。也可以直接双击 `index.html`，再通过“选择 manifest”和“选择 steps”载入文件；浏览器出于 `file://` 安全限制无法直接读取默认路径时，这是预期行为。

也可以让 starter 在模拟完成后自动启动 viewer：

```powershell
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391 --days 365 --open-viewer
```

该选项会启动仅监听 `127.0.0.1` 的临时静态服务器，打开浏览器并通过 URL
参数自动载入本次运行生成的 trace。终端会保持运行以提供静态文件，按
`Ctrl+C` 停止；使用 `--viewer-port 0`（默认）自动选择可用端口。

默认样例路径为：

```text
artifacts/traces/ming-fiscal-reference/<fixture>/manifest.json
artifacts/traces/ming-fiscal-reference/<fixture>/steps.jsonl
```

当前查看器提供：

- 结算边界时间线和阶段过滤；
- 引擎变化、领域变化、knowledge 和 allocation 摘要；
- 财政扩展的 procedure revision、计数和 holder-relative projections；
- boundary evidence 展开查看；
- 当前 frame 的复制和下载；
- 任意未知领域的通用 JSON 摘要。

这是开发者/研究者的 trusted-host 调试工具，不是玩家视图。玩家界面仍应读取角色相对视图，而不是直接展示完整 authoritative trace。
