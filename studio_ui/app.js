"use strict";
const $ = (selector) => document.querySelector(selector);
const icon = (name) => `<svg aria-hidden="true"><use href="#i-${name}"/></svg>`;
const editor = $("#script-input");
const token = $("meta[name=studio-token]").content;
let analysis = null,
  activeJob = null,
  selectedJob = null,
  pollTimer = null,
  analyzeTimer = null;
let pendingReplace = null,
  tourStep = 0,
  draftLoaded = false,
  analyzeVersion = 0;
let tourTarget = null,
  tourReturnFocus = null,
  tourFrame = null,
  editorFrame = null,
  synchronizingEditor = false,
  draftEdited = false,
  cancelRequested = false,
  trackingVersion = 0,
  historyVersion = 0;
let preferenceCache = {},
  preferenceChanges = {},
  preferenceTimer,
  preferenceRequest = null,
  preferencesReady = false,
  preferenceRetries = 0;
const pendingPreferencesKey = "studio-pending-preferences";
const preferenceKeys = new Set([
  "manim-draft",
  "manim-name",
  "manim-quality",
  "manim-resolution",
  "manim-fps",
  "manim-tour-seen",
  "manim-theme",
  "manim-check-updates",
]);
try {
  const pending = JSON.parse(
    localStorage.getItem(pendingPreferencesKey) || "{}",
  );
  // Python counts Unicode code points; browser string lengths count UTF-16 units.
  for (const [key, value] of Object.entries(pending || {}))
    if (
      preferenceKeys.has(key) &&
      typeof value === "string" &&
      (value.length <= 500000 ||
        (value.length <= 1000000 && Array.from(value).length <= 500000))
    )
      preferenceChanges[key] = value;
  preferenceCache = { ...preferenceChanges };
} catch {}
function journalPreferences() {
  try {
    if (Object.keys(preferenceChanges).length)
      localStorage.setItem(
        pendingPreferencesKey,
        JSON.stringify(preferenceChanges),
      );
    else localStorage.removeItem(pendingPreferencesKey);
  } catch {
    // The in-memory queue still retries when browser storage is unavailable.
  }
}
async function persistPreferences(unloading = false) {
  clearTimeout(preferenceTimer);
  if (
    !preferencesReady ||
    preferenceRequest ||
    !Object.keys(preferenceChanges).length
  )
    return preferenceRequest;
  const changes = { ...preferenceChanges };
  const body = JSON.stringify({ values: changes });
  // Chromium limits keepalive bodies to 64 KiB; larger drafts stay journaled on exit.
  if (unloading && new TextEncoder().encode(body).length > 60000) return;
  preferenceRequest = (async () => {
    try {
      const response = await fetch("/api/preferences", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "X-Studio-Token": token,
        },
        body,
        keepalive: unloading,
      });
      if (!response.ok) throw new Error("Preferences could not be saved");
      for (const [key, value] of Object.entries(changes))
        if (preferenceChanges[key] === value) delete preferenceChanges[key];
      preferenceRetries = 0;
      journalPreferences();
    } catch {
      preferenceRetries++;
      if (preferenceRetries === 1)
        toast(
          "Couldn’t save your session. Studio will retry. Save a script copy if you need to close.",
        );
    } finally {
      preferenceRequest = null;
      // Stop automatic retries after three failures; another edit or reload retries.
      if (Object.keys(preferenceChanges).length && preferenceRetries < 3)
        preferenceTimer = setTimeout(
          persistPreferences,
          preferenceRetries ? 1000 * 2 ** preferenceRetries : 250,
        );
    }
  })();
  return preferenceRequest;
}
const storage = {
  get(key) {
    if (Object.hasOwn(preferenceCache, key)) return preferenceCache[key];
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  },
  set(key, value) {
    if (this.get(key) === value) return;
    preferenceCache[key] = value;
    preferenceChanges[key] = value;
    preferenceRetries = 0;
    try {
      localStorage.setItem(key, value);
    } catch {}
    journalPreferences();
    clearTimeout(preferenceTimer);
    if (key === "manim-tour-seen") persistPreferences();
    else preferenceTimer = setTimeout(persistPreferences, 250);
  },
};
const legacyOutputSettings = {
  preview: { width: 854, height: 480, fps: 15 },
  standard: { width: 1280, height: 720, fps: 30 },
  final: { width: 1920, height: 1080, fps: 60 },
};
let outputSettings = { ...legacyOutputSettings.preview },
  outputHistory = [];
function validResolution(width, height) {
  return [width, height].every(
    (value) =>
      Number.isInteger(value) &&
      value >= 2 &&
      value <= 32766 &&
      value % 2 === 0,
  );
}
function resolutionValue({ width, height }) {
  return `${width}x${height}`;
}
function sameOutputSettings(a, b) {
  return a.width === b.width && a.height === b.height && a.fps === b.fps;
}
function parseResolution(value) {
  const match = String(value || "").match(/^(\d+)x(\d+)$/);
  if (!match) return null;
  const width = Number(match[1]),
    height = Number(match[2]);
  return validResolution(width, height) ? { width, height } : null;
}
function outputQuality(settings) {
  return (
    Object.keys(legacyOutputSettings).find((key) => {
      const preset = legacyOutputSettings[key];
      return (
        settings.width === preset.width &&
        settings.height === preset.height &&
        settings.fps === preset.fps
      );
    }) || "standard"
  );
}
function formatFrameRate(value) {
  return Number(Number(value).toPrecision(10)).toString();
}
function jobOutputSettings(job) {
  const preset =
    legacyOutputSettings[job.quality] || legacyOutputSettings.standard;
  const dimensions =
    job.width && job.height
      ? { width: Number(job.width), height: Number(job.height) }
      : parseResolution(String(job.resolution).replace(/\s*[×x]\s*/, "x"));
  return { ...(dimensions || preset), fps: Number(job.fps) || preset.fps };
}
function isHighQuality(job) {
  const settings = jobOutputSettings(job);
  return (
    settings.width === 1920 && settings.height === 1080 && settings.fps === 60
  );
}
function selectOutputValue(selector, value, label) {
  const select = $(selector),
    oldCustom = select.querySelector("option[data-custom]");
  if (oldCustom && oldCustom.value !== value) oldCustom.remove();
  if (![...select.options].some((option) => option.value === value)) {
    const option = new Option(label, value);
    option.dataset.custom = "true";
    select.add(option, select.querySelector('option[value="custom"]'));
  }
  select.value = value;
}
function updateOutputEstimate() {
  const samples = outputHistory
    .filter((job) => {
      const settings = jobOutputSettings(job);
      return (
        settings.width === outputSettings.width &&
        settings.height === outputSettings.height &&
        settings.fps === outputSettings.fps &&
        job.elapsed > 0
      );
    })
    .slice(0, 8)
    .map((job) => job.elapsed)
    .sort((a, b) => a - b);
  $("#render-estimate").textContent = samples.length
    ? `About ${Math.max(1, Math.round(samples[Math.floor(samples.length / 2)]))}s based on recent renders with these settings. Script complexity can change this estimate.`
    : "Render time depends on your script and computer. No recent renders use these settings.";
}
function setOutputSettings(settings, persist = true) {
  if (
    !validResolution(settings.width, settings.height) ||
    !Number.isFinite(settings.fps) ||
    settings.fps <= 0
  )
    return false;
  outputSettings = { ...settings };
  selectOutputValue(
    "#resolution-select",
    resolutionValue(settings),
    `${settings.width} × ${settings.height} · Custom`,
  );
  selectOutputValue(
    "#fps-select",
    String(settings.fps),
    `${formatFrameRate(settings.fps)} fps · Custom`,
  );
  $("#resolution-select").title =
    `${settings.width} × ${settings.height} pixels`;
  $("#fps-select").title = `${formatFrameRate(settings.fps)} frames per second`;
  if (persist) {
    storage.set("manim-resolution", resolutionValue(settings));
    storage.set("manim-fps", String(settings.fps));
    storage.set("manim-quality", outputQuality(settings));
  }
  updateOutputEstimate();
  return true;
}
function restoreOutputSettings() {
  const preset =
    legacyOutputSettings[storage.get("manim-quality")] ||
    legacyOutputSettings.preview;
  const dimensions = parseResolution(storage.get("manim-resolution")) || preset;
  const fps = Number(storage.get("manim-fps"));
  setOutputSettings({
    width: dimensions.width,
    height: dimensions.height,
    fps: Number.isFinite(fps) && fps > 0 ? fps : preset.fps,
  });
}
const EXAMPLES = [
  {
    title: "A change of shape",
    detail: "Watch a circle become a square. Your first little animation.",
    meta: "About 4 seconds · No extra setup",
    art: "◯ → □",
    source: `from manim import *

class MainScene(Scene):
    def construct(self):
        self.camera.background_color = "#101012"
        circle = Circle(color="#a7bbff", radius=1.3)
        circle.set_fill("#a7bbff", opacity=0.15)
        square = Square(color="#c6adef", side_length=2.6)
        square.set_fill("#c6adef", opacity=0.15)

        self.play(Create(circle), run_time=1.2)
        self.play(Transform(circle, square), run_time=1.5)
        self.play(circle.animate.rotate(PI / 4), run_time=1)
        self.wait(0.8)
`,
  },
  {
    title: "Words in motion",
    detail: "A title, a gentle reveal, and a perfectly timed exit.",
    meta: "About 5 seconds · No extra setup",
    art: "Aa",
    source: `from manim import *

class MainScene(Scene):
    def construct(self):
        self.camera.background_color = "#101012"
        title = Text("Make something move.", font_size=44,
                     color="#a7bbff")
        subtitle = Text("A little code. A lot of possibility.",
                        font_size=24, color="#c6adef")
        subtitle.next_to(title, DOWN, buff=0.5)

        self.play(Write(title), run_time=1.5)
        self.play(FadeIn(subtitle, shift=UP * 0.2))
        self.wait(1.5)
        self.play(FadeOut(title), FadeOut(subtitle))
`,
  },
  {
    title: "Follow the curve",
    detail: "Build a coordinate plane and trace a smooth sine wave.",
    meta: "About 5 seconds · No LaTeX needed",
    art: "∿",
    source: `from manim import *
import numpy as np

class MainScene(Scene):
    def construct(self):
        self.camera.background_color = "#101012"
        axes = Axes(x_range=[-PI, PI, PI / 2],
                    y_range=[-1.5, 1.5, 0.5],
                    x_length=9, y_length=4,
                    axis_config={"color": "#8fa69b"})
        curve = axes.plot(lambda x: np.sin(x),
                          color="#a7bbff")
        label = Text("A little rhythm.", font_size=28,
                     color="#c6adef").to_edge(UP)
        self.play(Create(axes), FadeIn(label))
        self.play(Create(curve), run_time=2.5)
        self.wait(1)
`,
  },
];
const PROMPT = `Write a complete Python animation script for Manim Community Edition 0.21.0 using the Cairo renderer. Include "from manim import *" and a scene class named MainScene with a construct() method. Use Text for ordinary text; use Tex or MathTex only when mathematical typesetting is needed (typesetting is included in Studio). Use relative paths under assets/ for images or audio. Include self.play() or self.wait() so the scene produces a video. Return the full Python script, ready to paste into Manim Studio. My animation idea is: `;

async function api(path, data) {
  const response = await fetch(path, {
    method: data === undefined ? "GET" : "POST",
    headers:
      data === undefined
        ? {}
        : { "Content-Type": "application/json", "X-Studio-Token": token },
    body: data === undefined ? undefined : JSON.stringify(data),
  });
  if (!response.ok) {
    let message = "Studio couldn't complete that action. Please try again.";
    try {
      const body = await response.json();
      if (typeof body.detail === "string") message = body.detail;
      else if (Array.isArray(body.detail)) {
        const labels = { width: "Width", height: "Height", fps: "Frame rate" };
        const details = body.detail
          .filter((item) => typeof item?.msg === "string")
          .slice(0, 5)
          .map((item) => {
            const field = Array.isArray(item.loc)
              ? labels[item.loc.at(-1)]
              : null;
            const detail = item.msg.replace(/^Value error, /, "");
            return field ? `${field}: ${detail}` : detail;
          });
        if (details.length) message = details.join(" ");
      }
    } catch {
      /* Handle non-JSON errors. */
    }
    throw new Error(message);
  }
  return response.json();
}
let toastTimer;
function toast(message) {
  $("#toast").textContent = message;
  $("#toast").hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => ($("#toast").hidden = true), 4500);
}
async function copy(text, message) {
  try {
    await navigator.clipboard.writeText(text);
    toast(message);
  } catch {
    const temporary = document.createElement("textarea");
    temporary.value = text;
    temporary.style.position = "fixed";
    temporary.style.opacity = "0";
    document.body.append(temporary);
    temporary.select();
    const ok = document.execCommand("copy");
    temporary.remove();
    toast(
      ok
        ? message
        : "Clipboard unavailable. Select and copy the text manually.",
    );
  }
}
function escapeHtml(value) {
  return String(value).replace(
    /[&<>"']/g,
    (char) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        char
      ],
  );
}
function updateEditor() {
  const source = editor.value;
  if (window.codeEditor && window.codeEditor.getSource() !== source) {
    synchronizingEditor = true;
    try {
      window.codeEditor.setSource(source);
    } finally {
      synchronizingEditor = false;
    }
  }
  const lines = source.split("\n").length;
  $("#line-count").textContent = `${lines} ${lines === 1 ? "line" : "lines"}`;
  $("#editor-empty").hidden = !!source.trim() || !!window.starterHelpDismissed;
  $("#format-script").disabled = !!window.updatingStudio || !source.trim();
  $("#copy-script").disabled = !source;
  if (preferencesReady) storage.set("manim-draft", source);
}
function syncScroll() {}
function sourceLineOffset(source) {
  const blankLines = (source.match(/^\s*/)?.[0] || "").split("\n").length - 1;
  return blankLines + (source.trim().startsWith("```") ? 1 : 0);
}
function invalidateAnalysis() {
  analysis = null;
  analyzeVersion++;
  window.renderOutcome = "idle";
  $("#script-status").className = "";
  $("#script-status").textContent = editor.value.trim()
    ? "Validating…"
    : "Add a script to begin";
  window.scriptProblems = [];
  $("#error-card").hidden = true;
  $("#validation-message").hidden = true;
  updateRenderButton();
  window.refreshProblems?.();
  window.syncSceneControl?.();
}
async function checkScript() {
  clearTimeout(analyzeTimer);
  const version = ++analyzeVersion;
  if (!editor.value.trim()) {
    analysis = null;
    $("#scene-select").innerHTML =
      "<option>Add a script to detect scenes</option>";
    $("#scene-select").disabled = true;
    window.syncSceneControl?.();
    $("#validation-message").hidden = true;
    $("#script-status").textContent = "Add a script to begin";
    $("#script-status").className = "";
    updateRenderButton();
    return;
  }
  try {
    const result = await api("/api/analyze", { source: editor.value });
    if (version !== analyzeVersion) return;
    analysis = result;
    window.scriptProblems = result.error
      ? [
          {
            line: result.line
              ? result.line + sourceLineOffset(editor.value)
              : null,
            message: result.error.replace(/^Line \d+: /, ""),
            severity: "error",
          },
        ]
      : [];
    window.refreshProblems?.();
    const previous = $("#scene-select").value;
    $("#scene-select").replaceChildren();
    for (const scene of result.scenes) {
      const option = document.createElement("option");
      option.value = scene;
      option.textContent = scene;
      $("#scene-select").append(option);
    }
    if (result.scenes.includes(previous)) $("#scene-select").value = previous;
    else if (result.scenes.includes("MainScene"))
      $("#scene-select").value = "MainScene";
    if (!result.scenes.length) {
      const option = document.createElement("option");
      option.textContent = "No scene detected";
      $("#scene-select").append(option);
    }
    $("#scene-select").disabled = !result.scenes.length || !!activeJob;
    $("#script-status").textContent = result.error
      ? "Script needs a quick fix"
      : `${result.scenes.length} ${result.scenes.length === 1 ? "scene" : "scenes"} detected · Ready`;
    $("#script-status").className = result.error ? "invalid" : "valid";
    $("#validation-message").hidden = true;
    $("#validation-message").textContent = result.error || "";
    updateRenderButton();
  } catch (error) {
    if (version !== analyzeVersion) return;
    analysis = null;
    window.scriptProblems = [{ message: error.message, severity: "error" }];
    window.refreshProblems?.();
    $("#script-status").textContent = "Couldn’t check script · Edit to retry";
    $("#validation-message").hidden = true;
    updateRenderButton();
  }
}
function updateRenderButton() {
  const trackedJob =
    selectedJob?.id === activeJob?.id ? selectedJob : activeJob;
  const finishing = trackedJob?.can_cancel === false;
  const cancelling = cancelRequested || trackedJob?.status === "cancelling";
  const validating = !analysis && !!editor.value.trim();
  const blocked =
    !!analysis?.needs_latex && window.engineHealth?.latex === false;
  const state = activeJob
    ? "rendering"
    : window.updatingStudio
      ? "updating"
      : analysis?.error ||
          blocked ||
          (!analysis && window.scriptProblems?.length)
        ? "error"
        : validating
          ? "validating"
          : window.renderOutcome || "idle";
  $("#render-button").disabled = activeJob
    ? !activeJob.id || finishing || cancelling
    : !!window.updatingStudio || !analysis || !!analysis.error || blocked;
  $("#render-button").dataset.state = state;
  $("#render-button span").textContent =
    state === "rendering"
      ? finishing
        ? "Finishing…"
        : cancelling
          ? "Cancelling…"
          : "Cancel"
      : state === "updating"
        ? "Updating Studio…"
        : state === "validating"
          ? "Validating…"
          : state === "success"
            ? "Render again"
            : state === "error" && analysis && !analysis.error && !blocked
              ? "Retry render"
              : "Render animation";
  $("#render-button use").setAttribute(
    "href",
    activeJob ? "#i-close" : "#i-play",
  );
  $("#render-button kbd").hidden = !!activeJob;
  $("#resolution-select").disabled = !!activeJob || !!window.updatingStudio;
  $("#fps-select").disabled = !!activeJob || !!window.updatingStudio;
  $("#scene-select").disabled =
    !analysis?.scenes.length || !!activeJob || !!window.updatingStudio;
  editor.readOnly = !!window.updatingStudio;
  window.codeEditor?.setReadOnly(!!window.updatingStudio);
  for (const selector of [
    "#animation-name",
    "#paste-button",
    "#import-button",
    "#file-input",
    "#examples-button",
    "#theme-button",
  ])
    $(selector).disabled = !!window.updatingStudio;
  $("#format-script").disabled =
    !!window.updatingStudio || !editor.value.trim();
  for (const reuse of document.querySelectorAll(".history-card button"))
    reuse.disabled = !!window.updatingStudio;
  window.syncSceneControl?.();
  window.refreshUpdateControls?.();
  if ($("#higher-quality"))
    $("#higher-quality").disabled =
      !!activeJob ||
      !!window.updatingStudio ||
      !!window.higherQualityPending ||
      (!!window.lastCompletedJob && isHighQuality(window.lastCompletedJob));
}
function normalizeSource(source) {
  // Match textarea line endings for imported Windows files and persisted scripts.
  return source.replace(/\r\n?/g, "\n");
}
function setSource(source, name) {
  if (window.updatingStudio) return;
  source = normalizeSource(source);
  draftEdited = true;
  editor.value = source;
  if (name) {
    $("#animation-name").value = name;
    storage.set("manim-name", name);
  }
  $("#script-filename").textContent = "animation.py";
  invalidateAnalysis();
  updateEditor();
  return checkScript();
}
function replaceSource(source, name, previewJob = null) {
  if (window.updatingStudio) return;
  if (document.querySelector("dialog[open]")) {
    toast("Close the current dialog before replacing your script.");
    return;
  }
  source = normalizeSource(source);
  if (editor.value.trim() && editor.value.trim() !== source.trim()) {
    pendingReplace = { source, name, previewJob };
    $("#confirm-dialog").showModal();
  } else {
    setSource(source, name);
    if (previewJob) showVideo(previewJob);
  }
}
function downloadDraft() {
  const blob = new Blob([editor.value], { type: "text/x-python" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "animation.py";
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
function showView(view) {
  if (view !== "workspace") closeTour(false);
  $("#workspace-view").hidden = view !== "workspace";
  $("#history-view").hidden = view !== "history";
  document.querySelectorAll("[data-view]").forEach((button) => {
    button.classList.toggle("active", button.dataset.view === view);
    if (button.dataset.view === view)
      button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  $("#breadcrumb-current").textContent =
    view === "history" ? "My renders" : "Create animation";
  if (view === "history") loadHistory();
}
async function openFolder(folder) {
  try {
    await api(`/api/open/${folder}`, {});
  } catch (error) {
    toast(error.message);
  }
}
function formatSize(bytes) {
  return bytes >= 1048576
    ? `${(bytes / 1048576).toFixed(1)} MB`
    : `${Math.round(bytes / 1024)} KB`;
}
function showVideo(job) {
  window.lastCompletedJob = job;
  selectedJob = job;
  $("#preview-placeholder").hidden = true;
  $("#video-player").hidden = false;
  $("#video-player").src = job.video;
  $("#video-player").playbackRate = Number($("#playback-speed").value);
  $("#video-caption").textContent = job.name;
  window.updateVideoMetadata?.(job);
  $("#download-video").href = job.video + "?download=true";
  $("#video-actions").hidden = false;
  $("#preview-footer").hidden = false;
  $("#playback-options").hidden = false;
  const settings = jobOutputSettings(job);
  $("#preview-badge").textContent =
    `${settings.width} × ${settings.height} · ${formatFrameRate(settings.fps)} fps`;
  $("#higher-quality").disabled = isHighQuality(job) || !!activeJob;
}
async function startRender() {
  if (
    document.querySelector("dialog[open]") ||
    window.updatingStudio ||
    activeJob ||
    !analysis ||
    analysis.error ||
    (analysis.needs_latex && window.engineHealth?.latex === false)
  )
    return;
  closeTour(false);
  // Reserve the UI immediately so repeated shortcuts cannot issue duplicate renders.
  activeJob = { id: null };
  cancelRequested = false;
  trackingVersion++;
  updateRenderButton();
  $("#error-card").hidden = true;
  $("#validation-message").hidden = true;
  window.scriptProblems = [];
  window.refreshProblems?.();
  try {
    $("#video-player").pause();
    window.renderedSource = editor.value;
    window.renderLineOffset = sourceLineOffset(editor.value);
    const job = await api("/api/render", {
      source: editor.value,
      scene: $("#scene-select").value,
      quality: outputQuality(outputSettings),
      width: outputSettings.width,
      height: outputSettings.height,
      fps: outputSettings.fps,
      name: $("#animation-name").value.trim() || $("#scene-select").value,
    });
    beginTracking(job);
  } catch (error) {
    activeJob = null;
    window.renderOutcome = "error";
    window.scriptProblems = [{ message: error.message, severity: "error" }];
    window.refreshProblems?.();
    $("#validation-message").textContent = error.message;
    $("#validation-message").hidden = false;
    updateRenderButton();
  }
}
function beginTracking(job) {
  trackingVersion++;
  cancelRequested = false;
  activeJob = job;
  selectedJob = job;
  $("#render-overlay").hidden = false;
  $("#log-panel").hidden = false;
  $("#video-actions").hidden = true;
  $("#error-card").hidden = true;
  $("#scene-select").disabled = true;
  window.syncSceneControl?.();
  $("#render-stage").textContent = "Preparing render…";
  window.prepareRenderUI?.(job);
  $("#cancel-button").disabled = false;
  $("#cancel-button").textContent = "Cancel render";
  updateRenderButton();
  clearTimeout(pollTimer);
  pollJob();
}
async function pollJob() {
  if (!activeJob?.id) return;
  const trackedJob = activeJob;
  try {
    const job = await api(`/api/renders/${trackedJob.id}`);
    if (activeJob !== trackedJob) return;
    selectedJob = job;
    const log = $("#render-log"),
      drawer = $("#drawer-body"),
      atEnd = drawer.scrollHeight - drawer.scrollTop - drawer.clientHeight < 50;
    const logText = job.log || "Preparing the render engine…";
    if (log.textContent !== logText) {
      log.textContent = logText;
      if (atEnd) drawer.scrollTop = drawer.scrollHeight;
    }
    window.updateProgress?.(job);
    const canCancel = job.can_cancel !== false;
    $("#cancel-button").disabled =
      !canCancel || cancelRequested || job.status === "cancelling";
    $("#cancel-button").textContent = !canCancel
      ? "Finishing render…"
      : cancelRequested || job.status === "cancelling"
        ? "Cancelling…"
        : "Cancel render";
    updateRenderButton();
    if (["completed", "failed", "cancelled"].includes(job.status)) {
      activeJob = null;
      window.renderOutcome =
        job.status === "completed"
          ? "success"
          : job.status === "failed"
            ? "error"
            : "idle";
      $("#render-overlay").hidden = true;
      $("#scene-select").disabled = !analysis?.scenes.length;
      updateRenderButton();
      $("#log-summary").innerHTML =
        `${job.status === "completed" ? "Completed" : job.status === "cancelled" ? "Cancelled" : "Error details"} <span>⌄</span>`;
      if (job.status === "completed") {
        showVideo(job);
        toast("Render complete. Saved in My renders.");
        loadHistory();
      } else if (job.status === "failed") {
        $("#error-card").hidden = false;
        $("#error-text").textContent = job.error;
        window.showRenderProblems?.(job);
        window.restoreCompletedPreview?.();
      } else {
        window.restoreCompletedPreview?.();
        toast("Render cancelled. Your script is still here.");
      }
      return;
    }
    $("#log-summary").innerHTML = "Live log <span>⌄</span>";
  } catch {
    if (activeJob !== trackedJob) return;
    $("#render-elapsed").textContent =
      "Connection interrupted. Reconnecting to your render…";
  }
  pollTimer = setTimeout(pollJob, 800);
}
async function loadHistory() {
  const version = ++historyVersion;
  try {
    const jobs = (await api("/api/renders")).filter(
      (job) => job.status === "completed",
    );
    if (version !== historyVersion) return jobs;
    $("#history-count").textContent = jobs.length;
    $("#history-count").hidden = jobs.length === 0;
    window.updateEstimates?.(jobs);
    $("#history-empty").hidden = jobs.length > 0;
    if ($("#history-view").hidden) return jobs;
    const list = $("#history-list");
    list.replaceChildren();
    for (const job of jobs) {
      const card = document.createElement("article");
      card.className = "history-card";
      card.innerHTML = `<video controls preload="none" playsinline aria-label="${escapeHtml(job.name)}" src="${escapeHtml(job.video)}"></video><div class="history-card-body"><h2>${escapeHtml(job.name)}</h2><p>${escapeHtml(job.resolution)} · ${escapeHtml(job.fps)} fps · ${escapeHtml(formatSize(job.size))} <br>${escapeHtml(new Date(job.created).toLocaleString())}</p><div class="history-card-actions"><a class="button primary" href="${escapeHtml(job.video)}?download=true" download>${icon("download")} Save video</a><button class="button secondary">${icon("code")} Reuse script</button></div></div>`;
      const reuse = card.querySelector("button");
      reuse.disabled = !!window.updatingStudio;
      reuse.addEventListener("click", async () => {
        if (activeJob || window.updatingStudio) {
          toast("Wait for the current render before reusing a script.");
          return;
        }
        const originalSource = editor.value;
        reuse.disabled = true;
        try {
          const response = await fetch(`/api/renders/${job.id}/script`);
          if (!response.ok) throw new Error("Couldn’t load this script.");
          const source = await response.text();
          if (
            activeJob ||
            window.updatingStudio ||
            editor.value !== originalSource
          ) {
            toast(
              "The workspace changed. Select Reuse script again when you’re ready.",
            );
            return;
          }
          showView("workspace");
          replaceSource(source, job.name, job);
        } catch (error) {
          toast(error.message);
        } finally {
          reuse.disabled = !!window.updatingStudio;
        }
      });
      list.append(card);
    }
    return jobs;
  } catch (error) {
    if (version !== historyVersion) return [];
    toast(error.message);
    return [];
  }
}
const TOUR = [
  {
    title: "Paste your script",
    text: "Copy the full Manim code from your AI, then click Paste. Import file opens a saved .py script. You can use these buttons while the tour is open.",
    target: "#paste-button",
  },
  {
    title: "Try an example",
    text: "No script yet? This button opens three ready-to-render examples. Loading one asks before replacing your draft.",
    target: "#examples-button",
  },
  {
    title: "Name your animation",
    text: "Give your animation a name to find it in My renders. If your script contains multiple scenes, a Scene chooser appears; each render makes a video of the selected scene.",
    target: ".name-field",
  },
  {
    title: "Choose your output",
    text: "Choose resolution and frame rate independently. Smaller dimensions and fewer frames render faster. Select Custom for another size or frame rate.",
    target: ".resolution-options",
  },
  {
    title: "Render your animation",
    text: "Click here when your script is ready, or press Ctrl+Enter. The preview shows progress and a Cancel button while rendering.",
    target: "#render-button",
  },
  {
    title: "Watch and save",
    text: "Play your video here, change playback speed or loop it, then Save video to export the MP4. Every completed render is saved in My renders.",
    target: "#preview-heading",
  },
  {
    title: "Find previous renders",
    text: "Every completed video and its script are kept here. Reuse a script whenever you want to make another version. Replay this tour from Help & quick tour.",
    target: "[data-view=history]",
  },
];
function clearTourTarget() {
  if (!tourTarget) return;
  tourTarget.classList.remove("tour-focus");
  const ids = (tourTarget.getAttribute("aria-describedby") || "")
    .split(" ")
    .filter((id) => id && id !== "tour-description");
  if (ids.length) tourTarget.setAttribute("aria-describedby", ids.join(" "));
  else tourTarget.removeAttribute("aria-describedby");
  tourTarget = null;
}
function positionTour() {
  const popup = $("#tour-popover");
  if (popup.hidden || !tourTarget) return;
  const rect = tourTarget.getBoundingClientRect(),
    w = popup.offsetWidth,
    h = popup.offsetHeight;
  const gap = 14,
    edge = 12,
    centerX = rect.left + rect.width / 2,
    centerY = rect.top + rect.height / 2;
  let side, x, y;
  if (rect.bottom + gap + h <= innerHeight - edge) {
    side = "bottom";
    x = centerX - w / 2;
    y = rect.bottom + gap;
  } else if (rect.top - gap - h >= edge) {
    side = "top";
    x = centerX - w / 2;
    y = rect.top - gap - h;
  } else if (rect.right + gap + w <= innerWidth - edge) {
    side = "right";
    x = rect.right + gap;
    y = centerY - h / 2;
  } else {
    side = "left";
    x = rect.left - gap - w;
    y = centerY - h / 2;
  }
  x = Math.max(edge, Math.min(x, innerWidth - w - edge));
  y = Math.max(edge, Math.min(y, innerHeight - h - edge));
  popup.style.left = `${Math.round(x)}px`;
  popup.style.top = `${Math.round(y)}px`;
  popup.dataset.side = side;
  popup.style.setProperty(
    "--arrow-offset",
    `${Math.max(20, Math.min(side === "top" || side === "bottom" ? centerX - x : centerY - y, (side === "top" || side === "bottom" ? w : h) - 20))}px`,
  );
  popup.classList.toggle(
    "offscreen-target",
    rect.bottom < 0 ||
      rect.top > innerHeight ||
      rect.right < 0 ||
      rect.left > innerWidth,
  );
}
function queueTourPosition() {
  if (!tourFrame && !$("#tour-popover").hidden)
    tourFrame = requestAnimationFrame(() => {
      tourFrame = null;
      positionTour();
    });
}
function renderTour() {
  clearTourTarget();
  const step = TOUR[tourStep],
    popup = $("#tour-popover");
  $("#tour-title").textContent = step.title;
  $("#tour-description").textContent = step.text;
  $("#tour-progress").textContent =
    `Quick tour · ${tourStep + 1} / ${TOUR.length}`;
  $("#tour-back").disabled = tourStep === 0;
  $("#tour-next").innerHTML =
    `${tourStep === TOUR.length - 1 ? "Done" : "Next"} ${icon("arrow")}`;
  tourTarget = $(step.target);
  tourTarget.classList.add("tour-focus");
  tourTarget.setAttribute(
    "aria-describedby",
    `${tourTarget.getAttribute("aria-describedby") || ""} tour-description`.trim(),
  );
  popup.hidden = false;
  const rect = tourTarget.getBoundingClientRect();
  if (rect.top < 12 || rect.bottom > innerHeight - 12)
    tourTarget.scrollIntoView({ block: "center", behavior: "instant" });
  positionTour();
}
function closeTour(restoreFocus = true) {
  if ($("#tour-popover").hidden) return;
  $("#tour-popover").hidden = true;
  clearTourTarget();
  storage.set("manim-tour-seen", "2");
  if (restoreFocus)
    (tourReturnFocus?.isConnected
      ? tourReturnFocus
      : $(".cm-content") || editor
    ).focus({ preventScroll: true });
}
function openTour(focus = true) {
  tourReturnFocus = document.activeElement;
  showView("workspace");
  tourStep = 0;
  renderTour();
  if (focus) $("#tour-next").focus({ preventScroll: true });
}
function openExamples() {
  if (window.updatingStudio) return;
  closeTour(false);
  $("#examples-dialog").showModal();
}
addEventListener("resize", queueTourPosition);
addEventListener("scroll", queueTourPosition, true);
new ResizeObserver(queueTourPosition).observe($("#tour-popover"));
document.fonts.ready.then(queueTourPosition);
document.addEventListener("keydown", (event) => {
  if (!$("#tour-popover").hidden && event.key === "Escape") {
    event.preventDefault();
    closeTour();
  }
});

editor.addEventListener("input", () => {
  if (synchronizingEditor) return;
  draftEdited = true;
  invalidateAnalysis();
  if (!editorFrame)
    editorFrame = requestAnimationFrame(() => {
      editorFrame = null;
      updateEditor();
    });
  clearTimeout(analyzeTimer);
  analyzeTimer = setTimeout(checkScript, 400);
});
editor.addEventListener("scroll", syncScroll);
editor.addEventListener("keydown", (event) => {
  if (event.key === "Tab" && !event.shiftKey) {
    event.preventDefault();
    const start = editor.selectionStart,
      end = editor.selectionEnd;
    editor.setRangeText("    ", start, end, "end");
    editor.dispatchEvent(new Event("input"));
  }
});
$("#animation-name").addEventListener("input", () =>
  storage.set("manim-name", $("#animation-name").value),
);
$("#resolution-select").addEventListener("change", () => {
  const value = $("#resolution-select").value;
  if (value === "custom") {
    $("#resolution-select").value = resolutionValue(outputSettings);
    $("#custom-width").value = outputSettings.width;
    $("#custom-height").value = outputSettings.height;
    $("#resolution-dialog").showModal();
  } else setOutputSettings({ ...outputSettings, ...parseResolution(value) });
});
$("#fps-select").addEventListener("change", () => {
  const value = $("#fps-select").value;
  if (value === "custom") {
    $("#fps-select").value = String(outputSettings.fps);
    $("#custom-fps").value = outputSettings.fps;
    $("#custom-fps").setCustomValidity("");
    $("#fps-dialog").showModal();
  } else setOutputSettings({ ...outputSettings, fps: Number(value) });
});
$("#resolution-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if (activeJob) {
    toast("Wait for the current render before changing output settings.");
    $("#resolution-dialog").close();
    return;
  }
  const width = $("#custom-width").valueAsNumber,
    height = $("#custom-height").valueAsNumber;
  if (!setOutputSettings({ ...outputSettings, width, height })) return;
  $("#resolution-dialog").close();
});
$("#custom-fps").addEventListener("input", () => {
  const fps = $("#custom-fps").valueAsNumber;
  $("#custom-fps").setCustomValidity(
    Number.isFinite(fps) && fps > 0
      ? ""
      : "Enter a positive, finite frame rate.",
  );
});
$("#fps-form").addEventListener("submit", (event) => {
  event.preventDefault();
  if (activeJob) {
    toast("Wait for the current render before changing output settings.");
    $("#fps-dialog").close();
    return;
  }
  if (
    !setOutputSettings({
      ...outputSettings,
      fps: $("#custom-fps").valueAsNumber,
    })
  )
    return;
  $("#fps-dialog").close();
});
for (const [dialog, control] of [
  ["#resolution-dialog", "#resolution-select"],
  ["#fps-dialog", "#fps-select"],
])
  $(dialog).addEventListener("close", () => {
    if (!$(control).disabled) $(control).focus();
    else
      ($("#render-button").disabled
        ? $("#theme-button")
        : $("#render-button")
      ).focus();
  });
$("#render-button").addEventListener("click", () => {
  if (activeJob) $("#cancel-button").click();
  else startRender();
});
document.addEventListener("keydown", (event) => {
  if (
    (event.ctrlKey || event.metaKey) &&
    event.key === "Enter" &&
    !document.querySelector("dialog[open]")
  ) {
    event.preventDefault();
    startRender();
  }
});
$("#cancel-button").addEventListener("click", async () => {
  if (!activeJob?.id || cancelRequested) return;
  cancelRequested = true;
  const jobId = activeJob.id;
  $("#cancel-button").disabled = true;
  $("#cancel-button").textContent = "Cancelling…";
  updateRenderButton();
  try {
    await api(`/api/renders/${jobId}/cancel`, {});
  } catch (error) {
    if (activeJob?.id !== jobId) return;
    cancelRequested = false;
    toast(error.message);
    $("#cancel-button").disabled = false;
    $("#cancel-button").textContent = "Cancel render";
    updateRenderButton();
  }
});
$("#paste-button").addEventListener("click", async () => {
  if (window.updatingStudio) return;
  try {
    const source = await navigator.clipboard.readText();
    if (window.updatingStudio) return;
    if (source.trim()) replaceSource(source);
    else toast("Your clipboard is empty. Copy a script first.");
  } catch {
    window.codeEditor?.focus();
    toast("Click in the editor and press Ctrl+V to paste your script.");
  }
});
$("#import-button").addEventListener("click", () => {
  if (!window.updatingStudio) $("#file-input").click();
});
$("#file-input").addEventListener("change", async (event) => {
  if (window.updatingStudio) return;
  const file = event.target.files[0];
  if (!file) return;
  if (file.size > 500000) {
    toast("Choose a Python script smaller than 500 KB.");
    event.target.value = "";
    return;
  }
  try {
    replaceSource(await file.text(), file.name.replace(/\.[^.]+$/, ""));
  } catch {
    toast("Couldn’t read that file. Please try again.");
  }
  event.target.value = "";
});
$("#examples-button").addEventListener("click", openExamples);
$("#load-sample").addEventListener("click", () =>
  replaceSource(EXAMPLES[0].source, EXAMPLES[0].title),
);
EXAMPLES.forEach((example) => {
  const button = document.createElement("button");
  button.className = "example-card";
  button.innerHTML = `<span class="example-art">${escapeHtml(example.art)}</span><span><strong>${example.title}</strong><small>${example.detail}</small><span class="example-meta">${example.meta}</span></span>${icon("arrow")}`;
  button.addEventListener("click", () => {
    $("#examples-dialog").close();
    showView("workspace");
    replaceSource(example.source, example.title);
  });
  $("#example-list").append(button);
});
document
  .querySelectorAll("[data-close]")
  .forEach((button) =>
    button.addEventListener("click", () =>
      document.getElementById(button.dataset.close).close(),
    ),
  );
$("#confirm-cancel").addEventListener("click", () => {
  pendingReplace = null;
  $("#confirm-dialog").close();
});
$("#confirm-replace").addEventListener("click", async () => {
  const pending = pendingReplace;
  pendingReplace = null;
  $("#confirm-dialog").close();
  if (pending) {
    if (
      pending.renderAfter &&
      pending.outputSnapshot &&
      !sameOutputSettings(pending.outputSnapshot, outputSettings)
    ) {
      toast(
        "Output settings changed. Select the 1080p render again when you’re ready.",
      );
      return;
    }
    if (
      window.updatingStudio ||
      ((pending.renderAfter || pending.previewJob) && activeJob)
    ) {
      toast("Wait for the current render before replacing its script.");
      return;
    }
    await setSource(pending.source, pending.name);
    if (editor.value !== pending.source || window.updatingStudio) return;
    if ((pending.renderAfter || pending.previewJob) && activeJob) return;
    if (pending.renderAfter && document.querySelector("dialog[open]")) return;
    if (
      pending.renderAfter &&
      pending.outputSnapshot &&
      !sameOutputSettings(pending.outputSnapshot, outputSettings)
    ) {
      toast(
        "Output settings changed. Select the 1080p render again when you’re ready.",
      );
      return;
    }
    if (pending.previewJob) showVideo(pending.previewJob);
    if (pending.renderAfter) {
      $("#scene-select").value = pending.scene;
      setOutputSettings(legacyOutputSettings.final);
      startRender();
    }
  }
});
$("#download-draft").addEventListener("click", downloadDraft);
$("#prompt-button").addEventListener("click", () =>
  copy(PROMPT, "Prompt copied. Add your idea and send it to your AI."),
);
$("#copy-error").addEventListener("click", () =>
  copy(
    `Manim Community 0.21.0 / Cairo render error\nScene: ${selectedJob?.scene || ""}\n${selectedJob?.error || ""}\n\n${selectedJob?.log || ""}`,
    "Error details copied. Send them to your AI with your script.",
  ),
);
$("#help-button").addEventListener("click", () => openTour());
$("#tour-close").addEventListener("click", closeTour);
$("#tour-next").addEventListener("click", () => {
  if (tourStep === TOUR.length - 1) closeTour();
  else {
    tourStep++;
    renderTour();
  }
});
$("#tour-back").addEventListener("click", () => {
  if (tourStep > 0) {
    tourStep--;
    renderTour();
  }
});
document
  .querySelectorAll("[data-view]")
  .forEach((button) =>
    button.addEventListener("click", () => showView(button.dataset.view)),
  );
$(".brand").addEventListener("click", () => showView("workspace"));
$("#history-create").addEventListener("click", () => showView("workspace"));
for (const selector of [
  "#open-outputs",
  "#show-video-folder",
  "#history-folder",
])
  $(selector).addEventListener("click", () => openFolder("outputs"));
$("#open-assets").addEventListener("click", () => openFolder("assets"));

async function init() {
  let savedPreferences = {};
  try {
    savedPreferences = await api("/api/preferences");
  } catch {}
  preferenceCache = { ...savedPreferences, ...preferenceChanges };
  preferencesReady = true;
  applyTheme(storage.get("manim-theme") || "dark");
  const draft = storage.get("manim-draft");
  if (draft && !draftEdited) {
    editor.value = draft;
    draftLoaded = true;
  }
  $("#animation-name").value = storage.get("manim-name") || "";
  restoreOutputSettings();
  updateEditor();
  if (Object.keys(preferenceChanges).length) persistPreferences();
  if (draftLoaded) checkScript();
  if (draftLoaded) toast("Last session restored.");
  const startupTrackingVersion = trackingVersion,
    startupSourceVersion = analyzeVersion,
    startupPreview = window.lastCompletedJob;
  try {
    const health = await api("/api/health");
    window.engineDisconnected = false;
    window.engineHealth = health;
    window.refreshProblems?.();
    updateRenderButton();
    const history = await loadHistory();
    if (
      history.length &&
      !activeJob &&
      trackingVersion === startupTrackingVersion &&
      analyzeVersion === startupSourceVersion &&
      window.lastCompletedJob === startupPreview
    )
      showVideo(history[0]);
    if (
      health.active &&
      !activeJob &&
      trackingVersion === startupTrackingVersion
    ) {
      const job = await api(`/api/renders/${health.active}`);
      if (!activeJob && trackingVersion === startupTrackingVersion)
        beginTracking(job);
    }
  } catch {
    window.engineDisconnected = true;
    window.refreshProblems?.();
    toast("Couldn’t reach the local engine. Close and reopen Studio.");
  }
  if (storage.get("manim-tour-seen") !== "2") openTour(false);
  window.studioInitialized = true;
  window.initUpdates?.(window.engineHealth);
}
init();

function applyTheme(theme) {
  const blocker = document.createElement("style");
  blocker.textContent = "*,*::before,*::after{transition:none!important}";
  document.head.append(blocker);
  document.documentElement.dataset.theme = theme;
  $("#theme-button").setAttribute(
    "aria-label",
    `Switch to ${theme === "dark" ? "light" : "dark"} theme`,
  );
  $("#theme-button span").innerHTML =
    theme === "dark"
      ? '<svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M5 19l1.4-1.4M17.6 6.4 19 5"/></svg>'
      : '<svg aria-hidden="true" viewBox="0 0 24 24"><path d="M20.5 14A9 9 0 0 1 10 3.5 9 9 0 1 0 20.5 14Z"/></svg>';
  $("#theme-button").title =
    `Switch to ${theme === "dark" ? "light" : "dark"} theme`;
  void document.body.offsetHeight;
  requestAnimationFrame(() => blocker.remove());
}
$("#theme-button").addEventListener("click", () => {
  const theme =
    document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  applyTheme(theme);
  storage.set("manim-theme", theme);
});
$("#mobile-help").addEventListener("click", () => openTour());
addEventListener("pagehide", () => persistPreferences(true));
const dropzone = $(".code-editor");
dropzone.addEventListener("dragover", (event) => {
  event.preventDefault();
  dropzone.classList.add("dragover");
});
dropzone.addEventListener("dragleave", () =>
  dropzone.classList.remove("dragover"),
);
dropzone.addEventListener("drop", async (event) => {
  event.preventDefault();
  dropzone.classList.remove("dragover");
  const file = event.dataTransfer.files[0];
  if (!file) return;
  if (!/\.(py|txt)$/i.test(file.name) || file.size > 500000) {
    toast("Drop a .py or .txt script smaller than 500 KB.");
    return;
  }
  replaceSource(await file.text(), file.name.replace(/\.[^.]+$/, ""));
});
