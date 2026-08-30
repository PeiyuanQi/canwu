(() => {
  "use strict";

  const state = {
    manifest: null,
    frames: [],
    selectedSequence: null,
    phase: "all",
    search: "",
    pendingManifest: null,
    pendingSteps: null,
    pendingPair: false,
    sourcePath: null,
    sourceLabel: null,
    sourceMode: "none",
    sourceByteOffset: 0,
    autoRefreshWanted: true,
    timelineStart: 0,
    refreshTimer: null,
    refreshing: false,
    diagnostics: [],
    notices: [],
    partialTail: false,
    totalFrameCount: 0,
    retainedStartOrdinal: 0,
    retainAllFrames: false,
    localManifestFile: null,
    localStepsFile: null,
    scanDiagnostics: [],
    formatVersions: new Set(),
    cryptoVerification: null,
    evidenceSearch: "",
    evidencePage: 0,
    evidenceFrameSequence: null,
    selectionNotice: null,
  };

  const $ = (id) => document.getElementById(id);
  const nodes = {
    loadDefaultButton: $("loadDefaultButton"),
    emptyLoadButton: $("emptyLoadButton"),
    refreshButton: $("refreshButton"),
    toggleSourceButton: $("toggleSourceButton"),
    autoRefreshInput: $("autoRefreshInput"),
    loadStatus: $("loadStatus"),
    manifestInput: $("manifestInput"),
    stepsInput: $("stepsInput"),
    folderInput: $("folderInput"),
    dropZone: $("dropZone"),
    errorBox: $("errorBox"),
    phaseFilter: $("phaseFilter"),
    filterStatus: $("filterStatus"),
    emptyState: $("emptyState"),
    viewerState: $("viewerState"),
    timeline: $("timeline"),
    timelineCount: $("timelineCount"),
    timelinePreviousButton: $("timelinePreviousButton"),
    timelineNextButton: $("timelineNextButton"),
    timelineLatestButton: $("timelineLatestButton"),
    timelineScrollPreviousButton: $("timelineScrollPreviousButton"),
    timelineScrollNextButton: $("timelineScrollNextButton"),
    loadAllFramesButton: $("loadAllFramesButton"),
    timelineRange: $("timelineRange"),
    frameSearch: $("frameSearch"),
    validationBox: $("validationBox"),
    detail: $("detail"),
  };

  const TIMELINE_PAGE_SIZE = 60;
  const MAX_FRAMES_IN_MEMORY = 512;
  const EVIDENCE_PAGE_SIZE = 50;

  const phaseLabels = {
    initial_state: "初始状态",
    open_assessment: "开始核算",
    authorize_execution: "批准执行",
    adapter_evidence: "收到外部凭证",
    fiscal_execution_receipt: "财政执行入账",
    report_materialization: "生成报告",
    canonical_boundary: "常规时间推进",
  };

  function isObject(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value);
  }

  function arrayLength(value) {
    return Array.isArray(value) ? value.length : 0;
  }

  function numeric(value, fallback = 0) {
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "string" && value.trim() !== "") {
      const parsed = Number(value);
      if (Number.isFinite(parsed)) return parsed;
    }
    return fallback;
  }

  function phaseLabel(value) {
    const key = String(value || "canonical_boundary");
    return phaseLabels[key] || key.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
  }

  function escapeHtml(value) {
    return String(value ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#039;");
  }

  function compact(value, limit = 120) {
    const text = typeof value === "string" ? value : JSON.stringify(value);
    if (!text) return "—";
    return text.length > limit ? `${text.slice(0, limit - 1)}…` : text;
  }

  function shortHash(value) {
    const text = String(value || "—");
    return text.length > 22 ? `${text.slice(0, 10)}…${text.slice(-8)}` : text;
  }

  function normalizeFrame(raw, index) {
    const source = isObject(raw) ? raw : {};
    const receipt = isObject(source.receipt) ? source.receipt : {};
    const boundary = isObject(source.boundary) ? source.boundary : (isObject(source.boundary_record) ? source.boundary_record : {});
    const phase = source.phase || source.semantic_phase || "canonical_boundary";
    const engine = {
      events: arrayLength(receipt.emitted_events) || arrayLength(boundary.admitted_events),
      ingress: arrayLength(receipt.generated_ingress) || arrayLength(boundary.generated_ingress),
      changes: numeric(receipt.change_count, arrayLength(boundary.changes)),
      recordChanges: numeric(receipt.record_change_count, arrayLength(boundary.record_changes)),
      knowledge: numeric(receipt.knowledge_record_count, arrayLength(boundary.knowledge_changes)),
      allocations: arrayLength(receipt.allocations) || arrayLength(boundary.allocations),
      randomDraws: arrayLength(receipt.random_draws) || arrayLength(boundary.random_draws),
    };
    return {
      raw,
      formatVersion: source.format_version ?? null,
      sequence: numeric(source.sequence, index),
      phase: String(phase),
      phaseLabel: phaseLabel(phase),
      at: receipt.settled_at ?? boundary.at ?? source.settled_at ?? "—",
      boundaryId: receipt.boundary_id ?? boundary.id ?? "—",
      revision: source.revision ?? "—",
      checkpointHash: source.checkpoint_hash ?? receipt.boundary_hash ?? boundary.hash ?? "—",
      engine,
      boundary,
      domains: extractDomains(source),
    };
  }

  function extractDomains(raw) {
    const domains = [];
    const reserved = new Set([
      "format_version", "sequence", "phase", "semantic_phase", "receipt", "boundary",
      "boundary_record", "settled_at", "revision", "checkpoint_hash", "domains",
      "domain_snapshots", "extensions",
    ]);
    const add = (key, payload) => {
      if (!isObject(payload) && !Array.isArray(payload)) return;
      if (domains.some((domain) => domain.key === key)) return;
      domains.push({ key, label: key.replaceAll("_", " "), payload });
    };
    ["fiscal", "technology", "society", "culture", "information", "correspondence", "history"].forEach((key) => add(key, raw[key]));
    ["domains", "domain_snapshots", "extensions"].forEach((containerKey) => {
      if (isObject(raw[containerKey])) {
        Object.entries(raw[containerKey]).forEach(([key, payload]) => add(key, payload));
      }
    });
    Object.entries(raw).forEach(([key, payload]) => {
      if (!reserved.has(key)) add(key, payload);
    });
    return domains;
  }

  async function parseSteps(source, startOrdinal = 0, maxFrames = MAX_FRAMES_IN_MEMORY) {
    const stream = typeof source?.stream === "function" ? source.stream() : source?.body;
    if (!stream?.getReader) {
      const text = await source.text();
      return parseSteps(new Blob([text], { type: "application/jsonl" }));
    }
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    const frames = [];
    const errors = [];
    const scanDiagnostics = [];
    const formatVersions = new Set();
    let buffer = "";
    let lineNumber = 0;
    let totalCount = 0;
    let partialTail = false;
    let previousSequence = null;
    let previousBoundaryHash = null;
    let lastCheckpointHash = null;
    let bytesRead = 0;

    const retain = (frame) => {
      frames.push(frame);
      if (Number.isFinite(maxFrames) && frames.length > maxFrames + 200) frames.splice(0, 200);
    };
    const observe = (raw) => {
      const frame = normalizeFrame(raw, startOrdinal + totalCount);
      frame.ordinal = startOrdinal + totalCount;
      totalCount += 1;
      if (previousSequence !== null && frame.sequence !== previousSequence + 1 && !scanDiagnostics.some((item) => item.includes("序号"))) {
        scanDiagnostics.push(`frame 序号在 ${previousSequence} → ${frame.sequence} 之间不连续`);
      }
      const sourceRaw = isObject(raw) ? raw : {};
      const hasBoundary = isObject(sourceRaw.boundary) || isObject(sourceRaw.boundary_record);
      const hasCheckpoint = Boolean(sourceRaw.checkpoint_hash || sourceRaw.receipt?.boundary_hash || frame.boundary?.hash);
      if ((!isObject(raw) || sourceRaw.sequence == null || !(sourceRaw.phase || sourceRaw.semantic_phase) || !isObject(sourceRaw.receipt) || !hasBoundary || !hasCheckpoint)
          && !scanDiagnostics.some((item) => item.includes("必填结构"))) {
        scanDiagnostics.push(`frame #${frame.sequence} 缺少 Canwu trace 的必填结构`);
      }
      const linkedHash = frame.boundary?.previous_hash;
      if (previousBoundaryHash && linkedHash && previousBoundaryHash !== linkedHash && !scanDiagnostics.some((item) => item.includes("hash 链"))) {
        scanDiagnostics.push(`boundary hash 链在 frame #${frame.sequence} 处断开`);
      }
      if (frame.boundary?.hash && sourceRaw.receipt?.boundary_hash && frame.boundary.hash !== sourceRaw.receipt.boundary_hash
          && !scanDiagnostics.some((item) => item.includes("receipt 与 boundary"))) {
        scanDiagnostics.push(`frame #${frame.sequence} 的 receipt 与 boundary hash 不一致`);
      }
      if (frame.formatVersion !== null) formatVersions.add(frame.formatVersion);
      previousSequence = frame.sequence;
      previousBoundaryHash = frame.boundary?.hash || previousBoundaryHash;
      lastCheckpointHash = frame.checkpointHash;
      retain(frame);
    };
    const parseLine = (line, isTail = false) => {
      lineNumber += 1;
      const trimmed = line.trim();
      if (!trimmed) return;
      try {
        observe(JSON.parse(trimmed));
      } catch (error) {
        if (isTail) partialTail = true;
        else errors.push(`steps.jsonl 第 ${lineNumber} 行无法解析：${error.message}`);
      }
    };

    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      bytesRead += value.byteLength;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split(/\r?\n/);
      buffer = lines.pop() || "";
      lines.forEach((line) => parseLine(line));
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
    buffer += decoder.decode();
    if (buffer.trim()) parseLine(buffer, true);
    if (errors.length) throw new Error(errors.slice(0, 3).join("\n"));
    if (Number.isFinite(maxFrames) && frames.length > maxFrames) frames.splice(0, frames.length - maxFrames);
    const unconsumedBytes = partialTail ? new TextEncoder().encode(buffer).byteLength : 0;
    return {
      frames,
      totalCount,
      retainedStartOrdinal: Math.max(startOrdinal, startOrdinal + totalCount - frames.length),
      partialTail,
      scanDiagnostics,
      formatVersions,
      lastCheckpointHash,
      bytesRead,
      consumedBytes: Math.max(0, bytesRead - unconsumedBytes),
    };
  }

  async function readTraceFiles(manifestFile, stepsFile, sourceLabel, strictPair = false) {
    if (!manifestFile || !stepsFile) throw new Error("需要同时提供 manifest.json 和 steps.jsonl。");
    setStatus(`流式读取 ${sourceLabel || "trace"}…`);
    const manifest = JSON.parse(await manifestFile.text());
    if (!isObject(manifest)) throw new Error("manifest.json 顶层必须是 JSON object。");
    const parsed = await parseSteps(stepsFile, 0, state.retainAllFrames ? Number.POSITIVE_INFINITY : MAX_FRAMES_IN_MEMORY);
    if (strictPair) {
      const validation = validateTrace(manifest, parsed, stepsFile.name);
      if (validation.pairErrors.length) {
        throw new Error(`manifest/steps 配对校验失败，已拒绝载入：${validation.pairErrors.join("；")}`);
      }
      activateLocalSource();
    }
    applyTrace(manifest, parsed, stepsFile.name, sourceLabel, false);
    state.localManifestFile = manifestFile;
    state.localStepsFile = stepsFile;
  }

  function applyTrace(manifest, parsed, stepsName, sourceLabel, strictPair, detectWindowShift = false) {
    const validation = validateTrace(manifest, parsed, stepsName);
    if (strictPair && validation.pairErrors.length) {
      throw new Error(`manifest/steps 配对校验失败，已拒绝载入：${validation.pairErrors.join("；")}`);
    }
    const frames = parsed.frames;
    const previousSequence = state.selectedSequence;
    const wasFollowingLatest = previousSequence !== null && previousSequence === state.frames.at(-1)?.sequence;
    const selectedStillRetained = frames.some((frame) => frame.sequence === previousSequence);
    state.manifest = manifest;
    state.frames = frames;
    state.totalFrameCount = parsed.totalCount;
    state.retainedStartOrdinal = parsed.retainedStartOrdinal;
    state.scanDiagnostics = [...parsed.scanDiagnostics];
    state.formatVersions = new Set(parsed.formatVersions);
    state.sourceByteOffset = parsed.consumedBytes;
    state.partialTail = parsed.partialTail;
    state.diagnostics = validation.diagnostics;
    state.notices = validation.notices;
    state.pendingPair = false;
    if (detectWindowShift && wasFollowingLatest) {
      state.selectedSequence = frames.at(-1)?.sequence ?? null;
      state.selectionNotice = null;
    } else if (detectWindowShift && previousSequence !== null && !selectedStillRetained && frames.length) {
      state.selectedSequence = frames[0].sequence;
      state.selectionNotice = `原先选中的 Frame #${previousSequence} 已退出最近 ${MAX_FRAMES_IN_MEMORY} 帧窗口；已明确切换到 Frame #${state.selectedSequence}。可点击“载入全部（内存）”恢复旧帧。`;
    } else {
      state.selectedSequence = frames.find((frame) => frame.sequence === previousSequence)?.sequence
        ?? frames.at(-1)?.sequence
        ?? null;
      state.selectionNotice = null;
    }
    state.phase = state.phase || "all";
    const visible = visibleFrames();
    const selectedIndex = visible.findIndex((frame) => frame.sequence === state.selectedSequence);
    state.timelineStart = selectedIndex >= 0
      ? timelinePageStartForIndex(selectedIndex)
      : lastTimelinePageStart(visible.length);
    hideError();
    render();
    const windowLabel = parsed.totalCount > frames.length ? `；内存中保留最近 ${frames.length} 帧` : "";
    setStatus(parsed.totalCount ? `已流式读取 ${parsed.totalCount} 个结算边界${windowLabel}` : "trace 已连接，等待新的结算 frame");
  }

  function validateTrace(manifest, parsed, stepsName) {
    const diagnostics = [...parsed.scanDiagnostics];
    const pairErrors = [];
    const notices = [];
    const manifestStepsName = String(manifest.steps_file || "").split(/[\\/]/).at(-1);
    if (manifestStepsName && manifestStepsName !== stepsName) {
      pairErrors.push(`manifest 指向 ${manifest.steps_file}，当前载入的是 ${stepsName}`);
    }
    const expected = numeric(manifest.step_count, Number.NaN);
    if (Number.isFinite(expected) && expected !== parsed.totalCount) {
      pairErrors.push(`manifest 记录 ${expected} 帧，当前读取到 ${parsed.totalCount} 帧`);
    }
    const versions = parsed.formatVersions;
    if (versions.size > 1 || (versions.size === 1 && manifest.format_version != null && !versions.has(manifest.format_version))) {
      pairErrors.push("manifest 与 frame 的格式版本不一致");
    }
    const expectedHash = manifest.final_checkpoint_hash || manifest.last_checkpoint_hash;
    if (expectedHash && parsed.lastCheckpointHash && expectedHash !== parsed.lastCheckpointHash) {
      pairErrors.push("manifest 的最终 checkpoint 与最后一个 frame 不一致");
    }
    diagnostics.push(...pairErrors);
    if (parsed.partialTail) diagnostics.push("最后一行尚未写完；已保留前面的完整 frame，并会在下次刷新重试");
    if (parsed.totalCount > parsed.frames.length) notices.push(`trace 共 ${parsed.totalCount} 帧；为避免浏览器耗尽内存，只保留最近 ${parsed.frames.length} 帧用于搜索和详情。`);
    return { diagnostics, pairErrors, notices };
  }

  async function loadPending() {
    if (!state.pendingManifest || !state.pendingSteps) {
      if (state.pendingManifest) setStatus(`已选择 ${state.pendingManifest.name}；请再选择 steps 文件`);
      else if (state.pendingSteps) setStatus(`已选择 ${state.pendingSteps.name}；请再选择 manifest 文件`);
      if (state.manifest) renderMeta();
      return;
    }
    try {
      await readTraceFiles(state.pendingManifest, state.pendingSteps, "本地文件", true);
    } catch (error) {
      showError(error.message);
      setStatus("载入失败");
    }
  }

  function pickFiles(files) {
    beginPendingLocalPair();
    const list = Array.from(files || []);
    const groups = new Map();
    list.forEach((file) => {
      const relative = file.webkitRelativePath || file.name;
      const directory = relative.includes("/") ? relative.slice(0, relative.lastIndexOf("/")) : "";
      if (!groups.has(directory)) groups.set(directory, []);
      groups.get(directory).push(file);
    });
    const pairs = [...groups.values()].map((group) => ({
      manifest: group.find((file) => file.name.toLowerCase() === "manifest.json"),
      steps: group.find((file) => ["steps.jsonl", "steps.ndjson"].includes(file.name.toLowerCase())),
    })).filter((pair) => pair.manifest && pair.steps);
    if (pairs.length > 1) {
      showError("目录中发现多组 trace。请进入具体运行目录，或分别选择一对 manifest 和 steps 文件。");
      return;
    }
    const manifest = pairs[0]?.manifest || list.find((file) => file.name.toLowerCase() === "manifest.json");
    const steps = pairs[0]?.steps || list.find((file) => ["steps.jsonl", "steps.ndjson"].includes(file.name.toLowerCase()));
    state.pendingManifest = manifest || null;
    state.pendingSteps = steps || null;
    nodes.manifestInput.value = "";
    nodes.stepsInput.value = "";
    if (!manifest || !steps) {
      showError("需要在同一目录中找到一对 manifest.json 和 steps.jsonl；未沿用上一次选择的文件。");
      return;
    }
    loadPending();
  }

  function activateLocalSource() {
    if (state.refreshTimer) clearInterval(state.refreshTimer);
    state.refreshTimer = null;
    state.sourcePath = null;
    state.sourceByteOffset = 0;
    state.sourceLabel = "本地文件选择";
    state.sourceMode = "local";
    state.cryptoVerification = null;
    state.pendingPair = false;
    nodes.refreshButton.disabled = true;
    nodes.autoRefreshInput.checked = false;
    nodes.autoRefreshInput.disabled = true;
  }

  function beginPendingLocalPair() {
    if (state.refreshTimer) clearInterval(state.refreshTimer);
    state.refreshTimer = null;
    state.pendingPair = true;
    state.retainAllFrames = false;
    nodes.refreshButton.disabled = true;
    nodes.autoRefreshInput.checked = false;
    nodes.autoRefreshInput.disabled = true;
    if (state.manifest) renderMeta();
  }

  async function loadTracePath(tracePath, sourceLabel) {
    const base = new URL(tracePath, window.location.href);
    if (base.origin !== window.location.origin) {
      throw new Error("trace 路径必须与查看器来自同一来源。");
    }
    base.search = "";
    base.hash = "";
    if (!base.pathname.endsWith("/")) base.pathname += "/";
    const sourceChanged = state.sourcePath && state.sourcePath !== base.href;
    if (sourceChanged) {
      state.retainAllFrames = false;
      state.selectionNotice = null;
    }
    const canAppend = state.sourceMode === "url"
      && state.sourcePath === base.href
      && state.sourceByteOffset > 0
      && state.manifest;
    state.sourcePath = base.href;
    state.sourceLabel = sourceLabel || "trace";
    state.sourceMode = "url";
    state.pendingPair = false;
    if (!canAppend) state.cryptoVerification = null;
    nodes.autoRefreshInput.disabled = false;
    nodes.autoRefreshInput.checked = state.autoRefreshWanted;
    const stepsHeaders = canAppend ? { Range: `bytes=${state.sourceByteOffset}-` } : {};
    let [manifestResponse, stepsResponse] = await Promise.all([
      fetch(new URL("manifest.json", base), { cache: "no-store" }),
      fetch(new URL("steps.jsonl", base), { cache: "no-store", headers: stepsHeaders }),
    ]);
    if (!manifestResponse.ok) {
      throw new Error("trace 不存在；请确认模拟已完成且 trace 文件仍在输出目录。");
    }
    const manifest = await manifestResponse.json();
    if (!isObject(manifest)) throw new Error("manifest.json 顶层必须是 JSON object。");
    if (manifest.status !== "running") nodes.autoRefreshInput.checked = false;
    if (canAppend && stepsResponse.status === 416) {
      const fileLength = numeric(stepsResponse.headers.get("X-Canwu-File-Length"), Number.NaN);
      if (Number.isFinite(fileLength) && fileLength < state.sourceByteOffset) {
        stepsResponse = await fetch(new URL("steps.jsonl", base), { cache: "no-store" });
      } else {
        const currentParsed = {
          frames: state.frames,
          totalCount: state.totalFrameCount,
          partialTail: state.partialTail,
          scanDiagnostics: state.scanDiagnostics,
          formatVersions: state.formatVersions,
          lastCheckpointHash: state.frames.at(-1)?.checkpointHash || null,
        };
        const validation = validateTrace(manifest, currentParsed, "steps.jsonl");
        if (manifest.status !== "running" && validation.pairErrors.length) {
          throw new Error(`URL trace 的 manifest/steps 配对校验失败，已保留上一次画面：${validation.pairErrors.join("；")}`);
        }
        state.manifest = manifest;
        state.diagnostics = validation.diagnostics;
        state.notices = validation.notices;
        await verifyTraceIfAvailable(base, manifest);
        renderMeta();
        setStatus("已检查 trace；没有新的完整 frame");
        nodes.refreshButton.disabled = false;
        startAutoRefresh();
        return;
      }
    }
    if (!stepsResponse.ok) {
      throw new Error("steps.jsonl 不可读取；请确认 trace 文件仍在输出目录。");
    }
    if (canAppend && stepsResponse.status === 206) {
      let parsed;
      try {
        parsed = await parseSteps(stepsResponse, state.totalFrameCount);
      } catch {
        parsed = null;
      }
      const existingLast = state.frames.at(-1);
      const appendedFirst = parsed?.frames[0];
      const tailConnects = !appendedFirst || !existingLast || (
        appendedFirst.sequence === existingLast.sequence + 1
        && (!existingLast.boundary?.hash || !appendedFirst.boundary?.previous_hash || existingLast.boundary.hash === appendedFirst.boundary.previous_hash)
      );
      if (!parsed || !tailConnects) {
        const fullResponse = await fetch(new URL("steps.jsonl", base), { cache: "no-store" });
        if (!fullResponse.ok) throw new Error("trace 尾部无法衔接，且完整重载失败。");
        applyTrace(manifest, await parseSteps(fullResponse, 0, state.retainAllFrames ? Number.POSITIVE_INFINITY : MAX_FRAMES_IN_MEMORY), "steps.jsonl", sourceLabel, manifest.status !== "running", canAppend);
      } else {
        appendTrace(manifest, parsed, "steps.jsonl");
      }
    } else {
      const parsed = await parseSteps(stepsResponse, 0, state.retainAllFrames ? Number.POSITIVE_INFINITY : MAX_FRAMES_IN_MEMORY);
      applyTrace(manifest, parsed, "steps.jsonl", sourceLabel, manifest.status !== "running", canAppend);
    }
    await verifyTraceIfAvailable(base, manifest);
    renderMeta();
    nodes.refreshButton.disabled = false;
    startAutoRefresh();
  }

  function appendTrace(manifest, parsed, stepsName) {
    const existingLast = state.frames.at(-1);
    const appendedFirst = parsed.frames[0];
    const crossDiagnostics = [];
    if (existingLast && appendedFirst && appendedFirst.sequence !== existingLast.sequence + 1) {
      crossDiagnostics.push(`增量 frame 序号在 ${existingLast.sequence} → ${appendedFirst.sequence} 之间不连续`);
    }
    if (existingLast?.boundary?.hash && appendedFirst?.boundary?.previous_hash
        && existingLast.boundary.hash !== appendedFirst.boundary.previous_hash) {
      crossDiagnostics.push(`增量 boundary hash 链在 frame #${appendedFirst.sequence} 处断开`);
    }
    const wasFollowingLatest = state.selectedSequence === existingLast?.sequence;
    const frames = [...state.frames, ...parsed.frames];
    if (!state.retainAllFrames && frames.length > MAX_FRAMES_IN_MEMORY) frames.splice(0, frames.length - MAX_FRAMES_IN_MEMORY);
    const scanDiagnostics = [...state.scanDiagnostics, ...crossDiagnostics, ...parsed.scanDiagnostics];
    const formatVersions = new Set([...state.formatVersions, ...parsed.formatVersions]);
    const totalCount = state.totalFrameCount + parsed.totalCount;
    const combined = {
      frames,
      totalCount,
      partialTail: parsed.partialTail,
      scanDiagnostics,
      formatVersions,
      lastCheckpointHash: parsed.lastCheckpointHash || existingLast?.checkpointHash || null,
    };
    const validation = validateTrace(manifest, combined, stepsName);
    if (manifest.status !== "running" && validation.pairErrors.length) {
      throw new Error(`URL trace 的 manifest/steps 配对校验失败，已保留上一次画面：${validation.pairErrors.join("；")}`);
    }
    state.manifest = manifest;
    state.frames = frames;
    state.totalFrameCount = totalCount;
    state.retainedStartOrdinal = Math.max(0, totalCount - frames.length);
    state.scanDiagnostics = scanDiagnostics;
    state.formatVersions = formatVersions;
    state.sourceByteOffset += parsed.consumedBytes;
    state.partialTail = parsed.partialTail;
    state.diagnostics = validation.diagnostics;
    state.notices = validation.notices;
    if (wasFollowingLatest && parsed.frames.length) {
      state.selectedSequence = frames.at(-1)?.sequence ?? state.selectedSequence;
      state.selectionNotice = null;
    } else if (state.selectedSequence !== null && !frames.some((frame) => frame.sequence === state.selectedSequence)) {
      const evictedSequence = state.selectedSequence;
      state.selectedSequence = frames[0]?.sequence ?? null;
      state.selectionNotice = `原先选中的 Frame #${evictedSequence} 已退出最近 ${MAX_FRAMES_IN_MEMORY} 帧窗口；已明确切换到 Frame #${state.selectedSequence}。可点击“载入全部（内存）”恢复旧帧。`;
    }
    render();
    setStatus(parsed.totalCount ? `增量追加 ${parsed.totalCount} 个 frame；trace 共 ${totalCount} 帧` : "已检查 trace；等待下一条完整 frame");
  }

  async function verifyTraceIfAvailable(base, manifest) {
    if (manifest.status === "running") {
      state.cryptoVerification = { status: "pending", message: "运行完成后校验 BLAKE3 边界内容" };
      return;
    }
    if (!base.pathname.startsWith("/__canwu_trace/")) {
      state.cryptoVerification = null;
      return;
    }
    try {
      const response = await fetch(new URL("verify.json", base), { cache: "no-store" });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const result = await response.json();
      state.cryptoVerification = {
        status: result.verified ? "verified" : "failed",
        framesChecked: numeric(result.frames_checked),
        message: result.verified
          ? `BLAKE3 已验证 ${numeric(result.frames_checked)} 个 boundary 的内容与前后链`
          : `BLAKE3 校验失败：${(result.errors || []).slice(0, 2).join("；") || "未知错误"}`,
      };
    } catch (error) {
      state.cryptoVerification = { status: "unavailable", message: `BLAKE3 校验服务不可用：${error.message}` };
    }
  }

  async function loadTraceFromQuery() {
    const tracePath = new URLSearchParams(window.location.search).get("trace");
    if (!tracePath) return;
    try {
      await loadTracePath(tracePath, "URL trace");
    } catch (error) {
      showError(error.message);
      setStatus("URL trace 载入失败");
    }
  }

  async function refreshTrace() {
    if (!state.sourcePath || state.refreshing) return;
    state.refreshing = true;
    try {
      await loadTracePath(state.sourcePath, state.sourceLabel || "trace");
    } catch (error) {
      showError(error.message);
      setStatus("刷新失败；保留上一次成功载入的内容");
    } finally {
      state.refreshing = false;
    }
  }

  function startAutoRefresh() {
    if (state.refreshTimer) clearInterval(state.refreshTimer);
    if (!nodes.autoRefreshInput.checked || !state.sourcePath) return;
    state.refreshTimer = setInterval(refreshTrace, 3000);
  }

  async function loadDefaultTrace() {
    try {
      setStatus("读取默认样例…");
      await loadTracePath("../../artifacts/traces/ming-fiscal-reference/hongwu-1391/", "默认样例");
    } catch (error) {
      showError(window.location.protocol === "file:" ? "浏览器禁止 file:// 页面直接读取默认路径。请用本地 HTTP server 打开，或选择两个文件。" : error.message);
      setStatus("默认样例不可用");
    }
  }

  function render() {
    nodes.emptyState.hidden = true;
    nodes.viewerState.hidden = false;
    renderMeta();
    renderFilters();
    renderOverview();
    renderTimeline();
    renderDetail();
  }

  function renderMeta() {
    const manifest = state.manifest || {};
    $("metaFixture").textContent = manifest.fixture_id || "generic-run";
    $("metaEngine").textContent = manifest.engine_version || "—";
    $("metaStatus").textContent = manifest.status || "—";
    $("metaSource").textContent = state.pendingPair
      ? `等待本地文件配对（当前仍显示 ${state.sourceLabel || "上一条 trace"}）`
      : state.sourceLabel || "—";
    $("metaConnection").textContent = connectionLabel(manifest);
    const crypto = state.cryptoVerification;
    $("metaIntegrity").textContent = state.diagnostics.length
      ? `结构异常 ${state.diagnostics.length} 项`
      : crypto?.status === "verified"
        ? `BLAKE3 边界通过 · ${crypto.framesChecked} 帧`
        : crypto?.status === "failed"
          ? "BLAKE3 边界校验失败"
          : crypto?.status === "pending"
            ? "结构通过 · 等待 BLAKE3"
            : "结构通过 · 未验 BLAKE3 内容";
    $("metaStepsFile").textContent = manifest.steps_file || "steps.jsonl";
    nodes.validationBox.hidden = false;
    nodes.validationBox.classList.toggle("is-valid", state.diagnostics.length === 0);
    const notices = state.notices.length
      ? `<ul>${state.notices.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>`
      : "";
    const cryptoMessage = crypto?.message ? `<p>${escapeHtml(crypto.message)}</p>` : "";
    nodes.validationBox.innerHTML = state.diagnostics.length
      ? `<strong>结构检查发现问题；详情顶部也会持续提示：</strong><ul>${state.diagnostics.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>${notices}`
      : crypto?.status === "verified"
        ? `<strong>BLAKE3 边界内容与前后链验证通过。</strong>${cryptoMessage}${notices}`
        : `<strong>基础结构检查通过。</strong> 已检查必填结构、frame 序号、数量、格式版本、boundary 链接和 checkpoint 引用。${cryptoMessage || " 未进行 BLAKE3 内容重算。"}${notices}`;
  }

  function connectionLabel(manifest) {
    if (state.pendingPair) return "刷新已暂停；补齐文件前不会切换当前画面";
    if (state.sourceMode === "local") return "本地静态副本（不会自动刷新）";
    if (state.sourceMode !== "url") return "—";
    if (manifest.status === "running") return nodes.autoRefreshInput.checked ? "实时 trace（每 3 秒刷新）" : "实时 trace（手动刷新）";
    return nodes.autoRefreshInput.checked ? "已完成 URL trace（自动刷新）" : "已完成 URL trace（手动刷新）";
  }

  function renderFilters() {
    const phases = [...new Set(state.frames.map((frame) => frame.phase))];
    nodes.phaseFilter.innerHTML = `<option value="all">全部阶段</option>${phases.map((phase) => `<option value="${escapeHtml(phase)}">${escapeHtml(phaseLabel(phase))}</option>`).join("")}`;
    nodes.phaseFilter.value = state.phase;
    const visibleCount = visibleFrames().length;
    nodes.filterStatus.textContent = state.totalFrameCount > state.frames.length
      ? `${visibleCount} / 最近 ${state.frames.length} 个 frame 可见（trace 共 ${state.totalFrameCount}）`
      : `${visibleCount} / ${state.frames.length} 个 frame 可见`;
    nodes.loadAllFramesButton.hidden = state.totalFrameCount <= state.frames.length;
    updateTimelineNavigation(visibleCount);
  }

  function renderOverview() {
    const last = state.frames[state.frames.length - 1];
    const domainRevision = last?.domains
      .map((domain) => domain.payload)
      .map((payload) => isObject(payload) ? (payload.procedure_revision ?? payload.revision ?? payload.version ?? payload.schema_version) : null)
      .find((value) => value !== null && value !== undefined) ?? "—";
    $("statSteps").textContent = String(state.totalFrameCount);
    $("statTime").textContent = last ? formatSimulationDate(last) : "—";
    $("statTime").title = last
      ? `公元日期以 ${historicalYearForFrame(last)}-01-01 00:00:00 为模拟起点；原始 SimTime=${last.at} 分钟`
      : "";
    $("statDomainRevision").textContent = String(domainRevision);
    $("statCheckpoint").textContent = shortHash(last?.checkpointHash || state.manifest?.final_checkpoint_hash);
    $("statCheckpoint").dataset.fullValue = last?.checkpointHash || state.manifest?.final_checkpoint_hash || "";
  }

  function visibleFrames() {
    const query = state.search.trim().toLowerCase();
    return state.frames.filter((frame) => {
      if (state.phase !== "all" && frame.phase !== state.phase) return false;
      return matchesFrameSearch(frame, query);
    });
  }

  function matchesFrameSearch(frame, query) {
    if (!query) return true;
    const structured = query.match(/^(frame|sequence|boundary|revision|phase|hash|time|simtime)\s*[:=#]\s*(.+)$/);
    if (structured) {
      const [, field, value] = structured;
      const candidates = {
        frame: frame.sequence,
        sequence: frame.sequence,
        boundary: frame.boundaryId,
        revision: frame.revision,
        phase: `${frame.phase} ${frame.phaseLabel}`,
        hash: frame.checkpointHash,
        time: `${frame.at} ${formatSimulationDate(frame)}`,
        simtime: frame.at,
      };
      if (["frame", "sequence", "boundary", "revision", "simtime"].includes(field)) {
        const expected = Number(value.trim());
        return Number.isFinite(expected) && numeric(candidates[field], Number.NaN) === expected;
      }
      return String(candidates[field] ?? "").toLowerCase().includes(value.trim());
    }
    return [frame.sequence, frame.phase, frame.phaseLabel, frame.boundaryId, frame.revision, frame.checkpointHash, frame.at, formatSimulationDate(frame)]
      .some((value) => String(value).toLowerCase().includes(query));
  }

  function historicalYearForFrame(frame) {
    const fiscal = frame?.domains?.find((domain) => domain.key === "fiscal")?.payload;
    const candidates = [
      fiscal?.historical_year,
      fiscal?.state?.historical_context?.year,
      state.manifest?.historical_year,
      state.manifest?.start_year,
    ];
    return candidates
      .map((value) => Number(value))
      .find((value) => Number.isInteger(value) && value >= 1 && value <= 9999) || 1970;
  }

  function formatSimulationDate(frame) {
    const minutes = numeric(frame?.at, Number.NaN);
    if (!Number.isFinite(minutes)) return "—";
    const date = new Date(0);
    date.setUTCFullYear(historicalYearForFrame(frame), 0, 1);
    date.setUTCHours(0, 0, 0, 0);
    date.setUTCMinutes(minutes);
    if (Number.isNaN(date.getTime())) return "—";
    const pad = (value, width = 2) => String(value).padStart(width, "0");
    return `${pad(date.getUTCFullYear(), 4)}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}:${pad(date.getUTCSeconds())}`;
  }

  function formatSimulationTimeLabel(frame) {
    const date = formatSimulationDate(frame);
    const minutes = numeric(frame?.at, Number.NaN);
    return date === "—" ? `SimTime ${escapeHtml(frame?.at ?? "—")} min` : `${date} · SimTime ${minutes} min`;
  }

  function renderTimeline() {
    const visible = visibleFrames();
    nodes.timelineCount.textContent = state.totalFrameCount > state.frames.length
      ? `${visible.length} / 最近 ${state.frames.length}（共 ${state.totalFrameCount}）`
      : `${visible.length} / ${state.frames.length}`;
    const maxStart = lastTimelinePageStart(visible.length);
    state.timelineStart = Math.min(state.timelineStart, maxStart);
    const windowed = visible.slice(state.timelineStart, state.timelineStart + TIMELINE_PAGE_SIZE);
    const previousScrollLeft = nodes.timeline.scrollLeft;
    const shouldRevealSelection = nodes.timeline.dataset.selectedSequence !== String(state.selectedSequence)
      || nodes.timeline.dataset.windowStart !== String(state.timelineStart);
    nodes.timeline.innerHTML = windowed.map((frame) => `
      <button class="timeline-row" type="button" role="option" data-sequence="${frame.sequence}" aria-selected="${frame.sequence === state.selectedSequence}">
        <span class="timeline-index">#${escapeHtml(frame.boundaryId)}</span>
        <span class="timeline-main">
          <span class="timeline-phase">${escapeHtml(frame.phaseLabel)}</span>
          <span class="timeline-time">${escapeHtml(formatSimulationDate(frame))} · Frame #${escapeHtml(frame.sequence)}</span>
        </span>
        <span class="timeline-delta">状态 ${frame.engine.changes} · 记录 ${frame.engine.recordChanges}</span>
      </button>`).join("");
    nodes.timeline.querySelectorAll("[data-sequence]").forEach((button) => {
      button.addEventListener("click", () => {
        state.selectedSequence = numeric(button.dataset.sequence);
        renderTimeline();
        renderDetail();
      });
    });
    nodes.timeline.scrollLeft = previousScrollLeft;
    nodes.timeline.dataset.selectedSequence = String(state.selectedSequence);
    nodes.timeline.dataset.windowStart = String(state.timelineStart);
    if (shouldRevealSelection) {
      requestAnimationFrame(() => {
        nodes.timeline.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest", inline: "center" });
        updateTimelineScrollButtons();
      });
    } else {
      requestAnimationFrame(updateTimelineScrollButtons);
    }
    updateTimelineNavigation(visible.length);
  }

  function updateTimelineNavigation(total) {
    const maxStart = lastTimelinePageStart(total);
    state.timelineStart = Math.min(Math.max(0, state.timelineStart), maxStart);
    const start = total ? state.timelineStart + 1 : 0;
    const end = total ? Math.min(total, state.timelineStart + TIMELINE_PAGE_SIZE) : 0;
    nodes.timelineRange.textContent = `${start}–${end} / ${total}`;
    nodes.timelinePreviousButton.disabled = state.timelineStart === 0;
    nodes.timelineNextButton.disabled = state.timelineStart >= maxStart;
    nodes.timelineLatestButton.disabled = total === 0;
  }

  function moveTimeline(delta) {
    const visible = visibleFrames();
    const total = visible.length;
    const maxStart = lastTimelinePageStart(total);
    state.timelineStart = Math.min(maxStart, Math.max(0, state.timelineStart + delta));
    state.selectedSequence = visible[state.timelineStart]?.sequence ?? null;
    renderTimeline();
    renderDetail();
  }

  function jumpTimelineLatest() {
    state.timelineStart = lastTimelinePageStart(visibleFrames().length);
    const visible = visibleFrames();
    state.selectedSequence = visible.at(-1)?.sequence ?? null;
    renderTimeline();
    renderDetail();
  }

  function updateTimelineScrollButtons() {
    const maxScroll = Math.max(0, nodes.timeline.scrollWidth - nodes.timeline.clientWidth);
    nodes.timelineScrollPreviousButton.disabled = nodes.timeline.scrollLeft <= 20;
    nodes.timelineScrollNextButton.disabled = nodes.timeline.scrollLeft >= maxScroll - 20;
  }

  function timelinePageStartForIndex(index) {
    return Math.floor(Math.max(0, index) / TIMELINE_PAGE_SIZE) * TIMELINE_PAGE_SIZE;
  }

  function lastTimelinePageStart(total) {
    return total > 0 ? timelinePageStartForIndex(total - 1) : 0;
  }

  function scrollTimeline(direction) {
    const distance = Math.max(240, Math.round(nodes.timeline.clientWidth * 0.72));
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    nodes.timeline.scrollBy({ left: distance * direction, behavior: reducedMotion ? "auto" : "smooth" });
  }

  function setSourceCollapsed(collapsed) {
    const shell = document.querySelector(".app-shell");
    shell.classList.toggle("source-collapsed", collapsed);
    nodes.toggleSourceButton.textContent = collapsed ? "载入其他 trace" : "隐藏来源栏";
    nodes.toggleSourceButton.setAttribute("aria-expanded", String(!collapsed));
  }

  async function loadAllFrames() {
    state.retainAllFrames = true;
    state.selectionNotice = null;
    setStatus("正在按流载入完整 trace；这会使用更多浏览器内存…");
    try {
      if (state.sourceMode === "url" && state.sourcePath) {
        state.sourceByteOffset = 0;
        await loadTracePath(state.sourcePath, state.sourceLabel || "URL trace");
      } else if (state.sourceMode === "local" && state.localManifestFile && state.localStepsFile) {
        await readTraceFiles(state.localManifestFile, state.localStepsFile, "本地文件", true);
      } else {
        throw new Error("当前来源不支持重新载入全部 frame。");
      }
    } catch (error) {
      state.retainAllFrames = false;
      showError(error.message);
      setStatus("完整 trace 载入失败；继续保留最近窗口");
    }
  }

  function renderDetail() {
    const visible = visibleFrames();
    const frame = visible.find((candidate) => candidate.sequence === state.selectedSequence) || visible[0];
    if (!frame) {
      nodes.detail.innerHTML = '<div class="empty-detail"><h2>当前筛选没有 frame</h2><p class="muted">请更改阶段或搜索条件。</p></div>';
      return;
    }
    const engine = frame.engine;
    const visibleIndex = visible.findIndex((candidate) => candidate.sequence === frame.sequence);
    const previousVisible = visibleIndex > 0 ? visible[visibleIndex - 1] : null;
    const nextVisible = visibleIndex >= 0 && visibleIndex < visible.length - 1 ? visible[visibleIndex + 1] : null;
    const actualIndex = state.frames.findIndex((candidate) => candidate.sequence === frame.sequence);
    const actualPrevious = actualIndex > 0 ? state.frames[actualIndex - 1] : null;
    const previousOutsideWindow = !actualPrevious && frame.ordinal > 0;
    if (state.evidenceFrameSequence !== frame.sequence) {
      state.evidenceFrameSequence = frame.sequence;
      state.evidenceSearch = "";
      state.evidencePage = 0;
    }
    nodes.detail.innerHTML = `
      <div class="detail-heading">
        <div>
          <span class="phase-mark">${escapeHtml(frame.phaseLabel)}</span>
          <h2>结算边界 #${escapeHtml(frame.boundaryId)} · Frame #${escapeHtml(frame.sequence)}</h2>
          <p>结算边界 ID <code>${escapeHtml(frame.boundaryId)}</code> · 公元时间 <code>${escapeHtml(formatSimulationDate(frame))}</code> · 模拟时间 <code>${escapeHtml(frame.at)} 分钟</code> · 引擎 revision <code>${escapeHtml(frame.revision)}</code></p>
        </div>
        <div class="detail-actions">
          <button id="previousFrameButton" class="button button-secondary" type="button" ${previousVisible ? "" : "disabled"}>上一个可见 frame</button>
          <button id="nextFrameButton" class="button button-secondary" type="button" ${nextVisible ? "" : "disabled"}>下一个可见 frame</button>
        </div>
      </div>
      ${state.diagnostics.length ? `<div class="detail-warning"><strong>当前 trace 有 ${state.diagnostics.length} 项结构警告。</strong> ${escapeHtml(state.diagnostics[0])}</div>` : ""}
      ${state.selectionNotice ? `<div class="detail-notice"><strong>选择已更新：</strong> ${escapeHtml(state.selectionNotice)}</div>` : ""}
      <section class="detail-section stage-overview">
        <h3>阶段总览 · 谁参与、现在是什么状态</h3>
        ${renderStageOverview(frame)}
      </section>
      <section class="detail-section">
        <h3>相对真实上一帧发生了什么</h3>
        ${renderFrameDifference(actualPrevious, frame, previousOutsideWindow)}
      </section>
      <section class="detail-section">
        <h3>引擎结算摘要</h3>
        <div class="metric-grid">
          ${metric("状态变化", engine.changes)}
          ${metric("领域记录变化", engine.recordChanges)}
          ${metric("知识记录", engine.knowledge)}
          ${metric("事件", engine.events)}
          ${metric("外部输入", engine.ingress)}
          ${metric("资源分配", engine.allocations)}
          ${metric("随机数抽取", engine.randomDraws)}
          ${metric("引擎 revision", frame.revision)}
        </div>
      </section>
      <section class="detail-section">
        <h3>检测到的领域</h3>
        <div class="domain-list">${frame.domains.length ? frame.domains.map(renderDomain).join("") : '<p class="muted">这一 frame 没有附带可识别的 domain snapshot。</p>'}</div>
      </section>
      <section class="detail-section">
        <h3>结算证据</h3>
        <div id="evidencePanel">${renderEvidence(frame)}</div>
      </section>
      <section class="detail-section">
        <div class="detail-section-heading">
          <h3>原始 frame</h3>
          <div class="detail-actions">
            <button id="copyFrameButton" class="button button-secondary" type="button">复制 frame JSON</button>
            <button id="downloadFrameButton" class="button button-secondary" type="button">下载 frame</button>
          </div>
        </div>
        <details><summary>打开原始 JSON</summary><pre>${escapeHtml(JSON.stringify(frame.raw, null, 2))}</pre></details>
      </section>`;
    $("previousFrameButton").addEventListener("click", () => selectFrame(previousVisible));
    $("nextFrameButton").addEventListener("click", () => selectFrame(nextVisible));
    $("copyFrameButton").addEventListener("click", () => copyText(JSON.stringify(frame.raw, null, 2), "frame JSON 已复制"));
    $("downloadFrameButton").addEventListener("click", () => downloadText(`frame-${frame.sequence}.json`, JSON.stringify(frame.raw, null, 2), "application/json"));
    bindEvidenceControls(frame);
  }

  function selectFrame(frame) {
    if (!frame) return;
    state.selectedSequence = frame.sequence;
    state.selectionNotice = null;
    const visible = visibleFrames();
    const index = visible.findIndex((candidate) => candidate.sequence === frame.sequence);
    if (index >= 0 && (index < state.timelineStart || index >= state.timelineStart + TIMELINE_PAGE_SIZE)) {
      state.timelineStart = timelinePageStartForIndex(index);
    }
    renderTimeline();
    renderDetail();
  }

  function renderFrameDifference(previous, current, previousOutsideWindow = false) {
    if (previousOutsideWindow) return '<p class="muted">真实上一帧位于已释放的旧数据窗口中；为保持浏览器内存有界，本页不猜测对比结果。</p>';
    if (!previous) return '<p class="muted">这是 JSONL 中的第一条 frame，没有真实上一帧可供比较。</p>';
    const changes = [];
    Object.entries(current.engine).forEach(([key, value]) => {
      const before = previous.engine[key];
      if (before !== value) changes.push(`本帧${humanMetricLabel(key)}：${value}（上一帧 ${before}）`);
    });
    const previousDomains = new Map(previous.domains.map((domain) => [domain.key, domain]));
    const currentDomains = new Map(current.domains.map((domain) => [domain.key, domain]));
    [...new Set([...previousDomains.keys(), ...currentDomains.keys()])].sort().forEach((key) => {
      const before = previousDomains.get(key);
      const after = currentDomains.get(key);
      if (!before) changes.push(`领域 ${key}：新增`);
      else if (!after) changes.push(`领域 ${key}：移除`);
      else collectChangedPaths(before.payload, after.payload, `domain.${key}`, changes, Number.POSITIVE_INFINITY);
    });
    if (!changes.length) return '<p class="muted">结构化计数和领域字段没有可见变化；可展开结算证据确认本边界的处理过程。</p>';
    const preview = changes.slice(0, 30);
    const remaining = changes.slice(30);
    return `<p class="muted">基线：Frame #${escapeHtml(previous.sequence)}（结算边界 #${escapeHtml(previous.boundaryId)}）；比较引擎与全部检测到的领域，共 ${changes.length} 条路径变化。</p><ul class="change-list">${preview.map((change) => `<li>${escapeHtml(change)}</li>`).join("")}</ul>${remaining.length ? `<details><summary>显示其余 ${remaining.length} 条完整差异</summary><ul class="change-list">${remaining.map((change) => `<li>${escapeHtml(change)}</li>`).join("")}</ul></details>` : ""}`;
  }

  function renderStageOverview(frame) {
    const entities = stageEntities(frame);
    const boundary = frame.boundary || {};
    const domainCount = frame.domains.length;
    const holderCount = new Set((Array.isArray(boundary.knowledge_changes) ? boundary.knowledge_changes : [])
      .map((change) => isObject(change) ? JSON.stringify(change.holder || change.records?.[0]?.holder || null) : null)
      .filter((holder) => holder && holder !== "null")).size;
    const summary = [
      ["参与实体", entities.length, "本阶段可定位的命令、输入、事件、记录和知识对象"],
      ["领域快照", domainCount, "本 frame 提供的当前领域状态"],
      ["知识持有人", holderCount, "本阶段产生或更新的观察视角"],
      ["边界证据", evidenceCount(frame), "可展开的处理证据条目"],
    ];
    const entityRow = (entity) => `
      <div class="stage-entity-row">
        <span class="stage-entity-name">${escapeHtml(entity.name)}</span>
        <span class="stage-entity-type">${escapeHtml(entity.type)}</span>
        <span class="stage-entity-state">${escapeHtml(entity.state)}</span>
        ${entity.detail ? `<span class="stage-entity-detail">${escapeHtml(entity.detail)}</span>` : ""}
      </div>`;
    const displayed = entities.slice(0, 28);
    const primaryRows = displayed.slice(0, 8).map(entityRow).join("");
    const remainingRows = displayed.slice(8).map(entityRow).join("");
    const omitted = Math.max(0, entities.length - 28);
    return `<div class="stage-summary-grid">${summary.map(([label, value, context]) => `<div class="stage-summary"><strong>${escapeHtml(value)}</strong><span>${escapeHtml(label)}</span><small>${escapeHtml(context)}</small></div>`).join("")}</div>
      <div class="stage-entity-table"><div class="stage-entity-head"><span>实体</span><span>类型</span><span>当前状态</span><span>说明</span></div>${primaryRows || '<p class="muted">这一阶段没有可从通用证据中定位的实体。</p>'}</div>
      ${remainingRows ? `<details class="stage-entity-more"><summary>显示其余 ${displayed.length - 8} 个参与实体</summary><div class="stage-entity-table">${remainingRows}</div></details>` : ""}
      ${omitted ? `<p class="muted stage-entity-note">已显示前 28 个实体，另有 ${omitted} 个实体；请结合领域状态或原始 JSON 继续检索。</p>` : ""}`;
  }

  function evidenceCount(frame) {
    const boundary = frame.boundary || {};
    return [boundary.admitted_events, boundary.generated_ingress, boundary.record_changes, boundary.knowledge_changes, boundary.emissions, boundary.random_draws]
      .filter(Array.isArray).reduce((total, values) => total + values.length, 0);
  }

  function stageEntities(frame) {
    const entities = [];
    const seen = new Set();
    const add = (name, type, state, detail = "") => {
      const key = `${type}:${name}:${state}`;
      if (seen.has(key)) return;
      seen.add(key);
      entities.push({ name, type, state, detail });
    };
    const boundary = frame.boundary || {};
    (Array.isArray(boundary.admitted_attempts) ? boundary.admitted_attempts : []).forEach((id) => add(`attempt.${id}`, "命令尝试", "已纳入本边界"));
    (Array.isArray(boundary.admitted_commands) ? boundary.admitted_commands : []).forEach((id) => add(`command.${id}`, "命令", "已纳入本边界"));
    (Array.isArray(boundary.admitted_ingress) ? boundary.admitted_ingress : []).forEach((id) => add(`ingress.${id}`, "外部输入", "已接纳"));
    (Array.isArray(boundary.generated_ingress) ? boundary.generated_ingress : []).forEach((item, index) => add(`generated-ingress.${isObject(item) ? item.id ?? index + 1 : index + 1}`, "生成输入", "由本边界产生", compact(item, 100)));
    (Array.isArray(boundary.admitted_events) ? boundary.admitted_events : []).forEach((id) => add(`event.${id}`, "事件", "已记录"));
    (Array.isArray(boundary.reservation_offers) ? boundary.reservation_offers : []).forEach((item, index) => add(`reservation-offer.${isObject(item) ? item.id ?? index + 1 : index + 1}`, "预约报价", "已记录", compact(item, 100)));
    (Array.isArray(boundary.reservation_requests) ? boundary.reservation_requests : []).forEach((item, index) => add(`reservation-request.${isObject(item) ? item.id ?? index + 1 : index + 1}`, "预约请求", "已记录", compact(item, 100)));
    (Array.isArray(boundary.allocations) ? boundary.allocations : []).forEach((item, index) => add(`allocation.${isObject(item) ? item.id ?? index + 1 : index + 1}`, "资源分配", "已结算", compact(item, 100)));
    (Array.isArray(boundary.random_draws) ? boundary.random_draws : []).forEach((id) => add(`random.${id}`, "随机抽取", "已记录"));
    (Array.isArray(boundary.record_changes) ? boundary.record_changes : []).forEach((change, index) => {
      if (!isObject(change)) {
        add(`record-change.${index + 1}`, "领域记录", "当前 frame 提供非结构化变化", compact(change, 100));
        return;
      }
      const current = isObject(change.current) ? change.current : {};
      const reference = isObject(current.reference) ? current.reference : {};
      const name = reference.id ?? change.id ?? `record-change.${index + 1}`;
      const owner = current.owner || change.plugin || "domain";
      add(String(name), `${owner} 记录`, `${change.operation || "updated"} · v${current.version ?? "—"}`, current.lifecycle?.state || change.summary || "");
    });
    (Array.isArray(boundary.knowledge_changes) ? boundary.knowledge_changes : []).forEach((change, index) => {
      if (!isObject(change)) {
        add(`knowledge.${index + 1}`, "知识视角", "当前 frame 提供非结构化变化", compact(change, 100));
        return;
      }
      const holder = change.holder ? compact(change.holder, 80) : "未知持有人";
      const records = Array.isArray(change.records) ? change.records.length : 0;
      add(`knowledge.${change.producer_correlation || index + 1}`, "知识视角", `${records} 条记录已发布`, `持有人 ${holder}`);
    });
    frame.domains.forEach((domain) => {
      const payload = domain.payload;
      const state = isObject(payload?.state) ? payload.state : payload;
      if (!isObject(state)) {
        add(`domain.${domain.key}`, `${domain.label} 领域`, "存在快照");
        return;
      }
      const maps = ["authority_bindings", "scope_bindings", "adoptions", "assessments", "remissions", "execution_requests", "execution_receipts", "audits", "action_outcomes", "transition_candidates", "aggregates"];
      let added = 0;
      maps.forEach((mapKey) => {
        if (!isObject(state[mapKey])) return;
        Object.entries(state[mapKey]).forEach(([id, value]) => {
          if (added >= 20) return;
          const status = isObject(value) ? (value.stage || value.disposition || value.status || value.lifecycle?.state || "当前") : compact(value, 70);
          add(id, `${domain.label} · ${mapKey}`, String(status), isObject(value) ? (value.mechanism || value.rule_id || value.institution?.id ? compact(value.mechanism || value.rule_id || value.institution?.id, 80) : "") : "");
          added += 1;
        });
      });
      if (added < 20) {
        Object.entries(state).forEach(([key, value]) => {
          if (added >= 20 || maps.includes(key)) return;
          if (isObject(value) || Array.isArray(value) || value !== null && value !== undefined) {
            const status = isObject(value)
              ? (value.stage || value.disposition || value.status || value.lifecycle?.state || value.state || "当前")
              : compact(value, 70);
            add(`domain.${domain.key}.${key}`, `${domain.label} · ${key}`, String(status), isObject(value) ? compact(value, 80) : "");
            added += 1;
          }
        });
      }
      if (!added) add(`domain.${domain.key}`, `${domain.label} 领域`, "存在快照", `字段 ${Object.keys(state).length} 个`);
    });
    return entities;
  }

  function collectChangedPaths(before, after, path, changes, limit) {
    if (changes.length >= limit || before === after) return;
    if (isObject(before) && isObject(after)) {
      [...new Set([...Object.keys(before), ...Object.keys(after)])].sort().forEach((key) => {
        if (changes.length < limit) collectChangedPaths(before[key], after[key], `${path}.${key}`, changes, limit);
      });
      return;
    }
    if (Array.isArray(before) && Array.isArray(after)) {
      if (JSON.stringify(before) !== JSON.stringify(after)) {
        changes.push(before.length === after.length
          ? `${path}：数组内容变化（${after.length} 项）`
          : `${path}：${before.length} 项 → ${after.length} 项`);
      }
      return;
    }
    if (JSON.stringify(before) !== JSON.stringify(after)) {
      changes.push(`${path}：${compact(before, 60)} → ${compact(after, 60)}`);
    }
  }

  function humanMetricLabel(key) {
    return ({ events: "事件", ingress: "外部输入", changes: "状态变化", recordChanges: "领域记录变化", knowledge: "知识记录", allocations: "资源分配", randomDraws: "随机数抽取" })[key] || key;
  }

  function metric(label, value) {
    return `<div class="metric"><span class="metric-label">${escapeHtml(label)}</span><strong class="metric-value">${escapeHtml(value)}</strong></div>`;
  }

  function renderDomain(domain) {
    const payload = domain.payload;
    if (domain.key === "fiscal" && isObject(payload)) return renderFiscalDomain(payload);
    const entries = isObject(payload) ? Object.entries(payload).filter(([, value]) => !isObject(value) && !Array.isArray(value)).slice(0, 8) : [];
    return `<article class="domain-block"><div class="domain-title"><strong>${escapeHtml(domain.label)}</strong><span>generic domain</span></div>${entries.length ? `<dl class="generic-summary">${entries.map(([key, value]) => `<div><dt>${escapeHtml(key)}</dt><dd>${escapeHtml(compact(value))}</dd></div>`).join("")}</dl>` : `<pre>${escapeHtml(JSON.stringify(payload, null, 2))}</pre>`}</article>`;
  }

  function renderFiscalDomain(payload) {
    const counts = isObject(payload.counts) ? payload.counts : {};
    const countRows = Object.entries(counts).map(([key, value]) => `<tr><th>${escapeHtml(key)}</th><td>${escapeHtml(value)}</td></tr>`).join("");
    const projections = isObject(payload.projections) ? Object.entries(payload.projections) : [];
    return `<article class="domain-block"><div class="domain-title"><strong>财政领域</strong><span>canwu-fiscal</span></div>${countRows ? `<table class="count-table"><tbody>${countRows}</tbody></table>` : ""}${projections.length ? `<div class="projection-list">${projections.map(([key, value]) => {
      const projection = isObject(value) ? value : {};
      const facts = isObject(projection.facts) ? Object.keys(projection.facts).length : 0;
      return `<div class="projection-row"><code>${escapeHtml(key)}</code><span><strong>${escapeHtml(projection.confidence_per_mille ?? "—")}</strong>‰ · ${facts} 条已知事实</span></div>`;
    }).join("")}</div>` : ""}<details><summary>财政状态 JSON</summary><pre>${escapeHtml(JSON.stringify(payload.state || payload, null, 2))}</pre></details></article>`;
  }

  function renderEvidence(frame) {
    const boundary = frame.boundary || {};
    const groups = [
      ["接纳事件", boundary.admitted_events],
      ["生成输入", boundary.generated_ingress],
      ["领域记录变化", boundary.record_changes],
      ["知识变化", boundary.knowledge_changes],
      ["发射记录", boundary.emissions],
      ["随机数抽取", boundary.random_draws],
    ].filter(([, value]) => Array.isArray(value));
    const all = groups.flatMap(([label, values]) => values.map((item, index) => ({ label, index, item, searchable: JSON.stringify(item).toLowerCase() })));
    const query = state.evidenceSearch.trim().toLowerCase();
    const filtered = query ? all.filter((entry) => `${entry.label} ${entry.searchable}`.toLowerCase().includes(query)) : all;
    const pageCount = Math.max(1, Math.ceil(filtered.length / EVIDENCE_PAGE_SIZE));
    state.evidencePage = Math.min(state.evidencePage, pageCount - 1);
    const start = state.evidencePage * EVIDENCE_PAGE_SIZE;
    const page = filtered.slice(start, start + EVIDENCE_PAGE_SIZE);
    if (!groups.length) return '<p class="muted">没有可展开的 boundary 数组；请查看原始 frame。</p>';
    return `
      <div class="evidence-toolbar">
        <label><span>搜索全部证据</span><input id="evidenceSearch" class="search-control" type="search" value="${escapeHtml(state.evidenceSearch)}" placeholder="事件 ID、plugin、system、operation 或任意字段" /></label>
        <span class="count-label">${filtered.length} / ${all.length} 条</span>
      </div>
      <div class="evidence-list">${page.length ? page.map((entry) => `<div class="evidence-item"><strong>${escapeHtml(entry.label)} #${entry.index + 1}</strong><span>${escapeHtml(compact(entry.item, 900))}</span><details><summary>查看这一条的完整 JSON</summary><pre>${escapeHtml(JSON.stringify(entry.item, null, 2))}</pre></details></div>`).join("") : '<p class="muted">没有匹配的证据。</p>'}</div>
      <div class="evidence-navigation">
        <button id="evidencePrevious" class="text-button" type="button" ${state.evidencePage === 0 ? "disabled" : ""}>上一页</button>
        <span class="count-label">${filtered.length ? start + 1 : 0}–${Math.min(filtered.length, start + EVIDENCE_PAGE_SIZE)} / ${filtered.length}</span>
        <button id="evidenceNext" class="text-button" type="button" ${state.evidencePage >= pageCount - 1 ? "disabled" : ""}>下一页</button>
      </div>`;
  }

  function bindEvidenceControls(frame) {
    const search = $("evidenceSearch");
    if (!search) return;
    const refreshPanel = (focusSearch = false) => {
      $("evidencePanel").innerHTML = renderEvidence(frame);
      bindEvidenceControls(frame);
      if (focusSearch) {
        const nextSearch = $("evidenceSearch");
        nextSearch.focus();
        nextSearch.setSelectionRange(nextSearch.value.length, nextSearch.value.length);
      }
    };
    search.addEventListener("input", (event) => {
      state.evidenceSearch = event.target.value;
      state.evidencePage = 0;
      refreshPanel(true);
    });
    $("evidencePrevious")?.addEventListener("click", () => {
      state.evidencePage = Math.max(0, state.evidencePage - 1);
      refreshPanel();
    });
    $("evidenceNext")?.addEventListener("click", () => {
      state.evidencePage += 1;
      refreshPanel();
    });
  }

  function setStatus(text) { nodes.loadStatus.textContent = text; }
  function showError(text) { nodes.errorBox.hidden = false; nodes.errorBox.textContent = text; }
  function hideError() { nodes.errorBox.hidden = true; nodes.errorBox.textContent = ""; }

  async function copyText(text, successMessage) {
    try {
      await navigator.clipboard.writeText(text);
      setStatus(successMessage);
    } catch {
      showError("浏览器不允许直接访问剪贴板；请从原始 frame 区域复制。");
    }
  }

  function downloadText(filename, text, type) {
    const link = document.createElement("a");
    link.href = URL.createObjectURL(new Blob([text], { type }));
    link.download = filename;
    link.click();
    URL.revokeObjectURL(link.href);
  }

  nodes.loadDefaultButton.addEventListener("click", loadDefaultTrace);
  nodes.emptyLoadButton.addEventListener("click", loadDefaultTrace);
  nodes.refreshButton.addEventListener("click", refreshTrace);
  nodes.toggleSourceButton.addEventListener("click", () => {
    const collapsed = document.querySelector(".app-shell").classList.contains("source-collapsed");
    setSourceCollapsed(!collapsed);
  });
  nodes.autoRefreshInput.addEventListener("change", () => {
    state.autoRefreshWanted = nodes.autoRefreshInput.checked;
    startAutoRefresh();
    if (state.manifest) renderMeta();
  });
  nodes.timelinePreviousButton.addEventListener("click", () => moveTimeline(-TIMELINE_PAGE_SIZE));
  nodes.timelineNextButton.addEventListener("click", () => moveTimeline(TIMELINE_PAGE_SIZE));
  nodes.timelineLatestButton.addEventListener("click", jumpTimelineLatest);
  nodes.timelineScrollPreviousButton.addEventListener("click", () => scrollTimeline(-1));
  nodes.timelineScrollNextButton.addEventListener("click", () => scrollTimeline(1));
  nodes.timeline.addEventListener("scroll", updateTimelineScrollButtons, { passive: true });
  nodes.timeline.addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    const visible = visibleFrames();
    if (!visible.length) return;
    event.preventDefault();
    const currentIndex = Math.max(0, visible.findIndex((frame) => frame.sequence === state.selectedSequence));
    const targetIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? visible.length - 1
        : Math.min(visible.length - 1, Math.max(0, currentIndex + (event.key === "ArrowRight" ? 1 : -1)));
    selectFrame(visible[targetIndex]);
  });
  nodes.loadAllFramesButton.addEventListener("click", loadAllFrames);
  nodes.frameSearch.addEventListener("input", (event) => {
    state.search = event.target.value;
    state.timelineStart = 0;
    const visible = visibleFrames();
    state.selectedSequence = visible[0]?.sequence ?? null;
    renderFilters();
    renderTimeline();
    renderDetail();
  });
  nodes.manifestInput.addEventListener("change", (event) => {
    beginPendingLocalPair();
    if (state.pendingManifest && state.pendingManifest !== event.target.files[0]) {
      state.pendingSteps = null;
      nodes.stepsInput.value = "";
    }
    state.pendingManifest = event.target.files[0] || null;
    loadPending();
  });
  nodes.stepsInput.addEventListener("change", (event) => {
    beginPendingLocalPair();
    if (state.pendingSteps && state.pendingSteps !== event.target.files[0]) {
      state.pendingManifest = null;
      nodes.manifestInput.value = "";
    }
    state.pendingSteps = event.target.files[0] || null;
    loadPending();
  });
  nodes.folderInput.addEventListener("change", (event) => pickFiles(event.target.files));
  nodes.phaseFilter.addEventListener("change", (event) => {
    state.phase = event.target.value;
    state.timelineStart = 0;
    const visible = visibleFrames();
    if (visible.length && !visible.some((frame) => frame.sequence === state.selectedSequence)) {
      state.selectedSequence = visible[0].sequence;
    }
    renderFilters();
    renderTimeline();
    renderDetail();
  });
  ["dragenter", "dragover"].forEach((eventName) => nodes.dropZone.addEventListener(eventName, (event) => { event.preventDefault(); nodes.dropZone.classList.add("is-dragging"); }));
  ["dragleave", "drop"].forEach((eventName) => nodes.dropZone.addEventListener(eventName, (event) => { event.preventDefault(); nodes.dropZone.classList.remove("is-dragging"); }));
  nodes.dropZone.addEventListener("drop", (event) => pickFiles(event.dataTransfer.files));
  nodes.dropZone.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") nodes.folderInput.click(); });
  loadTraceFromQuery();
})();
