export type ThemePreference = "dark" | "light" | "system";

const media = window.matchMedia("(prefers-color-scheme: dark)");

export function normalizeTheme(value: string | undefined): ThemePreference {
  return value === "light" || value === "system" ? value : "dark";
}

export function applyTheme(preference: ThemePreference) {
  const dark =
    preference === "dark" || (preference === "system" && media.matches);
  const root = document.documentElement;
  // Switch instantly; animating every surface at once looks like a flicker.
  root.classList.add("[&_*]:!transition-none");
  root.classList.toggle("dark", dark);
  window.requestAnimationFrame(() =>
    root.classList.remove("[&_*]:!transition-none"),
  );
  try {
    localStorage.setItem("studio-theme", preference);
  } catch {
    // Storage can be unavailable; the server preference still applies next launch.
  }
}

export function watchSystemTheme(getPreference: () => ThemePreference) {
  const listener = () => {
    if (getPreference() === "system") applyTheme("system");
  };
  media.addEventListener("change", listener);
  return () => media.removeEventListener("change", listener);
}
