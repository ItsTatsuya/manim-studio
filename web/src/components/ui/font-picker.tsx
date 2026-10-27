import { Autocomplete } from "@base-ui/react/autocomplete";
import { ChevronDownIcon } from "lucide-react";
import { useEffect, useState } from "react";

import type { FontList } from "~/lib/api";
import { cn } from "~/lib/utils";
import { popupSurface } from "./menu";

interface FontGroup {
  value: string;
  items: string[];
}

/** Studio's fonts, the project's uploads and installed fonts, as list groups. */
export function fontGroups(fonts: FontList | null): FontGroup[] {
  if (!fonts) return [];
  const seen = new Set<string>();
  const unique = (names: string[]) =>
    names.filter((name) => !seen.has(name) && !!seen.add(name));
  return [
    {
      value: "Studio fonts",
      items: unique(fonts.bundled?.map((font) => font.family) ?? []),
    },
    {
      value: "In this project",
      items: unique(fonts.project.map((font) => font.family)),
    },
    { value: "Installed", items: unique(fonts.system) },
  ].filter((group) => group.items.length > 0);
}

/**
 * A font family field with a searchable, grouped list drawn in each font.
 * Typing any name is allowed; the value is committed when a font is picked,
 * on Enter, or when the field loses focus.
 */
export function FontPicker({
  value,
  onCommit,
  fonts,
  placeholder = "Default font",
  disabled,
  className,
  "aria-label": ariaLabel,
}: {
  value: string;
  onCommit: (value: string) => void;
  fonts: FontList | null;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  "aria-label"?: string;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const commit = (next: string) => {
    const family = next.trim();
    if (family !== value) onCommit(family);
  };
  return (
    <Autocomplete.Root
      items={fontGroups(fonts)}
      value={text}
      onValueChange={(next, details) => {
        setText(next);
        if (details.reason === "item-press") commit(next);
      }}
      openOnInputClick
    >
      <div className={cn("relative min-w-0", className)}>
        <Autocomplete.Input
          aria-label={ariaLabel}
          disabled={disabled}
          placeholder={placeholder}
          maxLength={80}
          spellCheck={false}
          onBlur={() => commit(text)}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit(text);
            // Escape abandons the typed text instead of saving it on blur.
            if (event.key === "Escape") setText(value);
          }}
          style={{ fontFamily: text ? `"${text}", inherit` : undefined }}
          className="h-8 w-full min-w-0 rounded-[var(--control-radius)] border border-input bg-background ps-2.5 pe-7 text-foreground text-xs shadow-xs/5 outline-none transition-[box-shadow,border-color] placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:opacity-50 dark:bg-input/30"
        />
        <ChevronDownIcon
          aria-hidden
          className="pointer-events-none absolute end-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
        />
      </div>
      <Autocomplete.Portal>
        <Autocomplete.Positioner
          sideOffset={4}
          collisionPadding={8}
          className="z-[200] outline-none"
        >
          <Autocomplete.Popup
            className={cn(
              popupSurface,
              "w-[max(var(--anchor-width),14rem)] max-w-[var(--available-width)] overflow-hidden p-1",
            )}
          >
            <Autocomplete.Empty className="px-2 py-3 text-muted-foreground text-xs empty:hidden">
              No matching fonts. Press Enter to use “{text.trim()}”.
            </Autocomplete.Empty>
            <Autocomplete.List className="max-h-[min(18rem,var(--available-height))] scroll-py-1 overflow-y-auto overscroll-contain outline-none data-[empty]:p-0">
              {(group: FontGroup) => (
                <Autocomplete.Group
                  key={group.value}
                  items={group.items}
                  className="block pb-1 last:pb-0"
                >
                  <Autocomplete.GroupLabel className="px-2 pt-2 pb-1 font-medium text-2xs text-muted-foreground uppercase tracking-wider">
                    {group.value}
                  </Autocomplete.GroupLabel>
                  <Autocomplete.Collection>
                    {(family: string) => (
                      <Autocomplete.Item
                        key={family}
                        value={family}
                        className="flex min-h-8 cursor-pointer select-none items-center truncate rounded-sm px-2 text-sm outline-none data-[highlighted]:bg-accent"
                        style={{ fontFamily: `"${family}", inherit` }}
                      >
                        {family}
                      </Autocomplete.Item>
                    )}
                  </Autocomplete.Collection>
                </Autocomplete.Group>
              )}
            </Autocomplete.List>
          </Autocomplete.Popup>
        </Autocomplete.Positioner>
      </Autocomplete.Portal>
    </Autocomplete.Root>
  );
}
