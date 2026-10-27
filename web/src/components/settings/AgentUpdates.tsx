import { RefreshCwIcon } from "lucide-react";
import { useEffect, useState } from "react";
import {
  api,
  type AgentUpdateEntry,
  type AgentUpdateStatus,
  type ProviderId,
} from "~/lib/api";
import { relativeTime } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { Button } from "../ui/button";
import { Badge, Spinner } from "../ui/input";
import { Switch } from "../ui/switch";
import { toast } from "../ui/toast";
import { SettingsRow, SettingsSection } from "./SettingsLayout";

const LABELS: Record<AgentUpdateStatus, string> = {
  unknown: "Not checked",
  checking: "Checking…",
  current: "Up to date",
  available: "Update available",
  waiting: "Update waiting",
  updating: "Updating…",
  error: "Update problem",
  external: "Managed externally",
  disabled: "Turned off",
  missing: "Not installed",
};

const TONES: Partial<
  Record<AgentUpdateStatus, "success" | "warning" | "error" | "info">
> = {
  current: "success",
  available: "info",
  waiting: "warning",
  updating: "info",
  error: "error",
};

/** Agent CLI update status, loaded once and then kept current by events. */
export function useAgentUpdates() {
  const updates = useStudio((state) => state.agentUpdates);
  useEffect(() => {
    if (updates) return;
    api
      .agentUpdates()
      .then((agentUpdates) => useStudio.setState({ agentUpdates }))
      .catch(() => undefined);
  }, [updates]);
  return updates;
}

/** One provider's installed and latest version, with an Update now button. */
export function ProviderUpdateStatus({ provider }: { provider: ProviderId }) {
  const updates = useAgentUpdates();
  const [starting, setStarting] = useState(false);
  const entry: AgentUpdateEntry | undefined = updates?.providers[provider];
  if (!entry || entry.status === "disabled" || entry.status === "missing")
    return null;
  const busy =
    starting || entry.status === "updating" || entry.status === "waiting";
  const canUpdate =
    entry.available && (entry.method === "npm" || entry.method === "native");
  const update = async () => {
    setStarting(true);
    try {
      useStudio.setState({ agentUpdates: await api.updateAgent(provider) });
    } catch (error) {
      toast.error("Couldn't update", (error as Error).message);
    } finally {
      setStarting(false);
    }
  };
  return (
    <div
      className="mt-3 rounded-lg border px-3 py-2.5"
      aria-label={`${provider} CLI updates`}
    >
      <div className="flex items-center gap-2">
        <Badge tone={TONES[entry.status] ?? "neutral"}>
          {entry.status === "updating" && <Spinner />}
          {LABELS[entry.status]}
        </Badge>
        <p className="min-w-0 truncate text-xs text-muted-foreground">
          {entry.installed ? `Installed ${entry.installed}` : "Installed"}
          {entry.latest ? ` · Latest ${entry.latest}` : ""}
        </p>
        {(canUpdate || busy) && (
          <Button
            size="xs"
            variant="outline"
            className="ms-auto"
            disabled={busy}
            onClick={() => void update()}
          >
            {busy && <Spinner />} Update now
          </Button>
        )}
      </div>
      {entry.message && entry.status !== "current" && (
        <p className="mt-1.5 text-xs text-muted-foreground" role="status">
          {entry.message}
        </p>
      )}
      {entry.status === "error" && entry.output && (
        <details className="mt-1.5 text-xs">
          <summary className="cursor-pointer text-muted-foreground">
            Update output
          </summary>
          <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap rounded bg-code p-2 font-mono">
            {entry.output}
          </pre>
        </details>
      )}
    </div>
  );
}

/** The automatic-update switch and a manual check for every agent CLI. */
export function AgentUpdatesSection() {
  const updates = useAgentUpdates();
  const auto = useStudio(
    (state) => state.agents?.settings.auto_update_agents ?? true,
  );
  const [requested, setRequested] = useState(false);
  const checking = requested || !!updates?.checking;
  const check = async () => {
    setRequested(true);
    try {
      useStudio.setState({ agentUpdates: await api.checkAgentUpdates() });
    } catch (error) {
      toast.error("Couldn't check for updates", (error as Error).message);
    } finally {
      setRequested(false);
    }
  };
  const toggle = async (auto_update_agents: boolean) => {
    try {
      await api.saveAgentSettings({ auto_update_agents });
      await useStudio.getState().loadAgents();
    } catch (error) {
      toast.error("Couldn't save settings", (error as Error).message);
    }
  };
  const checked = relativeTime(updates?.last_check);
  return (
    <SettingsSection
      title="Updates"
      description={
        checked
          ? `Last checked ${
              checked === "now"
                ? "just now"
                : /^\d+[mhd]$/.test(checked)
                  ? `${checked} ago`
                  : `on ${checked}`
            }.`
          : undefined
      }
      action={
        <Button
          size="sm"
          variant="outline"
          disabled={checking}
          onClick={() => void check()}
        >
          {checking ? <Spinner /> : <RefreshCwIcon />} Check for updates
        </Button>
      }
    >
      <SettingsRow
        title="Update agent CLIs automatically"
        description="Checks once a day and updates npm or native installs while no chat is using them."
      >
        <Switch
          aria-label="Update agent CLIs automatically"
          checked={auto}
          onCheckedChange={(value) => void toggle(value)}
        />
      </SettingsRow>
    </SettingsSection>
  );
}
