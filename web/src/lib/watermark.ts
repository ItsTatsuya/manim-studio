/** Custom watermark settings: the user's logo or text on every finished video. */

import { request } from "./api";

export type WatermarkPosition =
  | "top-left"
  | "top-right"
  | "bottom-left"
  | "bottom-right"
  | "center"
  | "bottom-center";

export interface WatermarkSettings {
  type: "none" | "image" | "text";
  logo: string | null;
  logo_name: string;
  text: string;
  /** Pixels on a 1080-pixel-tall video; other heights scale. */
  font_size: number;
  color: string;
  position: WatermarkPosition;
  /** Percent of the video width. */
  margin: number;
  /** Logo width as a percent of the video width. */
  scale: number;
  opacity: number;
}

export interface ProjectWatermark {
  own: WatermarkSettings | null;
  settings: WatermarkSettings;
  source: "own" | "project" | "default";
  inherited: WatermarkSettings;
  inherited_from: "project" | "default";
}

export interface StoredLogo {
  logo: string;
  name: string;
  width: number;
  height: number;
}

export const DEFAULT_WATERMARK: WatermarkSettings = {
  type: "none",
  logo: null,
  logo_name: "",
  text: "",
  font_size: 40,
  color: "#ffffff",
  position: "bottom-right",
  margin: 3,
  scale: 15,
  opacity: 0.8,
};

export const WATERMARK_POSITIONS: {
  value: WatermarkPosition;
  label: string;
}[] = [
  { value: "top-left", label: "Top left" },
  { value: "top-right", label: "Top right" },
  { value: "center", label: "Centre" },
  { value: "bottom-left", label: "Bottom left" },
  { value: "bottom-center", label: "Bottom centre" },
  { value: "bottom-right", label: "Bottom right" },
];

export const LOGO_ACCEPT = ".png,.jpg,.jpeg,.webp,.gif,.bmp,.svg";
const LOGO_PATTERN = /\.(png|jpe?g|webp|gif|bmp|svg)$/i;

export function isLogoFile(name: string) {
  return LOGO_PATTERN.test(name);
}

/** A problem that would stop the server from saving these settings, if any. */
export function watermarkProblem(settings: WatermarkSettings): string | null {
  if (settings.type === "image" && !settings.logo)
    return "Upload or choose a logo image.";
  if (settings.type === "text" && !settings.text.trim())
    return "Enter the watermark text.";
  return null;
}

export function describeWatermark(settings: WatermarkSettings) {
  if (settings.type === "image")
    return `Logo${settings.logo_name ? ` · ${settings.logo_name}` : ""}`;
  if (settings.type === "text") return `Text · “${settings.text}”`;
  return "No watermark";
}

function workspacePath(path: string, workspace?: string | null) {
  return workspace
    ? `${path}?workspace=${encodeURIComponent(workspace)}`
    : path;
}

export const watermarkApi = {
  defaults: () => request<{ settings: WatermarkSettings }>("/api/watermark"),
  saveDefaults: (settings: WatermarkSettings) =>
    request<{ settings: WatermarkSettings }>("/api/watermark", {
      body: settings,
    }),
  uploadLogo: (file: File) =>
    request<StoredLogo>("/api/watermark/logos", {
      raw: file,
      headers: {
        "X-Filename": encodeURIComponent(file.name),
        "Content-Type": "application/octet-stream",
      },
    }),
  logoUrl: (logo: string) => `/api/watermark/logos/${logo}`,
  project: (projectId: string, workspace?: string | null) =>
    request<ProjectWatermark>(
      workspacePath(`/api/projects/${projectId}/watermark`, workspace),
    ),
  saveProject: (
    projectId: string,
    settings: WatermarkSettings | null,
    workspace?: string | null,
  ) =>
    request<ProjectWatermark>(
      workspacePath(`/api/projects/${projectId}/watermark`, workspace),
      { body: { settings } },
    ),
  logoFromAsset: (projectId: string, path: string, workspace?: string | null) =>
    request<StoredLogo>(
      workspacePath(`/api/projects/${projectId}/watermark/logo`, workspace),
      { body: { path } },
    ),
};
