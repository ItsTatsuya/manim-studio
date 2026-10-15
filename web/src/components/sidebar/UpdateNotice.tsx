import { Link } from "@tanstack/react-router";
import {
  CircleCheckIcon,
  DownloadIcon,
  RefreshCwIcon,
  RotateCwIcon,
  XIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { api, type UpdateState } from "~/lib/api";
import { isBusy, runUpdateAction } from "~/lib/updates";
import { useStudio } from "~/state/studio";
import { ReleaseNotes } from "../ReleaseNotes";
import { Button } from "../ui/button";
import { Spinner } from "../ui/input";
import {
  Popover,
  PopoverPopup,
  PopoverTitle,
  PopoverTrigger,
} from "../ui/popover";
import { Progress } from "../ui/progress";
import { toast } from "../ui/toast";
import { Tooltip } from "../ui/tooltip";

const RECHECK_MS = 30 * 60 * 1000;
const NOTIFIED_KEY = "studio-update-notified";

/** Update state that wants the user's attention, as opposed to idle or current. */
function useNeedsAttention() {
  const updates = useStudio((state) => state.updates);
  const status = updates?.status;
  const latest = updates?.latest_version;
  return {
    updates,
    status,
    latest,
    needsAttention:
      (status === "available" && !!latest) ||
      status === "ready" ||
      status === "downloading" ||
      (status === "error" && !!latest),
  };
}

/**
 * Keeps the update state fresh and tells the user when a new version exists.
 * Mounted once; the button below only reads the shared state.
 */
export function UpdateWatcher() {
  const updates = useStudio((state) => state.updates);
  const standalone = useStudio((state) => state.health?.standalone);
  const autoCheck = useStudio(
    (state) => state.prefs["manim-check-updates"] !== "false",
  );
  const status = updates?.status;
  const polling = status === "checking" || status === "downloading";

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
      description: "Open the update button in the sidebar to see what changed.",
      duration: 8000,
    });
  }, [updates?.latest_version, status]);

  return null;
}

function panelTitle(updates: UpdateState) {
  switch (updates.status) {
    case "downloading":
      return "Downloading update";
    case "ready":
      return "Update ready to install";
    case "error":
      return "Update couldn't finish";
    default:
      return updates.can_download
        ? "Update ready to download"
        : "Update available";
  }
}

/** The popover body: what changed, plus the one action that fits the state. */
function UpdatePanel({ updates }: { updates: UpdateState }) {
  const [busy, setBusy] = useState(false);
  const status = updates.status;

  const run = async (kind: "download" | "cancel" | "install") => {
    setBusy(true);
    try {
      await runUpdateAction(kind);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div>
        <PopoverTitle className="font-semibold text-base">
          {panelTitle(updates)}
        </PopoverTitle>
        <p className="mt-0.5 text-muted-foreground text-xs tabular-nums">
          {updates.current_version} → {updates.latest_version}
        </p>
      </div>

      <ReleaseNotes
        summary={updates.release_summary}
        items={updates.release_notes}
        url={updates.release_url}
      />

      {status === "downloading" && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2 text-xs tabular-nums">
            <Spinner className="size-3" /> Downloading…
            <span className="ms-auto">{Math.round(updates.progress)}%</span>
          </div>
          <Progress
            aria-label="Update progress"
            value={updates.progress}
            max="100"
          />
          {updates.can_cancel && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => void run("cancel")}
            >
              <XIcon /> Cancel download
            </Button>
          )}
        </div>
      )}

      {status === "ready" && (
        <>
          <Button
            disabled={busy || isBusy()}
            onClick={() => void run("install")}
          >
            <RotateCwIcon /> Restart to update
          </Button>
          {isBusy() && (
            <p className="text-muted-foreground text-xs">
              Finish running renders and agents first.
            </p>
          )}
        </>
      )}

      {status === "error" && (
        <>
          <p className="line-clamp-3 text-destructive-foreground text-xs">
            {updates.error}
          </p>
          {updates.can_download && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => void run("download")}
            >
              Retry download
            </Button>
          )}
        </>
      )}

      {status === "available" &&
        (updates.can_download ? (
          <Button disabled={busy} onClick={() => void run("download")}>
            <DownloadIcon /> Download update
          </Button>
        ) : (
          <Link
            to="/settings/about"
            className="text-info-foreground text-sm hover:underline"
          >
            See how to update
          </Link>
        ))}
    </div>
  );
}

/**
 * One icon button for update state. Idle, it checks for updates. When a new
 * version is waiting it carries a dot and opens a popover with the release
 * notes and the download, restart or retry action.
 */
export function UpdateButton({ collapsed }: { collapsed: boolean }) {
  const installing = useStudio((state) => state.installing);
  const version = useStudio((state) => state.health?.version);
  const { updates, status, latest, needsAttention } = useNeedsAttention();
  const [busy, setBusy] = useState(false);
  const [justChecked, setJustChecked] = useState(false);
  const manual = useRef(false);

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

  if (!updates || installing) return null;

  const checking = status === "checking";
  const downloading = status === "downloading";
  const label = downloading
    ? `Downloading update ${latest} (${Math.round(updates.progress)}%)`
    : needsAttention
      ? `Update ${latest} available`
      : checking
        ? "Checking for updates…"
        : justChecked
          ? `Up to date · ${version}`
          : `Version ${version} · Check for updates`;
  const side = collapsed ? "right" : "top";

  if (needsAttention) {
    return (
      <Popover>
        <Tooltip content={label} side={side}>
          <PopoverTrigger
            render={
              <Button
                size="icon-sm"
                variant="ghost-muted"
                aria-label={label}
                className="relative text-info-foreground"
              />
            }
          >
            {downloading ? <Spinner /> : <DownloadIcon />}
            <span
              aria-hidden="true"
              className="absolute end-1 top-1 size-1.5 rounded-full bg-primary ring-2 ring-sidebar"
            />
          </PopoverTrigger>
        </Tooltip>
        <PopoverPopup side={side} align="end">
          <UpdatePanel updates={updates} />
        </PopoverPopup>
      </Popover>
    );
  }

  return (
    <Tooltip content={label} side={side}>
      <Button
        size="icon-sm"
        variant="ghost-muted"
        aria-label={label}
        disabled={busy || checking}
        onClick={async () => {
          setBusy(true);
          manual.current = true;
          try {
            await runUpdateAction("check");
          } finally {
            setBusy(false);
          }
        }}
      >
        {checking ? (
          <Spinner />
        ) : justChecked ? (
          <CircleCheckIcon className="text-success-foreground" />
        ) : (
          <RefreshCwIcon />
        )}
      </Button>
    </Tooltip>
  );
}
