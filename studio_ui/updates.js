"use strict";

// Updates have one request owner; snapshots cannot overtake a newer operation.
(() => {
  const dialog = $("#updates-dialog"),
    button = $("#updates-button"),
    automatic = $("#updates-auto"),
    officialReleases = "https://github.com/ItsTatsuya/manim-studio/releases",
    interval = 6 * 60 * 60 * 1000;
  let snapshot = null,
    request = null,
    pollTimer = null,
    autoTimer = null,
    generation = 0,
    stopped = false,
    initialized = false,
    confirming = false,
    transportError = "",
    pollFailures = 0,
    notifiedVersion = "";

  function activeOperation() {
    return ["checking", "downloading", "installing"].includes(snapshot?.status);
  }
  function safeRelease(value) {
    try {
      const url = new URL(value);
      if (
        url.origin === "https://github.com" &&
        /^\/ItsTatsuya\/manim-studio\/releases(?:\/|$)/i.test(url.pathname)
      )
        return url.href;
    } catch {}
    return officialReleases;
  }
  function controls() {
    const busy = !!request || activeOperation(),
      rendering = !!activeJob;
    $("#updates-check").disabled =
      !!request || (activeOperation() && !transportError);
    $("#updates-check").textContent =
      transportError && activeOperation()
        ? "Refresh status"
        : "Check for updates";
    $("#updates-download").hidden = !(
      snapshot?.can_download && snapshot?.status === "available"
    );
    $("#updates-download").disabled = busy;
    $("#updates-cancel").hidden = snapshot?.status !== "downloading";
    $("#updates-cancel").disabled = !!request || snapshot?.can_cancel === false;
    $("#updates-cancel").textContent =
      snapshot?.can_cancel === false ? "Cancelling…" : "Cancel download";
    $("#updates-install").hidden = snapshot?.status !== "ready";
    $("#updates-install").disabled =
      busy || rendering || !snapshot?.can_install || !preferencesReady;
    $("#updates-confirm-install").disabled =
      busy || rendering || !snapshot?.can_install || !preferencesReady;
    automatic.disabled = !!window.updatingStudio;
    $("#updates-actions").hidden = confirming;
    $("#updates-confirm").hidden = !confirming;
    const focused = document.activeElement;
    if (
      dialog.open &&
      dialog.contains(focused) &&
      (focused.disabled || !focused.getClientRects().length)
    )
      $("#updates-close").focus();
  }
  function render() {
    const status = snapshot?.status || "idle",
      latest = snapshot?.latest_version || "the new version";
    const updating = status === "installing",
      installingChanged = window.updatingStudio !== updating;
    window.updatingStudio = updating;
    $("#updates-version").textContent =
      `Current version: ${snapshot?.current_version || "—"}`;
    const messages = {
      idle: "Check GitHub for a new version of Manim Studio.",
      checking: "Checking GitHub for updates…",
      current: "You’re using the latest version of Manim Studio.",
      available: `Manim Studio ${latest} is available.`,
      downloading: `Downloading Manim Studio ${latest}…`,
      ready: activeJob
        ? "Update downloaded. Finish or cancel the current render before installing."
        : `Manim Studio ${latest} is ready to install.`,
      installing: "Preparing the update. Studio will close and restart…",
      error: snapshot?.error || "Couldn’t check for updates. Try again.",
    };
    const message = `${messages[status] || messages.idle}${status === "ready" && snapshot?.error ? ` ${snapshot.error}` : ""}`;
    $("#updates-status").textContent = transportError
      ? `${message} Couldn’t refresh progress: ${transportError} Use Refresh status to retry.`
      : message;
    $("#updates-release").href = safeRelease(snapshot?.release_url);
    $("#updates-support").hidden = !snapshot || snapshot.can_install;
    const available = [
      "available",
      "downloading",
      "ready",
      "installing",
    ].includes(status);
    $("#updates-badge").hidden = !available;
    if (available && latest !== notifiedVersion) {
      notifiedVersion = latest;
      $("#updates-notification").textContent =
        `Manim Studio ${latest} is available. Open Updates to review it.`;
    } else if (status === "current") {
      notifiedVersion = "";
      $("#updates-notification").textContent = "";
    }
    button.setAttribute(
      "aria-label",
      available ? `Updates: Manim Studio ${latest} available` : "Updates",
    );
    button.title = available ? `Manim Studio ${latest} available` : "Updates";
    $("#updates-progress-section").hidden = status !== "downloading";
    const bytes = Math.max(0, Number(snapshot?.bytes_downloaded) || 0),
      total = Math.max(0, Number(snapshot?.total_bytes) || 0);
    if (total) {
      const percent = Math.min(100, Math.round((bytes / total) * 100));
      $("#updates-progress").value = percent;
      $("#updates-progress-text").textContent =
        `${percent}% · ${formatSize(bytes)} of ${formatSize(total)}`;
    } else {
      $("#updates-progress").removeAttribute("value");
      $("#updates-progress-text").textContent =
        `${formatSize(bytes)} downloaded`;
    }
    controls();
    if (installingChanged) updateRenderButton();
  }
  function schedulePoll() {
    clearTimeout(pollTimer);
    if (!stopped && activeOperation() && pollFailures < 3)
      pollTimer = setTimeout(
        () => fetchSnapshot(),
        transportError ? 1000 * 2 ** (pollFailures - 1) : 750,
      );
  }
  async function fetchSnapshot(action = "") {
    if (stopped || request) return;
    clearTimeout(pollTimer);
    const version = generation;
    if (action === "install") {
      snapshot = { ...snapshot, status: "installing" };
      render();
    }
    request = api(
      `/api/updates${action ? `/${action}` : ""}`,
      action ? {} : undefined,
    );
    controls();
    try {
      const result = await request;
      if (stopped || generation !== version) return;
      snapshot = result;
      transportError = "";
      pollFailures = 0;
    } catch (error) {
      if (stopped || generation !== version) return;
      if (activeOperation()) {
        transportError = error.message;
        pollFailures++;
      } else {
        snapshot = { ...snapshot, status: "error", error: error.message };
        transportError = "";
      }
    } finally {
      request = null;
      if (!stopped && generation === version) {
        render();
        schedulePoll();
      }
    }
  }
  function scheduleAutomatic() {
    clearTimeout(autoTimer);
    if (stopped || !automatic.checked) return;
    autoTimer = setTimeout(async () => {
      if (!request && !activeOperation()) await fetchSnapshot("check");
      scheduleAutomatic();
    }, interval);
  }
  async function saveSession() {
    if (!preferencesReady) return false;
    await persistPreferences();
    if (Object.keys(preferenceChanges).length) {
      $("#updates-status").textContent =
        "Your session could not be saved. Save a script copy and retry before installing.";
      return false;
    }
    return true;
  }
  button.addEventListener("click", () => {
    if (document.querySelector("dialog[open]")) return;
    if (!$("#tour-popover").hidden) closeTour();
    confirming = false;
    render();
    dialog.showModal();
    if (!request && !activeOperation()) fetchSnapshot();
  });
  $("#updates-close").addEventListener("click", () => dialog.close());
  $("#updates-save-draft").addEventListener("click", downloadDraft);
  dialog.addEventListener("close", () => {
    confirming = false;
    // Native close restores focus before this queued event; preserve subsequent user focus.
    if (
      document.activeElement === document.body ||
      dialog.contains(document.activeElement)
    )
      button.focus();
  });
  $("#updates-check").addEventListener("click", () => {
    pollFailures = 0;
    fetchSnapshot(transportError && activeOperation() ? "" : "check");
  });
  $("#updates-download").addEventListener("click", () =>
    fetchSnapshot("download"),
  );
  $("#updates-cancel").addEventListener("click", () => fetchSnapshot("cancel"));
  $("#updates-install").addEventListener("click", async () => {
    if (activeJob || !snapshot?.can_install || snapshot?.status !== "ready")
      return;
    if (
      !(await saveSession()) ||
      activeJob ||
      !dialog.open ||
      snapshot?.status !== "ready"
    )
      return;
    confirming = true;
    controls();
    $("#updates-back").focus();
  });
  $("#updates-back").addEventListener("click", () => {
    confirming = false;
    render();
    $("#updates-install").focus();
  });
  $("#updates-confirm-install").addEventListener("click", async () => {
    if (activeJob || !confirming || !snapshot?.can_install || request) return;
    if (!(await saveSession()) || activeJob || !confirming || !dialog.open)
      return;
    confirming = false;
    await fetchSnapshot("install");
  });
  automatic.addEventListener("change", () => {
    storage.set("manim-check-updates", String(automatic.checked));
    scheduleAutomatic();
  });
  window.refreshUpdateControls = () => {
    if (snapshot) render();
  };
  window.initUpdates = async (health) => {
    if (initialized) return;
    initialized = true;
    await fetchSnapshot();
    const saved = storage.get("manim-check-updates");
    automatic.checked =
      saved === null ? !!health?.standalone : saved === "true";
    if (automatic.checked && !request && !activeOperation())
      await fetchSnapshot("check");
    scheduleAutomatic();
  };
  addEventListener("pagehide", () => {
    stopped = true;
    generation++;
    clearTimeout(pollTimer);
    clearTimeout(autoTimer);
  });
  addEventListener("pageshow", async (event) => {
    if (!event.persisted || !stopped) return;
    stopped = false;
    await request?.catch(() => {});
    if (stopped) return;
    fetchSnapshot();
    scheduleAutomatic();
  });
  // A deferred asset can arrive after the engine/preferences initialization finishes.
  if (window.studioInitialized) window.initUpdates(window.engineHealth);
})();
