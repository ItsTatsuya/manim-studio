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

let bundledFonts: Promise<void> | null = null;

/** Register the fonts Studio ships so style previews draw them as renders do. */
export function loadBundledFonts() {
  bundledFonts ??= api
    .fonts()
    .then(async (list) => {
      await Promise.all(
        (list.bundled ?? []).map(async (font) => {
          const face = new FontFace(font.family, `url("${font.url}")`, {
            weight: "100 900",
          });
          document.fonts.add(await face.load());
        }),
      );
    })
    .catch(() => {
      bundledFonts = null;
    });
  return bundledFonts;
}

/** Subscribe to the library, loading it when first shown. */
export function useStyleLibrary() {
  const library = useStyles((state) => state.library);
  useEffect(() => {
    if (!useStyles.getState().library) void useStyles.getState().load();
    void loadBundledFonts();
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

/**
 * Options for a style select; "" follows the default (or parent) style.
 * With `showDefault` the default style itself stands for "" (marked as the
 * default) instead of appearing twice; otherwise `inherit` names "".
 */
export function styleOptions(
  library: StyleLibrary | null,
  inherit: string,
  showDefault = true,
) {
  const styles = library?.styles ?? [];
  const options = styles.map((style) => {
    const isDefault = showDefault && style.id === library?.default;
    return {
      value: isDefault ? "" : style.id,
      label: isDefault ? `${style.name} (default)` : style.name,
      group: style.builtin ? "Built-in" : "Your styles",
    };
  });
  return showDefault || !library
    ? options.length
      ? options
      : [{ value: "", label: inherit }]
    : [{ value: "", label: inherit }, ...options];
}
