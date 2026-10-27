import { create } from "zustand";

import {
  api,
  type AgentSnapshot,
  type AgentUpdateState,
  type Health,
  type NarrationStatus,
  type ProjectSummary,
  type QueueState,
  type Render,
  type Thread,
  type ThreadItem,
  type ThreadSummary,
  type Turn,
  type UpdateState,
  type VideoWorkflow,
} from "~/lib/api";

interface CompletedTurn {
  project: string;
  thread: string;
  turn: Turn;
}

interface StudioState {
  ready: boolean;
  connected: boolean;
  health: Health | null;
  prefs: Record<string, string>;
  projects: ProjectSummary[];
  threads: Record<string, ThreadSummary[]>;
  details: Record<string, Thread>;
  agents: AgentSnapshot | null;
  agentUpdates: AgentUpdateState | null;
  renders: Render[];
  videos: VideoWorkflow[];
  queue: QueueState | null;
  narration: NarrationStatus | null;
  updates: UpdateState | null;
  installing: boolean;
  /** Bumped when files change on disk so open editors reload. */
  projectVersions: Record<string, number>;
  completedTurn: CompletedTurn | null;

  bootstrap(): Promise<void>;
  setPref(key: string, value: string): void;
  flushPrefs(): Promise<boolean>;
  loadProjects(): Promise<ProjectSummary[]>;
  loadThreads(projectId: string): Promise<ThreadSummary[]>;
  loadThread(projectId: string, threadId: string): Promise<Thread>;
  loadRenders(): Promise<void>;
  loadVideos(): Promise<void>;
  loadQueue(): Promise<void>;
  upsertVideo(video: VideoWorkflow): void;
  loadAgents(): Promise<void>;
  loadNarration(): Promise<void>;
  setUpdates(state: UpdateState): void;
  upsertRender(render: Render): void;
  upsertThreadSummary(summary: ThreadSummary): void;
  removeThread(projectId: string, threadId: string): void;
  handleEvent(event: StudioEvent): void;
}

type StudioEvent =
  | { type: "hello"; version: string }
  | {
      type: "providers";
      providers: AgentSnapshot["providers"];
      settings: AgentSnapshot["settings"];
      probing: boolean;
    }
  | { type: "agent.updates"; updates: AgentUpdateState }
  | { type: "render"; render: Render }
  | { type: "video"; video: VideoWorkflow }
  | { type: "queue"; queue: QueueState }
  | { type: "thread.item"; project: string; thread: string; item: ThreadItem }
  | { type: "thread.turn"; project: string; thread: string; turn: Turn }
  | { type: "thread.meta"; thread: ThreadSummary }
  | { type: "thread.deleted"; project: string; thread: string }
  | {
      type: "project.changed";
      project: string;
      files: string[];
      thread?: string;
    };

function upsertById<T extends { id: string }>(list: T[], value: T) {
  const index = list.findIndex((item) => item.id === value.id);
  if (index < 0) return [...list, value];
  const next = list.slice();
  next[index] = { ...next[index], ...value };
  return next;
}

function mergeThread(remote: Thread, local?: Thread): Thread {
  if (!local) return remote;
  let turns = remote.turns;
  for (const turn of local.turns) {
    const existing = turns.find((item) => item.id === turn.id);
    if (
      !existing ||
      (!existing.finished && turn.finished) ||
      turn.finished === existing.finished
    )
      turns = upsertById(turns, turn);
  }
  turns = [...turns].sort((a, b) => a.started.localeCompare(b.started));
  let items = remote.items;
  for (const item of local.items) {
    const existing = items.find((entry) => entry.id === item.id);
    if (
      !existing ||
      (existing.streaming && !item.streaming) ||
      (item.text?.length ?? 0) >= (existing.text?.length ?? 0)
    )
      items = upsertById(items, item);
  }
  const latest = turns.at(-1);
  const metadata = local.updated > remote.updated ? local : remote;
  return {
    ...metadata,
    turns,
    items,
    status: latest?.status ?? metadata.status,
    locked: local.locked || remote.locked,
  };
}

let savePrefsTimer: number | undefined;
let savingPrefs: Promise<boolean> | null = null;
const pendingPrefs: Record<string, string> = {};

export const useStudio = create<StudioState>()((set, get) => ({
  ready: false,
  connected: false,
  health: null,
  prefs: {},
  projects: [],
  threads: {},
  details: {},
  agents: null,
  agentUpdates: null,
  renders: [],
  videos: [],
  queue: null,
  narration: null,
  updates: null,
  installing: false,
  projectVersions: {},
  completedTurn: null,

  async bootstrap() {
    const [health, prefs, projects] = await Promise.all([
      api.health(),
      api.preferences(),
      api.projects(),
    ]);
    set({ health, prefs, projects: projects.projects, ready: true });
    await Promise.allSettled([
      get().loadAgents(),
      get().loadRenders(),
      get().loadVideos(),
      get().loadQueue(),
      get().loadNarration(),
      api.updates().then((updates) => set({ updates })),
    ]);
    if (health.standalone && prefs["manim-check-updates"] !== "false") {
      void api
        .checkUpdates()
        .then((updates) => set({ updates }))
        .catch(() => undefined);
    }
  },

  setPref(key, value) {
    set((state) => ({ prefs: { ...state.prefs, [key]: value } }));
    pendingPrefs[key] = value;
    window.clearTimeout(savePrefsTimer);
    savePrefsTimer = window.setTimeout(() => void get().flushPrefs(), 300);
  },

  async flushPrefs() {
    window.clearTimeout(savePrefsTimer);
    if (savingPrefs) {
      if (!(await savingPrefs)) return false;
      return get().flushPrefs();
    }
    const values = { ...pendingPrefs };
    if (!Object.keys(values).length) return true;
    savingPrefs = api
      .savePreferences(values)
      .then(() => {
        for (const [key, value] of Object.entries(values))
          if (pendingPrefs[key] === value) delete pendingPrefs[key];
        return true;
      })
      .catch(() => false);
    const result = await savingPrefs;
    savingPrefs = null;
    if (result && Object.keys(pendingPrefs).length) return get().flushPrefs();
    return result;
  },

  async loadProjects() {
    const { projects } = await api.projects();
    set({ projects });
    return projects;
  },

  async loadVideos() {
    const videos = await api.videos();
    for (const video of videos) get().upsertVideo(video);
  },

  async loadQueue() {
    set({ queue: await api.queue() });
  },

  upsertVideo(video) {
    set((state) => {
      const current = state.videos.find((item) => item.id === video.id);
      if (current && current.updated > video.updated) return {};
      return {
        videos: upsertById(state.videos, video).sort((a, b) =>
          b.updated.localeCompare(a.updated),
        ),
      };
    });
  },

  async loadThreads(projectId) {
    const { threads } = await api.threads(projectId);
    set((state) => ({ threads: { ...state.threads, [projectId]: threads } }));
    return threads;
  },

  async loadThread(projectId, threadId) {
    const thread = await api.thread(projectId, threadId);
    set((state) => ({
      details: {
        ...state.details,
        [threadId]: mergeThread(thread, state.details[threadId]),
      },
    }));
    return thread;
  },

  async loadRenders() {
    const renders = await api.renders();
    set({ renders });
  },

  async loadAgents() {
    const agents = await api.agents();
    set({ agents });
  },

  async loadNarration() {
    const narration = await api.narration();
    set({ narration });
  },

  setUpdates(updates) {
    set({ updates });
  },

  upsertRender(render) {
    set((state) => {
      const existing = state.renders.find((item) => item.id === render.id);
      const merged = existing ? { ...existing, ...render } : render;
      const renders = [
        merged,
        ...state.renders.filter((item) => item.id !== render.id),
      ].sort((a, b) => b.created.localeCompare(a.created));
      return { renders };
    });
  },

  upsertThreadSummary(summary) {
    set((state) => {
      const detail = state.details[summary.id];
      const latest = detail?.turns.at(-1);
      const next =
        latest && latest.started > summary.updated
          ? {
              ...summary,
              status: latest.status,
              updated: latest.finished ?? latest.started,
            }
          : summary;
      const list = upsertById(state.threads[summary.project] ?? [], next).sort(
        (a, b) => b.updated.localeCompare(a.updated),
      );
      return {
        threads: { ...state.threads, [summary.project]: list },
        details: detail
          ? {
              ...state.details,
              [summary.id]: {
                ...detail,
                ...next,
                status: latest?.status ?? next.status,
              },
            }
          : state.details,
      };
    });
  },

  removeThread(projectId, threadId) {
    set((state) => {
      const details = { ...state.details };
      delete details[threadId];
      return {
        threads: {
          ...state.threads,
          [projectId]: (state.threads[projectId] ?? []).filter(
            (t) => t.id !== threadId,
          ),
        },
        details,
      };
    });
  },

  handleEvent(event) {
    switch (event.type) {
      case "providers":
        set({
          agents: {
            providers: event.providers,
            settings: event.settings,
            probing: event.probing,
          },
        });
        break;
      case "agent.updates":
        set({ agentUpdates: event.updates });
        break;
      case "render":
        get().upsertRender(event.render);
        break;
      case "video":
        get().upsertVideo(event.video);
        break;
      case "queue":
        set({ queue: event.queue });
        break;
      case "thread.item":
        set((state) => {
          const detail = state.details[event.thread];
          if (!detail) return {};
          return {
            details: {
              ...state.details,
              [event.thread]: {
                ...detail,
                items: upsertById(detail.items, event.item),
              },
            },
          };
        });
        break;
      case "thread.turn":
        set((state) => {
          const detail = state.details[event.thread];
          const completed =
            event.turn.status !== "running"
              ? {
                  completedTurn: {
                    project: event.project,
                    thread: event.thread,
                    turn: event.turn,
                  },
                }
              : {};
          if (!detail) return completed;
          return {
            ...completed,
            details: {
              ...state.details,
              [event.thread]: {
                ...detail,
                ...(() => {
                  const existing = detail.turns.find(
                    (turn) => turn.id === event.turn.id,
                  );
                  const turn =
                    existing?.finished && !event.turn.finished
                      ? existing
                      : event.turn;
                  const turns = upsertById(detail.turns, turn).sort((a, b) =>
                    a.started.localeCompare(b.started),
                  );
                  return {
                    status: turns.at(-1)?.status ?? detail.status,
                    turns,
                    updated: [detail.updated, turn.finished ?? turn.started]
                      .sort()
                      .at(-1)!,
                  };
                })(),
              },
            },
          };
        });
        break;
      case "thread.meta":
        get().upsertThreadSummary(event.thread);
        break;
      case "thread.deleted":
        get().removeThread(event.project, event.thread);
        break;
      case "project.changed":
        set((state) => ({
          projectVersions: {
            ...state.projectVersions,
            [event.project]: (state.projectVersions[event.project] ?? 0) + 1,
          },
        }));
        void get()
          .loadProjects()
          .catch(() => undefined);
        break;
      case "hello":
        break;
    }
  },
}));

/** Keep one EventSource open; on reconnect, resynchronize what may have been missed. */
export function connectEvents() {
  let source: EventSource | null = null;
  let retry: number | undefined;
  const open = () => {
    source = new EventSource("/api/events");
    source.onopen = () => {
      useStudio.setState({ connected: true });
      {
        const state = useStudio.getState();
        void state.loadProjects().catch(() => undefined);
        void state.loadAgents().catch(() => undefined);
        void state.loadRenders().catch(() => undefined);
        void state.loadVideos().catch(() => undefined);
        void state.loadQueue().catch(() => undefined);
        for (const projectId of Object.keys(state.threads))
          void state.loadThreads(projectId);
        for (const thread of Object.values(state.details))
          void state.loadThread(thread.project, thread.id);
      }
    };
    source.onmessage = (message) => {
      try {
        useStudio
          .getState()
          .handleEvent(JSON.parse(message.data) as StudioEvent);
      } catch {
        // Ignore malformed events; the next resync corrects state.
      }
    };
    source.onerror = () => {
      useStudio.setState({ connected: false });
      source?.close();
      window.clearTimeout(retry);
      retry = window.setTimeout(open, 1500);
    };
  };
  open();
  return () => {
    window.clearTimeout(retry);
    source?.close();
  };
}
