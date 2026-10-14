import { useEffect, useState } from "react";
import { api } from "~/lib/api";
import { useStudio } from "~/state/studio";
import { useProjectDoc } from "~/state/project";
import { Button } from "../ui/button";
import { confirmAction } from "../ui/dialog";
import { Spinner } from "../ui/input";
import { Switch } from "../ui/switch";
import { toast } from "../ui/toast";
import { SettingsHeading, SettingsRow } from "./SettingsLayout";

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
  const action = async (kind: "check" | "download" | "cancel" | "install") => {
    if (
      kind === "install" &&
      !(await confirmAction({
        title: "Install update and restart?",
        description:
          "Studio will close and reopen. Your projects, assets and render history are preserved.",
        confirm: "Install and restart",
      }))
    )
      return;
    setBusy(true);
    try {
      if (kind === "install") {
        if (
          !(await useProjectDoc.getState().flush()) ||
          !(await useStudio.getState().flushPrefs())
        )
          throw new Error(
            "Your changes couldn't be saved. Retry saving before updating.",
          );
        useStudio.setState({ installing: true });
      }
      const state = await {
        check: api.checkUpdates,
        download: api.downloadUpdate,
        cancel: api.cancelUpdate,
        install: api.installUpdate,
      }[kind]();
      useStudio.getState().setUpdates(state);
      if (kind === "install" && state.status !== "installing")
        useStudio.setState({ installing: false });
    } catch (error) {
      useStudio.setState({ installing: false });
      toast.error("Update action failed", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <SettingsHeading title="Manim Studio">
        Version {health?.version}
      </SettingsHeading>
      <div className="rounded-xl border bg-card p-5">
        <h2 className="font-medium text-sm">Updates</h2>
        <p className="mt-2 text-sm text-muted-foreground" role="status">
          {updates?.status === "current"
            ? "You're up to date."
            : updates?.latest_version
              ? `Latest version: ${updates.latest_version} · ${updates.status}`
              : updates?.status === "checking"
                ? "Checking for updates…"
                : "Check for a new version of Studio."}
        </p>
        {updates?.edition === "source" && (
          <p className="mt-2 text-xs text-muted-foreground">
            Download the desktop app to install updates.
          </p>
        )}
        {updates?.error && (
          <p className="mt-2 text-xs text-destructive-foreground" role="alert">
            {updates.error}
          </p>
        )}
        {pollError && (
          <p className="mt-2 text-xs text-destructive-foreground" role="alert">
            {pollError}
          </p>
        )}
        {updates?.status === "downloading" && (
          <progress
            aria-label="Update progress"
            value={updates.progress}
            max="100"
            className="mt-3 w-full"
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
          {updates?.release_url && (
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
    </>
  );
}
