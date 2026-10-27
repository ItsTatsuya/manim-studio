import { RefreshCwIcon } from "lucide-react";
import { useState } from "react";
import {
  api,
  type ProviderId,
  type ProviderSettings,
  type ProviderStatus,
} from "~/lib/api";
import { effortName } from "~/lib/models";
import { useStudio } from "~/state/studio";
import { providerStateLabel } from "../chat/ModelPicker";
import { ProviderIcon } from "../icons";
import { Button } from "../ui/button";
import { Field } from "../ui/field";
import { Badge, Input, Spinner, Textarea } from "../ui/input";
import { Select } from "../ui/select";
import { Switch } from "../ui/switch";
import { toast } from "../ui/toast";
import { AgentUpdatesSection, ProviderUpdateStatus } from "./AgentUpdates";
import { SettingsRow, SettingsSection } from "./SettingsLayout";

export function ProvidersSettings() {
  const agents = useStudio((state) => state.agents);
  const [refreshing, setRefreshing] = useState(false);
  const refresh = async () => {
    setRefreshing(true);
    try {
      useStudio.setState({ agents: await api.refreshAgents() });
    } catch (error) {
      toast.error("Couldn't check providers", (error as Error).message);
    } finally {
      setRefreshing(false);
    }
  };
  const changeDefault = async (default_provider: "" | ProviderId) => {
    try {
      await api.saveAgentSettings({ default_provider });
      await useStudio.getState().loadAgents();
    } catch (error) {
      toast.error("Couldn't save settings", (error as Error).message);
    }
  };
  return (
    <>
      <div className="mb-5 flex items-start justify-between gap-4">
        <p className="text-muted-foreground text-sm leading-relaxed">
          Connect your locally installed CLIs. Studio uses their existing login
          and resumes each chat's session.
        </p>
        <Button
          size="sm"
          variant="outline"
          disabled={refreshing || agents?.probing}
          onClick={() => void refresh()}
        >
          {refreshing || agents?.probing ? <Spinner /> : <RefreshCwIcon />}{" "}
          Check again
        </Button>
      </div>
      {!agents ? (
        <Spinner />
      ) : (
        <>
          <div className="mb-8 space-y-4">
            {agents.providers.map((provider) => (
              <ProviderCard
                key={provider.id}
                provider={provider}
                settings={agents.settings.providers[provider.id]}
              />
            ))}
          </div>
          <AgentUpdatesSection />
          <SettingsSection title="Defaults">
            <SettingsRow
              title="Default provider"
              description="Used when you start a new chat."
            >
              <Select
                aria-label="Default provider"
                value={agents.settings.default_provider}
                onChange={(value) => void changeDefault(value)}
                options={[
                  { value: "", label: "First available" },
                  { value: "codex", label: "Codex" },
                  { value: "claude", label: "Claude Code" },
                ]}
              />
            </SettingsRow>
          </SettingsSection>
        </>
      )}
    </>
  );
}

function ProviderCard({
  provider,
  settings,
}: {
  provider: ProviderStatus;
  settings: ProviderSettings;
}) {
  const [draft, setDraft] = useState<Partial<ProviderSettings>>({});
  const [customDraft, setCustomDraft] = useState<string>();
  const value = { ...settings, ...draft };
  const custom = customDraft ?? settings.custom_models.join("\n");
  const [saving, setSaving] = useState(false);
  const save = async () => {
    const submitted = draft;
    const submittedCustom = customDraft;
    setSaving(true);
    try {
      const custom_models = custom
        .split(/[\n,]/)
        .map((item) => item.trim())
        .filter(Boolean);
      if (
        custom_models.some(
          (item) => !/^[A-Za-z0-9][A-Za-z0-9._:/\[\]-]{0,99}$/.test(item),
        )
      )
        throw new Error("Use a model ID on each line, with no spaces.");
      await api.saveAgentSettings({
        providers: { [provider.id]: { ...value, custom_models } },
      });
      await useStudio.getState().loadAgents();
      setDraft((current) => {
        const remaining = { ...current };
        for (const key of Object.keys(submitted) as (keyof ProviderSettings)[])
          if (current[key] === submitted[key]) delete remaining[key];
        return remaining;
      });
      setCustomDraft((current) =>
        current === submittedCustom ? undefined : current,
      );
      toast.success(`${provider.name} settings saved`);
    } catch (error) {
      toast.error("Couldn't save provider", (error as Error).message);
    } finally {
      setSaving(false);
    }
  };
  // Like every other switch in Settings, enabling applies immediately.
  const toggle = async (enabled: boolean) => {
    const previous = value.enabled;
    setDraft((current) => ({ ...current, enabled }));
    try {
      await api.saveAgentSettings({
        providers: { [provider.id]: { enabled } },
      });
      await useStudio.getState().loadAgents();
      setDraft((current) => {
        if (current.enabled !== enabled) return current;
        const { enabled: _saved, ...remaining } = current;
        return remaining;
      });
    } catch (error) {
      setDraft((current) => ({ ...current, enabled: previous }));
      toast.error("Couldn't save provider", (error as Error).message);
    }
  };
  const dirty =
    value.binary !== settings.binary ||
    value.model !== settings.model ||
    value.effort !== settings.effort ||
    custom !== settings.custom_models.join("\n");
  const login = async () => {
    try {
      await api.agentLogin(provider.id);
      toast.success(
        "Login opened",
        "Complete login in the CLI window, then click Check again.",
      );
    } catch (error) {
      toast.error("Couldn't open login", (error as Error).message);
    }
  };
  const model = provider.models.find((model) => model.id === value.model);
  const models = provider.models.map((model) => ({
    value: model.id,
    label: model.name,
  }));
  if (value.model && !models.some((model) => model.value === value.model))
    models.push({ value: value.model, label: value.model });
  return (
    <section className="rounded-xl border bg-card p-4">
      <div className="flex items-center gap-2.5">
        <ProviderIcon provider={provider.id} className="size-5" />
        <h2 className="font-medium text-sm">{provider.name}</h2>
        <Badge
          tone={
            provider.status === "ready"
              ? "success"
              : provider.status === "error"
                ? "error"
                : "neutral"
          }
        >
          {providerStateLabel(provider)}
        </Badge>
        <Switch
          className="ms-auto"
          aria-label={`Enable ${provider.name}`}
          checked={value.enabled}
          disabled={saving}
          onCheckedChange={(enabled) => void toggle(enabled)}
        />
      </div>
      <p className="mt-3 text-xs text-muted-foreground" role="status">
        {provider.message}
      </p>
      {provider.version && (
        <p className="mt-1 text-xs text-muted-foreground">
          {provider.version}
          {provider.auth.account ? ` · ${provider.auth.account}` : ""}
        </p>
      )}
      {provider.installed && <ProviderUpdateStatus provider={provider.id} />}
      {!provider.installed && (
        <div className="mt-3 rounded-lg bg-code p-3">
          <p className="mb-2 text-xs text-muted-foreground">
            Install in PowerShell, then check again.
          </p>
          <code className="break-all text-xs">{provider.install}</code>
          <Button
            size="xs"
            variant="ghost"
            className="ms-2"
            onClick={() =>
              void navigator.clipboard
                .writeText(provider.install)
                .then(() => toast.success("Install command copied"))
                .catch(() => toast.error("Couldn't copy command"))
            }
          >
            Copy
          </Button>
        </div>
      )}
      <div className="mt-4 space-y-3">
        <Field label="CLI binary override">
          <Input
            aria-label={`${provider.name} binary`}
            placeholder="Auto-detect from PATH"
            value={value.binary}
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                binary: event.target.value,
              }))
            }
          />
        </Field>
        <div className="grid grid-cols-2 gap-3">
          <Field label="Default model">
            <Select
              aria-label={`${provider.name} default model`}
              value={value.model}
              onChange={(model) =>
                setDraft((current) => ({ ...current, model }))
              }
              options={[{ value: "", label: "CLI default" }, ...models]}
            />
          </Field>
          <Field label="Reasoning effort">
            <Select
              aria-label={`${provider.name} effort`}
              value={value.effort}
              onChange={(effort) =>
                setDraft((current) => ({ ...current, effort }))
              }
              options={[
                { value: "", label: "Default" },
                ...[
                  ...new Set(
                    [...(model?.efforts ?? []), value.effort].filter(Boolean),
                  ),
                ].map((effort) => ({
                  value: effort,
                  label: effortName(effort),
                })),
              ]}
            />
          </Field>
        </div>
        <Field label="Custom model IDs">
          <Textarea
            aria-label={`${provider.name} custom models`}
            placeholder="One model ID per line"
            className="min-h-16 font-mono text-xs"
            value={custom}
            onChange={(event) => setCustomDraft(event.target.value)}
          />
        </Field>
      </div>
      <div className="mt-4 flex items-center gap-2">
        <Button
          size="sm"
          disabled={saving || !dirty}
          onClick={() => void save()}
        >
          {saving && <Spinner />} Save provider
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={!provider.installed}
          onClick={() => void login()}
        >
          Log in
        </Button>
        <a
          href={provider.docs}
          target="_blank"
          rel="noreferrer"
          className="ms-auto text-xs text-muted-foreground hover:underline"
        >
          Setup documentation ↗
        </a>
      </div>
    </section>
  );
}
