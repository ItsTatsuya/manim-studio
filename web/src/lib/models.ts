import type { ProviderId, ProviderModel, ProviderStatus } from "./api";

const LEGACY: Record<string, string> = {
  opus: "Claude Opus (latest)",
  sonnet: "Claude Sonnet (latest)",
  haiku: "Claude Haiku (latest)",
  fable: "Claude Fable (latest)",
};

export function effortName(effort: string) {
  if (effort === "xhigh") return "Extra high";
  return effort ? effort[0].toUpperCase() + effort.slice(1) : "";
}

function findModel(
  providers: ProviderStatus[] | undefined,
  provider: ProviderId,
  model: string,
): ProviderModel | undefined {
  return providers
    ?.find((item) => item.id === provider)
    ?.models.find((item) => item.id === model);
}

/** A readable model name that always includes the version when it is known. */
export function modelName(
  providers: ProviderStatus[] | undefined,
  provider: ProviderId,
  model: string,
) {
  if (!model) return "Default model";
  return findModel(providers, provider, model)?.name ?? LEGACY[model] ?? model;
}

/** The effort the CLI will really use, and whether the user chose it. */
export function effortInfo(
  providers: ProviderStatus[] | undefined,
  provider: ProviderId,
  model: string,
  effort: string,
) {
  const meta = findModel(providers, provider, model);
  const fallback = meta?.default_effort ?? "";
  if (effort) return { label: effortName(effort), isDefault: false };
  if (fallback)
    return { label: `${effortName(fallback)} (default)`, isDefault: true };
  return { label: "Default effort", isDefault: true };
}
