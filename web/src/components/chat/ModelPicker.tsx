import { Link } from "@tanstack/react-router";
import { ChevronDownIcon, SettingsIcon } from "lucide-react";

import type { ProviderId, ProviderStatus } from "~/lib/api";
import { effortInfo, effortName, modelName } from "~/lib/models";
import { useStudio } from "~/state/studio";
import { ProviderIcon } from "../icons";
import { Badge } from "../ui/input";
import {
  Menu,
  MenuItem,
  MenuLabel,
  MenuPopup,
  MenuRadioGroup,
  MenuRadioItem,
  MenuSeparator,
  MenuTrigger,
} from "../ui/menu";
import { Select } from "../ui/select";

export interface ModelChoice {
  provider: ProviderId;
  model: string;
  effort: string;
}

const EMPTY_PROVIDERS: ProviderStatus[] = [];

/** The one set of words for a provider's state, shared by chat, settings and setup. */
export function providerStateLabel(status: ProviderStatus | undefined) {
  if (!status || status.status === "checking") return "Checking";
  if (status.status === "disabled" || !status.enabled) return "Disabled";
  if (!status.installed) return "Not installed";
  if (status.auth.status === "unauthenticated") return "Login needed";
  if (status.status === "ready") return "Ready";
  return "Needs attention";
}

export function providerReadiness(status: ProviderStatus | undefined) {
  const ready = providerStateLabel(status) === "Ready";
  return { ready, label: ready ? "" : providerStateLabel(status) };
}

export function ModelPicker({
  value,
  onChange,
  lockedProvider,
  disabled = false,
}: {
  value: ModelChoice;
  onChange: (choice: ModelChoice) => void;
  lockedProvider?: ProviderId;
  disabled?: boolean;
}) {
  const providers = useStudio(
    (state) => state.agents?.providers ?? EMPTY_PROVIDERS,
  );
  const settings = useStudio((state) => state.agents?.settings);
  const current = providers.find((provider) => provider.id === value.provider);
  const models = current?.models ?? [];
  const efforts =
    models.find((model) => model.id === value.model)?.efforts ?? [];
  const label = modelName(providers, value.provider, value.model);
  const effort = effortInfo(
    providers,
    value.provider,
    value.model,
    value.effort,
  );

  return (
    <div className="flex min-w-0 items-center gap-0.5">
      <Menu>
        <MenuTrigger
          disabled={disabled}
          aria-label={`Choose agent and model: ${label}`}
          className="flex h-8 min-w-0 items-center gap-1.5 rounded-md px-2 text-ui text-muted-foreground outline-none transition-[color,background-color] hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[popup-open]:bg-accent data-[popup-open]:text-foreground"
        >
          <ProviderIcon provider={value.provider} className="size-3.5" />
          <span className="truncate">{label}</span>
          <ChevronDownIcon className="size-3 shrink-0 opacity-60" />
        </MenuTrigger>
        <MenuPopup side="top" className="w-80">
          <MenuRadioGroup
            value={`${value.provider}:${value.model}`}
            onValueChange={(next: string) => {
              const [provider, ...rest] = next.split(":");
              const model = rest.join(":");
              const status = providers.find((item) => item.id === provider);
              const meta = status?.models.find((item) => item.id === model);
              const effort = meta?.efforts.includes(value.effort)
                ? value.effort
                : "";
              onChange({ provider: provider as ProviderId, model, effort });
            }}
          >
            {providers.map((provider, index) => {
              const readiness = providerReadiness(provider);
              const locked = !!lockedProvider && lockedProvider !== provider.id;
              const configured = settings?.providers[provider.id]?.model ?? "";
              return (
                <div key={provider.id}>
                  {index > 0 && <MenuSeparator />}
                  <MenuLabel className="flex items-center gap-1.5">
                    <ProviderIcon provider={provider.id} className="size-3" />
                    {provider.name}
                    {readiness.label && (
                      <Badge
                        tone="warning"
                        className="ms-auto h-4 px-1.5 text-3xs"
                      >
                        {readiness.label}
                      </Badge>
                    )}
                    {locked && !readiness.label && (
                      <span className="ms-auto font-normal text-3xs">
                        New chat to switch
                      </span>
                    )}
                  </MenuLabel>
                  {!configured && (
                    <MenuRadioItem
                      value={`${provider.id}:`}
                      disabled={!readiness.ready || locked}
                    >
                      Default model
                    </MenuRadioItem>
                  )}
                  {provider.models.map((model) => (
                    <MenuRadioItem
                      key={model.id}
                      value={`${provider.id}:${model.id}`}
                      disabled={!readiness.ready || locked}
                    >
                      <span className="flex min-w-0 flex-col">
                        <span className="flex items-center gap-1.5">
                          <span className="truncate">{model.name}</span>
                          {model.id === configured && (
                            <Badge className="h-4 px-1.5 text-3xs">
                              Default
                            </Badge>
                          )}
                          {model.custom && (
                            <Badge className="h-4 px-1.5 text-3xs">
                              Custom
                            </Badge>
                          )}
                        </span>
                        {model.description && (
                          <span className="truncate text-2xs text-muted-foreground">
                            {model.description}
                          </span>
                        )}
                        <span className="font-mono text-3xs text-muted-foreground">
                          {model.id}
                        </span>
                      </span>
                    </MenuRadioItem>
                  ))}
                </div>
              );
            })}
          </MenuRadioGroup>
          <MenuSeparator />
          <MenuItem render={<Link to="/settings/providers" />}>
            <SettingsIcon /> Manage providers
          </MenuItem>
        </MenuPopup>
      </Menu>
      {efforts.length > 0 && (
        <Select
          disabled={disabled}
          aria-label="Reasoning effort"
          size="xs"
          variant="ghost"
          value={value.effort}
          onChange={(effort) => onChange({ ...value, effort })}
          options={[
            {
              value: "",
              label: effort.isDefault
                ? `Effort: ${effort.label}`
                : "Default effort",
            },
            ...efforts.map((level) => ({
              value: level,
              label: effortName(level),
            })),
          ]}
        />
      )}
    </div>
  );
}

/** The model a new chat should start with, from settings and what is installed. */
export function defaultChoice(): ModelChoice {
  const agents = useStudio.getState().agents;
  const ready = agents?.providers.find(
    (provider) => providerReadiness(provider).ready,
  )?.id;
  const provider: ProviderId =
    agents?.settings.default_provider || ready || "codex";
  const config = agents?.settings.providers[provider];
  return { provider, model: config?.model ?? "", effort: config?.effort ?? "" };
}
