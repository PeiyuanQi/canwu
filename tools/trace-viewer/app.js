(() => {
  "use strict";

  const state = {
    manifest: null,
    frames: [],
    selectedSequence: null,
    phase: "all",
    pendingManifest: null,
    pendingSteps: null,
  };

  const $ = (id) => document.getElementById(id);
  const nodes = {
    loadDefaultButton: $("loadDefaultButton"),
    emptyLoadButton: $("emptyLoadButton"),
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
    detail: $("detail"),
  };

  const phaseLabels = {
    initial_state: "Initial state",
    open_assessment: "Open assessment",
    authorize_execution: "Authorize execution",
    adapter_evidence: "Adapter evidence",
    fiscal_execution_receipt: "Fiscal execution receipt",
    report_materialization: "Report materialization",
    canonical_boundary: "Canonical boundary",
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
    const receipt = isObject(raw.receipt) ? raw.receipt : {};
    const boundary = isObject(raw.boundary) ? raw.boundary : (isObject(raw.boundary_record) ? raw.boundary_record : {});
    const phase = raw.phase || raw.semantic_phase || "canonical_boundary";
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
      formatVersion: raw.format_version ?? null,
      sequence: numeric(raw.sequence, index),
      phase: String(phase),
      phaseLabel: phaseLabel(phase),
      at: receipt.settled_at ?? boundary.at ?? raw.settled_at ?? "—",
      boundaryId: receipt.boundary_id ?? boundary.id ?? "—",
      revision: raw.revision ?? "—",
      checkpointHash: raw.checkpoint_hash ?? receipt.boundary_hash ?? boundary.hash ?? "—",
      engine,
      boundary,
      domains: extractDomains(raw),
    };
  }

  function extractDomains(raw) {
    const domains = [];
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
    return domains;
  }

  function parseSteps(text) {
    const errors = [];
    const frames = [];
    text.split(/\r?\n/).forEach((line, index) => {
      const trimmed = line.trim();
      if (!trimmed) return;
      try {
        frames.push(JSON.parse(trimmed));
      } catch (error) {
        errors.push(`steps.jsonl 第 ${index + 1} 行无法解析：${error.message}`);
      }
    });
    if (errors.length) throw new Error(errors.slice(0, 3).join("\n"));
    if (!frames.length) throw new Error("steps.jsonl 没有可读取的 frame。");
    return frames.map(normalizeFrame);
  }

  async function readTraceFiles(manifestFile, stepsFile, sourceLabel) {
    if (!manifestFile || !stepsFile) throw new Error("需要同时提供 manifest.json 和 steps.jsonl。");
    setStatus(`读取 ${sourceLabel || "trace"}…`);
    const manifest = JSON.parse(await manifestFile.text());
    const frames = parseSteps(await stepsFile.text());
    state.manifest = manifest;
    state.frames = frames;
    state.selectedSequence = frames[0].sequence;
    state.phase = "all";
    hideError();
    render();
    setStatus(`已载入 ${frames.length} 个结算边界`);
  }

  async function loadPending() {
    if (!state.pendingManifest || !state.pendingSteps) return;
    try {
      await readTraceFiles(state.pendingManifest, state.pendingSteps, "本地文件");
    } catch (error) {
      showError(error.message);
      setStatus("载入失败");
    }
  }

  function pickFiles(files) {
    const list = Array.from(files || []);
    const manifest = list.find((file) => file.name.toLowerCase() === "manifest.json");
    const steps = list.find((file) => ["steps.jsonl", "steps.ndjson"].includes(file.name.toLowerCase()));
    if (manifest) state.pendingManifest = manifest;
    if (steps) state.pendingSteps = steps;
    if (!manifest && !steps) {
      showError("没有找到 manifest.json 或 steps.jsonl。");
      return;
    }
    loadPending();
  }

  async function loadTracePath(tracePath, sourceLabel) {
    const base = new URL(tracePath, window.location.href);
    if (base.origin !== window.location.origin) {
      throw new Error("trace 路径必须与查看器来自同一来源。");
    }
    base.search = "";
    base.hash = "";
    if (!base.pathname.endsWith("/")) base.pathname += "/";
    const [manifestResponse, stepsResponse] = await Promise.all([
      fetch(new URL("manifest.json", base)),
      fetch(new URL("steps.jsonl", base)),
    ]);
    if (!manifestResponse.ok || !stepsResponse.ok) {
      throw new Error("trace 不存在；请确认模拟已完成且 trace 文件仍在输出目录。");
    }
    const manifest = new File([await manifestResponse.text()], "manifest.json", { type: "application/json" });
    const steps = new File([await stepsResponse.text()], "steps.jsonl", { type: "application/jsonl" });
    await readTraceFiles(manifest, steps, sourceLabel);
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
    $("metaStepsFile").textContent = manifest.steps_file || "steps.jsonl";
  }

  function renderFilters() {
    const phases = [...new Set(state.frames.map((frame) => frame.phase))];
    nodes.phaseFilter.innerHTML = `<option value="all">全部阶段</option>${phases.map((phase) => `<option value="${escapeHtml(phase)}">${escapeHtml(phaseLabel(phase))}</option>`).join("")}`;
    nodes.phaseFilter.value = state.phase;
    const visibleCount = visibleFrames().length;
    nodes.filterStatus.textContent = `${visibleCount} / ${state.frames.length} 个 frame 可见`;
  }

  function renderOverview() {
    const last = state.frames[state.frames.length - 1];
    const domainRevision = last?.domains
      .map((domain) => domain.payload)
      .map((payload) => isObject(payload) ? (payload.procedure_revision ?? payload.revision ?? payload.version ?? payload.schema_version) : null)
      .find((value) => value !== null && value !== undefined) ?? "—";
    $("statSteps").textContent = String(state.frames.length);
    $("statTime").textContent = last ? formatSimulationDate(last) : "—";
    $("statTime").title = last
      ? `公元日期以 ${historicalYearForFrame(last)}-01-01 00:00:00 为模拟起点；原始 SimTime=${last.at} 分钟`
      : "";
    $("statDomainRevision").textContent = String(domainRevision);
    $("statCheckpoint").textContent = shortHash(last?.checkpointHash || state.manifest?.final_checkpoint_hash);
    $("statCheckpoint").dataset.fullValue = last?.checkpointHash || state.manifest?.final_checkpoint_hash || "";
  }

  function visibleFrames() {
    return state.phase === "all" ? state.frames : state.frames.filter((frame) => frame.phase === state.phase);
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
    nodes.timelineCount.textContent = `${visible.length} / ${state.frames.length}`;
    nodes.timeline.innerHTML = visible.map((frame) => `
      <button class="timeline-row" type="button" data-sequence="${frame.sequence}" aria-pressed="${frame.sequence === state.selectedSequence}">
        <span class="timeline-index">${escapeHtml(frame.sequence)}</span>
        <span class="timeline-main">
          <span class="timeline-phase">${escapeHtml(frame.phaseLabel)}</span>
          <span class="timeline-time">${escapeHtml(formatSimulationTimeLabel(frame))} · boundary=${escapeHtml(frame.boundaryId)}</span>
        </span>
        <span class="timeline-delta">Δ ${frame.engine.changes} / R ${frame.engine.recordChanges}</span>
      </button>`).join("");
    nodes.timeline.querySelectorAll("[data-sequence]").forEach((button) => {
      button.addEventListener("click", () => {
        state.selectedSequence = numeric(button.dataset.sequence);
        renderTimeline();
        renderDetail();
      });
    });
  }

  function renderDetail() {
    const frame = state.frames.find((candidate) => candidate.sequence === state.selectedSequence) || state.frames[0];
    if (!frame) {
      nodes.detail.innerHTML = "";
      return;
    }
    const engine = frame.engine;
    nodes.detail.innerHTML = `
      <div class="detail-heading">
        <div>
          <span class="phase-mark">${escapeHtml(frame.phaseLabel)}</span>
          <h2>第 ${escapeHtml(frame.sequence)} 个结算边界</h2>
          <p><code>${escapeHtml(frame.boundaryId)}</code> · 公元时间 <code>${escapeHtml(formatSimulationDate(frame))}</code> · SimTime <code>${escapeHtml(frame.at)} 分钟</code> · revision <code>${escapeHtml(frame.revision)}</code></p>
        </div>
        <div class="detail-actions">
          <button id="copyFrameButton" class="button button-secondary" type="button">复制 frame JSON</button>
          <button id="downloadFrameButton" class="button button-secondary" type="button">下载 frame</button>
        </div>
      </div>
      <section class="detail-section">
        <h3>引擎结算摘要</h3>
        <div class="metric-grid">
          ${metric("changes", engine.changes)}
          ${metric("record changes", engine.recordChanges)}
          ${metric("knowledge records", engine.knowledge)}
          ${metric("events", engine.events)}
          ${metric("ingress", engine.ingress)}
          ${metric("allocations", engine.allocations)}
          ${metric("random draws", engine.randomDraws)}
          ${metric("revision", frame.revision)}
        </div>
      </section>
      <section class="detail-section">
        <h3>检测到的领域</h3>
        <div class="domain-list">${frame.domains.length ? frame.domains.map(renderDomain).join("") : '<p class="muted">这一 frame 没有附带可识别的 domain snapshot。</p>'}</div>
      </section>
      <section class="detail-section">
        <h3>结算证据</h3>
        ${renderEvidence(frame)}
      </section>
      <section class="detail-section">
        <h3>原始 frame</h3>
        <details open><summary>JSON</summary><pre>${escapeHtml(JSON.stringify(frame.raw, null, 2))}</pre></details>
      </section>`;
    $("copyFrameButton").addEventListener("click", () => copyText(JSON.stringify(frame.raw, null, 2), "frame JSON 已复制"));
    $("downloadFrameButton").addEventListener("click", () => downloadText(`frame-${frame.sequence}.json`, JSON.stringify(frame.raw, null, 2), "application/json"));
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
    return `<article class="domain-block"><div class="domain-title"><strong>fiscal</strong><span>canwu-fiscal</span></div>${countRows ? `<table class="count-table"><tbody>${countRows}</tbody></table>` : ""}${projections.length ? `<div class="projection-list">${projections.map(([key, value]) => `<div class="projection-row"><code>${escapeHtml(key)}</code><span><strong>${escapeHtml(value.confidence_per_mille ?? "—")}</strong>‰ · ${Object.keys(value.facts || {}).length} facts</span></div>`).join("")}</div>` : ""}<details><summary>Fiscal state JSON</summary><pre>${escapeHtml(JSON.stringify(payload.state || payload, null, 2))}</pre></details></article>`;
  }

  function renderEvidence(frame) {
    const boundary = frame.boundary || {};
    const entries = [
      ["admitted events", boundary.admitted_events],
      ["generated ingress", boundary.generated_ingress],
      ["record changes", boundary.record_changes],
      ["knowledge changes", boundary.knowledge_changes],
      ["emissions", boundary.emissions],
      ["random draws", boundary.random_draws],
    ].filter(([, value]) => Array.isArray(value));
    return `<div class="evidence-grid">${entries.length ? entries.map(([label, value]) => `<details><summary>${escapeHtml(label)} · ${value.length}</summary><div class="evidence-list">${value.slice(0, 40).map((item) => `<div class="evidence-item">${escapeHtml(compact(item, 420))}</div>`).join("")}${value.length > 40 ? `<div class="muted">仅显示前 40 条；完整内容见原始 frame。</div>` : ""}</div></details>`).join("") : '<p class="muted">没有可展开的 boundary 数组；请查看原始 frame。</p>'}</div>`;
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
  nodes.manifestInput.addEventListener("change", (event) => { state.pendingManifest = event.target.files[0]; loadPending(); });
  nodes.stepsInput.addEventListener("change", (event) => { state.pendingSteps = event.target.files[0]; loadPending(); });
  nodes.folderInput.addEventListener("change", (event) => pickFiles(event.target.files));
  nodes.phaseFilter.addEventListener("change", (event) => {
    state.phase = event.target.value;
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
