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

先在 Canwu 仓库根目录生成样例：

```powershell
cargo run -p canwu-ming-fiscal-reference --example ming_fiscal_starter -- hongwu-1391
```

然后在 `canwu` 仓库根目录启动一个静态服务器：

```powershell
python -m http.server 8000
```

打开 `/tools/trace-viewer/`，点击“读取默认样例”。也可以直接双击 `index.html`，再通过“选择 manifest”和“选择 steps”载入文件；浏览器出于 `file://` 安全限制无法直接读取默认路径时，这是预期行为。

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
