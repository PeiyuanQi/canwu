# Canwu promo video (simplified Chinese) / 参伍引擎宣传片

A 1 minute 58 second, 1920×1080 promotional video that introduces Canwu's key
features, how to start using it, and why to choose it. Subtitles are in
simplified Chinese and are burned into the picture; the same cues are written
to an `.srt` sidecar for platforms that take separate subtitle files.

Everything here is generated from source, so the video can be edited and
re-rendered:

| File | Purpose |
| --- | --- |
| `timeline.json` | Single source of truth: frame size, frame rate, tempo, scene timing, and subtitle cues. |
| `scenes.html` | Every scene, drawn by `renderAt(t)` as a pure function of time. |
| `music.py` | The original soundtrack, synthesized from code (no samples). |
| `render.mjs` | Renders frames with headless Chromium and muxes them with the music into an MP4 and an SRT. |
| `fetch_fonts.py` | Downloads the SIL OFL fonts the scenes use into `fonts/`. |
| `cover.html` | Bilibili covers in the video's style; `?ratio=16x9` or `?ratio=4x3`. |
| `canwu-cover-16x9.jpg`, `canwu-cover-4x3.jpg` | The rendered covers (1920×1080 and 1440×1080). |
| `canwu-promo-zh-cn.mp4` | The rendered video, stored with Git LFS. |
| `canwu-promo-zh-cn.srt` | The subtitle cues as an SRT sidecar. |

## Storyboard

| Time | Scene | Content |
| --- | --- | --- |
| 0:00 | epigraph | 「参伍以变，错综其数」（《周易·系辞上》） |
| 0:05 | hook | Nobody in history sees the whole picture; reports arrive late or wrong. |
| 0:10 | logo | Ink reveal of the official banner. |
| 0:14 | what | Headless historical simulation engine; host applications use `canwu-api`. |
| 0:24 | replay | Deterministic replay: load, fork, and replay match the checkpoint hash. |
| 0:34 | knowledge | Ground truth versus one actor's view as a letter travels from 无锡 to 北京. |
| 0:43 | commands | Validated commands at a boundary; a detained person's command is rejected. |
| 0:53 | causality | Causal evidence traced back from a result to its origin. |
| 1:02 | extensions | Domain-neutral core with optional domain extensions. |
| 1:12 | agents | Agents and players share one API (actor-relative reads, typed commands). |
| 1:22 | start | Add `canwu-api = "=0.13.1"`, run the starter example in the repository linked from canwu.org. |
| 1:31 | skills | `$canwu-game-create`, `$canwu-history-create`, `$canwu-engine-usage`. |
| 1:38 | why | Strategy games, historical research, agent environments, education; Apache 2.0. |
| 1:48 | outro | Logo and canwu.org. |

The knowledge, command, and causality scenes use illustrative examples; the
replay hash and terminal output come from running
`cargo run -p canwu-reference-world --example starter` at version 0.13.1.

## Rebuild

Requirements: Python 3 with `numpy` and `scipy`, Node.js 18+ with the
`playwright` package and its Chromium, and `ffmpeg` with `libx264`.

```text
python3 fetch_fonts.py
python3 music.py
node render.mjs
```

Outputs land in `build/` (ignored by git): `canwu-promo-zh-cn.mp4` and
`canwu-promo-zh-cn.srt`. To update the published render, copy both next to
this README; `.gitattributes` stores the MP4 with Git LFS. Useful options:

- `node render.mjs --stills 12.5,40` writes PNG stills for quick inspection.
- `node render.mjs --from 30 --to 40` re-renders only that range of frames;
  add `--skip-frames` to re-mux existing frames after changing the music.
- Serve this folder over HTTP and open `scenes.html?play` to preview in real
  time, or `scenes.html?t=42` to inspect a single moment.
- `node render.mjs --covers` writes `build/canwu-cover-16x9.jpg` and
  `build/canwu-cover-4x3.jpg`.

When you change subtitles or scene timing, edit `timeline.json`; the scenes,
the SRT, and the music's tempo all read it. Keep Chinese copy aligned with
[`docs/terminology.md`](../../../docs/terminology.md). The video points viewers
to canwu.org rather than to a repository URL, so it stays correct if the
repository moves.

## Publishing on Bilibili

Bilibili asks for a 4:3 cover, shown in the mobile feed, and a 16:9 cover for
other placements. Upload `canwu-cover-4x3.jpg` and `canwu-cover-16x9.jpg`. Both
keep key content out of the bottom strip, where feed cards overlay play counts
and duration.

- Title: 【开源】参伍引擎：用 Rust 构建可重放的历史模拟，每个角色只看见自己知道的世界
- Subtitles: the video already has burned-in subtitles, so don't also upload
  the SRT as closed captions, or viewers will see them twice.
- Tags: 参伍引擎、Rust、开源项目、历史模拟、游戏引擎、大战略游戏、AI智能体、数字人文

Description:

```text
参伍引擎（Canwu）是用 Rust 编写的开源无界面历史模拟引擎。它负责推进时间、验证命令、
记录事件与因果，并分别记录每个角色掌握的信息。

· 确定性重放：读档、派生分支、重放，检查点哈希都与原运行一致
· 每个角色所知不同：世界的真实状态与角色知识分开保存
· 所有客户端共用一套 API：游戏、研究工具和 AI 智能体调用同一组接口

开源许可：Apache License 2.0，商业使用免版税
官网：https://canwu.org
配乐为原创，由代码合成。
```

## Credits and licenses

- Logo and banner: the official assets in `assets/branding/`, used unmodified
  per [`docs/community/branding.md`](../../../docs/community/branding.md).
- Fonts: Noto Serif SC, Noto Sans SC, Ma Shan Zheng, and JetBrains Mono, all
  under the SIL Open Font License 1.1.
- Music: original, generated by `music.py` in this folder.
