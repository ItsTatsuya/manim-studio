import { Link } from "@tanstack/react-router";
import { ChevronDownIcon, RotateCcwIcon, UploadIcon } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";

import {
  api,
  type FontList,
  type HouseStyle,
  type ProjectFonts,
} from "~/lib/api";
import { styleById, useStyleLibrary } from "~/lib/styles";
import { cn } from "~/lib/utils";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { Button } from "../ui/button";
import { FontPicker } from "../ui/font-picker";
import { Popover, PopoverPopup, PopoverTrigger } from "../ui/popover";
import { toast } from "../ui/toast";
import { StyleChips, StyleSwatch } from "./StyleSwatch";

const ROLES = [
  { key: "heading", label: "Headings" },
  { key: "body", label: "Body text" },
  { key: "code", label: "Code" },
] as const;

const loadedFaces = new Set<string>();

/**
 * Installed fonts plus the font files in this project's assets. Uploaded fonts
 * are registered with the page so previews draw them as the render will.
 */
function useProjectFonts(
  projectId: string,
  workspace: string | null,
  open: boolean,
) {
  const [fonts, setFonts] = useState<FontList | null>(null);
  const [version, setVersion] = useState(0);
  useEffect(() => {
    if (!open) return;
    let current = true;
    api
      .fonts(projectId, workspace)
      .then((list) => {
        if (!current) return;
        setFonts(list);
        for (const font of list.project) {
          const key = `${projectId}/${workspace ?? ""}/${font.path}`;
          if (loadedFaces.has(key)) continue;
          loadedFaces.add(key);
          fetch(
            api.assetUrl(
              projectId,
              font.path.replace(/^assets\//, ""),
              workspace,
            ),
          )
            .then((response) => response.arrayBuffer())
            .then((data) => new FontFace(font.family, data).load())
            .then((face) => document.fonts.add(face))
            .catch(() => loadedFaces.delete(key));
        }
      })
      .catch(() => current && setFonts({ system: [], project: [] }));
    return () => {
      current = false;
    };
  }, [projectId, workspace, open, version]);
  return { fonts, refresh: () => setVersion((value) => value + 1) };
}

/** The style a project renders with: its house style plus its own fonts. */
function withFonts(style: HouseStyle, fonts: ProjectFonts | undefined) {
  return fonts && Object.keys(fonts).length
    ? { ...style, fonts: { ...style.fonts, ...fonts } }
    : style;
}

function FontChoice({
  label,
  styleFont,
  value,
  fonts,
  disabled,
  onCommit,
}: {
  label: string;
  styleFont: string;
  value: string;
  fonts: FontList | null;
  disabled: boolean;
  onCommit: (value: string) => void;
}) {
  return (
    <div className="grid grid-cols-[5.5rem_1fr_auto] items-center gap-2">
      <span className="text-muted-foreground text-xs">{label}</span>
      <FontPicker
        aria-label={`${label} font`}
        value={value}
        fonts={fonts}
        disabled={disabled}
        placeholder={`${styleFont || "Default font"} (style)`}
        onCommit={onCommit}
      />
      <Button
        size="icon-xs"
        variant="ghost-muted"
        aria-label={`Use the style's ${label.toLowerCase()} font`}
        disabled={disabled || !value}
        onClick={() => onCommit("")}
      >
        <RotateCcwIcon />
      </Button>
    </div>
  );
}

/**
 * Choose the house style the open project (or batch video) uses, override its
 * fonts with installed or uploaded ones, and preview the result.
 */
export function ProjectStylePicker({ projectId }: { projectId: string }) {
  const library = useStyleLibrary();
  const project = useProjectDoc((state) => state.project);
  const workspace = useProjectDoc((state) => state.workspace) ?? null;
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const { fonts, refresh } = useProjectFonts(projectId, workspace, open);
  const upload = useRef<HTMLInputElement>(null);
  const labelId = useId();
  const valueId = useId();
  const value = project?.style ?? "";
  const overrides = project?.fonts ?? {};
  const base = styleById(library, value || null);
  const shown = base ? withFonts(base, overrides) : undefined;

  const save = async (changes: { style?: string; fonts?: ProjectFonts }) => {
    if (!project) return;
    setBusy(true);
    try {
      await api.updateProject(projectId, changes, workspace);
      const current = useProjectDoc.getState();
      if (current.id === projectId && current.project) {
        const next = { ...current.project };
        if (changes.style !== undefined)
          next.style = changes.style || undefined;
        if (changes.fonts !== undefined)
          next.fonts = Object.keys(changes.fonts).length
            ? changes.fonts
            : undefined;
        useProjectDoc.setState({ project: next });
      }
      await useStudio.getState().loadProjects();
    } catch (error) {
      toast.error("Couldn't change the style", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const setFont = (role: keyof ProjectFonts, family: string) => {
    const next: ProjectFonts = { ...overrides };
    if (family) next[role] = family;
    else delete next[role];
    void save({ fonts: next });
  };
  const addFont = async (file: File) => {
    setBusy(true);
    try {
      await api.uploadAsset(projectId, file, workspace);
      const list = await api.fonts(projectId, workspace);
      const added = list.project.find((font) =>
        font.path.endsWith(`/${file.name}`),
      );
      refresh();
      toast.success(
        added ? `Added ${added.family}` : "Font added",
        "Choose it for headings, body text or code.",
      );
    } catch (error) {
      toast.error("Couldn't add the font", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };

  // A project without a style follows the default one, so that style's card is
  // the selected one and choosing it again keeps following the default. A
  // batch video can instead follow its project.
  const defaultId = library?.default;
  const choices: {
    value: string;
    label: string;
    style?: HouseStyle;
    selected: boolean;
    badge?: string;
  }[] = [
    ...(workspace
      ? [{ value: "", label: "Same as project", selected: !value }]
      : []),
    ...(library?.styles ?? []).map((style) => {
      const isDefault = !workspace && style.id === defaultId;
      return {
        value: isDefault ? "" : style.id,
        label: style.name,
        style,
        selected: value === style.id || (isDefault && !value),
        badge: style.id === defaultId ? "Default" : undefined,
      };
    }),
  ];
  const customised = Object.keys(overrides).length > 0;

  return (
    <div className="flex min-w-0 flex-col gap-1.5 text-muted-foreground text-xs">
      <span className="flex items-center justify-between gap-2">
        <span id={labelId}>House style</span>
        <Link
          to="/settings/styles"
          className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
        >
          Manage
        </Link>
      </span>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger
          disabled={!project}
          aria-labelledby={`${labelId} ${valueId}`}
          className="inline-flex h-9 w-full min-w-0 items-center justify-between gap-2 rounded-[var(--control-radius)] border border-input bg-background px-3 text-left text-foreground text-sm shadow-xs/5 outline-none transition-colors hover:bg-accent/50 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:opacity-50 dark:bg-input/30"
        >
          <span id={valueId} className="min-w-0 truncate">
            {value || !workspace ? (base?.name ?? value) : "Same as project"}
            {customised && (
              <span className="text-muted-foreground"> · custom fonts</span>
            )}
          </span>
          <span className="flex shrink-0 items-center gap-2">
            {shown && <StyleChips style={shown} />}
            <ChevronDownIcon className="size-4 opacity-60" />
          </span>
        </PopoverTrigger>
        <PopoverPopup
          side="top"
          align="start"
          className="max-h-[min(40rem,calc(100vh-2rem))] w-[min(30rem,calc(100vw-1rem))] overflow-y-auto p-3"
        >
          {shown && (
            <figure className="mb-3">
              <StyleSwatch style={shown} detailed />
              <figcaption className="mt-1.5 text-muted-foreground text-xs">
                How a slide in {shown.name}
                {customised ? " with your fonts" : ""} looks
              </figcaption>
            </figure>
          )}
          <div
            role="radiogroup"
            aria-label="Styles"
            className="grid grid-cols-2 gap-2"
          >
            {choices.map((choice) => (
              <button
                key={choice.style?.id ?? "inherit"}
                type="button"
                role="radio"
                aria-checked={choice.selected}
                disabled={busy}
                onClick={() =>
                  !choice.selected && void save({ style: choice.value })
                }
                className={cn(
                  "flex min-w-0 flex-col gap-1 rounded-lg border p-1.5 text-left text-foreground text-xs outline-none transition-colors hover:bg-accent/50 focus-visible:ring-[3px] focus-visible:ring-ring/30",
                  choice.selected && "border-primary ring-1 ring-primary",
                )}
              >
                {choice.style ? (
                  <StyleSwatch
                    style={withFonts(choice.style, overrides)}
                    className="rounded-md"
                  />
                ) : (
                  <span className="flex aspect-video w-full items-center justify-center rounded-md border border-dashed text-muted-foreground">
                    Same as project
                  </span>
                )}
                <span className="flex min-w-0 items-center gap-1.5 px-0.5">
                  <span className="truncate">{choice.label}</span>
                  {choice.badge && (
                    <span className="shrink-0 rounded-full border px-1.5 text-2xs text-muted-foreground">
                      {choice.badge}
                    </span>
                  )}
                </span>
              </button>
            ))}
          </div>

          <fieldset className="mt-4 flex flex-col gap-2" disabled={busy}>
            <legend className="mb-2 font-medium text-foreground text-sm">
              Fonts
            </legend>
            {ROLES.map((role) => (
              <FontChoice
                key={role.key}
                label={role.label}
                styleFont={base?.fonts[role.key] ?? ""}
                value={overrides[role.key] ?? ""}
                fonts={fonts}
                disabled={busy || !project}
                onCommit={(family) => setFont(role.key, family)}
              />
            ))}
            <div className="mt-1 flex items-center justify-between gap-2">
              <span className="min-w-0 text-muted-foreground text-xs">
                {fonts
                  ? `${fonts.bundled?.length ?? 0} Studio fonts, ${fonts.system.length} installed${fonts.project.length ? `, ${fonts.project.length} in this project` : ""}.`
                  : "Loading fonts…"}{" "}
                Empty uses the style's font.
              </span>
              <input
                ref={upload}
                type="file"
                accept=".ttf,.otf,.ttc"
                hidden
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (file) void addFont(file);
                }}
              />
              <Button
                size="xs"
                variant="outline"
                disabled={busy || !project}
                onClick={() => upload.current?.click()}
              >
                <UploadIcon />
                Add font file
              </Button>
            </div>
          </fieldset>
        </PopoverPopup>
      </Popover>
    </div>
  );
}
