import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import {
  api,
  type Analysis,
  type Health,
  type Quality,
  type Render,
  type VideoWorkflow,
} from "./api";

export interface OutputPreset {
  id: string;
  label: string;
  detail: string;
  quality: Quality;
  width: number;
  height: number;
  fps: number;
}

export const OUTPUT_PRESETS: OutputPreset[] = [
  {
    id: "preview",
    label: "Draft",
    detail: "480p · 15 fps",
    quality: "preview",
    width: 854,
    height: 480,
    fps: 15,
  },
  {
    id: "standard",
    label: "Standard",
    detail: "720p · 30 fps",
    quality: "standard",
    width: 1280,
    height: 720,
    fps: 30,
  },
  {
    id: "final",
    label: "High",
    detail: "1080p · 30 fps",
    quality: "final",
    width: 1920,
    height: 1080,
    fps: 30,
  },
  {
    id: "4k",
    label: "4K",
    detail: "2160p · 30 fps",
    quality: "final",
    width: 3840,
    height: 2160,
    fps: 30,
  },
  {
    id: "vertical",
    label: "Vertical",
    detail: "1080×1920 · 30 fps",
    quality: "standard",
    width: 1080,
    height: 1920,
    fps: 30,
  },
];

export function currentPreset(prefs: Record<string, string>): OutputPreset {
  const quality = prefs["manim-quality"];
  const resolution = prefs["manim-resolution"];
  const fps = Number(prefs["manim-fps"]);
  const match = OUTPUT_PRESETS.find(
    (preset) =>
      (resolution
        ? `${preset.width}x${preset.height}` === resolution
        : preset.quality === quality) &&
      (!fps || preset.fps === fps),
  );
  if (match) return match;
  const [width, height] = (resolution || "").split("x").map(Number);
  if (
    Number.isInteger(width) &&
    Number.isInteger(height) &&
    width > 0 &&
    height > 0 &&
    width <= 32766 &&
    height <= 32766 &&
    width % 2 === 0 &&
    height % 2 === 0 &&
    Number.isFinite(fps) &&
    fps > 0
  ) {
    return {
      id: "custom",
      label: "Custom",
      detail: `${width}×${height} · ${fps} fps`,
      quality: "standard",
      width,
      height,
      fps,
    };
  }
  return OUTPUT_PRESETS[1];
}

export function renderProblem(
  analysis: Analysis | null,
  health: Health | null,
): string | null {
  if (!analysis) return "Checking the script…";
  if (analysis.error) return analysis.error;
  if (analysis.needs_latex && health && !health.latex)
    return "This scene needs LaTeX and dvisvgm. Use MathTypst or install LaTeX.";
  if (analysis.needs_typst && health && !health.typst)
    return "Typst is unavailable in this runtime.";
  return null;
}

export function choosePreset(preset: OutputPreset) {
  const { setPref } = useStudio.getState();
  setPref("manim-quality", preset.quality);
  setPref("manim-resolution", `${preset.width}x${preset.height}`);
  setPref("manim-fps", String(preset.fps));
}

export function activeRender(renders: Render[]) {
  return (
    renders.find(
      (render) => render.status === "queued" || render.status === "rendering",
    ) ?? null
  );
}

export function latestRender(renders: Render[], projectId: string | undefined) {
  return renders.find((render) => render.project === projectId) ?? null;
}

/** Render a project: the open editor's text when it is the current project, otherwise what is on disk. */
export async function renderProject(
  projectId: string,
  options: {
    narration?: boolean;
    workspace?: string | null;
    thread?: string;
    repair?: boolean;
    render?: string;
  } = {},
): Promise<VideoWorkflow> {
  const doc = useProjectDoc.getState();
  const workspace =
    options.workspace === undefined
      ? doc.id === projectId
        ? doc.workspace
        : null
      : options.workspace;
  const open =
    doc.id === projectId && doc.workspace === workspace && doc.project;
  if (open && !(await doc.flush()))
    throw new Error("Save your project changes before rendering.");
  const project = open
    ? { ...doc.project!, source: doc.source, narration: doc.narration! }
    : await api.project(projectId, workspace);
  const preset = currentPreset(useStudio.getState().prefs);
  const custom = preset.id !== preset.quality;
  const video = await api.startVideo(projectId, {
    workspace,
    thread: options.thread,
    repair: options.repair,
    render: options.render,
    scene: open ? doc.scene : "",
    quality: preset.quality,
    narration: {
      ...project.narration,
      enabled: project.narration.enabled && options.narration !== false,
    },
    ...(custom
      ? { width: preset.width, height: preset.height, fps: preset.fps }
      : {}),
  });
  useStudio.getState().upsertVideo(video);
  return video;
}
