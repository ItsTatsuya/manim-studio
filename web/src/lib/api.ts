/** Typed client for the local Studio server. Mutations carry the window token. */

const token =
  document.querySelector<HTMLMetaElement>('meta[name="studio-token"]')
    ?.content ?? "";

class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

const FIELD_LABELS: Record<string, string> = {
  width: "Width",
  height: "Height",
  fps: "Frame rate",
};

async function errorMessage(response: Response) {
  try {
    const body = await response.json();
    if (typeof body.detail === "string") return body.detail;
    if (Array.isArray(body.detail)) {
      const details = body.detail
        .filter((item: { msg?: unknown }) => typeof item?.msg === "string")
        .slice(0, 4)
        .map((item: { msg: string; loc?: unknown[] }) => {
          const field = Array.isArray(item.loc)
            ? FIELD_LABELS[String(item.loc.at(-1))]
            : undefined;
          const detail = item.msg.replace(/^Value error, /, "");
          return field ? `${field}: ${detail}` : detail;
        });
      if (details.length) return details.join(" ");
    }
  } catch {
    // Fall through to the generic message.
  }
  return response.status === 403
    ? "This request must come from your Studio window. Reload Studio."
    : "Studio couldn't complete that action. Please try again.";
}

async function request<T>(
  path: string,
  options: {
    method?: string;
    body?: unknown;
    raw?: BodyInit;
    headers?: Record<string, string>;
  } = {},
): Promise<T> {
  const method =
    options.method ??
    (options.body === undefined && options.raw === undefined ? "GET" : "POST");
  const headers: Record<string, string> = { ...options.headers };
  if (method !== "GET") headers["X-Studio-Token"] = token;
  if (options.body !== undefined) headers["Content-Type"] = "application/json";
  const response = await fetch(path, {
    method,
    headers,
    body:
      options.raw ??
      (options.body === undefined ? undefined : JSON.stringify(options.body)),
  });
  if (!response.ok)
    throw new ApiError(await errorMessage(response), response.status);
  const type = response.headers.get("content-type") ?? "";
  return (
    type.includes("application/json") ? response.json() : response.blob()
  ) as Promise<T>;
}

// ------------------------------------------------------------------ types

export interface Health {
  manim: string;
  python: string;
  latex: boolean;
  typst: boolean;
  active: string | null;
  outputs: string;
  standalone: boolean;
  native_file_picker?: boolean;
  portable: boolean;
  data: string;
  version: string;
}

export interface NarrationSettings {
  enabled: boolean;
  text: string;
  voice: string;
  speed: number;
  quality: "standard" | "high";
}

export interface ProjectSummary {
  id: string;
  name: string;
  created: string;
  updated: string;
  folder: string;
  narration: NarrationSettings;
}

export interface Project extends ProjectSummary {
  source: string;
  warnings?: string[];
}

export interface Asset {
  path: string;
  size: number;
  kind: "image" | "vector" | "audio" | "video" | "file";
}

export interface Analysis {
  source: string;
  scenes: string[];
  error: string | null;
  line?: number;
  needs_latex?: boolean;
  needs_typst?: boolean;
}

export type Quality = "preview" | "standard" | "final";

export interface Render {
  id: string;
  name: string;
  folder?: string;
  scene: string;
  quality: Quality;
  created: string;
  status: "queued" | "rendering" | "completed" | "failed" | "cancelled";
  stage?: string;
  elapsed?: number;
  error?: string;
  problem?: { line: number; message: string };
  warnings?: string[];
  video?: string;
  resolution?: string;
  width?: number;
  height?: number;
  fps?: number;
  size?: number;
  project?: string;
  project_name?: string;
  narrated?: boolean;
  can_cancel?: boolean;
  log?: string;
}

interface Voice {
  id: string;
  name: string;
  language: string;
  gender: string;
}

export interface NarrationStatus {
  voices: Voice[];
  models: Record<
    "standard" | "high",
    { installed: boolean; download_size: number }
  >;
  download: {
    status: "idle" | "downloading" | "ready" | "failed" | "cancelled";
    quality: string | null;
    done: number;
    total: number;
    error?: string | null;
  };
}

export type ProviderId = "codex" | "claude";

export interface ProviderModel {
  id: string;
  name: string;
  description?: string;
  efforts: string[];
  default_effort?: string | null;
  custom?: boolean;
  alias?: boolean;
}

export interface ProviderStatus {
  id: ProviderId;
  name: string;
  enabled: boolean;
  installed: boolean;
  binary?: string | null;
  version?: string | null;
  status: "ready" | "warning" | "error" | "disabled" | "checking";
  message: string;
  auth: {
    status: "authenticated" | "unauthenticated" | "unknown";
    label?: string;
    account?: string | null;
  };
  models: ProviderModel[];
  install: string;
  docs: string;
  checked?: string;
}

export interface ProviderSettings {
  enabled: boolean;
  binary: string;
  model: string;
  effort: string;
  custom_models: string[];
}

export interface AgentSettings {
  providers: Record<ProviderId, ProviderSettings>;
  default_provider: "" | ProviderId;
  access: "full" | "edits";
  auto_render: boolean;
}

export interface AgentSnapshot {
  providers: ProviderStatus[];
  settings: AgentSettings;
  probing: boolean;
}

type ThreadStatus = "idle" | "running" | "completed" | "failed" | "interrupted";

export interface ThreadSummary {
  id: string;
  project: string;
  title: string;
  provider: ProviderId;
  model: string;
  effort: string;
  created: string;
  updated: string;
  status: ThreadStatus;
  locked: boolean;
  forked_from?: { thread: string; title: string; turn: string } | null;
}

export interface ThreadItem {
  id: string;
  turn: string;
  created: string;
  kind:
    | "user"
    | "assistant"
    | "reasoning"
    | "command"
    | "file_change"
    | "tool"
    | "plan"
    | "error"
    | "notice";
  text?: string;
  title?: string;
  detail?: string;
  status?: "in_progress" | "completed" | "failed";
  streaming?: boolean;
  files?: { path: string; kind: string }[];
  entries?: { text: string; done: boolean }[];
  /** Stable machine-readable failure code on `error` items. */
  code?: string;
  hint?: string;
  reset?: string;
  http_status?: number;
}

export interface Turn {
  id: string;
  status: ThreadStatus;
  provider: ProviderId;
  model: string;
  effort: string;
  started: string;
  finished: string | null;
  changed: string[];
  error: string | null;
  error_code?: string | null;
}

export interface Thread extends ThreadSummary {
  items: ThreadItem[];
  turns: Turn[];
}

export interface UpdateState {
  status:
    | "idle"
    | "checking"
    | "current"
    | "available"
    | "downloading"
    | "ready"
    | "installing"
    | "error";
  progress: number;
  bytes_downloaded: number;
  total_bytes: number;
  error: string | null;
  current_version: string;
  edition: "portable" | "installed" | "source";
  latest_version: string | null;
  release_url: string;
  last_checked: number | null;
  can_download: boolean;
  can_install: boolean;
  can_cancel: boolean;
}

// ------------------------------------------------------------------ calls

export const api = {
  health: () => request<Health>("/api/health"),
  preferences: () => request<Record<string, string>>("/api/preferences"),
  savePreferences: (values: Record<string, string>) =>
    request("/api/preferences", { body: { values } }),

  projects: () => request<{ projects: ProjectSummary[] }>("/api/projects"),
  project: (id: string) => request<Project>(`/api/projects/${id}`),
  createProject: (name: string, source = "") =>
    request<Project>("/api/projects", { body: { name, source } }),
  updateProject: (
    id: string,
    changes: { name?: string; source?: string; narration?: NarrationSettings },
  ) => request<ProjectSummary>(`/api/projects/${id}`, { body: changes }),
  deleteProject: (id: string) =>
    request<{ ok: true; replacement: Project | null }>(`/api/projects/${id}`, {
      method: "DELETE",
    }),
  importProject: (form: FormData) =>
    request<Project>("/api/projects/import", { raw: form }),
  openProjectFolder: (id: string) =>
    request(`/api/projects/${id}/open-folder`, { method: "POST" }),
  openProjectAssets: (id: string) =>
    request(`/api/projects/${id}/open-assets`, { method: "POST" }),
  assets: (id: string) =>
    request<{ assets: Asset[] }>(`/api/projects/${id}/assets`),
  uploadAsset: (id: string, file: File) =>
    request<Asset>(`/api/projects/${id}/assets`, {
      raw: file,
      headers: {
        "X-Filename": encodeURIComponent(file.name),
        "Content-Type": "application/octet-stream",
      },
    }),
  chooseAssets: (id: string, kind: "documents" | "media") =>
    request<{ assets: Asset[]; errors: { file: string; message: string }[] }>(
      `/api/projects/${id}/assets/choose`,
      { body: { kind } },
    ),
  deleteAsset: (id: string, path: string) =>
    request(
      `/api/projects/${id}/assets/${path.split("/").map(encodeURIComponent).join("/")}`,
      { method: "DELETE" },
    ),
  assetUrl: (id: string, path: string) =>
    `/api/projects/${id}/assets/${path.split("/").map(encodeURIComponent).join("/")}`,

  analyze: (source: string) =>
    request<Analysis>("/api/analyze", { body: { source } }),
  format: (source: string) =>
    request<{ source: string }>("/api/format", { body: { source } }),

  renders: () => request<Render[]>("/api/renders"),
  render: (id: string) => request<Render>(`/api/renders/${id}`),
  startRender: (body: Record<string, unknown>) =>
    request<Render>("/api/render", { body }),
  cancelRender: (id: string) =>
    request(`/api/renders/${id}/cancel`, { method: "POST" }),
  openOutputs: () => request("/api/open/outputs", { method: "POST" }),

  narration: () => request<NarrationStatus>("/api/narration"),
  downloadVoice: (quality: string) =>
    request<NarrationStatus>("/api/narration/download", { body: { quality } }),
  cancelVoiceDownload: () =>
    request<NarrationStatus>("/api/narration/cancel", { method: "POST" }),
  previewVoice: (settings: NarrationSettings) =>
    request<Blob>("/api/narration/preview", { body: settings }),

  agents: () => request<AgentSnapshot>("/api/agents"),
  refreshAgents: () =>
    request<AgentSnapshot>("/api/agents/refresh", { method: "POST" }),
  saveAgentSettings: (
    changes: Partial<Omit<AgentSettings, "providers">> & {
      providers?: Partial<Record<ProviderId, Partial<ProviderSettings>>>;
    },
  ) => request<AgentSettings>("/api/agents/settings", { body: changes }),
  agentLogin: (provider: ProviderId) =>
    request(`/api/agents/${provider}/login`, { method: "POST" }),
  agentInstall: (provider: ProviderId) =>
    request(`/api/agents/${provider}/install`, { method: "POST" }),

  threads: (projectId: string) =>
    request<{ threads: ThreadSummary[] }>(`/api/projects/${projectId}/threads`),
  thread: (projectId: string, threadId: string) =>
    request<Thread>(`/api/projects/${projectId}/threads/${threadId}`),
  createThread: (
    projectId: string,
    body: {
      provider?: ProviderId;
      model?: string;
      effort?: string;
      title?: string;
    },
  ) => request<Thread>(`/api/projects/${projectId}/threads`, { body }),
  updateThread: (
    projectId: string,
    threadId: string,
    body: {
      title?: string;
      provider?: ProviderId;
      model?: string;
      effort?: string;
    },
  ) =>
    request<ThreadSummary>(`/api/projects/${projectId}/threads/${threadId}`, {
      body,
    }),
  forkThread: (
    projectId: string,
    threadId: string,
    body: {
      turn?: string;
      title?: string;
      provider?: ProviderId;
      model?: string;
      effort?: string;
    } = {},
  ) =>
    request<Thread>(`/api/projects/${projectId}/threads/${threadId}/fork`, {
      body,
    }),
  deleteThread: (projectId: string, threadId: string) =>
    request(`/api/projects/${projectId}/threads/${threadId}`, {
      method: "DELETE",
    }),
  sendTurn: (
    projectId: string,
    threadId: string,
    body: { text: string; context?: string; model?: string; effort?: string },
  ) =>
    request<Turn>(`/api/projects/${projectId}/threads/${threadId}/turns`, {
      body,
    }),
  interrupt: (projectId: string, threadId: string) =>
    request(`/api/projects/${projectId}/threads/${threadId}/interrupt`, {
      method: "POST",
    }),

  updates: () => request<UpdateState>("/api/updates"),
  checkUpdates: () =>
    request<UpdateState>("/api/updates/check", { method: "POST" }),
  downloadUpdate: () =>
    request<UpdateState>("/api/updates/download", { method: "POST" }),
  cancelUpdate: () =>
    request<UpdateState>("/api/updates/cancel", { method: "POST" }),
  installUpdate: () =>
    request<UpdateState>("/api/updates/install", { method: "POST" }),
};
