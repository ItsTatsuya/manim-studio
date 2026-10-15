import { create } from "zustand";

import {
  api,
  type Analysis,
  type NarrationSettings,
  type Project,
} from "~/lib/api";
import { useStudio } from "./studio";

type SaveStatus = "saved" | "pending" | "saving" | "error";

interface ProjectDocState {
  id: string | null;
  workspace: string | null;
  project: Project | null;
  source: string;
  narration: NarrationSettings | null;
  analysis: Analysis | null;
  scene: string;
  saveStatus: SaveStatus;
  saveError: string | null;
  /** Increments whenever the source is replaced from outside the editor. */
  revision: number;

  open(id: string, workspace?: string | null): Promise<void>;
  reload(): Promise<void>;
  setSource(source: string): void;
  replaceSource(source: string): void;
  setNarration(changes: Partial<NarrationSettings>): void;
  setScene(scene: string): void;
  flush(): Promise<boolean>;
}

let saveTimer: number | undefined;
let analyzeTimer: number | undefined;
let analyzeRequest = 0;
let dirtySource = false;
let dirtyNarration = false;
let saving: Promise<boolean> | null = null;
let openRequest = 0;

function backupDraft() {
  const state = useProjectDoc.getState();
  if (!state.id || !state.project) return;
  try {
    const key = `studio-draft-v1:${state.id}${state.workspace ? `:${state.workspace}` : ""}`;
    if (!dirtySource && !dirtyNarration && !saving)
      localStorage.removeItem(key);
    else
      localStorage.setItem(
        key,
        JSON.stringify({
          source: state.source,
          narration: state.narration,
          baseSource: state.project.source,
          baseNarration: state.project.narration,
          dirtySource,
          dirtyNarration,
        }),
      );
  } catch {
    /* Saving to disk still works if browser storage is unavailable. */
  }
}

function scheduleSave() {
  backupDraft();
  window.clearTimeout(saveTimer);
  useProjectDoc.setState({ saveStatus: "pending" });
  saveTimer = window.setTimeout(
    () => void useProjectDoc.getState().flush(),
    700,
  );
}

function scheduleAnalysis(immediate = false) {
  window.clearTimeout(analyzeTimer);
  const run = async () => {
    const { source, id } = useProjectDoc.getState();
    const request = ++analyzeRequest;
    if (!source.trim()) {
      useProjectDoc.setState({
        analysis: { source, scenes: [], error: "The script is empty." },
      });
      return;
    }
    try {
      const analysis = await api.analyze(source);
      if (request !== analyzeRequest || useProjectDoc.getState().id !== id)
        return;
      const scene = useProjectDoc.getState().scene;
      useProjectDoc.setState({
        analysis,
        scene: analysis.scenes.includes(scene)
          ? scene
          : (analysis.scenes.at(-1) ?? ""),
      });
    } catch (error) {
      if (request === analyzeRequest)
        useProjectDoc.setState({
          analysis: { source, scenes: [], error: (error as Error).message },
        });
    }
  };
  if (immediate) void run();
  else analyzeTimer = window.setTimeout(run, 350);
}

export const useProjectDoc = create<ProjectDocState>()((set, get) => ({
  id: null,
  workspace: null,
  project: null,
  source: "",
  narration: null,
  analysis: null,
  scene: "",
  saveStatus: "saved",
  saveError: null,
  revision: 0,

  async open(id, workspace = null) {
    if (get().id === id && get().workspace === workspace && get().project)
      return;
    const request = ++openRequest;
    if (!(await get().flush()))
      throw new Error(
        "Your changes couldn't be saved. Retry saving before switching projects.",
      );
    if (request !== openRequest) return;
    window.clearTimeout(analyzeTimer);
    ++analyzeRequest;
    set({
      id,
      workspace,
      project: null,
      source: "",
      narration: null,
      analysis: null,
      scene: "",
    });
    const project = await api.project(id, workspace);
    if (get().id !== id || request !== openRequest) return;
    dirtySource = dirtyNarration = false;
    let recoveredSource = project.source;
    let recoveredNarration = project.narration;
    try {
      const draft = JSON.parse(
        localStorage.getItem(
          `studio-draft-v1:${id}${workspace ? `:${workspace}` : ""}`,
        ) || "null",
      );
      if (
        draft?.dirtySource &&
        draft.baseSource === project.source &&
        typeof draft.source === "string" &&
        draft.source.length <= 500_000
      ) {
        recoveredSource = draft.source;
        dirtySource = true;
      }
      if (
        draft?.dirtyNarration &&
        JSON.stringify(draft.baseNarration) ===
          JSON.stringify(project.narration) &&
        typeof draft.narration?.text === "string" &&
        draft.narration.text.length <= 100_000
      ) {
        recoveredNarration = draft.narration;
        dirtyNarration = true;
      }
    } catch {
      /* Ignore an invalid recovery record. */
    }
    set((state) => ({
      project,
      source: recoveredSource,
      narration: recoveredNarration,
      saveStatus: "saved",
      saveError: null,
      revision: state.revision + 1,
    }));
    scheduleAnalysis(true);
    if (dirtySource || dirtyNarration) scheduleSave();
  },

  /** Take the files on disk, e.g. after an agent edited them. */
  async reload() {
    const id = get().id;
    if (!id) return;
    window.clearTimeout(saveTimer);
    if (saving && !(await saving)) return;
    // Local edits take priority over a delayed file-change notification.
    if (dirtySource || dirtyNarration) return;
    const workspace = get().workspace;
    const project = await api.project(id, workspace);
    if (
      get().id !== id ||
      get().workspace !== workspace ||
      dirtySource ||
      dirtyNarration
    )
      return;
    dirtySource = dirtyNarration = false;
    set((state) => ({
      project,
      source: project.source,
      narration: project.narration,
      saveStatus: "saved",
      revision:
        project.source === state.source ? state.revision : state.revision + 1,
    }));
    scheduleAnalysis(true);
  },

  setSource(source) {
    if (source === get().source) return;
    dirtySource = true;
    set({ source });
    scheduleSave();
    scheduleAnalysis();
  },

  replaceSource(source) {
    dirtySource = true;
    set((state) => ({ source, revision: state.revision + 1 }));
    scheduleSave();
    scheduleAnalysis(true);
  },

  setNarration(changes) {
    const narration = get().narration;
    if (!narration) return;
    dirtyNarration = true;
    set({ narration: { ...narration, ...changes } });
    scheduleSave();
  },

  setScene(scene) {
    set({ scene });
  },

  async flush() {
    window.clearTimeout(saveTimer);
    if (saving) {
      if (!(await saving)) return false;
      return get().flush();
    }
    const { id, workspace, source, narration } = get();
    if (!id || (!dirtySource && !dirtyNarration)) {
      if (get().saveStatus === "pending") set({ saveStatus: "saved" });
      return true;
    }
    const changes: { source?: string; narration?: NarrationSettings } = {};
    if (dirtySource) changes.source = source;
    if (dirtyNarration && narration) changes.narration = narration;
    dirtySource = dirtyNarration = false;
    if (!Object.keys(changes).length) {
      set({ saveStatus: "saved" });
      return true;
    }
    set({ saveStatus: "saving" });
    saving = (async () => {
      try {
        const summary = await api.updateProject(id, changes, workspace);
        if (get().id === id && get().project)
          set({
            project: {
              ...get().project!,
              ...summary,
              ...(changes.source !== undefined
                ? { source: changes.source }
                : {}),
              ...(changes.narration ? { narration: changes.narration } : {}),
            },
          });
        if (get().id === id)
          set({
            saveStatus: dirtySource || dirtyNarration ? "pending" : "saved",
            saveError: null,
          });
        if (!workspace)
          useStudio.setState((state) => ({
            projects: state.projects.map((item) =>
              item.id === id ? { ...item, ...summary } : item,
            ),
          }));
        return true;
      } catch (error) {
        if (changes.source !== undefined) dirtySource = true;
        if (changes.narration) dirtyNarration = true;
        set({ saveStatus: "error", saveError: (error as Error).message });
        return false;
      }
    })();
    const result = await saving;
    saving = null;
    backupDraft();
    if (result && (dirtySource || dirtyNarration)) return get().flush();
    return result;
  },
}));

window.addEventListener(
  "beforeunload",
  () => void useProjectDoc.getState().flush(),
);
