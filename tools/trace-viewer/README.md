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
参数自动载入本次运行生成的 trace。服务器和浏览器会在模拟循环开始前启动，查看器默认
每 3 秒刷新一次，适合边运行边观察；
也可以点击“刷新 trace”或关闭“自动刷新”。如果在第一帧写入前打开查看器，会先显示
“trace 已连接，等待新的结算 frame”，而不是把空文件当成错误。终端会保持运行以提供
静态文件，按 `Ctrl+C` 停止；使用 `--viewer-port 0`（默认）自动选择可用端口。

默认样例路径为：

```text
artifacts/traces/ming-fiscal-reference/<fixture>/manifest.json
artifacts/traces/ming-fiscal-reference/<fixture>/steps.jsonl
```

当前查看器提供：

- 结算边界时间线和阶段过滤；
- 主视口横向时间线：边界按时间从左到右排列，轨道占满详情区宽度并在内部横向滚动，支持轨道箭头与键盘方向键，阶段报告置于其下；
- 可手动收起来源栏，把桌面调试空间让给时间线和阶段报告；
- 自动刷新、手动刷新，以及最新/分页导航（每页最多 60 个 frame）；
- URL trace 首次按流读取 JSONL；内置 Rust viewer server 支持 HTTP byte range，后续刷新只读取新增尾部；不支持 range 的普通静态服务器会自动回退为完整刷新；
- 浏览器默认最多保留最近 512 个完整 frame，trace 总数仍单独显示，避免长期运行耗尽页面内存；需要调查旧历史时，可明确点击“载入全部（内存）”重新按流读取完整 trace；
- 明确的数据来源与连接方式：实时 URL、已完成 URL 或本地静态副本；
- 结构化查找，例如 `boundary=4`、`frame=6`、`phase=财政`、`hash=<片段>`；
- 当前 frame 与 JSONL 中真实上一帧的结构化差异，覆盖所有检测到的领域，不会把筛选后的上一条误当作因果基线；
- 引擎变化、领域变化、knowledge 和 allocation 摘要；
- 阶段总览：按本 frame 的证据和当前领域快照，列出参与实体、实体类型、当前状态、持有人和边界证据数量；
- 财政扩展的 procedure revision、计数和 holder-relative projections；
- boundary evidence 的跨类别搜索、分页查看，以及每条证据的完整 JSON 展开；
- 当前 frame 的复制和下载；
- 任意未知领域的通用 JSON 摘要。

“数据检查”会检查必填结构、frame 序号、manifest 数量/版本、boundary 前后链接、
receipt/boundary hash 引用和最终 checkpoint 引用。Canwu 的承诺使用 BLAKE3；由 starter
启动的内置 Rust viewer server 会在运行完成后重算每个 boundary 的内容 hash，并校验
previous-hash 链。纯静态服务器、本地文件和运行中的 trace 仍只做结构检查，因此界面会
明确显示“结构通过 · 未验 BLAKE3 内容”或“等待运行完成”，而不是容易误解的笼统“通过”。
这里的 BLAKE3 状态只证明 trace 中 boundary 内容与 boundary 链的一致性，不等同于外部签名
或完整 checkpoint 防篡改证明。

这是开发者/研究者的 trusted-host 调试工具，不是玩家视图。玩家界面仍应读取角色相对视图，而不是直接展示完整 authoritative trace。
