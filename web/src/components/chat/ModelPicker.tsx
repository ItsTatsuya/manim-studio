import { Link } from "@tanstack/react-router";
import { ChevronDownIcon, SettingsIcon } from "lucide-react";

import type { ProviderId, ProviderStatus } from "~/lib/api";
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

export function providerReadiness(status: ProviderStatus | undefined) {
  if (!status || status.status === "checking")
    return { ready: false, label: "Checking…" };
  if (status.status === "disabled") return { ready: false, label: "Off" };
  if (!status.installed) return { ready: false, label: "Not installed" };
  if (status.auth.status === "unauthenticated")
    return { ready: false, label: "Sign in" };
  if (status.status === "error") return { ready: false, label: "Unavailable" };
  return { ready: true, label: "" };
}

function modelLabel(status: ProviderStatus | undefined, model: string) {
  if (!model) return "Default model";
  return status?.models.find((item) => item.id === model)?.name ?? model;
}

export function ModelPicker({
  value,
  onChange,
  lockedProvider,
}: {
  value: ModelChoice;
  onChange: (choice: ModelChoice) => void;
  lockedProvider?: ProviderId;
}) {
  const providers = useStudio(
    (state) => state.agents?.providers ?? EMPTY_PROVIDERS,
  );
  const settings = useStudio((state) => state.agents?.settings);
  const current = providers.find((provider) => provider.id === value.provider);
  const models = current?.models ?? [];
  const efforts =
    models.find((model) => model.id === value.model)?.efforts ?? [];

  return (
    <div className="flex min-w-0 items-center gap-0.5">
      <Menu>
        <MenuTrigger
          aria-label={`Choose agent and model: ${modelLabel(current, value.model)}`}
          className="flex h-7 min-w-0 items-center gap-1.5 rounded-md px-2 text-[13px] text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[popup-open]:bg-accent data-[popup-open]:text-foreground"
        >
          <ProviderIcon provider={value.provider} className="size-3.5" />
          <span className="truncate">{modelLabel(current, value.model)}</span>
          <ChevronDownIcon className="size-3 shrink-0 opacity-60" />
        </MenuTrigger>
        <MenuPopup side="top" className="w-72">
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
                        className="ms-auto h-4 px-1.5 text-[10px]"
                      >
                        {readiness.label}
                      </Badge>
                    )}
                    {locked && !readiness.label && (
                      <span className="ms-auto font-normal text-[10px]">
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
                      <span className="truncate">{model.name}</span>
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
          aria-label="Reasoning effort"
          size="xs"
          variant="ghost"
          value={value.effort}
          onChange={(effort) => onChange({ ...value, effort })}
          options={[
            { value: "", label: "Default effort" },
            ...efforts.map((effort) => ({
              value: effort,
              label:
                effort === "xhigh"
                  ? "Extra high"
                  : effort[0].toUpperCase() + effort.slice(1),
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
