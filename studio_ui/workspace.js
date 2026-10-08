"use strict";
let pane = "editor",
  drawerTab = "problems";
function switchPane(value) {
  pane = value;
  $(".workspace-grid").dataset.pane = value;
  for (const name of ["editor", "preview"]) {
    const tab = $(`#${name}-tab`);
    tab.setAttribute("aria-selected", String(name === value));
    tab.tabIndex = name === value ? 0 : -1;
  }
  queueTourPosition();
}
for (const name of ["editor", "preview"])
  $(`#${name}-tab`).addEventListener("click", () => switchPane(name));
$(".pane-tabs").addEventListener("keydown", (event) => {
  if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    switchPane(
      event.key === "Home"
        ? "editor"
        : event.key === "End"
          ? "preview"
          : pane === "editor"
            ? "preview"
            : "editor",
    );
    $(`#${pane}-tab`).focus();
  }
});
function selectDrawer(value) {
  drawerTab = value;
  $("#problems-panel").hidden = value !== "problems";
  $("#log-output").hidden = value !== "log";
  for (const name of ["problems", "log"]) {
    $(`#${name}-tab`).setAttribute("aria-selected", String(name === value));
    $(`#${name}-tab`).tabIndex = name === value ? 0 : -1;
  }
}
$("#problems-tab").addEventListener("click", () => selectDrawer("problems"));
$("#log-tab").addEventListener("click", () => selectDrawer("log"));
$(".drawer-heading [role=tablist]").addEventListener("keydown", (event) => {
  if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    selectDrawer(
      event.key === "Home"
        ? "problems"
        : event.key === "End"
          ? "log"
          : drawerTab === "log"
            ? "problems"
            : "log",
    );
    $(`#${drawerTab}-tab`).focus();
  }
});
$("#drawer-toggle").addEventListener("click", () => {
  const closed = $("#log-panel").classList.toggle("collapsed");
  $("#drawer-toggle").setAttribute("aria-expanded", String(!closed));
  $("#drawer-toggle").setAttribute(
    "aria-label",
    `${closed ? "Expand" : "Collapse"} output drawer`,
  );
  $("#drawer-toggle").textContent = closed ? "⌃" : "⌄";
});
function expandDrawer() {
  if ($("#log-panel").classList.contains("collapsed"))
    $("#drawer-toggle").click();
}
$("#dismiss-banner").addEventListener("click", () => {
  window.starterHelpDismissed = true;
  $("#editor-empty").hidden = true;
});
$("#banner-prompt").addEventListener("click", () =>
  copy(PROMPT, "Starter prompt copied."),
);
const divider = $("#split-divider"),
  grid = $(".workspace-grid");
let resizing = false;
function resizeSplit(value) {
  const width = grid.clientWidth - 12;
  const min = Math.max(35, (390 / width) * 100),
    max = Math.min(70, ((width - 320) / width) * 100);
  const percent = Math.max(min, Math.min(max, value));
  grid.style.setProperty("--split", `${percent}%`);
  divider.setAttribute("aria-valuenow", String(Math.round(percent)));
  window.codeEditor?.requestMeasure();
  queueTourPosition();
}
divider.addEventListener("pointerdown", (event) => {
  resizing = true;
  divider.setPointerCapture(event.pointerId);
  document.body.classList.add("resizing");
});
divider.addEventListener("pointermove", (event) => {
  if (resizing)
    resizeSplit(
      ((event.clientX - grid.getBoundingClientRect().left) / grid.clientWidth) *
        100,
    );
});
for (const eventName of ["pointerup", "pointercancel"])
  divider.addEventListener(eventName, () => {
    resizing = false;
    document.body.classList.remove("resizing");
  });
divider.addEventListener("keydown", (event) => {
  if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    resizeSplit(
      event.key === "Home"
        ? 35
        : event.key === "End"
          ? 70
          : Number(divider.getAttribute("aria-valuenow")) +
            (event.key === "ArrowRight" ? 2 : -2),
    );
  }
});
const originalRenderTour = renderTour;
renderTour = function () {
  if (innerWidth < 1100)
    switchPane(
      TOUR[tourStep].target === "#preview-heading" ? "preview" : "editor",
    );
  originalRenderTour();
};
$("#copy-script").addEventListener("click", () =>
  copy(editor.value, "Script copied."),
);
$("#format-script").addEventListener("click", async () => {
  if (window.updatingStudio) return;
  const originalSource = editor.value;
  $("#format-script").disabled = true;
  try {
    const result = await api("/api/format", { source: originalSource });
    if (window.updatingStudio) return;
    if (editor.value !== originalSource) {
      toast("Script changed during formatting. Click Format again.");
      return;
    }
    await setSource(result.source);
    toast("Code whitespace normalized.");
  } catch (error) {
    toast(error.message);
  } finally {
    updateEditor();
  }
});
$("#loop-video").addEventListener(
  "change",
  (event) => ($("#video-player").loop = event.target.checked),
);
$("#playback-speed").addEventListener(
  "change",
  (event) => ($("#video-player").playbackRate = Number(event.target.value)),
);
$("#fullscreen-video").addEventListener("click", () =>
  $("#video-player")
    .requestFullscreen?.()
    .catch(() => toast("Fullscreen is unavailable in this window.")),
);
$("#save-library").addEventListener("click", async () => {
  const jobs = await loadHistory();
  if (jobs.some((job) => job.id === window.lastCompletedJob?.id))
    toast("Saved in My renders. Completed renders are added automatically.");
  else
    toast(
      "Could not verify the saved video. Open the videos folder to check it.",
    );
});
$("#download-video").addEventListener("click", () =>
  toast("Video download started."),
);
$("#higher-quality").addEventListener("click", async () => {
  if (activeJob || window.updatingStudio) return;
  const job = window.lastCompletedJob;
  if (!job) return;
  const originalSource = editor.value;
  const outputSnapshot = { ...outputSettings };
  window.higherQualityPending = true;
  $("#higher-quality").disabled = true;
  try {
    const response = await fetch(`/api/renders/${job.id}/script`);
    if (!response.ok)
      throw new Error(
        "Could not load the rendered script. Use My renders to try again.",
      );
    const source = normalizeSource(await response.text());
    if (
      activeJob ||
      window.updatingStudio ||
      document.querySelector("dialog[open]") ||
      window.lastCompletedJob?.id !== job.id ||
      editor.value !== originalSource ||
      !sameOutputSettings(outputSnapshot, outputSettings)
    ) {
      toast(
        "The workspace changed. Select higher quality again when you’re ready.",
      );
      return;
    }
    if (editor.value.trim() && editor.value.trim() !== source.trim()) {
      pendingReplace = {
        source,
        name: job.name,
        scene: job.scene,
        renderAfter: true,
        outputSnapshot,
      };
      $("#confirm-dialog").showModal();
    } else {
      await setSource(source, job.name);
      if (
        editor.value !== source ||
        activeJob ||
        window.updatingStudio ||
        document.querySelector("dialog[open]") ||
        !sameOutputSettings(outputSnapshot, outputSettings)
      )
        return;
      $("#scene-select").value = job.scene;
      setOutputSettings(legacyOutputSettings.final);
      startRender();
    }
  } catch (error) {
    toast(error.message);
  } finally {
    window.higherQualityPending = false;
    updateRenderButton();
  }
});
switchPane("editor");
selectDrawer("problems");

// Keep the existing source value and API flow; CodeMirror owns editing and focus.
editor.hidden = true;
$("#line-numbers").hidden = true;
$("#code-highlight").hidden = true;
$(".code-area").hidden = true;
$(".code-editor").classList.add("cm-mounted");
window.codeEditor = StudioEditor.create($(".code-editor"), {
  source: editor.value,
  onChange(source) {
    editor.value = source;
    editor.dispatchEvent(new Event("input"));
  },
  onSelection(line, column) {
    $("#cursor-position").textContent = `Ln ${line}, Col ${column}`;
  },
});
function editorSetting(key, value) {
  try {
    if (value === undefined) return localStorage.getItem(key);
    localStorage.setItem(key, String(value));
  } catch {}
}
let fontSize = Math.max(
  12,
  Math.min(22, Number(editorSetting("studio-editor-font-size")) || 14),
);
function changeFont(amount) {
  fontSize = Math.max(12, Math.min(22, fontSize + amount));
  document.documentElement.style.setProperty("--editor-size", `${fontSize}px`);
  $("#font-decrease").disabled = fontSize <= 12;
  $("#font-increase").disabled = fontSize >= 22;
  editorSetting("studio-editor-font-size", fontSize);
  codeEditor.requestMeasure();
}
$("#font-decrease").addEventListener("click", () => changeFont(-1));
$("#font-increase").addEventListener("click", () => changeFont(1));
changeFont(0);
let wrapped = editorSetting("studio-editor-wrap") === "true";
function setWrapped(value) {
  wrapped = value;
  codeEditor.wrap(value);
  $("#wrap-toggle").setAttribute("aria-pressed", String(value));
  editorSetting("studio-editor-wrap", value);
}
$("#wrap-toggle").addEventListener("click", () => setWrapped(!wrapped));
setWrapped(wrapped);
$("#find-button").addEventListener("click", () => codeEditor.find());

let sceneOptionsSignature = "";
window.syncSceneControl = function () {
  const row = $(".scene-row"),
    trigger = $("#scene-trigger"),
    list = $("#scene-list");
  if (
    !analysis &&
    editor.value.trim() &&
    !window.scriptProblems?.some((problem) => problem.severity === "error")
  ) {
    // Keep the last detected layout while validation is pending, with selection disabled.
    trigger.disabled = true;
    if (row.contains(document.activeElement)) $("#animation-name").focus();
    closeSceneList();
    return;
  }
  const scenes = analysis?.scenes || [],
    multiple = scenes.length > 1;
  if (
    (!multiple || activeJob || window.updatingStudio) &&
    row.contains(document.activeElement)
  )
    $("#animation-name").focus();
  row.hidden = !multiple;
  $("#scene-trigger").hidden = !multiple;
  const selected = $("#scene-select").value;
  if ($("#scene-trigger").dataset.scene !== selected) {
    $("#scene-trigger").innerHTML =
      `<span class="scene-name">${escapeHtml(selected)}</span><span aria-hidden="true">⌄</span>`;
    $("#scene-trigger").dataset.scene = selected;
  }
  $("#scene-trigger").title = $("#scene-select").value;
  $("#scene-trigger").setAttribute("aria-label", `Scene: ${selected}`);
  $("#scene-trigger").disabled = !!activeJob || !!window.updatingStudio;
  if (!multiple || activeJob || window.updatingStudio) {
    closeSceneList();
  }
  const signature = JSON.stringify(scenes);
  if (signature === sceneOptionsSignature) {
    for (const option of list.children) {
      option.setAttribute(
        "aria-selected",
        String(option.textContent === selected),
      );
      option.tabIndex = option.textContent === selected ? 0 : -1;
    }
    return;
  }
  if (!list.hidden) closeSceneList(true);
  sceneOptionsSignature = signature;
  list.replaceChildren();
  for (const scene of scenes) {
    const option = document.createElement("button");
    option.type = "button";
    option.setAttribute("role", "option");
    option.textContent = scene;
    option.tabIndex = scene === selected ? 0 : -1;
    option.setAttribute(
      "aria-selected",
      String(scene === $("#scene-select").value),
    );
    option.addEventListener("click", () => {
      $("#scene-select").value = scene;
      closeSceneList();
      syncSceneControl();
      $("#scene-trigger").focus();
    });
    list.append(option);
  }
};
function closeSceneList(restoreFocus = false) {
  if (restoreFocus && $("#scene-list").contains(document.activeElement))
    $("#scene-trigger").focus();
  $("#scene-list").hidden = true;
  $("#scene-trigger").setAttribute("aria-expanded", "false");
}
$("#scene-trigger").addEventListener("click", () => {
  const open = $("#scene-list").hidden;
  $("#scene-list").hidden = !open;
  $("#scene-trigger").setAttribute("aria-expanded", String(open));
  if (open) $("#scene-list [aria-selected=true]")?.focus();
});
$("#scene-trigger").addEventListener("keydown", (event) => {
  if (["ArrowDown", "ArrowUp"].includes(event.key)) {
    event.preventDefault();
    if ($("#scene-list").hidden) $("#scene-trigger").click();
  }
});
$("#scene-list").addEventListener("keydown", (event) => {
  const options = [...$("#scene-list").children],
    index = options.indexOf(document.activeElement);
  if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    options[
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? options.length - 1
          : (index + (event.key === "ArrowDown" ? 1 : -1) + options.length) %
            options.length
    ]?.focus();
  } else if (event.key === "Escape") {
    closeSceneList();
    $("#scene-trigger").focus();
  } else if (event.key === "Tab") closeSceneList(true);
});
document.addEventListener("pointerdown", (event) => {
  if (!event.target.closest(".scene-row")) closeSceneList();
});

window.scriptProblems = window.scriptProblems || [];
window.refreshProblems = function () {
  const problems = [...(window.scriptProblems || [])];
  if (window.engineHealth?.latex === false)
    problems.push({
      message:
        "LaTeX is unavailable. Re-run the installer to repair equation tools, or use Text in your script.",
      severity: analysis?.needs_latex ? "error" : "warning",
    });
  if (window.engineDisconnected)
    problems.push({
      message:
        "The local render engine is disconnected. Close and reopen Manim Studio.",
      severity: "error",
    });
  const list = $("#problem-list");
  list.replaceChildren();
  $("#problem-count").textContent = problems.length;
  if (!problems.length) {
    const empty = document.createElement("p");
    empty.className = "problem-empty";
    empty.textContent = "No problems detected.";
    list.append(empty);
  }
  for (const problem of problems) {
    const item = document.createElement(problem.line ? "button" : "div");
    item.className = `problem-item ${problem.severity || "error"}`;
    item.innerHTML = `<span class="problem-symbol" aria-hidden="true">${problem.severity === "warning" ? "△" : "!"}</span><span>${problem.line ? `<strong>Line ${problem.line}</strong> · ` : ""}${escapeHtml(problem.message)}</span>`;
    if (problem.line)
      item.addEventListener("click", () => {
        switchPane("editor");
        codeEditor.jump(problem.line);
      });
    list.append(item);
  }
  codeEditor.diagnostics(problems);
  if (problems.some((problem) => problem.severity === "error")) {
    selectDrawer("problems");
    expandDrawer();
  }
};
window.showRenderProblems = function (job) {
  const log = job.log || "",
    matches = [...log.matchAll(/(?:animation\.py[:", ]+(?:line\s+)?)(\d+)/g)];
  const line = job.problem?.line
    ? job.problem.line + (window.renderLineOffset || 0)
    : matches.length
      ? Number(matches.at(-1)[1]) + (window.renderLineOffset || 0)
      : null;
  const detail = log
    .split("\n")
    .map((text) => text.trim().replace(/^[│|]\s*|\s*[│|]$/g, ""))
    .reverse()
    .find((text) =>
      /^(?:[\w.]+(?:Error|Exception)|KeyboardInterrupt):/.test(text),
    );
  const dependency =
    /ffmpeg.*(?:not found|couldn.t|no such)|couldn.t.*ffmpeg/i.test(log);
  window.scriptProblems = [
    {
      line,
      message: dependency
        ? "FFmpeg is unavailable. Re-run the installer to repair audio tools."
        : job.problem?.message ||
          detail ||
          job.error ||
          "Rendering failed. Open the Render log for details.",
      severity: "error",
    },
  ];
  if (window.renderedSource && window.renderedSource !== editor.value) {
    window.scriptProblems[0].line = null;
    window.scriptProblems[0].message += ` (Rendered script, line ${line || "unknown"}. The editor has changed since this render started.)`;
  }
  refreshProblems();
  switchPane("editor");
};
window.prepareRenderUI = function (job) {
  switchPane("preview");
  selectDrawer("log");
  expandDrawer();
  closeSceneList();
  $("#preview-progress").removeAttribute("value");
  $("#render-log").textContent = "Preparing the render engine…";
  $("#error-card").hidden = true;
  $("#video-player").hidden = true;
};
window.updateProgress = function (job) {
  const percent = job.stage?.match(/(\d+)%/)?.[1];
  const elapsed = Math.max(
    0,
    Math.round(job.elapsed || (Date.now() - Date.parse(job.created)) / 1000),
  );
  if (percent === undefined) $("#preview-progress").removeAttribute("value");
  else $("#preview-progress").value = Number(percent);
  $("#render-stage").textContent = job.stage;
  $("#render-elapsed").textContent =
    `${elapsed}s elapsed${percent === undefined ? "" : ` · ${percent}% of current animation`}`;
};
window.updateVideoMetadata = function (job) {
  const video = $("#video-player"),
    duration = Number.isFinite(video.duration)
      ? `${video.duration.toFixed(1)}s`
      : "Loading duration";
  const resolution = video.videoWidth
    ? `${video.videoWidth} × ${video.videoHeight}`
    : job.resolution;
  $("#video-meta").textContent =
    `${resolution} · ${formatFrameRate(job.fps)}fps · ${duration} · Rendered in ${(job.elapsed || 0).toFixed(1)}s · ${formatSize(job.size)}`;
};
$("#video-player").addEventListener("loadedmetadata", () => {
  if (window.lastCompletedJob) updateVideoMetadata(lastCompletedJob);
});
window.restoreCompletedPreview = function () {
  if (window.lastCompletedJob) {
    const previous = selectedJob;
    showVideo(lastCompletedJob);
    selectedJob = previous;
  } else $("#preview-placeholder").hidden = false;
};
window.updateEstimates = function (jobs) {
  outputHistory = jobs;
  updateOutputEstimate();
};
syncSceneControl();
refreshProblems();
updateEditor();
updateRenderButton();
addEventListener("resize", () => {
  if (innerWidth >= 1100)
    resizeSplit(Number(divider.getAttribute("aria-valuenow")));
  else codeEditor.requestMeasure();
  adaptShortScreen();
});
let autoCollapsedDrawer = false;
function adaptShortScreen() {
  const compact = innerWidth < 700 && innerHeight < 740;
  if (compact && !$("#log-panel").classList.contains("collapsed")) {
    $("#drawer-toggle").click();
    autoCollapsedDrawer = true;
  } else if (!compact && autoCollapsedDrawer) {
    if ($("#log-panel").classList.contains("collapsed"))
      $("#drawer-toggle").click();
    autoCollapsedDrawer = false;
  }
}
adaptShortScreen();
