import { Link } from "@tanstack/react-router";
import {
  CircleCheckIcon,
  DownloadIcon,
  RefreshCwIcon,
  RotateCwIcon,
  XIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { api } from "~/lib/api";
import { isBusy, runUpdateAction } from "~/lib/updates";
import { useStudio } from "~/state/studio";
import { Button } from "../ui/button";
import { Spinner } from "../ui/input";
import { toast } from "../ui/toast";
import { Tooltip } from "../ui/tooltip";

const RECHECK_MS = 30 * 60 * 1000;
const NOTIFIED_KEY = "studio-update-notified";

/** Keeps the update state fresh and tells the user when a new version exists. */
export function UpdateNotice({ collapsed }: { collapsed: boolean }) {
  const updates = useStudio((state) => state.updates);
  const standalone = useStudio((state) => state.health?.standalone);
  const autoCheck = useStudio(
    (state) => state.prefs["manim-check-updates"] !== "false",
  );
  const version = useStudio((state) => state.health?.version);
  const installing = useStudio((state) => state.installing);
  const [busy, setBusy] = useState(false);
  const [justChecked, setJustChecked] = useState(false);
  const status = updates?.status;
  const polling = status === "checking" || status === "downloading";
  const manual = useRef(false);

  // Follow a running check or download to its end, wherever the user is.
  useEffect(() => {
    if (!polling) return;
    const timer = window.setInterval(
      () =>
        void api
          .updates()
          .then((state) => useStudio.getState().setUpdates(state))
          .catch(() => undefined),
      700,
    );
    return () => window.clearInterval(timer);
  }, [polling]);

  // Re-check while Studio stays open and whenever the window regains focus.
  useEffect(() => {
    if (!standalone || !autoCheck) return;
    const check = () => {
      if (document.visibilityState === "visible")
        void api
          .checkUpdates()
          .then((state) => useStudio.getState().setUpdates(state))
          .catch(() => undefined);
    };
    const timer = window.setInterval(check, RECHECK_MS);
    document.addEventListener("visibilitychange", check);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", check);
    };
  }, [standalone, autoCheck]);

  // Notify once per version.
  useEffect(() => {
    const latest = updates?.latest_version;
    if (!latest || !["available", "ready"].includes(status ?? "")) return;
    if (window.localStorage.getItem(NOTIFIED_KEY) === latest) return;
    window.localStorage.setItem(NOTIFIED_KEY, latest);
    toast(`Update ${latest} is available`, {
      description: "Download it from the sidebar when you're ready.",
      duration: 8000,
    });
  }, [updates?.latest_version, status]);

  // Confirm a manual check that finds nothing new.
  useEffect(() => {
    if (!manual.current || status === "checking") return;
    manual.current = false;
    if (status === "current") {
      setJustChecked(true);
      const timer = window.setTimeout(() => setJustChecked(false), 4000);
      return () => window.clearTimeout(timer);
    }
  }, [status]);

  const run = async (kind: "check" | "download" | "cancel" | "install") => {
    setBusy(true);
    if (kind === "check") manual.current = true;
    try {
      await runUpdateAction(kind);
    } finally {
      setBusy(false);
    }
  };

  if (!updates || installing) return null;
  const latest = updates.latest_version;
  const needsAttention =
    (status === "available" && !!latest) ||
    status === "ready" ||
    status === "downloading" ||
    (status === "error" && !!latest);

  if (collapsed) {
    return needsAttention ? (
      <Tooltip content={`Update ${latest} available`} side="right">
        <Link
          to="/settings/about"
          aria-label={`Update ${latest} available`}
          className="mb-1 grid h-8 place-items-center rounded-md bg-primary/10 text-info-foreground hover:bg-primary/15"
        >
          <DownloadIcon className="size-4" />
        </Link>
      </Tooltip>
    ) : null;
  }

  return (
    <div className="mb-1 flex flex-col gap-1.5">
      {needsAttention && (
        <div
          role="status"
          className="rounded-lg border border-primary/25 bg-primary/10 p-2.5 text-xs"
        >
          {status === "downloading" ? (
            <>
              <div className="flex items-center gap-2 font-medium text-info-foreground">
                <Spinner className="size-3" /> Downloading {latest}…
                <span className="ms-auto tabular-nums">
                  {Math.round(updates.progress)}%
                </span>
                {updates.can_cancel && (
                  <Button
                    size="icon-xs"
                    variant="ghost-muted"
                    aria-label="Cancel download"
                    onClick={() => void run("cancel")}
                  >
                    <XIcon />
                  </Button>
                )}
              </div>
              <progress
                aria-label="Update progress"
                value={updates.progress}
                max="100"
                className="mt-2 h-1.5 w-full"
              />
            </>
          ) : status === "ready" ? (
            <>
              <div className="font-medium text-info-foreground">
                Update {latest} is ready
              </div>
              <Button
                size="sm"
                className="mt-2 w-full"
                disabled={busy || isBusy()}
                onClick={() => void run("install")}
              >
                <RotateCwIcon /> Restart to update
              </Button>
              {isBusy() && (
                <p className="mt-1.5 text-muted-foreground">
                  Finish running renders and agents first.
                </p>
              )}
            </>
          ) : status === "error" ? (
            <>
              <div className="font-medium text-destructive-foreground">
                Update {latest} couldn't finish
              </div>
              <p className="mt-1 line-clamp-2 text-muted-foreground">
                {updates.error}
              </p>
              {updates.can_download && (
                <Button
                  size="sm"
                  variant="outline"
                  className="mt-2 w-full"
                  disabled={busy}
                  onClick={() => void run("download")}
                >
                  Retry download
                </Button>
              )}
            </>
          ) : (
            <>
              <div className="font-medium text-info-foreground">
                Update {latest} available
              </div>
              <p className="mt-0.5 text-muted-foreground">
                You have {updates.current_version}.
              </p>
              {updates.can_download ? (
                <Button
                  size="sm"
                  className="mt-2 w-full"
                  disabled={busy}
                  onClick={() => void run("download")}
                >
                  <DownloadIcon /> Download update
                </Button>
              ) : (
                <Link
                  to="/settings/about"
                  className="mt-1 inline-block text-info-foreground hover:underline"
                >
                  See how to update
                </Link>
              )}
            </>
          )}
        </div>
      )}
      {!needsAttention && (
        <button
          type="button"
          disabled={busy || status === "checking"}
          onClick={() => void run("check")}
          className="flex items-center gap-2 rounded-md px-2 py-1 text-left text-[11px] text-sidebar-muted-foreground hover:text-sidebar-foreground disabled:opacity-70"
        >
          {status === "checking" ? (
            <Spinner className="size-3" />
          ) : justChecked ? (
            <CircleCheckIcon className="size-3 text-success-foreground" />
          ) : (
            <RefreshCwIcon className="size-3" />
          )}
          <span>
            {status === "checking"
              ? "Checking for updates…"
              : justChecked
                ? `Up to date · ${version}`
                : `Version ${version} · Check for updates`}
          </span>
        </button>
      )}
    </div>
  );
}
