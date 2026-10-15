import { type CxOptions, cx } from "class-variance-authority";
import { extendTailwindMerge } from "tailwind-merge";

// Register the extra font sizes from index.css so cn() keeps them.
const twMerge = extendTailwindMerge({
  extend: { theme: { text: ["2xs", "3xs", "ui"] } },
});

export function cn(...inputs: CxOptions) {
  return twMerge(cx(inputs));
}

export function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

export function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds)) return "";
  if (seconds < 60)
    return `${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}m ${Math.round(seconds % 60)}s`;
}

/** Compact "2h", "3d" style ages, as in T3 Code's sidebar. */
export function relativeTime(iso: string | null | undefined) {
  if (!iso) return "";
  const elapsed = (Date.now() - new Date(iso).getTime()) / 1000;
  if (!Number.isFinite(elapsed)) return "";
  if (elapsed < 60) return "now";
  if (elapsed < 3600) return `${Math.floor(elapsed / 60)}m`;
  if (elapsed < 86400) return `${Math.floor(elapsed / 3600)}h`;
  if (elapsed < 86400 * 30) return `${Math.floor(elapsed / 86400)}d`;
  return new Date(iso).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}
