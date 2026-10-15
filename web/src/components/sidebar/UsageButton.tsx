import { GaugeIcon, RefreshCwIcon } from "lucide-react";
import { useState } from "react";
import { api, type UsageSnapshot } from "~/lib/api";
import { Button } from "../ui/button";
import { Spinner } from "../ui/input";
import {
  Popover,
  PopoverPopup,
  PopoverTitle,
  PopoverTrigger,
} from "../ui/popover";
import { Progress } from "../ui/progress";
import { Tooltip } from "../ui/tooltip";

export function UsageButton({ collapsed }: { collapsed: boolean }) {
  const [state, setState] = useState<UsageSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const load = async (refresh = false) => {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      setState(await api.usage(refresh));
    } catch (error) {
      setError((error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Popover
      onOpenChange={(open) => {
        if (open) void load();
      }}
    >
      <Tooltip content="Usage limits" side={collapsed ? "right" : "top"}>
        <PopoverTrigger
          render={
            <Button
              size="icon-sm"
              variant="ghost-muted"
              aria-label="Usage limits"
            />
          }
        >
          <GaugeIcon />
        </PopoverTrigger>
      </Tooltip>
      <PopoverPopup
        side={collapsed ? "right" : "top"}
        align="start"
        className="max-h-[min(36rem,calc(100dvh-2rem))] overflow-y-auto"
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <PopoverTitle className="text-sm font-semibold">
            Usage limits
          </PopoverTitle>
          <Button
            size="icon-xs"
            variant="ghost-muted"
            aria-label="Refresh usage limits"
            disabled={busy}
            onClick={() => void load(true)}
          >
            {busy ? <Spinner /> : <RefreshCwIcon />}
          </Button>
        </div>
        {busy && !state && (
          <p role="status" className="text-sm text-muted-foreground">
            Checking your plan limits…
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive-foreground">
            {error}
          </p>
        )}
        <div className="space-y-5">
          {state?.providers.map((provider) => (
            <section key={provider.id} aria-label={`${provider.name} usage`}>
              <h3 className="mb-2 text-sm font-medium">
                {provider.id === "codex" ? "ChatGPT / Codex" : "Claude"}
                {provider.plan && (
                  <span className="ms-2 text-xs capitalize text-muted-foreground">
                    {provider.plan}
                  </span>
                )}
              </h3>
              {!provider.available ? (
                <p className="text-xs text-muted-foreground">
                  {provider.error || "Usage isn't available."}
                </p>
              ) : (
                <div className="space-y-3">
                  {provider.windows.map((window) => (
                    <div key={window.id}>
                      <div className="mb-1 flex justify-between gap-2 text-xs">
                        <span>{window.label}</span>
                        <span className="tabular-nums">
                          {window.left_percent}% left
                        </span>
                      </div>
                      <Progress
                        aria-label={`${provider.name} ${window.label} used`}
                        value={window.used_percent}
                        max={100}
                      />
                      {window.resets_at && (
                        <p className="mt-1 text-2xs text-muted-foreground">
                          Resets{" "}
                          {new Date(window.resets_at * 1000).toLocaleString(
                            undefined,
                            {
                              month: "short",
                              day: "numeric",
                              hour: "numeric",
                              minute: "2-digit",
                            },
                          )}
                        </p>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </section>
          ))}
        </div>
        {state && (
          <p className="mt-4 border-t pt-3 text-2xs text-muted-foreground">
            Reported by your signed-in CLIs · Updated{" "}
            {new Date(state.updated * 1000).toLocaleTimeString(undefined, {
              hour: "numeric",
              minute: "2-digit",
            })}
          </p>
        )}
      </PopoverPopup>
    </Popover>
  );
}
