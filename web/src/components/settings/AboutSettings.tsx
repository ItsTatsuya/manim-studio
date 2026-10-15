import { useEffect, useState } from "react";
import { api, type UpdateState } from "~/lib/api";
import { runUpdateAction, type UpdateAction } from "~/lib/updates";
import { useStudio } from "~/state/studio";
import { ReleaseNotes } from "../ReleaseNotes";
import { Button } from "../ui/button";
import { Spinner } from "../ui/input";
import { Progress } from "../ui/progress";
import { Switch } from "../ui/switch";
import { SettingsRow, SettingsSection } from "./SettingsLayout";

function checkedAgo(seconds: number | null | undefined) {
  if (!seconds) return "";
  const minutes = Math.floor((Date.now() / 1000 - seconds) / 60);
  if (minutes < 1) return " Checked just now.";
  if (minutes < 60) return ` Checked ${minutes} min ago.`;
  return ` Checked ${Math.floor(minutes / 60)} h ago.`;
}

function updateMessage(updates: UpdateState | null | undefined) {
  if (!updates) return "Checking for updates…";
  const latest = updates.latest_version;
  switch (updates.status) {
    case "checking":
      return "Checking for updates…";
    case "current":
      return `You're up to date. Version ${updates.current_version} is the latest.${checkedAgo(updates.last_checked)}`;
    case "available":
      return updates.can_download
        ? `Version ${latest} is available. You have ${updates.current_version}.`
        : `Version ${latest} is available. Download the desktop app to install it.`;
    case "downloading":
      return `Downloading version ${latest}… ${Math.round(updates.progress)}%`;
    case "ready":
      return `Version ${latest} is downloaded and ready to install.`;
    case "installing":
      return `Installing version ${latest}…`;
    case "error":
      return latest
        ? `Version ${latest} is available, but the last attempt failed.`
        : "Couldn't check for updates.";
    default:
      return "Not checked yet. Select Check for updates.";
  }
}

export function AboutSettings() {
  const health = useStudio((state) => state.health);
  const updates = useStudio((state) => state.updates);
  const autoCheck = useStudio(
    (state) => state.prefs["manim-check-updates"] !== "false",
  );
  const running = useStudio(
    (state) =>
      state.renders.some((render) =>
        ["queued", "rendering"].includes(render.status),
      ) ||
      Object.values(state.threads).some((threads) =>
        threads.some((thread) => thread.status === "running"),
      ),
  );
  const [busy, setBusy] = useState(false);
  const [pollError, setPollError] = useState("");
  const polling =
    updates &&
    ["checking", "downloading", "installing"].includes(updates.status);
  useEffect(() => {
    if (!polling) return;
    const timer = window.setInterval(
      () =>
        void api
          .updates()
          .then((state) => {
            setPollError("");
            useStudio.getState().setUpdates(state);
            if (state.status === "error")
              useStudio.setState({ installing: false });
          })
          .catch(() =>
            setPollError(
              "Couldn't refresh update progress. Check your connection and retry.",
            ),
          ),
      700,
    );
    return () => window.clearInterval(timer);
  }, [polling]);
  const action = async (kind: UpdateAction) => {
    setBusy(true);
    try {
      await runUpdateAction(kind);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <SettingsSection title="Manim Studio">
        <SettingsRow title="Version">
          <span className="text-muted-foreground text-sm tabular-nums">
            {health?.version}
          </span>
        </SettingsRow>
        <div className="p-4">
          <h3 className="font-medium text-sm">Updates</h3>
          <p
            className="mt-2 text-sm text-muted-foreground tabular-nums"
            role="status"
          >
            {updateMessage(updates)}
          </p>
          {updates?.latest_version && updates.status !== "current" && (
            <div className="mt-4 rounded-lg border bg-muted/40 p-3">
              <p className="mb-2 font-medium text-sm">
                Version {updates.latest_version}
              </p>
              <ReleaseNotes
                summary={updates.release_summary}
                items={updates.release_notes}
                url={updates.release_url}
                limit={12}
              />
            </div>
          )}
          {updates?.edition === "source" && (
            <p className="mt-2 text-xs text-muted-foreground">
              Download the desktop app to install updates.
            </p>
          )}
          {updates?.error && (
            <p
              className="mt-2 text-xs text-destructive-foreground"
              role="alert"
            >
              {updates.error}
            </p>
          )}
          {pollError && (
            <p
              className="mt-2 text-xs text-destructive-foreground"
              role="alert"
            >
              {pollError}
            </p>
          )}
          {updates?.status === "downloading" && (
            <Progress
              aria-label="Update progress"
              value={updates.progress}
              max="100"
              className="mt-3"
            />
          )}
          <div className="mt-4 flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !!polling}
              onClick={() => void action("check")}
            >
              {busy && <Spinner />} Check for updates
            </Button>
            {updates?.can_download && (
              <Button
                size="sm"
                disabled={busy}
                onClick={() => void action("download")}
              >
                Download update
              </Button>
            )}
            {updates?.can_cancel && (
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => void action("cancel")}
              >
                Cancel download
              </Button>
            )}
            {updates?.can_install && (
              <Button
                size="sm"
                disabled={busy || running}
                onClick={() => void action("install")}
              >
                Install and restart
              </Button>
            )}
            {updates?.latest_version && updates.release_url && (
              <a
                href={updates.release_url}
                target="_blank"
                rel="noreferrer"
                className="self-center text-xs text-muted-foreground hover:underline"
              >
                Release notes ↗
              </a>
            )}
          </div>
          {running && updates?.can_install && (
            <p className="mt-2 text-xs text-muted-foreground">
              Wait for running agents and renders before installing.
            </p>
          )}
        </div>
        {health?.standalone && (
          <SettingsRow title="Check for updates at startup">
            <Switch
              aria-label="Check for updates at startup"
              checked={autoCheck}
              onCheckedChange={(checked) =>
                useStudio
                  .getState()
                  .setPref("manim-check-updates", String(checked))
              }
            />
          </SettingsRow>
        )}
      </SettingsSection>
    </>
  );
}
