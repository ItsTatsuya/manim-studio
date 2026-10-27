import { useEffect } from "react";
import { create } from "zustand";

import { api, type HouseStyle, type StyleLibrary } from "./api";

/** The house-style library, loaded on first use and refreshed after changes. */
export const useStyles = create<{
  library: StyleLibrary | null;
  error: string | null;
  load(): Promise<StyleLibrary | null>;
  set(library: StyleLibrary): void;
}>()((set) => ({
  library: null,
  error: null,
  async load() {
    try {
      const library = await api.styles();
      set({ library, error: null });
      return library;
    } catch (error) {
      set({ error: (error as Error).message });
      return null;
    }
  },
  set(library) {
    set({ library, error: null });
  },
}));

/** Subscribe to the library, loading it when first shown. */
export function useStyleLibrary() {
  const library = useStyles((state) => state.library);
  useEffect(() => {
    if (!useStyles.getState().library) void useStyles.getState().load();
  }, []);
  return library;
}

export function styleById(
  library: StyleLibrary | null,
  id: string | null | undefined,
): HouseStyle | undefined {
  if (!library) return undefined;
  return (
    library.styles.find((style) => style.id === id) ??
    library.styles.find((style) => style.id === library.default)
  );
}

/** Options for a style select; "" follows the default (or parent) style. */
export function styleOptions(
  library: StyleLibrary | null,
  inherit: string,
  showDefault = true,
) {
  const fallback = showDefault ? styleById(library, null) : undefined;
  return [
    {
      value: "",
      label: fallback ? `${inherit} · ${fallback.name}` : inherit,
    },
    ...(library?.styles ?? []).map((style) => ({
      value: style.id,
      label: style.name,
      group: style.builtin ? "Built-in" : "Your styles",
    })),
  ];
}
